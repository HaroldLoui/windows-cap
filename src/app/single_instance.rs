//! 单实例：命名互斥体 + 激活已有实例。

use windows::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HWND, LPARAM, WPARAM,
};
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::WindowsAndMessaging::{FindWindowW, PostMessageW, WM_APP};
use windows::core::{PCWSTR, w};

use crate::util::error::Result;

pub const WM_APP_ACTIVATE: u32 = WM_APP + 20;

pub struct SingleInstance {
    handle: HANDLE,
    already_running: bool,
}

impl SingleInstance {
    pub fn acquire() -> Result<Self> {
        unsafe {
            let handle = CreateMutexW(None, false, w!("Local\\window-cap-singleton"))?;
            let already_running = GetLastError() == ERROR_ALREADY_EXISTS;
            Ok(Self {
                handle,
                already_running,
            })
        }
    }

    pub fn already_running(&self) -> bool {
        self.already_running
    }
}

impl Drop for SingleInstance {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}

/// 通知已运行的实例；成功返回 true。
pub fn activate_existing() -> bool {
    unsafe {
        let Ok(hwnd) = FindWindowW(w!("window_cap_hotkey"), PCWSTR::null()) else {
            return false;
        };
        if hwnd == HWND(std::ptr::null_mut()) {
            return false;
        }
        PostMessageW(Some(hwnd), WM_APP_ACTIVATE, WPARAM(0), LPARAM(0)).is_ok()
    }
}
