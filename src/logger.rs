use std::fs::OpenOptions;
use std::io::{self, Write};
use std::fs::{File, self};

const LOGS_DIRECTORY: &str = ".logs";


pub struct Logger {
    file: File,
}

impl Logger {
    pub fn new() -> Result<Self, io::Error> {
        fs::create_dir_all(LOGS_DIRECTORY)?;

        let filename = format!(
            "{}/run_{}.jsonl",
            LOGS_DIRECTORY,
            chrono::Local::now().format("%Y-%m-%d_%H-%M")
        );

        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(filename)?;

        Ok(Self { file })
    }

    pub fn log(&mut self, json_str: &str) -> Result<(), io::Error> {
        writeln!(self.file, "{}", json_str)?;
        self.file.flush()?;
        Ok(())
    }
}


