//! 日志：按级别写入 app / app-warn / app-error，按天归档并保留 3 天。

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use log::{Level, LevelFilter, Log, Metadata, Record};
use windows::Win32::System::SystemInformation::GetLocalTime;

use crate::util::error::Result;

const KEEP_DAYS: u64 = 3;
const KEEP_SECONDS: u64 = KEEP_DAYS * 24 * 60 * 60;

struct LogFiles {
    date: String,
    all: File,
    warn: File,
    error: File,
}

struct Logger {
    dir: PathBuf,
    files: Mutex<LogFiles>,
}

impl Log for Logger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let line = format_line(record);
        #[cfg(debug_assertions)]
        eprint!("{line}");

        let Ok(mut files) = self.files.lock() else {
            return;
        };
        let today = date_key();
        if files.date != today && rotate(&self.dir, &mut files, &today).is_err() {
            return;
        }
        let _ = files.all.write_all(line.as_bytes());
        if record.level() <= Level::Warn {
            let _ = files.warn.write_all(line.as_bytes());
        }
        if record.level() <= Level::Error {
            let _ = files.error.write_all(line.as_bytes());
        }
    }

    fn flush(&self) {
        if let Ok(mut files) = self.files.lock() {
            let _ = files.all.flush();
            let _ = files.warn.flush();
            let _ = files.error.flush();
        }
    }
}

/// 初始化日志：默认 Info 级别，清理 3 天前归档，轮转旧文件，打开今天的日志。
pub fn init() -> Result<()> {
    let Some(dir) = log_dir() else {
        return Ok(());
    };
    prune_old(&dir);
    remove_legacy(&dir);

    let today = date_key();
    let marker = dir.join(".app-date");
    if let Ok(old_date) = fs::read_to_string(&marker) {
        let old_date = old_date.trim();
        if !old_date.is_empty() && old_date != today {
            archive_current(&dir, old_date);
        }
    }
    let files = open_files(&dir, today.clone())?;
    let _ = fs::write(&marker, &today);

    let logger: &'static Logger = Box::leak(Box::new(Logger {
        dir,
        files: Mutex::new(files),
    }));
    let _ = log::set_logger(logger);
    log::set_max_level(LevelFilter::Info);
    Ok(())
}

fn format_line(record: &Record<'_>) -> String {
    let time = unsafe { GetLocalTime() };
    format!(
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
    )
}

fn today_parts() -> (u16, u16, u16) {
    let time = unsafe { GetLocalTime() };
    (time.wYear, time.wMonth, time.wDay)
}

fn date_key() -> String {
    let (year, month, day) = today_parts();
    format!("{year:04}-{month:02}-{day:02}")
}

fn log_dir() -> Option<PathBuf> {
    let base = std::env::var_os("LOCALAPPDATA").map(PathBuf::from)?;
    let dir = base.join("window-cap").join("logs");
    fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

fn open_file(path: &Path) -> Option<File> {
    OpenOptions::new().create(true).append(true).open(path).ok()
}

fn open_files(dir: &Path, date: String) -> Result<LogFiles> {
    let all = open_file(&dir.join("app.log"))
        .ok_or_else(|| crate::util::error::AppError::msg("打开 app.log 失败"))?;
    let warn = open_file(&dir.join("app-warn.log"))
        .ok_or_else(|| crate::util::error::AppError::msg("打开 app-warn.log 失败"))?;
    let error = open_file(&dir.join("app-error.log"))
        .ok_or_else(|| crate::util::error::AppError::msg("打开 app-error.log 失败"))?;
    Ok(LogFiles {
        date,
        all,
        warn,
        error,
    })
}

/// 日期切换时把当前日志归档为 `app-YYYY-MM-DD.log` 并打开新文件。
fn rotate(dir: &Path, files: &mut LogFiles, today: &str) -> Result<()> {
    let old_date = files.date.clone();
    files.all.flush().ok();
    files.warn.flush().ok();
    files.error.flush().ok();
    archive_current(dir, &old_date);
    *files = open_files(dir, today.to_string())?;
    let _ = fs::write(dir.join(".app-date"), today);
    Ok(())
}

fn archive_current(dir: &Path, date: &str) {
    for (current, archived) in [
        ("app.log", format!("app-{date}.log")),
        ("app-warn.log", format!("app-warn-{date}.log")),
        ("app-error.log", format!("app-error-{date}.log")),
    ] {
        let source = dir.join(current);
        if !source.exists() {
            continue;
        }
        let target = dir.join(archived);
        if let (Ok(mut from), Some(mut to)) = (
            File::open(&source),
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(&target)
                .ok(),
        ) {
            let _ = std::io::copy(&mut from, &mut to);
        }
        let _ = fs::remove_file(&source);
    }
}

fn prune_old(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        if !(name.starts_with("app-") && name.ends_with(".log")) {
            continue;
        }
        let old = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|modified| SystemTime::now().duration_since(modified).ok())
            .map(|age| age > Duration::from_secs(KEEP_SECONDS))
            .unwrap_or(false);
        if old {
            let _ = fs::remove_file(&path);
        }
    }
}

fn remove_legacy(dir: &Path) {
    for name in ["cli.log", "window-cap.log"] {
        let _ = fs::remove_file(dir.join(name));
    }
}
