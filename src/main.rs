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

use libc::{c_void, getsockopt, socklen_t, TCP_INFO};

const SERVER_ADDR: &str = "0.0.0.0:9393";
/// Array of 1024 bytes filled with 0x01
const DUMMY_DATA: &[u8] = &[0x01; 1024];
/// Log TCP parameters every 100 milliseconds
const LOG_INTERVAL_MS: u64 = 10;
/// Duration to send dummy data in seconds
const TRANSMISSION_DURATION_SECS: u64 = 10;
const LOGS_DIRECTORY: &str = ".logs";

#[repr(C)]
#[derive(Debug, Default)]
struct TcpInfo {
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

fn log_tcp_info(stream: &TcpStream, timedata: &mut HashMap<u64, TcpStatsLog>) -> Result<()> {
    let fd = stream.as_raw_fd();

    // Prepare the buffer for TCP_INFO
    let mut tcp_info: TcpInfo = TcpInfo::default();
    let mut tcp_info_len = mem::size_of::<TcpInfo>() as socklen_t;

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

fn handle_client(mut stream: TcpStream, logger: &mut Arc<Mutex<Logger>>) -> Result<()> {
    let client_ip = stream.peer_addr()?.ip().to_string();
    let mut timedata = HashMap::new();

    let start_time = SystemTime::now();
    while start_time.elapsed()?.as_secs() < TRANSMISSION_DURATION_SECS {
        stream.write_all(DUMMY_DATA)?;
        log_tcp_info(&stream, &mut timedata)?;
        thread::sleep(Duration::from_millis(LOG_INTERVAL_MS));
    }
    stream.write_all(b"Transmission complete. Thank you!\n")?;
    stream.flush()?; // Flush again to ensure the completion message is sen
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

fn start_server() -> Result<(), io::Error> {
    let logger = Arc::new(Mutex::new(Logger::new().expect("Failed to create logger")));
    let listener = TcpListener::bind(SERVER_ADDR)?;
    println!("Server listening on {}", SERVER_ADDR);


    // Pass Arc<Mutex<Logger>> to threads
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let mut logger_ref = Arc::clone(&logger);
                thread::spawn(move || {
                    if let Err(err) = handle_client(stream, &mut logger_ref) {
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

fn main() {
    if let Err(err) = start_server() {
        eprintln!("Server error: {}", err);
    }
}
