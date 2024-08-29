use crate::{client::TcpStatsLog, TransmissionType};
use std::{collections::HashMap, ffi::{CString, CStr}, fs::File, io::{Write, Read}};
use serde::{Deserialize, Serialize};
use sysctl::Sysctl;
use anyhow::{anyhow, Result};
use libc::{c_void, getsockopt, setsockopt, socklen_t, TCP_INFO, TCP_CONGESTION, SOL_PACKET};
use std::mem;

// Linux Kernel PACKET_STATISTICS (compare:
// https://github.com/rust-lang/libc/blob/5c8a32d724be45b53521d34428d4ef669fa588b6/src/unix/linux_like/linux/mod.rs#L3194)
#[allow(dead_code)]
const PACKET_STATISTICS: i32 = 6;

pub const DEFAULT_BUS_SIZE: usize = 100;
pub const THREAD_SLEEP_TIME_SHORT_US: u64 = 10;
pub const THREAD_SLEEP_TIME_SHORT_MS: u64 = 10;
pub const THREAD_SLEEP_FINISH_MS: u64 = 1000;

pub const CONSTANT_BIT_TO_BYTE: u64 = 8;
pub const CONSTANT_US_TO_MS: u64 = 3000;

#[allow(dead_code)]
/// Use this in combination with proc_set_u8() to adapt the tcp_friendliness behavior of CUBIC
pub const PROC_PATH_TCP_CUBIC_TCP_FRIENDLINESS: &str = "/sys/module/tcp_cubic/parameters/tcp_friendliness";

#[derive(Debug, Clone)]
pub enum DynamicValue<T> {
    Dynamic,
    Fixed(T),
}

// Compare: https://man7.org/linux/man-pages/man7/packet.7.html
#[repr(C)]
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct TPacketStats {
    pub tp_packets: u32,
    pub tp_drops: u32,
}

// Compare: https://www.man7.org/linux/man-pages/man7/tcp.7.html
#[repr(C)]
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct TcpInfo {
    pub tcpi_state: u8,
    pub tcpi_ca_state: u8,
    pub tcpi_retransmits: u8,
    pub tcpi_probes: u8,
    pub tcpi_backoff: u8,
    pub tcpi_options: u8,
    pub tcpi_snd_wscale: u8,
    pub tcpi_rcv_wscale: u8,

    pub tcpi_rto: u32,
    pub tcpi_ato: u32,
    pub tcpi_snd_mss: u32,
    pub tcpi_rcv_mss: u32,

    pub tcpi_unacked: u32,
    pub tcpi_sacked: u32,
    pub tcpi_lost: u32,
    pub tcpi_retrans: u32,
    pub tcpi_fackets: u32,

    // Times
    pub tcpi_last_data_sent: u32,
    pub tcpi_last_ack_sent: u32,
    pub tcpi_last_data_recv: u32,
    pub tcpi_last_ack_recv: u32,

    // Metrics
    pub tcpi_pmtu: u32,
    pub tcpi_rcv_ssthresh: u32,
    pub tcpi_rtt: u32,
    pub tcpi_rttvar: u32,
    pub tcpi_snd_ssthresh: u32,
    pub tcpi_snd_cwnd: u32,
    pub tcpi_advmss: u32,
    pub tcpi_reordering: u32,

    pub tcpi_rcv_rtt: u32,
    pub tcpi_rcv_space: u32,

    pub tcpi_total_retrans: u32,
}

/// Returns the default TCP congestion control algorithm as a string.
pub fn get_tcp_congestion_control() -> String {
    let ctl = sysctl::Ctl::new("net.ipv4.tcp_congestion_control").unwrap();
    ctl.value_string().unwrap()
}

/// Returns the current kernel version as a string.
pub fn get_kernel_version() -> String {
    sys_info::os_release().unwrap()
}

