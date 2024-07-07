use crate::external::ClientMetrics;
use crate::external::MetricTypes;
use crate::logger::Logger;
use crate::parse::FlattenedArguments;
use crate::util::CONSTANT_BIT_TO_BYTE;
use crate::util::CONSTANT_US_TO_MS;
use crate::util::calculate_statistics;
use crate::util::DynamicValue;
use crate::util::StockTcpInfo;
use crate::util::THREAD_SLEEP_FINISH_MS;
use crate::util::THREAD_SLEEP_TIME_SHORT_US;
use anyhow::{anyhow, Result, Context};
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::mem;
use std::sync::Arc;
use std::sync::Mutex;
use std::{
    collections::HashMap,
    net::TcpStream,
    os::unix::io::AsRawFd,
    thread,
    time::{Duration, SystemTime},
};

use libc::{c_void, getsockopt, setsockopt, socklen_t, TCP_INFO};

/// Array of 8192 bytes filled with 0x01
const DUMMY_DATA_SIZE: usize = 32768;// 16384; // 8192
//const DUMMY_DATA: &[u8] = &[0x01; 8192];

/* PATCH CONSTATNS */
pub const TCP_SET_DIRECT_CWND: u32 = 60;
pub const TCP_SET_INIT_CWND: u32 = 61;
pub const TCP_SET_UPPER_CWND: u32 = 62;

