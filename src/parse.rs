use clap::Parser;

#[derive(Parser, Debug, Clone)]
#[command(about, long_about = None)]
pub struct Arguments {
    /// Print additional information in the terminal
    #[clap(short, long)]
    #[clap(default_value = "false")]
    pub verbose: bool,

    /// Requires patch: Set the initial congestion window size (uint)
    #[clap(short, long)]
    #[clap(default_value = None)]
    pub initial_cwnd: Option<u32>,

    /// Requires patch: Set the congestion window size (uint)
    #[clap(short, long)]
    #[clap(default_value = None)]
    pub cwnd: Option<u32>,

    /// Lenght of the transmission in ms
    #[clap(short, long)]
    #[clap(default_value = "10000")]
    pub transmission_duration_ms: u64,

    /// Logging interval
    #[clap(short, long)]
    #[clap(default_value = "1000")]
    pub logging_interval_us: u64,
}

