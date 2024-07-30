use anyhow::{anyhow, Result, Context};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::io;
use std::io::Write;
use std::sync::Arc;
use std::sync::Mutex;
use std::thread::JoinHandle;
use std::{
    collections::HashMap,
    net::TcpStream,
    os::unix::io::AsRawFd,
    thread,
    time::Duration,
};

use crate::TransmissionType;
use crate::external::ClientMetrics;
use crate::external::MetricTypes;
use crate::logger::Logger;
use crate::parse::FlattenedArguments;
use crate::util::CONSTANT_BIT_TO_BYTE;
use crate::util::CONSTANT_US_TO_MS;
use crate::util::THREAD_SLEEP_TIME_SHORT_MS;
use crate::util::calculate_statistics;
use crate::util::DynamicValue;
use crate::util::THREAD_SLEEP_FINISH_MS;
use crate::util::THREAD_SLEEP_TIME_SHORT_US;
use crate::util::get_active_cca;
use crate::util::sockopt_get_tcp_info;
use crate::util::sockopt_patch_cwnd;
use crate::util::sockopt_prepare_transmission;


/// Array of 8192 bytes filled with 0x01
const DUMMY_DATA_SIZE: usize = 8192;
//const DUMMY_DATA: &[u8] = &[0x01; 8192];

/* PATCH CONSTATNS */
pub const TCP_SET_DIRECT_CWND: u32 = 60;
pub const TCP_SET_INIT_CWND: u32 = 61;
pub const TCP_SET_UPPER_CWND: u32 = 62;

#[derive(Debug, Serialize, Deserialize)]
pub struct Transmission {
    pub client_ip: String,
    pub transmission_type: TransmissionType,
    pub transmission_duration_ms: u64,
    pub start_timestamp_us: u64,
    pub end_timestamp_us: u64,
    pub min_rtt_us: u64,
    pub rtt_mean: Option<u32>,
    pub cwnd_mean: Option<u32>,
    pub rtt_median: Option<u32>,
    pub cwnd_median: Option<u32>,
    pub total_packet_loss: u32,
    pub timedata: HashMap<u64, TcpStatsLog>,
}

