use std::net::TcpListener;
use std::sync::mpsc::Receiver;
use std::thread;
use anyhow::Result;
use bus::Bus;
use serde::{Deserialize, Serialize};

use std::sync::{mpsc, Arc, Mutex};

use crate::client::{handle_client, ClientArgs};
use crate::logger::Logger;
use crate::util::{get_kernel_version, get_tcp_congestion_control, DynamicValue, DEFAULT_BUS_SIZE};
use external::{deploy_external_interface, ClientMetrics, ExternalInterfaceArgs};

mod client;
mod external;
mod logger;
mod parse;
mod util;

use parse::{Arguments, FlattenedArguments};


#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum TransmissionType {
    Bbr,

    Reno,

    Cubic,

    L2BFair0Init,
    L2BFair0Upper,
    L2BFair0InitAndUpper,
    L2BFair0Direct,

    L2BFair1Init,
    L2BFair1Upper,
    L2BFair1InitAndUpper,
    L2BFair1Direct,

    L2BFair2Init,
    L2BFair2Upper,
    L2BFair2InitAndUpper,
    L2BFair2Direct,
}

impl TransmissionType {
    pub fn all() -> Vec<TransmissionType> {
        vec![
            TransmissionType::Bbr,

            TransmissionType::Reno,

            TransmissionType::Cubic,

            TransmissionType::L2BFair0Init,
            TransmissionType::L2BFair0Upper,
            TransmissionType::L2BFair0InitAndUpper,
            TransmissionType::L2BFair0Direct,

            TransmissionType::L2BFair1Init,
            TransmissionType::L2BFair1Upper,
            TransmissionType::L2BFair1InitAndUpper,
            TransmissionType::L2BFair1Direct,

            TransmissionType::L2BFair2Init,
            TransmissionType::L2BFair2Upper,
            TransmissionType::L2BFair2InitAndUpper,
            TransmissionType::L2BFair2Direct,
        ]
    }

    pub fn is_l2b(&self) -> bool {
        !matches!(self,
            TransmissionType::Reno |
            TransmissionType::Bbr |
            TransmissionType::Cubic)
    }

    pub fn path(&self) -> &str {
        match self {
            TransmissionType::Bbr => "/bbr",
            TransmissionType::Reno => "/reno",
            TransmissionType::Cubic=> "/cubic",

            TransmissionType::L2BFair0Init => "/l2b/fair0/init",
            TransmissionType::L2BFair0Upper => "/l2b/fair0/upper",
            TransmissionType::L2BFair0InitAndUpper => "/l2b/fair0/init_and_upper",
            TransmissionType::L2BFair0Direct => "/l2b/fair0/direct",

            TransmissionType::L2BFair1Init => "/l2b/fair1/init",
            TransmissionType::L2BFair1Upper => "/l2b/fair1/upper",
            TransmissionType::L2BFair1InitAndUpper => "/l2b/fair1/init_and_upper",
            TransmissionType::L2BFair1Direct => "/l2b/fair1/direct",

            TransmissionType::L2BFair2Init => "/l2b/fair2/init",
            TransmissionType::L2BFair2Upper => "/l2b/fair2/upper",
            TransmissionType::L2BFair2InitAndUpper => "/l2b/fair2/init_and_upper",
            TransmissionType::L2BFair2Direct => "/l2b/fair2/direct",
        }
    }

    pub fn from_path(path: &str) -> Option<Self> {
        for item in TransmissionType::all() {
            if path == item.path() {
                return Some(item)
            }
        }
        None
    }

}


#[derive(Clone, Debug)]
pub enum StatusMessage {
    Stop(String),
}

#[derive(Debug)]
pub struct MainVariables {
    tx_main: Bus<StatusMessage>,
    rx_external: Option<Receiver<StatusMessage>>,
    client_metrics: Option<Arc<Mutex<ClientMetrics>>>,
}