pub fn calculate_statistics(
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

pub fn sockopt_patch_cwnd(socket_file_descriptor: i32, upper_cwnd: u32, patch_type: u32) -> Result<()> {
    // Prepare the buffer for TCP_INFO
    let cwnd_typed: *const c_void = &upper_cwnd as *const _ as *const c_void;
    let size_of_cwnd = mem::size_of::<u32>() as libc::socklen_t;

    let ret = unsafe {
        setsockopt(
            socket_file_descriptor,
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

pub fn sockopt_prepare_transmission(
    socket_file_descriptor: i32,
    transmission_type: &TransmissionType,
) -> Result<()> {
    /* Set CC */
    let cc_name = match transmission_type {
        TransmissionType::Reno => "reno",
        TransmissionType::Bbr => "bbr",
        _ => "cubic",
    };
    sockopt_set_cc(socket_file_descriptor, cc_name)?;
    Ok(())
}

#[allow(dead_code)]
pub fn proc_set_u8(
    path: &str,
    value: &str,
) -> Result<()> {
    let mut file = File::create(path).map_err(|e| anyhow!("[proc_set_u8] error creating file reference '{}': {}", path, e))?;
    let write_buffer = value.to_string();
    file.write_all(write_buffer.as_bytes()).map_err(|e| anyhow!("[proc_set_u8] error writing file '{}': {}", path, e))?;
    Ok(())
}

#[allow(dead_code)]
pub fn proc_get(
    path: &str,
) -> Result<String> {
    let mut file = File::open(path).map_err(|e| anyhow!("[proc_get] error opening file '{}': {}", path, e))?;
    let mut read_buffer = String::new();
    file.read_to_string(&mut read_buffer)?;
    Ok(read_buffer.trim().to_string())
}

pub fn sockopt_set_cc(
    socket_file_descriptor: i32,
    cc_name: &str,
) -> Result<()> {
    let cc_name_cstr = CString::new(cc_name).map_err(|_| anyhow!("Failed to convert cc_name to CString"))?;

    // Set the congestion control algorithm using setsockopt
    let ret = unsafe {
        setsockopt(
            socket_file_descriptor,
            libc::IPPROTO_TCP,
            TCP_CONGESTION,
            cc_name_cstr.as_ptr() as *const c_void,
            cc_name.len() as socklen_t,
        )
    };

    if ret != 0 {
        return Err(anyhow!("An error occurred running libc::setsockopt: {:?}", std::io::Error::last_os_error()));
    }
    let active_cca = get_active_cca(socket_file_descriptor)?;
    if active_cca != cc_name {
        return Err(anyhow!("After setting the CCA, the currently set CCA does not \
                           correpond to the expected algorithm: {} != {}",active_cca, cc_name));
    }
    Ok(())
}

pub fn get_active_cca(socket_file_descriptor: i32) -> Result<String> {
    // Define a buffer to hold the congestion control algorithm name
    let mut cc_name: [u8; 16] = [0; 16];
    let mut cc_name_len = cc_name.len() as socklen_t;

    // Use getsockopt to retrieve the current congestion control algorithm
    let ret = unsafe {
        getsockopt(
            socket_file_descriptor,
            libc::IPPROTO_TCP,
            TCP_CONGESTION,
            cc_name.as_mut_ptr() as *mut c_void,
            &mut cc_name_len,
        )
    };

    if ret != 0 {
        return Err(anyhow!("An error occurred running libc::getsockopt: {:?}", std::io::Error::last_os_error()));
    }

    let cc_name_cstr = unsafe { CStr::from_ptr(cc_name.as_ptr() as *const i8) };
    let cc_name_str = cc_name_cstr.to_str().map_err(|_| anyhow!("Failed to convert CCA name to &str"))?;

    Ok(cc_name_str.to_string())
}

pub fn sockopt_get_tcp_info(socket_file_descriptor: i32) -> Result<TcpInfo> {
    let mut tcp_info: TcpInfo = TcpInfo::default();
    let mut tcp_info_len = mem::size_of::<TcpInfo>() as socklen_t;

    let ret = unsafe {
        getsockopt(
            socket_file_descriptor,
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

// Compare: https://unix.stackexchange.com/a/556793
// NEEDS a Layer 2 SOCKET!!!
#[allow(dead_code)]
pub fn sockopt_get_tpacket_stats(socket_file_descriptor: i32) -> Result<TPacketStats, std::io::Error> {
    let mut stats: TPacketStats = TPacketStats::default();
    let mut stats_len = mem::size_of::<TPacketStats>() as socklen_t;

    let ret = unsafe {
        getsockopt(
            socket_file_descriptor,
            SOL_PACKET,
            PACKET_STATISTICS,
            &mut stats as *mut _ as *mut c_void,
            &mut stats_len,
        )
    };

    if ret != 0 {
        return Err(std::io::Error::last_os_error());
    }

    Ok(stats)
}
