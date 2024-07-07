use anyhow::{anyhow, Result};
use bus::BusReader;
use std::collections::HashMap;
use std::net::UdpSocket;
use std::sync::mpsc::{SyncSender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use crate::{util::THREAD_SLEEP_TIME_SHORT_US, StatusMessage};

pub const SOCKET_PACKET_BUFFER_SIZE: usize = 5000;

pub const METRIC_HEADER_LENGTH: usize = 5;
pub const METRIC_INITIAL_INDEX_START: usize = 0;
pub const METRIC_INITIAL_INDEX_END: usize = 4;
pub const METRIC_INITIAL: [u8; 4] = [0x11, 0x21, 0x12, 0x22];
pub const METRIC_TYPE_INDEX: usize = 4;
pub const METRIC_TYPE_START: usize = 5;
pub const METRIC_TYPE_A: u8 = 1;
pub const METRIC_TYPE_A_SIZE: usize = 48;
pub const METRIC_TYPE_B: u8 = 2;

pub struct ExternalInterfaceArgs {
    pub interface_addr: String,
    pub rx_main: BusReader<StatusMessage>,
    pub tx_main: SyncSender<StatusMessage>,
    pub client_metrics: Arc<Mutex<ClientMetrics>>,
}

/* Wrapping messages */
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MetricTypes {
    A(MetricA),
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MetricA {
    /// Timestamp when the metric was calculated
    timestamp_us: u64,
    /// Fair share send rate [bits/subframe] = [bits/ms]
    fair_share_send_rate: u64,
    /// Timestamp of the latest DCI used to calculate the metric
    latest_dci_timestamp_us: u64,
    /// Timestamp of the oldest DCI used to calculate the metric
    oldest_dci_timestamp_us: u64,
    /// Number of DCIs used to calculate the metric
    nof_dci: u16,
    /// Number of phy-layer re-transmissions
    nof_re_tx: u16,
    /// Flag, signalling whether phy_rate was averagerd over all RNTIs or just our UE RNTI
    flag_phy_rate_all_rnti: u8,
    /// Average bit per PRB (either over all RNTIs or just the UE RNTI)
    phy_rate: u64,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct ClientMetrics {
    pub clients: HashMap<String, MetricTypes>,
}

pub fn deploy_external_interface(mut args: ExternalInterfaceArgs) -> Result<()> {
    let builder = thread::Builder::new().name("[external]".to_string());
    builder.spawn(move || {
        let _ = run_external_interface(&mut args);
        finish_external_interface(&mut args);
    })?;
    Ok(())
}

fn check_not_stopped(reader: &mut BusReader<StatusMessage>) -> Result<()> {
    match reader.try_recv() {
        Ok(StatusMessage::Stop(_)) => Err(anyhow!("BusReader received GeneralState::Stopped!")),
        Err(TryRecvError::Empty) => Ok(()),
        Err(TryRecvError::Disconnected) => Err(anyhow!("BusReader disconnected!")),
    }
}

fn init_socket_buffer() -> Box<[u8]> {
    let mut vec: Vec<u8> = Vec::<u8>::with_capacity(SOCKET_PACKET_BUFFER_SIZE);
    /* Fill the vector with zeros */
    vec.resize_with(SOCKET_PACKET_BUFFER_SIZE, || 0);
    vec.into_boxed_slice()
}

fn run_external_interface(args: &mut ExternalInterfaceArgs) -> Result<()> {
    let sleep_duration = Duration::from_micros(THREAD_SLEEP_TIME_SHORT_US);
    std::thread::sleep(sleep_duration);

    let socket = UdpSocket::bind(args.interface_addr.clone())?;
    let mut socket_buffer: Box<[u8]> = init_socket_buffer();

    println!("[external] started interface on \t{}", args.interface_addr);

    loop {
        std::thread::sleep(sleep_duration);
        if check_not_stopped(&mut args.rx_main).is_err() {
            break;
        }

        /* Catch incoming messages and update the client_metrics hash */
        let (nof_recv, src_addr) = socket.recv_from(&mut socket_buffer)?;
        if let Ok(metric_type) = MetricTypes::from_bytes(&socket_buffer[..nof_recv]) {
            let src_addr_str = src_addr.to_string().split_once(':').unwrap().0.to_string();
            let mut metrics = args.client_metrics.lock().unwrap();
            metrics.clients.insert(src_addr_str, metric_type);
        }
    }
    Ok(())
}

fn finish_external_interface(args: &mut ExternalInterfaceArgs) {
    println!("[external] stopped");
    let _ = args
        .tx_main
        .send(StatusMessage::Stop("[external]".to_string()));
}

impl MetricTypes {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self> {
        if bytes.len() < METRIC_HEADER_LENGTH {
            return Err(anyhow!("decode metric: length smaller than header size"));
        }
        let header_bytes = &bytes[METRIC_INITIAL_INDEX_START..METRIC_INITIAL_INDEX_END];
        if METRIC_INITIAL != header_bytes {
            return Err(anyhow!("decode metric: headers unknown"));
        }
        let metric_type = bytes[METRIC_TYPE_INDEX];
        let payload = &bytes[METRIC_TYPE_START..];
        match metric_type {
            METRIC_TYPE_A => {
                if payload.len() < METRIC_TYPE_A_SIZE {
                    return Err(anyhow!(
                        "decode metric: tried decoding metric A, but not enoug payload"
                    ));
                }
                Ok(MetricTypes::A(MetricA::from_bytes(
                    payload[0..METRIC_TYPE_A_SIZE].try_into()?,
                )?))
            }
            METRIC_TYPE_B => Err(anyhow!("decode metric: metric B not implemented!")),
            _ => Err(anyhow!(
                "decode metric: metric type field unknown '{:?}'",
                metric_type
            )),
        }
    }

    pub fn get_rate(&self) -> u64 {
        match self {
            MetricTypes::A(metric_a) => metric_a.fair_share_send_rate,
        }
    }

    pub fn get_timestamp_us(&self) -> u64 {
        match self {
            MetricTypes::A(metric_a) => metric_a.timestamp_us,
        }
    }
}

impl MetricA {
    fn from_bytes(bytes: [u8; METRIC_TYPE_A_SIZE]) -> Result<Self> {
        let metric: &MetricA = unsafe { &*bytes.as_ptr().cast() };
        Ok(*metric)
    }
}