impl fmt::Display for Transmission {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(
            f,
            "\tClient IP:         {}\n\
             \tTransmission Type: {:?}\n\
             \tDuration (ms):     {}\n\
             \tRTT Mean:          {:?}\n\
             \tCWND Mean:         {:?}\n\
             \tRTT Median:        {:?}\n\
             \tCWND Median:       {:?}\n\
             \tPacket loss:       {:?}\n\
             \tTimedata Size:     {}",
            self.client_ip,
            self.transmission_type,
            self.transmission_duration_ms,
            self.rtt_mean,
            self.cwnd_mean,
            self.rtt_median,
            self.cwnd_median,
            self.total_packet_loss,
            self.timedata.len(),
        )
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TcpStatsLog {
    pub rtt: u32,
    pub cwnd: u32,
    pub packet_loss: u32,
    pub set_initital_cwnd: Option<u32>,
    pub set_upper_cwnd: Option<u32>,
    pub set_direct_cwnd: Option<u32>,
}

#[derive(Debug)]
pub struct ClientArgs {
    pub args: FlattenedArguments,
    pub stream: TcpStream,
    pub logger: Arc<Mutex<Logger>>,
    pub client_metrics: Option<Arc<Mutex<ClientMetrics>>>,
    pub transmission_type: TransmissionType,
    pub transmission_duration_ms: u64,
    pub set_initial_cwnd: Option<DynamicValue<u32>>,
    pub set_upper_bound_cwnd: Option<DynamicValue<u32>>,
    pub set_direct_cwnd: Option<DynamicValue<u32>>,
    pub path: String,
}

fn init_dummy_data() -> Box<[u8]> {
    let mut vec: Vec<u8> = Vec::<u8>::with_capacity(DUMMY_DATA_SIZE);
    /* Fill the vector with zeros */
    vec.resize_with(DUMMY_DATA_SIZE, || 0x01);
    vec.into_boxed_slice()
}

pub fn rate_to_cwnd(socket_file_descriptor: i32, rate_bit_per_ms: u64) -> Result<u32> {
    let tcp_info = sockopt_get_tcp_info(socket_file_descriptor)?;
    let rtt_us: f64 = tcp_info.tcpi_rtt as f64;
    let mss: u64 = tcp_info.tcpi_snd_mss as u64;

    let rtt_ms = (rtt_us / CONSTANT_US_TO_MS as f64).ceil() as u64;

    // Ensure RTT is non-zero to avoid division by zero errors
    if rtt_ms == 0 {
        return Err(anyhow!("rate_to_cwnd error: RTT is zero, invalid TCP info"));
    }

    // Calculate BDP in bits
    let bdp_bits = rtt_ms.checked_mul(rate_bit_per_ms)
        .ok_or_else(|| anyhow!("rate_to_cwnd error: BDP calculation overflow"))?;
    let bdp_bytes: u64 = bdp_bits / CONSTANT_BIT_TO_BYTE;


    let cwnd: u32 = bdp_bytes.div_ceil(mss) as u32;

    Ok(cwnd)
}

fn check_uses_external(
    external_interface: &bool,
    set_initial_cwnd: &Option<DynamicValue<u32>>,
    set_direct_cwnd: &Option<DynamicValue<u32>>,
    set_upper_bound_cwnd: &Option<DynamicValue<u32>>,
    client_metrics: &Option<Arc<Mutex<ClientMetrics>>>,
) -> Result<Option<()>> {
    if *external_interface
        && (set_initial_cwnd
            .as_ref()
            .map_or(false, |v| matches!(v, DynamicValue::Dynamic))
            || set_upper_bound_cwnd
                .as_ref()
                .map_or(false, |v| matches!(v, DynamicValue::Dynamic))
            || set_direct_cwnd
                .as_ref()
                .map_or(false, |v| matches!(v, DynamicValue::Dynamic)))
    {
        if client_metrics.is_none() {
            return Err(anyhow!(
                "handle_client: has a DynamicValue field but client_metrics is None!"
            ));
        }
        return Ok(Some(()));
    }
    Ok(None)
}

fn wait_until_client_metric(client_metrics: &Arc<Mutex<ClientMetrics>>, client_addr: &str) {
    loop {
        if client_metrics
            .lock()
            .unwrap()
            .clients
            .contains_key(client_addr)
        {
            break;
        }
        thread::sleep(Duration::from_millis(THREAD_SLEEP_FINISH_MS));
    }
}

fn unpack_latest_rate(
    client_metrics: &Arc<Mutex<ClientMetrics>>,
    client_addr: &str,
) -> Result<MetricTypes> {
    // Lock the mutex
    let metrics = client_metrics
        .lock()
        .map_err(|e| anyhow::anyhow!("Failed to lock client metrics: {}", e))?;
    
    // Retrieve the client's metrics
    let client_metric = *metrics.clients.get(client_addr)
        .with_context(|| format!("Client address not found: {}", client_addr))?;
    
    // Return the cloned metric
    Ok(client_metric)
}

fn patch_initial_cwnd(
    initial_cwnd_dyn: DynamicValue<u32>,
    socket_file_descriptor: i32,
    client_metrics: &Option<Arc<Mutex<ClientMetrics>>>,
    client_addr: &str,
) -> Result<Option<u32>> {
    let initial_cwnd: u32 = match initial_cwnd_dyn {
        DynamicValue::Dynamic => {
            let latest_metric: MetricTypes =
                unpack_latest_rate(&client_metrics.clone().unwrap(), client_addr)?;
            let rate: u64 = latest_metric.get_rate();
            rate_to_cwnd(socket_file_descriptor, rate)?
        }
        DynamicValue::Fixed(fixed_cwnd) => fixed_cwnd,
    };
    sockopt_patch_cwnd(socket_file_descriptor, initial_cwnd, TCP_SET_INIT_CWND)?;
    Ok(Some(initial_cwnd))
}

fn patch_upper_cwnd(
    upper_cwnd_dyn: DynamicValue<u32>,
    socket_file_descriptor: i32,
    client_metrics: &Option<Arc<Mutex<ClientMetrics>>>,
    client_addr: &str,
) -> Result<Option<u32>> {
    let upper_cwnd: u32 = match upper_cwnd_dyn {
        DynamicValue::Dynamic => {
            let latest_metric: MetricTypes =
                unpack_latest_rate(&client_metrics.clone().unwrap(), client_addr)?;
            let rate: u64 = latest_metric.get_rate();
            rate_to_cwnd(socket_file_descriptor, rate)?
        }
        DynamicValue::Fixed(fixed_cwnd) => fixed_cwnd,
    };
    sockopt_patch_cwnd(socket_file_descriptor, upper_cwnd, TCP_SET_UPPER_CWND)?;
    Ok(Some(upper_cwnd))
}

fn patch_direct_cwnd(
    direct_cwnd_dyn: DynamicValue<u32>,
    socket_file_descriptor: i32,
    client_metrics: &Option<Arc<Mutex<ClientMetrics>>>,
    client_addr: &str,
) -> Result<Option<u32>> {
    let direct_cwnd: u32 = match direct_cwnd_dyn {
        DynamicValue::Dynamic => {
            let latest_metric: MetricTypes =
                unpack_latest_rate(&client_metrics.clone().unwrap(), client_addr)?;
            let rate: u64 = latest_metric.get_rate();
            rate_to_cwnd(socket_file_descriptor, rate)?
        }
        DynamicValue::Fixed(fixed_cwnd) => fixed_cwnd,
    };
    sockopt_patch_cwnd(socket_file_descriptor, direct_cwnd, TCP_SET_DIRECT_CWND)?;
    Ok(Some(direct_cwnd))
}

fn patch_upper_cwnd_if_new_metric(
    upper_cwnd_dyn: DynamicValue<u32>,
    socket_file_descriptor: i32,
    client_metrics: &Option<Arc<Mutex<ClientMetrics>>>,
    client_addr: &str,
    last_metric_timestamp_us: &mut u64,
) -> Result<Option<u32>> {
     let upper_cwnd: u32 = match upper_cwnd_dyn {
        DynamicValue::Dynamic => {
            let latest_metric: MetricTypes =
                unpack_latest_rate(&client_metrics.clone().unwrap(), client_addr)?;
            if latest_metric.get_timestamp_us() > *last_metric_timestamp_us {
                *last_metric_timestamp_us = latest_metric.get_timestamp_us();
                let rate: u64 = latest_metric.get_rate();
                rate_to_cwnd(socket_file_descriptor, rate)?
            } else {
                return Ok(None)
            }
        }
        DynamicValue::Fixed(fixed_cwnd) => fixed_cwnd,
    };
    sockopt_patch_cwnd(socket_file_descriptor, upper_cwnd, TCP_SET_UPPER_CWND)?;
    Ok(Some(upper_cwnd))
   
}

fn patch_direct_cwnd_if_new_metric(
    direct_cwnd_dyn: DynamicValue<u32>,
    socket_file_descriptor: i32,
    client_metrics: &Option<Arc<Mutex<ClientMetrics>>>,
    client_addr: &str,
    last_metric_timestamp_us: &mut u64,
) -> Result<Option<u32>> {
    let direct_cwnd: u32 = match direct_cwnd_dyn {
        DynamicValue::Dynamic => {
            let latest_metric: MetricTypes =
                unpack_latest_rate(&client_metrics.clone().unwrap(), client_addr)?;
            if latest_metric.get_timestamp_us() > *last_metric_timestamp_us {
                *last_metric_timestamp_us = latest_metric.get_timestamp_us();
            let rate: u64 = latest_metric.get_rate();
            rate_to_cwnd(socket_file_descriptor, rate)?
            } else {
                return Ok(None)
            }
        }
        DynamicValue::Fixed(fixed_cwnd) => fixed_cwnd,
    };
    sockopt_patch_cwnd(socket_file_descriptor, direct_cwnd, TCP_SET_DIRECT_CWND)?;
    Ok(Some(direct_cwnd))
}

pub fn handle_client(mut client_args: ClientArgs) -> Result<()> {
    // TODO: Decouple tcp_info logging and cwnd setting to an additional thread
    let stream: TcpStream = client_args.stream;
    let client_metrics = client_args.client_metrics;
    let is_external = client_args.args.external_interface;
    let transmission_type = client_args.transmission_type;
    let set_upper_bound_cwnd = client_args.set_upper_bound_cwnd;
    let set_initial_cwnd = client_args.set_initial_cwnd;
    let set_direct_cwnd = client_args.set_direct_cwnd;
    let logging_interval_us = client_args.args.logging_interval_us;
    let transmission_duration_ms = client_args.transmission_duration_ms;
    let logger: &mut Arc<Mutex<Logger>> = &mut client_args.logger;
    let socket_file_descriptor: i32 = stream.as_raw_fd();

    logger.lock().unwrap().log_stdout(&format!("[client] new client ({})", chrono::Local::now().format("%Y-%m-%d %H:%M:%S")))?;
    logger.lock().unwrap().log_stdout(&format!("  client_args.transmission_type: {:?}", transmission_type))?;
    logger.lock().unwrap().log_stdout(&format!("  client_args.transmission_duration_ms: {:?}", transmission_duration_ms))?;
    logger.lock().unwrap().log_stdout(&format!("  client_args.set_initial_cwnd: {:?}", set_initial_cwnd))?;
    logger.lock().unwrap().log_stdout(&format!("  client_args.set_upper_bound_cwnd: {:?}", set_upper_bound_cwnd))?;
    logger.lock().unwrap().log_stdout(&format!("  client_args.set_direct_cwnd: {:?}", set_direct_cwnd))?;

    let mut timedata = HashMap::new();
    let client_addr: String = stream.peer_addr()?.ip().to_string();

    println!("  stream.fd:    \t{:?}", socket_file_descriptor);
    println!("  stream.cwnd:  \t{:?}", sockopt_get_tcp_info(socket_file_descriptor)?.tcpi_snd_cwnd);
    println!("  stream.rtt_us:\t{:?}", sockopt_get_tcp_info(socket_file_descriptor)?.tcpi_rtt);
    if let Ok(latest_metric) = unpack_latest_rate(&client_metrics.clone().unwrap(), &client_addr) {
        println!("  fair_share_send_rate: {:?}", latest_metric.get_rate());
    }

    sockopt_prepare_transmission(socket_file_descriptor, &transmission_type)?;
    println!("  active CCA:\t{}", get_active_cca(socket_file_descriptor)?);
    println!();

    let uses_external: bool = check_uses_external(
        &is_external,
        &set_initial_cwnd,
        &set_direct_cwnd,
        &set_upper_bound_cwnd,
        &client_metrics,
    )?
    .is_some();

    if uses_external {
        wait_until_client_metric(&client_metrics.clone().unwrap(), &client_addr);
    }

    let initial_cwnd_option = if let Some(initial_cwnd_dyn) = set_initial_cwnd.clone() {
        patch_initial_cwnd(initial_cwnd_dyn, socket_file_descriptor, &client_metrics, &client_addr)?
    } else {
        None
    };
    let upper_cwnd_option = if let Some(upper_cwnd_dyn) = set_upper_bound_cwnd.clone() {
        patch_upper_cwnd(upper_cwnd_dyn, socket_file_descriptor, &client_metrics, &client_addr)?
    } else {
        None
    };
    let direct_cwnd_option = if let Some(direct_cwnd_dyn) = set_direct_cwnd.clone() {
        patch_direct_cwnd(direct_cwnd_dyn, socket_file_descriptor, &client_metrics, &client_addr)?
    } else {
        None
    };
    append_tcp_info_to_stats_log(socket_file_descriptor,
                                 &mut timedata,
                                 initial_cwnd_option,
                                 upper_cwnd_option,
                                 direct_cwnd_option)?;


    let min_rtt_us: u64 = sockopt_get_tcp_info(socket_file_descriptor)?.tcpi_rtt as u64;

    let start_timestamp_us = chrono::Local::now().timestamp_micros() as u64;
    let end_timestamp_us = start_timestamp_us + (transmission_duration_ms * 1000);
    let mut last_logging_timestamp_us = 0;
    let mut last_metric_timestamp_us = 0;
    let mut last_set_cwnd_timestamp_us = 0;

    let mut joined_stream: TcpStream;
    let join_handle_stream: JoinHandle<Result<TcpStream>> =
        deploy_sending_thread(stream, end_timestamp_us);

    loop {
        if join_handle_stream.is_finished() {
            joined_stream = match join_handle_stream.join() {
                Ok(result) => result?,
                Err(e) => return Err(anyhow!("[client] error joining finished TcpStream: {:?}", e)),
            };
            break;
        }

        let now_us = chrono::Local::now().timestamp_micros() as u64;

        if now_us - last_logging_timestamp_us >= logging_interval_us {
            append_tcp_info_to_stats_log(socket_file_descriptor,
                                         &mut timedata,
                                         None,
                                         None,
                                         None)?;
            last_logging_timestamp_us = now_us;
        }
        if transmission_type.is_l2b() && now_us - last_set_cwnd_timestamp_us >= min_rtt_us {
            last_set_cwnd_timestamp_us = chrono::Local::now().timestamp_micros() as u64;
            if let Some(upper_cwnd_dyn) = set_upper_bound_cwnd.clone()  {
                let upper_cwnd_option = patch_upper_cwnd_if_new_metric(upper_cwnd_dyn,
                                               socket_file_descriptor,
                                               &client_metrics,
                                               &client_addr,
                                               &mut last_metric_timestamp_us)?;

                append_tcp_info_to_stats_log(socket_file_descriptor,
                                             &mut timedata,
                                             None,
                                             upper_cwnd_option,
                                             None)?;
            }
            if let Some(direct_cwnd_dyn) = set_direct_cwnd.clone() {
                let direct_cwnd_option = patch_direct_cwnd_if_new_metric(direct_cwnd_dyn,
                                                socket_file_descriptor,
                                                &client_metrics,
                                                &client_addr,
                                                &mut last_metric_timestamp_us)?;

                append_tcp_info_to_stats_log(socket_file_descriptor,
                                             &mut timedata,
                                             None,
                                             None,
                                             direct_cwnd_option)?;
            }
        }
        thread::sleep(Duration::from_micros(THREAD_SLEEP_TIME_SHORT_US));
    }

    finish_transmission(&mut joined_stream)?;

    let (rtt_mean, cwnd_mean, rtt_median, cwnd_median) = calculate_statistics(&timedata);
    let total_packet_loss = if let Some((_, info)) = timedata.iter().max_by_key(|(&key, _)| key) {
        info.packet_loss
    } else {
        0
    };
    let transmission = Transmission {
        client_ip: client_addr.clone(),
        transmission_type: transmission_type.clone(),
        min_rtt_us,
        start_timestamp_us,
        end_timestamp_us,
        transmission_duration_ms,
        rtt_mean,
        cwnd_mean,
        rtt_median,
        cwnd_median,
        total_packet_loss,
        timedata,
    };

    logger.lock().unwrap().log_stdout(&format!("[client] tranmission finished ({}):\n{}",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),  &transmission))?;
    let json_str = serde_json::to_string(&transmission)?;
    logger.lock().unwrap().log_transmission(&json_str, client_args.path)?;