#[derive(Debug, Serialize, Deserialize)]
pub struct Transmission {
    pub client_ip: String,
    pub rtt_mean: Option<u32>,
    pub cwnd_mean: Option<u32>,
    pub rtt_median: Option<u32>,
    pub cwnd_median: Option<u32>,
    pub timedata: HashMap<u64, TcpStatsLog>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TcpStatsLog {
    pub rtt: u32,
    pub cwnd: u32,
    pub ack_sent: u64,
}

#[derive(Debug)]
pub struct ClientArgs {
    pub args: FlattenedArguments,
    pub stream: TcpStream,
    pub logger: Arc<Mutex<Logger>>,
    pub client_metrics: Option<Arc<Mutex<ClientMetrics>>>,
    pub set_initial_cwnd: Option<DynamicValue<u32>>,
    pub set_upper_bound_cwnd: Option<DynamicValue<u32>>,
    pub set_direct_cwnd: Option<DynamicValue<u32>>,
}

fn init_dummy_data() -> Box<[u8]> {
    let mut vec: Vec<u8> = Vec::<u8>::with_capacity(DUMMY_DATA_SIZE);
    /* Fill the vector with zeros */
    vec.resize_with(DUMMY_DATA_SIZE, || 0x01);
    vec.into_boxed_slice()
}

pub fn rate_to_cwnd(stream: &TcpStream, rate_bit_per_ms: u64) -> Result<u32> {
    let tcp_info = sockopt_get_tcp_info(stream)?;
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
    stream: &TcpStream,
    client_metrics: &Option<Arc<Mutex<ClientMetrics>>>,
    client_addr: &str,
) -> Result<()> {
    println!("DEBUG [client] patch_initial_cwnd");
    let initial_cwnd: u32 = match initial_cwnd_dyn {
        DynamicValue::Dynamic => {
            let latest_metric: MetricTypes =
                unpack_latest_rate(&client_metrics.clone().unwrap(), client_addr)?;
            println!("  latest_metric.get_timestamp_us(): {:?}", latest_metric.get_timestamp_us());
            let rate: u64 = latest_metric.get_rate();
            rate_to_cwnd(stream, rate)?
        }
        DynamicValue::Fixed(fixed_cwnd) => fixed_cwnd,
    };
    println!("  patching initial_cwnd: {:?}", initial_cwnd);
    sockopt_patch_cwnd(stream, initial_cwnd, TCP_SET_INIT_CWND)?;
    println!("  after patching cwnd: {:?}", sockopt_get_tcp_info(stream)?.tcpi_snd_cwnd);
    Ok(())
}

fn patch_upper_cwnd(
    upper_cwnd_dyn: DynamicValue<u32>,
    stream: &TcpStream,
    client_metrics: &Option<Arc<Mutex<ClientMetrics>>>,
    client_addr: &str,
) -> Result<()> {
    println!("DEBUG [client] patch_upper_cwnd");
    let upper_cwnd: u32 = match upper_cwnd_dyn {
        DynamicValue::Dynamic => {
            let latest_metric: MetricTypes =
                unpack_latest_rate(&client_metrics.clone().unwrap(), client_addr)?;
            let rate: u64 = latest_metric.get_rate();
            rate_to_cwnd(stream, rate)?
        }
        DynamicValue::Fixed(fixed_cwnd) => fixed_cwnd,
    };
    println!("  to set cwnd: \t{:?}", upper_cwnd);
    sockopt_patch_cwnd(stream, upper_cwnd, TCP_SET_UPPER_CWND)?;
    println!("  tcp_info.cwnd: \t{:?}", sockopt_get_tcp_info(stream)?.tcpi_snd_cwnd);
    Ok(())
}

fn patch_direct_cwnd(
    direct_cwnd_dyn: DynamicValue<u32>,
    stream: &TcpStream,
    client_metrics: &Option<Arc<Mutex<ClientMetrics>>>,
    client_addr: &str,
) -> Result<()> {
    let direct_cwnd: u32 = match direct_cwnd_dyn {
        DynamicValue::Dynamic => {
            let latest_metric: MetricTypes =
                unpack_latest_rate(&client_metrics.clone().unwrap(), client_addr)?;
            let rate: u64 = latest_metric.get_rate();
            rate_to_cwnd(stream, rate)?
        }
        DynamicValue::Fixed(fixed_cwnd) => fixed_cwnd,
    };
    sockopt_patch_cwnd(stream, direct_cwnd, TCP_SET_DIRECT_CWND)?;
    Ok(())
}

pub fn handle_client(mut client_args: ClientArgs) -> Result<()> {
    println!("[client] new client:");
    println!("  client_args.set_initial_cwnd: {:?}", client_args.set_initial_cwnd);
    println!("  client_args.set_upper_bound_cwnd: {:?}", client_args.set_upper_bound_cwnd);
    println!("  client_args.set_direct_cwnd: {:?}", client_args.set_direct_cwnd);

    // TODO: Decouple tcp_info logging and cwnd setting to an additional thread
    let mut stream: TcpStream = client_args.stream;
    let client_metrics = client_args.client_metrics;
    let is_external = client_args.args.external_interface;
    let set_upper_bound_cwnd = client_args.set_upper_bound_cwnd;
    let set_initial_cwnd = client_args.set_initial_cwnd;
    let set_direct_cwnd = client_args.set_direct_cwnd;
    let logging_interval_us = client_args.args.logging_interval_us;
    let transmission_duration_ms = client_args.args.transmission_duration_ms;
    let logger: &mut Arc<Mutex<Logger>> = &mut client_args.logger;

    let dummy_data: Box<[u8]> = init_dummy_data();
    let mut timedata = HashMap::new();
    let client_addr: String = stream.peer_addr()?.ip().to_string();

    println!("  stream.fd: {:?}", &stream.as_raw_fd());
    println!("  stream.cwnd: {:?}", sockopt_get_tcp_info(&stream)?.tcpi_snd_cwnd);
    println!("  stream.rtt_us: {:?}", sockopt_get_tcp_info(&stream)?.tcpi_rtt);
    if let Ok(latest_metric) = unpack_latest_rate(&client_metrics.clone().unwrap(), &client_addr) {
        println!("  fair_share_send_rate: {:?}", latest_metric.get_rate());
    }

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

    if let Some(upper_cwnd_dyn) = set_upper_bound_cwnd.clone() {
        patch_upper_cwnd(upper_cwnd_dyn, &stream, &client_metrics, &client_addr)?;
    }

    if let Some(initial_cwnd_dyn) = set_initial_cwnd.clone() {
        patch_initial_cwnd(initial_cwnd_dyn, &stream, &client_metrics, &client_addr)?;
    }

    let start_time = SystemTime::now();
    let mut last_logging_timestamp_us =
        chrono::Utc::now().timestamp_micros() as u64 - logging_interval_us;

    while (start_time.elapsed()?.as_millis() as u64) < transmission_duration_ms {
        let now_us = chrono::Utc::now().timestamp_micros() as u64;
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
                     // closing the socket
    thread::sleep(Duration::from_millis(THREAD_SLEEP_FINISH_MS));
    stream.shutdown(std::net::Shutdown::Write)?;

    let (rtt_mean, cwnd_mean, rtt_median, cwnd_median) = calculate_statistics(&timedata);
    let transmission = Transmission {
        client_ip: client_addr.clone(),
        rtt_mean,
        cwnd_mean,
        rtt_median,
        cwnd_median,
        timedata,
    };
    let json_str = serde_json::to_string(&transmission)?;
    logger.lock().unwrap().log(&json_str)?;

    Ok(())
}

fn sockopt_patch_cwnd(stream: &TcpStream, upper_cwnd: u32, patch_type: u32) -> Result<()> {
    let fd = stream.as_raw_fd();

    // Prepare the buffer for TCP_INFO
    let cwnd_typed: *const c_void = &upper_cwnd as *const _ as *const c_void;
    let size_of_cwnd = mem::size_of::<u32>() as libc::socklen_t;

    let ret = unsafe {
        setsockopt(
            fd,
            libc::SOL_TCP,
            patch_type as libc::c_int,
            cwnd_typed,
            size_of_cwnd,
        )
    };

    if ret != 0 {
        // Capture errno
        let errno_val = unsafe { *libc::__errno_location() };
        let error_message = unsafe { std::ffi::CStr::from_ptr(libc::strerror(errno_val)) }
            .to_string_lossy()
            .into_owned();
        return Err(anyhow!(
            "An error occurred running libc::setsockopt: {}",
            error_message
        ));
    }

    Ok(())
}

fn sockopt_get_tcp_info(stream: &TcpStream) -> Result<StockTcpInfo> {
    let fd = stream.as_raw_fd();

    let mut tcp_info: StockTcpInfo = StockTcpInfo::default();
    let mut tcp_info_len = mem::size_of::<StockTcpInfo>() as socklen_t;

    let ret = unsafe {
        getsockopt(
            fd,
            libc::IPPROTO_TCP,
            TCP_INFO,
            &mut tcp_info as *mut _ as *mut c_void,
            &mut tcp_info_len,
        )
    };

    // Check if getsockopt was successful
    if ret != 0 {
        return Err(anyhow!("An error occured running libc::getsockopt"));
    }
    Ok(tcp_info)
}

fn append_tcp_info_to_stats_log(
    stream: &TcpStream,
    timedata: &mut HashMap<u64, TcpStatsLog>,
) -> Result<()> {
    let tcp_info = sockopt_get_tcp_info(stream)?;

    let timestamp_us = chrono::Utc::now().timestamp_micros() as u64;
    let rtt = tcp_info.tcpi_rtt;
    let cwnd = tcp_info.tcpi_snd_cwnd;
    let ack_sent = tcp_info.tcpi_last_ack_sent as u64;

    timedata.insert(timestamp_us, TcpStatsLog { rtt, cwnd, ack_sent });
    Ok(())
}
