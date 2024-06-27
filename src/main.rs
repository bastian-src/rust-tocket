use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    net::{TcpListener, TcpStream},
    os::unix::io::AsRawFd,
    thread,
    time::{Duration, SystemTime},
};

use std::sync::{Arc, Mutex};
use std::mem;

mod parse;

use libc::{c_void, getsockopt, socklen_t, TCP_INFO, setsockopt};
use parse::Arguments;
use clap::Parser;
use sysctl::Sysctl;




/* PATCH CONSTATNS */
pub const TCP_SET_CWND: u32 = 50;
pub const TCP_SET_INIT_CWND: u32 = 51;


/// Default server addres
const SERVER_ADDR: &str = "0.0.0.0:9393";
/// Array of 8192 bytes filled with 0x01
const DUMMY_DATA: &[u8] = &[0x01; 8192];
const LOGS_DIRECTORY: &str = ".logs";

const THREAD_SLEEP_TIME_US: u64 = 500;
const THREAD_SLEEP_FINISH_MS: u64 = 500;


#[repr(C)]
#[derive(Debug, Default)]
struct StockTcpInfo {
    tcpi_state: u8,
    tcpi_ca_state: u8,
    tcpi_retransmits: u8,
    tcpi_probes: u8,
    tcpi_backoff: u8,
    tcpi_options: u8,
    tcpi_snd_wscale: u8,
    tcpi_rcv_wscale: u8,

    tcpi_rto: u32,
    tcpi_ato: u32,
    tcpi_snd_mss: u32,
    tcpi_rcv_mss: u32,

    tcpi_unacked: u32,
    tcpi_sacked: u32,
    tcpi_lost: u32,
    tcpi_retrans: u32,
    tcpi_fackets: u32,

    // Times
    tcpi_last_data_sent: u32,
    tcpi_last_ack_sent: u32,
    tcpi_last_data_recv: u32,
    tcpi_last_ack_recv: u32,

    // Metrics
    tcpi_pmtu: u32,
    tcpi_rcv_ssthresh: u32,
    tcpi_rtt: u32,
    tcpi_rttvar: u32,
    tcpi_snd_ssthresh: u32,
    tcpi_snd_cwnd: u32,
    tcpi_advmss: u32,
    tcpi_reordering: u32,

    tcpi_rcv_rtt: u32,
    tcpi_rcv_space: u32,

    tcpi_total_retrans: u32,
}

#[derive(Debug, Serialize, Deserialize)]
struct Transmission {
    client_ip: String,
    rtt_mean: Option<u32>,
    cwnd_mean: Option<u32>,
    rtt_median: Option<u32>,
    cwnd_median: Option<u32>,
    timedata: HashMap<u64, TcpStatsLog>,
}

#[derive(Debug, Serialize, Deserialize)]
struct TcpStatsLog {
    rtt: u32,
    cwnd: u32,
}

struct Logger {
    file: File,
}

impl Logger {
    fn new() -> Result<Self, io::Error> {
        fs::create_dir_all(LOGS_DIRECTORY)?;

        let filename = format!(
            "{}/run_{}.jsonl",
            LOGS_DIRECTORY,
            chrono::Local::now().format("%Y-%m-%d_%H-%M")
        );

        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(filename)?;

        Ok(Self { file })
    }

    fn log(&mut self, json_str: &str) -> Result<(), io::Error> {
        writeln!(self.file, "{}", json_str)?;
        self.file.flush()?;
        Ok(())
    }
}