    Ok(())
}

fn deploy_sending_thread(
    stream: TcpStream,
    finish_timestamp_us: u64,
) -> JoinHandle<Result<TcpStream>> {
    let handle: thread::JoinHandle<Result<TcpStream>> = thread::spawn(move || {
        run_sending_thread(stream, finish_timestamp_us)
    });
    handle
}


fn run_sending_thread(
    mut stream: TcpStream,
    finish_timestamp_us: u64,
) -> Result<TcpStream> {
    let mut dummy_data: Box<[u8]> = init_dummy_data();
    let socket_file_descriptor: i32 = stream.as_raw_fd();

    stream.set_nonblocking(true)?;

    let mut now: u64;
    loop {
        now = chrono::Local::now().timestamp_micros() as u64;
        if now >= finish_timestamp_us {
            break;
        }
        encode_rtt_in_payload(socket_file_descriptor, &mut dummy_data);
        match stream.write(&dummy_data) {
            Ok(nof_bytes_written) => {
                if nof_bytes_written <= (dummy_data.len() / 10) {
                    thread::sleep(Duration::from_micros(THREAD_SLEEP_TIME_SHORT_MS));
                }
            }
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                // Sending buffer is full, wait for a while before retrying
                thread::sleep(Duration::from_micros(THREAD_SLEEP_TIME_SHORT_MS));
            }
            Err(e) => return Err(e.into()), // Other I/O errors
        }

        thread::sleep(Duration::from_micros(THREAD_SLEEP_TIME_SHORT_US));
    }

    // Restore the blocking mode
    stream.set_nonblocking(false)?;

    Ok(stream)
}

