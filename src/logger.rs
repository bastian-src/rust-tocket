use anyhow::Result;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};

const LOGS_DIRECTORY: &str = ".logs.tocket";

#[derive(Debug)]
pub struct Logger {
    run_dir: String,
    run_timestamp: String,
}

impl Logger {
    pub fn new() -> Result<Self, io::Error> {
        let run_timestamp =  chrono::Local::now().format("%Y-%m-%d_%H-%M").to_string();
        let run_dir = format!("{}/run_{}/", LOGS_DIRECTORY, run_timestamp);
        fs::create_dir_all(&run_dir)?;

        Ok(Self {
            run_dir,
            run_timestamp,
        })
    }


    pub fn log_transmission(&self, msg: &str, path: String) -> Result<()> {
        let file_path = format!(
            "{}run_{}_cc_{}.jsonl",
            self.run_dir,
            self.run_timestamp,
            path.replace('/', "_")
        );
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(file_path)?;
        writeln!(file, "{}", msg)?;
        file.flush()?;
        Ok(())
    }

    pub fn log_stdout(&self, msg: &str) -> Result<()> {
        println!("{}", msg);
        let file_path = format!(
            "{}run_{}_stdout.log",
            self.run_dir,
            self.run_timestamp,
        );
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(file_path)?;
        writeln!(file, "{}", msg)?;
        file.flush()?;
        Ok(())
    }
}
