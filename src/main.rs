use std::thread;
use std::net::TcpListener;
use std::io;
use anyhow::Result;

use std::sync::{Arc, Mutex};

use crate::client::handle_client;
use crate::logger::Logger;
use crate::util::{get_kernel_version, get_tcp_congestion_control};

mod parse;
mod util;
mod client;
mod logger;

use parse::Arguments;
use clap::Parser;

/// Default server addres
const SERVER_ADDR: &str = "0.0.0.0:9393";


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
