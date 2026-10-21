//! 单实例：命名互斥体 + 激活已有实例。

use std::ffi::c_void;

use windows::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HWND, LPARAM, WPARAM,
};
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::System::Threading::CreateMutexW;
use windows::Win32::UI::WindowsAndMessaging::{
    FindWindowW, PostMessageW, SendMessageW, WM_APP, WM_COPYDATA,
};
use windows::core::{PCWSTR, w};

use crate::util::error::Result;

pub const WM_APP_ACTIVATE: u32 = WM_APP + 20;
/// 转发消息标记（`WCAP`）。
pub const FORWARD_MAGIC: usize = 0x5743_4150;

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

/// 把命令行参数转发给已运行的实例（`\x1f` 分隔）。
pub fn forward_to_existing(args: &[String]) -> bool {
    unsafe {
        let Ok(hwnd) = FindWindowW(w!("window_cap_hotkey"), PCWSTR::null()) else {
            return false;
        };
        if hwnd == HWND(std::ptr::null_mut()) {
            return false;
        }

        let payload: Vec<u16> = args
            .join("\u{1f}")
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let data = COPYDATASTRUCT {
            dwData: FORWARD_MAGIC,
            cbData: (payload.len() * 2) as u32,
            lpData: payload.as_ptr() as *mut c_void,
        };

        SendMessageW(
            hwnd,
            WM_COPYDATA,
            Some(WPARAM(0)),
            Some(LPARAM(&data as *const COPYDATASTRUCT as isize)),
        )
        .0 != 0
    }
}
