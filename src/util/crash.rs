//! 崩溃诊断：panic 钩子、常驻会话标记与上次异常退出检测。

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use windows::Win32::System::SystemInformation::GetLocalTime;

/// 常驻会话标记：正常退出时删除，残留即表示上次异常退出。
const SESSION_FILE: &str = "session.flag";
/// 最近一次 panic 的摘要，便于用户直接回传。
const CRASH_FILE: &str = "last-crash.txt";

/// 安装 panic 钩子：panic 时把现场写入 `app-error.log` 与 `last-crash.txt`。
///
/// 这里直接写文件而不走 `log` 宏，避免 panic 发生在日志锁内时二次卡死。
pub fn install(dir: Option<PathBuf>) {
    std::panic::set_hook(Box::new(move |info| {
        let report = format_report(info);
        #[cfg(debug_assertions)]
        eprint!("{report}");
        let Some(dir) = &dir else {
            return;
        };
        append(&dir.join("app-error.log"), &report);
        let _ = fs::write(dir.join(CRASH_FILE), &report);
    }));
}

/// 启动常驻模式前调用：若发现上次会话标记，读取后清除，返回其内容。
pub fn take_previous_session(dir: Option<&Path>) -> Option<String> {
    let path = dir?.join(SESSION_FILE);
    let content = fs::read_to_string(&path).ok()?;
    let _ = fs::remove_file(&path);
    Some(content.trim().to_string())
}

/// 会话守卫：写入标记，正常退出（Drop）时删除。
///
/// release 使用 `panic=abort`，panic 时 Drop 不会执行，标记因此留存到下次启动。
pub struct SessionGuard {
    path: Option<PathBuf>,
}

impl SessionGuard {
    /// 写入本次会话标记。
    pub fn start(dir: Option<&Path>) -> Self {
        let path = dir.map(|dir| dir.join(SESSION_FILE));
        if let Some(path) = &path {
            let _ = fs::write(path, session_info());
        }
        Self { path }
    }
}

impl Drop for SessionGuard {
    fn drop(&mut self) {
        if let Some(path) = &self.path {
            let _ = fs::remove_file(path);
        }
    }
}

fn format_report(info: &std::panic::PanicHookInfo<'_>) -> String {
    let thread = std::thread::current()
        .name()
        .unwrap_or("<unnamed>")
        .to_string();
    let message = info
        .payload()
        .downcast_ref::<&str>()
        .map(|text| (*text).to_string())
        .or_else(|| info.payload().downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "<非字符串 panic 负载>".to_string());
    let location = info
        .location()
        .map(|location| {
            format!(
                "{}:{}:{}",
                location.file(),
                location.line(),
                location.column()
            )
        })
        .unwrap_or_else(|| "<unknown>".to_string());
    // backtrace 由 std 提供，不引入额外依赖；发行版 strip 后仍能看到调用帧地址。
    let backtrace = std::backtrace::Backtrace::force_capture();
    format!(
        "{} PANIC thread={thread} at {location}: {message}\n{backtrace}\n",
        timestamp()
    )
}

fn session_info() -> String {
    let args: Vec<String> = std::env::args().collect();
    format!(
        "pid={} started={} args={}",
        std::process::id(),
        timestamp(),
        args.join(" ")
    )
}

fn timestamp() -> String {
    let time = unsafe { GetLocalTime() };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
        time.wYear,
        time.wMonth,
        time.wDay,
        time.wHour,
        time.wMinute,
        time.wSecond,
        time.wMilliseconds
    )
}

fn append(path: &Path, text: &str) {
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = file.write_all(text.as_bytes());
        let _ = file.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir() -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_nanos())
            .unwrap_or(0);
        let dir =
            std::env::temp_dir().join(format!("window-cap-crash-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&dir).expect("创建临时目录");
        dir
    }

    #[test]
    fn clean_exit_removes_session_marker() {
        let dir = temp_dir();
        {
            let _guard = SessionGuard::start(Some(&dir));
            assert!(dir.join(SESSION_FILE).exists());
        }
        assert!(!dir.join(SESSION_FILE).exists());
        assert_eq!(take_previous_session(Some(&dir)), None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn lingering_session_marker_reports_previous_crash() {
        let dir = temp_dir();
        let guard = SessionGuard::start(Some(&dir));
        std::mem::forget(guard); // 模拟异常退出：Drop 未执行
        let info = take_previous_session(Some(&dir)).expect("应检测到上次异常退出");
        assert!(info.contains("pid="));
        assert!(!dir.join(SESSION_FILE).exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn panic_hook_writes_crash_report() {
        let dir = temp_dir();
        install(Some(dir.clone()));
        let result = std::panic::catch_unwind(|| panic!("boom test"));
        assert!(result.is_err());
        let crash = fs::read_to_string(dir.join(CRASH_FILE)).expect("应写入 last-crash.txt");
        assert!(crash.contains("boom test"));
        assert!(crash.contains("PANIC"));
        let error_log =
            fs::read_to_string(dir.join("app-error.log")).expect("应写入 app-error.log");
        assert!(error_log.contains("boom test"));
        let _ = fs::remove_dir_all(&dir);
    }
}
