use anyhow::Result;
use bus::Bus;
use external::{deploy_external_interface, ClientMetrics, ExternalInterfaceArgs};
use std::io;
use std::net::TcpListener;
use std::sync::mpsc::Receiver;
use std::thread;
use util::{DynamicValue, DEFAULT_BUS_SIZE};

use std::sync::{mpsc, Arc, Mutex};

use crate::client::{handle_client, ClientArgs};
use crate::logger::Logger;
use crate::util::{get_kernel_version, get_tcp_congestion_control};

mod client;
mod external;
mod logger;
mod parse;
mod util;

use parse::{Arguments, FlattenedArguments};

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

fn start_server(args: &FlattenedArguments, main_vars: &mut MainVariables) -> Result<(), io::Error> {
    if args.verbose {
        println!(
            "System Congestion Control Algorithm: \t{}",
            get_tcp_congestion_control()
        );
        println!("System Kernel Version: \t\t\t{}", get_kernel_version());
    }

    let logger = Arc::new(Mutex::new(Logger::new().expect("Failed to create logger")));
    let listener = TcpListener::bind(&args.server_addr)?;
    println!("Server listening on: \t\t\t{}", args.server_addr);

    // Pass Arc<Mutex<Logger>> to threads
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let mut client_args = ClientArgs {
                    args: args.clone(),
                    stream,
                    logger: Arc::clone(&logger),
                    client_metrics: main_vars.client_metrics.clone(),
                    set_initial_cwnd: None,
                    set_upper_bound_cwnd: None,
                    set_direct_cwnd: None,
                };
                evaluate_client_args(args, &mut client_args);
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

fn evaluate_client_args(args: &FlattenedArguments, client_args: &mut ClientArgs) {
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
            match path {
                "/init_and_upper" => {
                    client_args.set_initial_cwnd = Some(DynamicValue::Dynamic);
                    client_args.set_upper_bound_cwnd = Some(DynamicValue::Dynamic);
                }
                "/init" => {
                    client_args.set_initial_cwnd = Some(DynamicValue::Dynamic);
                }
                "/upper" => {
                    client_args.set_upper_bound_cwnd = Some(DynamicValue::Dynamic);
                }
                "/direct" => {
                    client_args.set_direct_cwnd = Some(DynamicValue::Dynamic);
                }
                _ => {}
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