fn sockopt_patch_set_cwnd(stream: &TcpStream, cwnd: u32) -> Result<()> {
    let fd = stream.as_raw_fd();

    // Prepare the buffer for TCP_INFO
    let cwnd_typed: *const c_void = &cwnd as *const _ as *const c_void;
    let size_of_cwnd = mem::size_of::<u32>() as libc::socklen_t;

    /* This one works fine! */
    // let ret = unsafe {
    //     setsockopt(
    //         fd,
    //         libc::SOL_TCP,
    //         libc::TCP_NODELAY, // Change to a known option
    //         &1 as *const _ as *const c_void, // Setting TCP_NODELAY to 1
    //         mem::size_of::<libc::c_int>() as libc::socklen_t,
    //     )
    // };
    let ret = unsafe {
        setsockopt(
            fd,
            libc::SOL_TCP,
            TCP_SET_CWND as libc::c_int,
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
        return Err(anyhow!("An error occurred running libc::setsockopt: {}", error_message));
    }

    Ok(())
}

fn sockopt_patch_set_initial_cwnd(stream: &TcpStream, initial_cwnd: u32) -> Result<()> {
    let fd = stream.as_raw_fd();

    // Prepare the buffer for TCP_INFO
    let cwnd_typed: *const c_void = &initial_cwnd as *const _ as *const c_void;
    let size_of_cwnd = mem::size_of::<u32>() as libc::socklen_t;

    /* This one works fine! */
    // let ret = unsafe {
    //     setsockopt(
    //         fd,
    //         libc::SOL_TCP,
    //         libc::TCP_NODELAY, // Change to a known option
    //         &1 as *const _ as *const c_void, // Setting TCP_NODELAY to 1
    //         mem::size_of::<libc::c_int>() as libc::socklen_t,
    //     )
    // };
    let ret = unsafe {
        setsockopt(
            fd,
            libc::SOL_TCP,
            TCP_SET_INIT_CWND as libc::c_int,
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
        return Err(anyhow!("An error occurred running libc::setsockopt: {}", error_message));
    }

    Ok(())
}

fn sockopt_get_tcp_info(stream: &TcpStream, timedata: &mut HashMap<u64, TcpStatsLog>) -> Result<()> {
    let fd = stream.as_raw_fd();

    // Prepare the buffer for TCP_INFO
    let mut tcp_info: StockTcpInfo = StockTcpInfo::default();
    let mut tcp_info_len = mem::size_of::<StockTcpInfo>() as socklen_t;

    // Call getsockopt to get TCP_INFO
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
    let timestamp_us = chrono::Utc::now().timestamp_micros() as u64;
    let rtt = tcp_info.tcpi_rtt;
    let cwnd = tcp_info.tcpi_snd_cwnd;

    timedata.insert(timestamp_us, TcpStatsLog { rtt, cwnd });
    Ok(())
}

/// Returns the default TCP congestion control algorithm as a string.
fn get_tcp_congestion_control() -> String {
    let ctl = sysctl::Ctl::new("net.ipv4.tcp_congestion_control").unwrap();
    ctl.value_string().unwrap()
}

/// Returns the current kernel version as a string.
fn get_kernel_version() -> String {
    sys_info::os_release().unwrap()
}

fn calculate_statistics(
    timedata: &HashMap<u64, TcpStatsLog>,
) -> (Option<u32>, Option<u32>, Option<u32>, Option<u32>) {
    let mut rtt_values: Vec<u32> = timedata.values().map(|ts| ts.rtt).collect();
    let mut cwnd_values: Vec<u32> = timedata.values().map(|ts| ts.cwnd).collect();

    let rtt_mean = if !rtt_values.is_empty() {
        Some(rtt_values.iter().sum::<u32>() / rtt_values.len() as u32)
    } else {
        None
    };

    let cwnd_mean = if !cwnd_values.is_empty() {
        Some(cwnd_values.iter().sum::<u32>() / cwnd_values.len() as u32)
    } else {
        None
    };

    rtt_values.sort();
    cwnd_values.sort();

    let rtt_median = if !rtt_values.is_empty() {
        Some(rtt_values[rtt_values.len() / 2])
    } else {
        None
    };

    let cwnd_median = if !cwnd_values.is_empty() {
        Some(cwnd_values[cwnd_values.len() / 2])
    } else {
        None
    };

    (rtt_mean, cwnd_mean, rtt_median, cwnd_median)
}

fn handle_client(args: Arguments, mut stream: TcpStream, logger: &mut Arc<Mutex<Logger>>) -> Result<()> {
    let logging_interval_us = args.logging_interval_us;
    let transmission_duration_ms = args.transmission_duration_ms;

    let client_ip = stream.peer_addr()?.ip().to_string();
    let mut timedata = HashMap::new();

    if let Some(initial_cwnd) = args.initial_cwnd {
        sockopt_patch_set_initial_cwnd(&stream, initial_cwnd)?;
    }

    let start_time = SystemTime::now();
    let mut last_logging_timestamp_us = chrono::Utc::now().timestamp_micros() as u64 - logging_interval_us;
    while (start_time.elapsed()?.as_millis() as u64) < transmission_duration_ms {
        let now_us = chrono::Utc::now().timestamp_micros() as u64;
        if now_us - last_logging_timestamp_us >= logging_interval_us {
            sockopt_get_tcp_info(&stream, &mut timedata)?;
            last_logging_timestamp_us = now_us;
        }
        if let Some(cwnd) = args.cwnd {
            sockopt_patch_set_cwnd(&stream, cwnd)?;
        }
        stream.write_all(DUMMY_DATA)?;
        thread::sleep(Duration::from_micros(THREAD_SLEEP_TIME_US));
    }
    stream.write_all(b"Transmission complete. Thank you!\n")?;
    stream.flush()?; // Flush again to ensure the completion message is sent before
                     // closing the socket
    thread::sleep(Duration::from_millis(THREAD_SLEEP_FINISH_MS));
    stream.shutdown(std::net::Shutdown::Write)?;

    let (rtt_mean, cwnd_mean, rtt_median, cwnd_median) = calculate_statistics(&timedata);
    let transmission = Transmission {
        client_ip: client_ip.clone(),
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

fn start_server(args: Arguments) -> Result<(), io::Error> {
    println!("---\n{:#?}\n---\n", &args);
    println!("System Congestion Control Algorithm: \t{}", get_tcp_congestion_control());
    println!("System Kernel Version: \t\t\t{}", get_kernel_version());

    let logger = Arc::new(Mutex::new(Logger::new().expect("Failed to create logger")));
    let listener = TcpListener::bind(SERVER_ADDR)?;
    println!("Server listening on: \t\t\t{}", SERVER_ADDR);

    // Pass Arc<Mutex<Logger>> to threads
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let mut logger_ref = Arc::clone(&logger);
                let cloned_args = args.clone();
                thread::spawn(move || {
                    if let Err(err) = handle_client(cloned_args, stream, &mut logger_ref) {
                        eprintln!("Error handling client: {}", err);
                    }
                });
            }
            Err(e) => {
                eprintln!("Error accepting connection: {}", e);
            }
        }
    }

    Ok(())
}

fn main() -> Result<()> {
    let args = Arguments::parse();


    if let Err(err) = start_server(args) {
        eprintln!("Server error: {}", err);
    }

    Ok(())
}
