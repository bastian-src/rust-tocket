use crate::client::TcpStatsLog;
use std::collections::HashMap;
use sysctl::Sysctl;

pub const DEFAULT_BUS_SIZE: usize = 100;
pub const THREAD_SLEEP_TIME_SHORT_US: u64 = 10;
pub const THREAD_SLEEP_FINISH_MS: u64 = 1000;

pub const CONSTANT_BIT_TO_BYTE: u64 = 8;
pub const CONSTANT_US_TO_MS: u64 = 1000;

#[derive(Debug, Clone)]
pub enum DynamicValue<T> {
    Dynamic,
    Fixed(T),
}

#[repr(C)]
#[derive(Debug, Default)]
pub struct StockTcpInfo {
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
