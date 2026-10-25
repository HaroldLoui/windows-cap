//! 日志初始化与 stderr/文件输出。

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

use log::{LevelFilter, Log, Metadata, Record};
use windows::Win32::System::SystemInformation::GetLocalTime;

use crate::util::error::Result;

struct Logger {
    level: LevelFilter,
    file: Option<Mutex<File>>,
}

impl Log for Logger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.level() <= self.level
    }

    fn log(&self, record: &Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }

        let time = unsafe { GetLocalTime() };
        let line = format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03} {:<5} {} {}\n",
            time.wYear,
            time.wMonth,
            time.wDay,
            time.wHour,
            time.wMinute,
            time.wSecond,
            time.wMilliseconds,
            record.level(),
            record.target(),
            record.args()
        );

        eprint!("{line}");
        if let Some(file) = &self.file
            && let Ok(mut file) = file.lock()
        {
            let _ = file.write_all(line.as_bytes());
        }
    }

    fn flush(&self) {
        if let Some(file) = &self.file
            && let Ok(mut file) = file.lock()
        {
            let _ = file.flush();
        }
    }
}

/// 初始化日志：stderr + `%LOCALAPPDATA%\window-cap\logs\window-cap.log`。
pub fn init(verbose: bool) -> Result<()> {
    let level = if verbose {
        LevelFilter::Debug
    } else {
        LevelFilter::Info
    };

    let file =
        log_file().and_then(|path| OpenOptions::new().create(true).append(true).open(path).ok());
    let logger = Logger {
        level,
        file: file.map(Mutex::new),
    };

    let logger: &'static Logger = Box::leak(Box::new(logger));
    let _ = log::set_logger(logger);
    log::set_max_level(level);
    Ok(())
}

fn log_file() -> Option<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from)?;
    let dir = base.join("window-cap").join("logs");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir.join("window-cap.log"))
}
