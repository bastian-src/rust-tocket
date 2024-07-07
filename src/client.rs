use std::io::Write;
use std::sync::Arc;
use std::sync::Mutex;
use crate::logger::Logger;
use crate::parse::Arguments;
use crate::util::THREAD_SLEEP_TIME_US;
use crate::util::THREAD_SLEEP_FINISH_MS;
use crate::util::calculate_statistics;
use crate::util::StockTcpInfo;
use std::{
    collections::HashMap,
    net::TcpStream,
    os::unix::io::AsRawFd,
    thread,
    time::{Duration, SystemTime},
};
use std::mem;
use serde::{Deserialize, Serialize};
use anyhow::{anyhow, Result};

use libc::{c_void, getsockopt, socklen_t, TCP_INFO, setsockopt};

/// Array of 8192 bytes filled with 0x01
const DUMMY_DATA: &[u8] = &[0x01; 8192];

/* PATCH CONSTATNS */
pub const TCP_SET_CWND: u32 = 50;
pub const TCP_SET_INIT_CWND: u32 = 51;

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
}


pub fn handle_client(args: Arguments, mut stream: TcpStream, logger: &mut Arc<Mutex<Logger>>) -> Result<()> {
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