fn encode_rtt_in_payload(socket_file_descriptor: i32, payload: &mut Box<[u8]>) {
    if let Ok(tcp_info) = sockopt_get_tcp_info(socket_file_descriptor) {
        let rtt_us: u32 = tcp_info.tcpi_rtt;
        let encoded_rtt = encode_rtt_to_byte_array(rtt_us);
        repeat_slice_in_array(payload, encoded_rtt);
    }
}

fn encode_rtt_to_byte_array(rtt_us: u32) -> [u8; 10] {
    let mut rtt_array = [0u8; 10];

    rtt_array[0] = 0xAA;
    rtt_array[1] = 0xAB;
    rtt_array[2] = 0xAC;

    // Encode u32 into 4 bytes
    rtt_array[3] = (rtt_us >> 24) as u8;
    rtt_array[4] = (rtt_us >> 16) as u8;
    rtt_array[5] = (rtt_us >> 8) as u8;
    rtt_array[6] = rtt_us as u8;

    rtt_array[7] = 0xBA;
    rtt_array[8] = 0xBB;
    rtt_array[9] = 0xBC;

    rtt_array
}

fn repeat_slice_in_array(dst: &mut Box<[u8]>, src: [u8; 10]) {
    let src_len = src.len();
    let dst_len = dst.len();
    let mut i = 0;

    while i + src_len <= dst_len {
        dst[i..i + src_len].copy_from_slice(&src);
        i += src_len;
    }
}

