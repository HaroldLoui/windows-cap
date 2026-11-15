//! 配置文件监听：轮询文件内容指纹，变化时通知常驻窗口重载配置。

use std::ffi::c_void;
use std::path::Path;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_APP};

/// 配置文件变化通知消息。
pub const WM_APP_CONFIG_CHANGED: u32 = WM_APP + 21;

/// 轮询间隔：配置文件很小，1 秒内的延迟对人工编辑足够。
const POLL_INTERVAL: Duration = Duration::from_millis(1000);
/// 抖动等待：等编辑器完成落盘后再取稳定指纹，避免读到半写内容。
const SETTLE_DELAY: Duration = Duration::from_millis(300);

/// 配置文件监听线程；Drop 时停止并等待线程退出。
pub struct ConfigWatcher {
    stop: Sender<()>,
    handle: Option<JoinHandle<()>>,
}

impl ConfigWatcher {
    /// 在后台监听 `path` 的内容变化，变化后向 `hwnd` 发送 [`WM_APP_CONFIG_CHANGED`]。
    pub fn spawn(hwnd: HWND, path: &Path) -> Self {
        let (stop, stop_rx) = mpsc::channel();
        let window = hwnd.0 as isize;
        let path = path.to_path_buf();
        let handle = thread::Builder::new()
            .name("config-watch".to_string())
            .spawn(move || watch_loop(window, &path, &stop_rx))
            .ok();

        Self { stop, handle }
    }

    /// 停止监听并等待线程退出。
    pub fn stop(&mut self) {
        let _ = self.stop.send(());
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

impl Drop for ConfigWatcher {
    fn drop(&mut self) {
        self.stop();
    }
}

fn watch_loop(window: isize, path: &Path, stop: &Receiver<()>) {
    let mut seen = fingerprint(path);
    loop {
        if wait(stop, POLL_INTERVAL) {
            return;
        }
        if fingerprint(path) == seen {
            continue;
        }
        // 文件正在写入：再等一小段，取稳定指纹后只通知一次。
        if wait(stop, SETTLE_DELAY) {
            return;
        }
        let settled = fingerprint(path);
        seen = settled;
        // 文件被删除或暂时不存在时保持当前配置，等它重新出现。
        if settled.is_none() {
            continue;
        }
        if !post_change(window) {
            return;
        }
    }
}

/// 等待停止信号或超时；返回 true 表示应停止。
fn wait(stop: &Receiver<()>, duration: Duration) -> bool {
    match stop.recv_timeout(duration) {
        Ok(()) | Err(RecvTimeoutError::Disconnected) => true,
        Err(RecvTimeoutError::Timeout) => false,
    }
}

fn post_change(window: isize) -> bool {
    let hwnd = HWND(window as *mut c_void);
    // SAFETY: 窗口销毁后 PostMessageW 只返回错误，不会访问悬垂指针。
    unsafe { PostMessageW(Some(hwnd), WM_APP_CONFIG_CHANGED, WPARAM(0), LPARAM(0)).is_ok() }
}

/// 文件内容指纹；不存在或不可读时返回 None。
fn fingerprint(path: &Path) -> Option<u64> {
    std::fs::read(path).ok().map(|bytes| hash(&bytes))
}

/// FNV-1a：只用于判断内容是否变化，避免引入哈希依赖。
fn hash(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for &byte in bytes {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "window-cap-watch-{}-{}-{name}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_nanos())
                .unwrap_or(0)
        ))
    }

    #[test]
    fn fingerprint_tracks_content_changes() {
        let path = temp_path("fingerprint.toml");
        assert!(fingerprint(&path).is_none(), "缺失文件应返回 None");

        std::fs::write(&path, "[hotkey]\ncapture = \"Ctrl+Alt+A\"\n").expect("写入失败");
        let first = fingerprint(&path).expect("应有指纹");
        std::fs::write(&path, "[hotkey]\ncapture = \"Ctrl+Alt+B\"\n").expect("写入失败");
        let second = fingerprint(&path).expect("应有指纹");
        assert_ne!(first, second, "内容变化应改变指纹");

        std::fs::remove_file(&path).expect("删除失败");
        assert!(fingerprint(&path).is_none());
    }

    #[test]
    fn hash_is_deterministic() {
        assert_eq!(hash(b"window-cap"), hash(b"window-cap"));
        assert_ne!(hash(b"window-cap"), hash(b"window-caP"));
    }
}
