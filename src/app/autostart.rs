//! 开机自启：写入/删除 HKCU Run 项。

use std::ffi::c_void;
use std::ptr::null_mut;

use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, WIN32_ERROR};
use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_SZ, RRF_RT_REG_SZ, RegCloseKey, RegDeleteValueW,
    RegGetValueW, RegOpenKeyExW, RegSetValueExW,
};
use windows::core::w;

use crate::util::error::{AppError, Result};

pub fn is_enabled() -> bool {
    let mut buffer = [0u16; 1024];
    let mut size = (buffer.len() * 2) as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run"),
            w!("window-cap"),
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr() as *mut c_void),
            Some(&mut size),
        )
    };
    status == ERROR_SUCCESS && size > 2
}

pub fn set_enabled(enabled: bool) -> Result<()> {
    let mut key = HKEY(null_mut());
    let status = unsafe {
        RegOpenKeyExW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run"),
            None,
            KEY_SET_VALUE,
            &mut key,
        )
    };

    if enabled {
        check(status)?;
        let executable = std::env::current_exe()?;
        let command = format!("\"{}\"", executable.display());
        let wide: Vec<u16> = command.encode_utf16().chain(std::iter::once(0)).collect();
        let bytes =
            unsafe { std::slice::from_raw_parts(wide.as_ptr() as *const u8, wide.len() * 2) };
        let status = unsafe { RegSetValueExW(key, w!("window-cap"), None, REG_SZ, Some(bytes)) };
        unsafe {
            let _ = RegCloseKey(key);
        }
        check(status)
    } else {
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(());
        }
        check(status)?;
        let status = unsafe { RegDeleteValueW(key, w!("window-cap")) };
        unsafe {
            let _ = RegCloseKey(key);
        }
        if status == ERROR_SUCCESS || status == ERROR_FILE_NOT_FOUND {
            Ok(())
        } else {
            Err(registry_error(status))
        }
    }
}

/// 返回 `方式: 命令`；未启用时返回 `未启用`。
pub fn describe() -> String {
    let mut buffer = [0u16; 1024];
    let mut size = (buffer.len() * 2) as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run"),
            w!("window-cap"),
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr() as *mut c_void),
            Some(&mut size),
        )
    };
    if status != ERROR_SUCCESS || size <= 2 {
        return "未启用".to_string();
    }
    let length = (size as usize / 2).saturating_sub(1);
    let command = String::from_utf16_lossy(&buffer[..length.min(buffer.len())]);
    format!("已启用，命令: {command}")
}

pub fn run(action: super::cli::AutoStartAction) -> Result<()> {
    use super::cli::AutoStartAction;
    match action {
        AutoStartAction::On => {
            set_enabled(true)?;
            println!("开机自启: 已启用");
        }
        AutoStartAction::Off => {
            set_enabled(false)?;
            println!("开机自启: 已关闭");
        }
        AutoStartAction::Status => {
            println!("开机自启: {}", describe());
        }
    }
    Ok(())
}

fn check(status: WIN32_ERROR) -> Result<()> {
    if status == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(registry_error(status))
    }
}

fn registry_error(status: WIN32_ERROR) -> AppError {
    AppError::msg(format!("注册表操作失败（错误码 {}）", status.0))
}