fn start_server(args: &FlattenedArguments, main_vars: &mut MainVariables) -> Result<()> {
    if args.verbose {
        println!(
            "System Congestion Control Algorithm: \t{}",
            get_tcp_congestion_control()
        );
        println!("System Kernel Version: \t\t\t{}", get_kernel_version());
    }

    let logger = Arc::new(Mutex::new(Logger::new().expect("Failed to create logger")));
    let listener = TcpListener::bind(&args.server_addr)?;
    logger.lock().unwrap().log_stdout(&format!("Server listening on: \t\t\t{}", args.server_addr))?;

    // Pass Arc<Mutex<Logger>> to threads
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let mut client_args = ClientArgs {
                    args: args.clone(),
                    stream,
                    logger: Arc::clone(&logger),
                    client_metrics: main_vars.client_metrics.clone(),
                    transmission_type: TransmissionType::Cubic,
                    set_initial_cwnd: None,
                    set_upper_bound_cwnd: None,
                    set_direct_cwnd: None,
                    transmission_duration_ms: args.default_transmission_duration_ms,
                    path: "".to_string(),
                };
                evaluate_client_args(logger.clone(), args, &mut client_args);
                thread::spawn(move || {
                    if let Err(err) = handle_client(client_args) {
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

fn evaluate_client_args(logger: Arc<Mutex<Logger>>, args: &FlattenedArguments, client_args: &mut ClientArgs) {
    if let Some(fixed_initial_cwnd) = args.initial_cwnd {
        client_args.set_initial_cwnd = Some(DynamicValue::Fixed(fixed_initial_cwnd));
    }
    if let Some(fixed_direct_cwnd) = args.direct_cwnd {
        client_args.set_initial_cwnd = Some(DynamicValue::Fixed(fixed_direct_cwnd));
    }
    if let Some(fixed_upper_bound_cwnd) = args.upper_bound_cwnd {
        client_args.set_initial_cwnd = Some(DynamicValue::Fixed(fixed_upper_bound_cwnd));
    }

    let mut buffer = [0; 512];
    if let Ok(bytes_read) = client_args.stream.peek(&mut buffer) {
        let mut headers = [httparse::EMPTY_HEADER; 16];
        let mut req = httparse::Request::new(&mut headers);

        if req.parse(&buffer[..bytes_read]).is_ok() {
            let path = req.path.unwrap_or("/");
            if path.len() < 4 {
                return
            }
            client_args.path = path.to_string();
            let path_time = &path[0..4];
            match path_time {
                "/10s" => {
                    client_args.transmission_duration_ms = 10000;
                },
                "/60s" => {
                    client_args.transmission_duration_ms = 60000;
                }
                _ => {
                    let _ = logger.lock().unwrap().log_stdout(&format!("Error interpreting client path time: {}", path_time));
                    return;
                }
            }
            if path.len() <= 5 {
                return
            }
            let path_algo = &path[4..];
            if let Some(algo_item) = TransmissionType::from_path(path_algo) {
                client_args.transmission_type = algo_item.clone();
                match algo_item {
                    TransmissionType::Bbr |
                    TransmissionType::Reno |
                    TransmissionType::Cubic => {},

                    TransmissionType::L2BFair0Init |
                    TransmissionType::L2BFair1Init |
                    TransmissionType::L2BFair2Init => {
                        client_args.set_initial_cwnd = Some(DynamicValue::Dynamic);
                    }

                    TransmissionType::L2BFair0Upper |
                    TransmissionType::L2BFair1Upper |
                    TransmissionType::L2BFair2Upper => {
                        client_args.set_upper_bound_cwnd = Some(DynamicValue::Dynamic);
                    }

                    TransmissionType::L2BFair0InitAndUpper |
                    TransmissionType::L2BFair1InitAndUpper |
                    TransmissionType::L2BFair2InitAndUpper => {
                        client_args.set_initial_cwnd = Some(DynamicValue::Dynamic);
                        client_args.set_upper_bound_cwnd = Some(DynamicValue::Dynamic);
                    }

                    TransmissionType::L2BFair0Direct |
                    TransmissionType::L2BFair1Direct |
                    TransmissionType::L2BFair2Direct => {
                        client_args.set_direct_cwnd = Some(DynamicValue::Dynamic);
                    }
                }
            } else {
                let _ = logger.lock().unwrap().log_stdout(&format!("Error interpreting client path: {}", path));
            }
        }
    }
}

fn start_external_interface(
    args: &FlattenedArguments,
    main_vars: &mut MainVariables,
) -> Result<()> {
    let (tx, rx) = mpsc::sync_channel(DEFAULT_BUS_SIZE);
    main_vars.rx_external = Some(rx);
    let client_metrics_arc = Arc::new(Mutex::new(ClientMetrics::default()));
    main_vars.client_metrics = Some(client_metrics_arc.clone());

    let ext_args: ExternalInterfaceArgs = ExternalInterfaceArgs {
        interface_addr: args.external_interface_addr.clone(),
        rx_main: main_vars.tx_main.add_rx(),
        tx_main: tx,
        client_metrics: client_metrics_arc,
    };

    deploy_external_interface(ext_args)
}

fn main() -> Result<()> {
    let args = Arguments::build()?;
    let flat_args = FlattenedArguments::from_unflattened(args)?;
    let mut main_vars: MainVariables = MainVariables {
        tx_main: Bus::<StatusMessage>::new(DEFAULT_BUS_SIZE),
        rx_external: None,
        client_metrics: None,
    };

    if flat_args.external_interface {
        if let Err(err) = start_external_interface(&flat_args, &mut main_vars) {
            eprintln!("[main] error starting external interface: {}", err);
        }
    }

    if let Err(err) = start_server(&flat_args, &mut main_vars) {
        eprintln!("[main] Server error: {}", err);
    }

    // TODO: Add properly sigint-handling
    main_vars
        .tx_main
        .broadcast(StatusMessage::Stop("".to_string()));

    Ok(())
}
