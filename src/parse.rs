/// Credits: https://stackoverflow.com/questions/55133351/is-there-a-way-to-get-clap-to-use-default-values-from-a-file
use anyhow::Result;
use clap::{Command, CommandFactory, Parser};
use serde::{Deserialize, Serialize};
use std::{default, path::PathBuf};

#[derive(Debug, Clone, PartialEq, Parser, Serialize, Deserialize)]
#[command(author, version, about, long_about = None, next_line_help = true)]
#[command(propagate_version = true)]
pub struct Arguments {
    /// Print additional information in the terminal
    #[clap(short, long)]
    pub verbose: Option<bool>,

    /// Requires patch: Set the initial congestion window size (uint)
    #[clap(short, long)]
    pub initial_cwnd: Option<u32>,

    /// Requires patch: Set the congestion window size (uint)
    #[clap(short, long)]
    pub cwnd: Option<u32>,

    /// Lenght of the transmission in ms
    #[clap(short, long)]
    pub transmission_duration_ms: Option<u64>,

    /// Logging interval
    #[clap(short, long)]
    pub logging_interval_us: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct FlattenedArguments {
    pub verbose: bool,
    pub initial_cwnd: Option<u32>,
    pub cwnd: Option<u32>,
    pub transmission_duration_ms: u64,
    pub logging_interval_us: u64,
}

impl default::Default for Arguments {
    fn default() -> Self {
        Arguments {
            verbose: Some(false),
            initial_cwnd: None,
            cwnd: None,
            transmission_duration_ms: Some(10000),
            logging_interval_us: Some(1000),
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
        self.initial_cwnd = self.initial_cwnd.or(config_file.initial_cwnd);
        self.cwnd = self.cwnd.or(config_file.cwnd);
        self.transmission_duration_ms = self
            .transmission_duration_ms
            .or(config_file.transmission_duration_ms);
        self.logging_interval_us = self.logging_interval_us.or(config_file.logging_interval_us);

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
            initial_cwnd: args.initial_cwnd,
            cwnd: args.cwnd,
            transmission_duration_ms: args.transmission_duration_ms.unwrap(),
            logging_interval_us: args.logging_interval_us.unwrap(),
        })
    }
}
