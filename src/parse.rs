/// Credits: https://stackoverflow.com/questions/55133351/is-there-a-way-to-get-clap-to-use-default-values-from-a-file
use anyhow::Result;
use clap::{Command, CommandFactory, Parser};
use serde::{Deserialize, Serialize};
use std::{default, path::PathBuf};

pub const DEFAULT_VERBOSE: Option<bool> = Some(true);
pub const DEFAULT_SERVER_ADDR: &str = "0.0.0.0:9393";
pub const DEFAULT_INITIAL_CWND: Option<u32> = None;
pub const DEFAULT_DIRECT_CWND: Option<u32> = None;
pub const DEFAULT_UPPER_BOUND_CWND: Option<u32> = None;
pub const DEFAULT_TRANSMISSION_DURATION_MS: Option<u64> = Some(10000);
pub const DEFAULT_LOGGING_INTERVAL_US: Option<u64> = Some(1000);
pub const DEFAULT_EXTERNAL_INTERFACE: Option<bool> = Some(true);
pub const DEFAULT_EXTERNAL_INTERFACE_ADDR: &str = "0.0.0.0:9494";

#[derive(Debug, Clone, PartialEq, Parser, Serialize, Deserialize)]
#[command(author, version, about, long_about = None, next_line_help = true)]
#[command(propagate_version = true)]
pub struct Arguments {
    /// Print additional information in the terminal
    #[clap(short, long)]
    pub verbose: Option<bool>,

    /// Local server addr
    #[clap(short, long)]
    pub server_addr: Option<String>,

    /// Requires patch: Set the initial congestion window size (uint)
    #[clap(short, long)]
    pub initial_cwnd: Option<u32>,

    /// Requires patch: Set the congestion window size (uint)
    #[clap(short, long)]
    pub direct_cwnd: Option<u32>,

    /// Requires patch: Set an upper bound for the congestion window size (uint)
    #[clap(short, long)]
    pub upper_bound_cwnd: Option<u32>,

    /// Lenght of the transmission in ms
    #[clap(short, long)]
    pub transmission_duration_ms: Option<u64>,

    /// Logging interval
    #[clap(short, long)]
    pub logging_interval_us: Option<u64>,

    /// Enable socket for receiving external rate information
    #[clap(long)]
    pub external_interface: Option<bool>,

    /// Local address for external-interface socket address
    #[clap(long)]
    pub external_interface_addr: Option<String>,
}

#[derive(Debug, Clone)]
pub struct FlattenedArguments {
    pub verbose: bool,
    pub server_addr: String,
    pub initial_cwnd: Option<u32>,
    pub direct_cwnd: Option<u32>,
    pub upper_bound_cwnd: Option<u32>,
    pub transmission_duration_ms: u64,
    pub logging_interval_us: u64,
    pub external_interface: bool,
    pub external_interface_addr: String,
}

impl default::Default for Arguments {
    fn default() -> Self {
        Arguments {
            verbose: DEFAULT_VERBOSE,
            server_addr: Some(DEFAULT_SERVER_ADDR.to_string()),
            initial_cwnd: DEFAULT_INITIAL_CWND,
            direct_cwnd: DEFAULT_DIRECT_CWND,
            upper_bound_cwnd: DEFAULT_UPPER_BOUND_CWND,
            transmission_duration_ms: DEFAULT_TRANSMISSION_DURATION_MS,
            logging_interval_us: DEFAULT_LOGGING_INTERVAL_US,
            external_interface: DEFAULT_EXTERNAL_INTERFACE,
            external_interface_addr: Some(DEFAULT_EXTERNAL_INTERFACE_ADDR.to_string()),
        }
    }
}

impl Arguments {
    /// Build Arguments struct
    pub fn build() -> Result<Self> {
        let app: Command = Arguments::command();
        let app_name: &str = app.get_name();

        let parsed_args = Arguments::parse();
        match parsed_args.clone().get_config_file(app_name) {
            Ok(parsed_config_args) => {
                let printed_args = parsed_config_args.print_config_file(app_name)?;
                Ok(printed_args)
            }
            Err(_) => {
                let printed_args = parsed_args
                    .set_config_file(app_name)?
                    .print_config_file(app_name)?;
                Ok(printed_args)
            }
        }
    }

    /// Get configuration file.
    /// A new configuration file is created with default values if none exists.
    fn get_config_file(mut self, app_name: &str) -> Result<Self> {
        let config_file: Arguments = confy::load(app_name, None)?;

        self.verbose = self.verbose.or(config_file.verbose);
        self.server_addr = self.server_addr.or(config_file.server_addr);
        self.initial_cwnd = self.initial_cwnd.or(config_file.initial_cwnd);
        self.direct_cwnd = self.direct_cwnd.or(config_file.direct_cwnd);
        self.upper_bound_cwnd = self.upper_bound_cwnd.or(config_file.upper_bound_cwnd);
        self.transmission_duration_ms = self
            .transmission_duration_ms
            .or(config_file.transmission_duration_ms);
        self.logging_interval_us = self.logging_interval_us.or(config_file.logging_interval_us);
        self.external_interface = self.external_interface.or(config_file.external_interface);
        self.external_interface_addr = self
            .external_interface_addr
            .or(config_file.external_interface_addr);

        Ok(self)
    }

    /// Save changes made to a configuration object
    fn set_config_file(self, app_name: &str) -> Result<Self> {
        let default_args: Arguments = Default::default();
        confy::store(app_name, None, default_args)?;
        Ok(self)
    }

    /// Print configuration file path and its contents
    fn print_config_file(self, app_name: &str) -> Result<Self> {
        if self.verbose.unwrap_or(true) {
            let file_path: PathBuf = confy::get_configuration_file_path(app_name, None)?;
            println!(
                "DEBUG [parse] Configuration file: '{}'",
                file_path.display()
            );

            let yaml: String = serde_yaml::to_string(&self)?;
            println!("\t{}", yaml.replace('\n', "\n\t"));
        }

        Ok(self)
    }
}

impl FlattenedArguments {
    pub fn from_unflattened(args: Arguments) -> Result<FlattenedArguments> {
        Ok(FlattenedArguments {
            verbose: args.verbose.unwrap(),
            server_addr: args.server_addr.unwrap(),
            initial_cwnd: args.initial_cwnd,
            direct_cwnd: args.direct_cwnd,
            upper_bound_cwnd: args.upper_bound_cwnd,
            transmission_duration_ms: args.transmission_duration_ms.unwrap(),
            logging_interval_us: args.logging_interval_us.unwrap(),
            external_interface: args.external_interface.unwrap(),
            external_interface_addr: args.external_interface_addr.unwrap(),
        })
    }
}