fn finish_transmission(stream: &mut TcpStream) -> Result<()> {
    stream.shutdown(std::net::Shutdown::Write)?;
    thread::sleep(Duration::from_millis(THREAD_SLEEP_FINISH_MS));
    Ok(())
}

/*
fn run_sending_thread() -> Result<()> {

    while (start_time.elapsed()?.as_millis() as u64) < transmission_duration_ms {
        let now_us = chrono::Local::now().timestamp_micros() as u64;
        if now_us - last_logging_timestamp_us >= logging_interval_us {
            append_tcp_info_to_stats_log(&stream, &mut timedata)?;
            last_logging_timestamp_us = now_us;
        }
        if let Some(upper_cwnd_dyn) = set_upper_bound_cwnd.clone() {
            patch_upper_cwnd(upper_cwnd_dyn, &stream, &client_metrics, &client_addr)?;
        }

        if let Some(direct_cwnd_dyn) = set_direct_cwnd.clone() {
            patch_direct_cwnd(direct_cwnd_dyn, &stream, &client_metrics, &client_addr)?;
        }

        stream.write_all(&dummy_data)?;
        thread::sleep(Duration::from_micros(THREAD_SLEEP_TIME_SHORT_US));
    }
    stream.write_all(b"Transmission complete. Thank you!\n")?;
    thread::sleep(Duration::from_millis(THREAD_SLEEP_FINISH_MS));
    stream.flush()?; // Flush again to ensure the completion message is sent before
}
*/

fn append_tcp_info_to_stats_log(
    socket_file_descriptor: i32,
    timedata: &mut HashMap<u64, TcpStatsLog>,
    initial_cwnd_option: Option<u32>,
    upper_cwnd_option: Option<u32>,
    direct_cwnd_option: Option<u32>,
) -> Result<()> {
    let latest_tcp_info = sockopt_get_tcp_info(socket_file_descriptor)?;

    let timestamp_us = chrono::Local::now().timestamp_micros() as u64;
    let rtt = latest_tcp_info.tcpi_rtt;
    let cwnd = latest_tcp_info.tcpi_snd_cwnd;
    let packet_loss = latest_tcp_info.tcpi_lost;

    timedata.insert(timestamp_us, TcpStatsLog {
        rtt,
        cwnd,
        packet_loss,
        set_initital_cwnd: initial_cwnd_option,
        set_upper_cwnd: upper_cwnd_option,
        set_direct_cwnd: direct_cwnd_option,
    });
    Ok(())
}

