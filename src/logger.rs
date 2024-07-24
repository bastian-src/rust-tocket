use anyhow::Result;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};

use crate::TransmissionType;

const LOGS_DIRECTORY: &str = ".logs.tocket";

#[derive(Debug)]
pub struct Logger {
    file_path_stdout: String,
    file_path_cc_cubic: String,
    file_path_cc_bbr: String,
    file_path_cc_pbe_init: String,
    file_path_cc_pbe_upper: String,
    file_path_cc_pbe_init_and_upper: String,
    file_path_cc_pbe_direct: String,
}

impl Logger {
    pub fn new() -> Result<Self, io::Error> {
        let run_timestamp =  chrono::Local::now().format("%Y-%m-%d_%H-%M");
        let run_dir = format!("{}/run_{}/", LOGS_DIRECTORY, run_timestamp);
        fs::create_dir_all(&run_dir)?;

        let file_path_stdout = format!(
            "{}run_{}_stdout.log",
            run_dir,
            run_timestamp,
        );
        let file_path_cc_cubic = format!(
            "{}run_{}_cc_cubic.jsonl",
            run_dir,
            run_timestamp,
        );
        let file_path_cc_bbr = format!(
            "{}run_{}_cc_bbr.jsonl",
            run_dir,
            run_timestamp,
        );
        let file_path_cc_pbe_init = format!(
            "{}run_{}_cc_pbe_init.jsonl",
            run_dir,
            run_timestamp,
        );
        let file_path_cc_pbe_upper = format!(
            "{}run_{}_cc_pbe_upper.jsonl",
            run_dir,
            run_timestamp,
        );
        let file_path_cc_pbe_init_and_upper = format!(
            "{}run_{}_cc_pbe_init_and_upper.jsonl",
            run_dir,
            run_timestamp,
        );
        let file_path_cc_pbe_direct = format!(
            "{}run_{}_cc_pbe_direct.jsonl",
            run_dir,
            run_timestamp,
        );

        Ok(Self {
            file_path_stdout,
            file_path_cc_cubic,
            file_path_cc_bbr,
            file_path_cc_pbe_init,
            file_path_cc_pbe_upper,
            file_path_cc_pbe_init_and_upper,
            file_path_cc_pbe_direct,
        })
    }


    pub fn log_transmission(&self, msg: &str, log_type: TransmissionType) -> Result<()> {
        let file_path = match log_type {
            TransmissionType::Bbr => &self.file_path_cc_bbr,
            TransmissionType::Cubic => &self.file_path_cc_cubic,
            TransmissionType::PbeInit => &self.file_path_cc_pbe_init,
            TransmissionType::PbeUpper => &self.file_path_cc_pbe_upper,
            TransmissionType::PbeInitAndUpper => &self.file_path_cc_pbe_init_and_upper,
            TransmissionType::PbeDirect => &self.file_path_cc_pbe_direct,
        };
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
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.file_path_stdout)?;
        writeln!(file, "{}", msg)?;
        file.flush()?;
        Ok(())
    }
}
