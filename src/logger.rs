use log::{Level, Log, Metadata, Record};
use std::fs::{create_dir_all, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

pub struct FileLogger {
    file: Mutex<std::fs::File>,
    level: Level,
}

impl FileLogger {
    pub fn init(level: Level) -> Result<(), Box<dyn std::error::Error>> {
        let log_dir = PathBuf::from("logs");
        create_dir_all(&log_dir)?;

        let date = chrono::Local::now().format("%Y-%m-%d");
        let log_file = log_dir.join(format!("jjdag-{}.log", date));

        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_file)?;

        let logger = Box::new(FileLogger {
            file: Mutex::new(file),
            level,
        });

        log::set_boxed_logger(logger)?;
        log::set_max_level(level.to_level_filter());

        Ok(())
    }
}

impl Log for FileLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= self.level
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }

        let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
        let level = record.level();
        let target = record.target();
        let args = record.args();

        let line = format!("[{}] [{}] {}: {}\n", timestamp, level, target, args);

        if let Ok(mut file) = self.file.lock() {
            let _ = file.write_all(line.as_bytes());
            let _ = file.flush();
        }
    }

    fn flush(&self) {
        if let Ok(mut file) = self.file.lock() {
            let _ = file.flush();
        }
    }
}
