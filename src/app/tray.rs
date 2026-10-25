//! 托盘图标与右键菜单。

use std::mem::size_of;

use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
use windows::Win32::UI::Shell::{
    ExtractIconExW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIIF_ERROR, NIIF_INFO, NIM_ADD,
    NIM_DELETE, NIM_MODIFY, NIM_SETVERSION, NOTIFYICON_VERSION_4, NOTIFYICONDATAW,
    NOTIFYICONIDENTIFIER, Shell_NotifyIconGetRect, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, DestroyMenu, GetCursorPos, HICON, HMENU, IDI_APPLICATION,
    LoadIconW, MF_CHECKED, MF_SEPARATOR, MF_STRING, PostMessageW, SetForegroundWindow,
    TPM_LEFTALIGN, TPM_RETURNCMD, TPM_RIGHTBUTTON, TPM_TOPALIGN, TrackPopupMenu, WM_APP, WM_NULL,
};
use windows::core::PCWSTR;

use crate::util::error::{AppError, Result};

/// 托盘回调消息。
pub const WM_TRAY: u32 = WM_APP + 10;
const TRAY_ID: u32 = 1;

/// 托盘菜单动作。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayCommand {
    Capture,
    DelayedCapture,
    LastRegion,
    ReloadConfig,
    ToggleAutostart,
    OpenConfig,
    Quit,
}

impl TrayCommand {
    fn id(self) -> usize {
        match self {
            Self::Capture => 1,
            Self::DelayedCapture => 2,
            Self::LastRegion => 3,
            Self::ReloadConfig => 4,
            Self::ToggleAutostart => 5,
            Self::OpenConfig => 6,
            Self::Quit => 7,
        }
    }

    fn from_id(id: usize) -> Option<Self> {
        match id {
            1 => Some(Self::Capture),
            2 => Some(Self::DelayedCapture),
            3 => Some(Self::LastRegion),
            4 => Some(Self::ReloadConfig),
            5 => Some(Self::ToggleAutostart),
            6 => Some(Self::OpenConfig),
            7 => Some(Self::Quit),
            _ => None,
        }
    }
}

/// 托盘图标及其菜单。
pub struct Tray {
    hwnd: HWND,
    icon: HICON,
    added: bool,
}

impl Tray {
    /// 添加托盘图标。
    pub fn add(hwnd: HWND, tooltip: &str) -> Result<Self> {
        let icon = load_icon()?;
        let mut data = base_data(hwnd, icon);
        data.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
        data.uCallbackMessage = WM_TRAY;
        copy_wide(tooltip, &mut data.szTip);

        if unsafe { Shell_NotifyIconW(NIM_ADD, &data) }.0 == 0 {
            return Err(AppError::msg("添加托盘图标失败"));
        }

        unsafe {
            data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
            let _ = Shell_NotifyIconW(NIM_SETVERSION, &data);

            let identifier = NOTIFYICONIDENTIFIER {
                cbSize: size_of::<NOTIFYICONIDENTIFIER>() as u32,
                hWnd: hwnd,
                uID: TRAY_ID,
                guidItem: windows::core::GUID::default(),
            };
            match Shell_NotifyIconGetRect(&identifier) {
                Ok(rect) => ::log::info!(
                    "tray icon registered: ({}, {})-({}, {})",
                    rect.left,
                    rect.top,
                    rect.right,
                    rect.bottom
                ),
                Err(error) => ::log::warn!("tray icon rect query failed: {error}"),
            }
        }

        Ok(Self {
            hwnd,
            icon,
            added: true,
        })
    }

    /// 更新悬停提示。
    pub fn set_tooltip(&self, tooltip: &str) {
        let mut data = base_data(self.hwnd, self.icon);
        data.uFlags = NIF_TIP;
        copy_wide(tooltip, &mut data.szTip);
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &data);
        }
    }

    /// 显示气泡通知。
    pub fn notify(&self, message: &str, is_error: bool) {
        let mut data = base_data(self.hwnd, self.icon);
        data.uFlags = NIF_INFO;
        data.dwInfoFlags = if is_error { NIIF_ERROR } else { NIIF_INFO };
        copy_wide("window-cap", &mut data.szInfoTitle);
        copy_wide(message, &mut data.szInfo);
        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &data);
        }
    }

    /// 弹出右键菜单并返回用户选择。
    pub fn show_menu(
        &self,
        autostart: bool,
        capture_label: &str,
        last_region: Option<crate::capture::RectI>,
    ) -> Option<TrayCommand> {
        // 步骤 1：构建菜单项。
        let menu = unsafe { CreatePopupMenu().ok()? };
        let capture_text = format!("截图    {capture_label}");
        let last_text = match last_region {
            Some(rect) => format!("上次选区截图    {}×{}", rect.width, rect.height),
            None => "上次选区截图".to_string(),
        };

        unsafe {
            append(menu, MF_STRING, TrayCommand::Capture.id(), &capture_text);
            append(
                menu,
                MF_STRING,
                TrayCommand::DelayedCapture.id(),
                "延迟 3 秒截图",
            );
            append(menu, MF_STRING, TrayCommand::LastRegion.id(), &last_text);
            append(menu, MF_SEPARATOR, 0, "");
            append(
                menu,
                MF_STRING,
                TrayCommand::ReloadConfig.id(),
                "重新加载配置",
            );
            append(
                menu,
                MF_STRING,
                TrayCommand::OpenConfig.id(),
                "打开配置文件",
            );
            append(
                menu,
                if autostart {
                    MF_STRING | MF_CHECKED
                } else {
                    MF_STRING
                },
                TrayCommand::ToggleAutostart.id(),
                "开机自启",
            );
            append(menu, MF_SEPARATOR, 0, "");
            append(menu, MF_STRING, TrayCommand::Quit.id(), "退出");

            // 步骤 2：置前并弹出菜单。
            let mut point = POINT::default();
            let _ = GetCursorPos(&mut point);
            let _ = SetForegroundWindow(self.hwnd);
            let command = TrackPopupMenu(
                menu,
                TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_LEFTALIGN | TPM_TOPALIGN,
                point.x,
                point.y,
                Some(0),
                self.hwnd,
                None,
            );
            // 步骤 3：销毁菜单并补发 WM_NULL，避免菜单残留。
            let _ = DestroyMenu(menu);
            let _ = PostMessageW(Some(self.hwnd), WM_NULL, WPARAM(0), LPARAM(0));

            TrayCommand::from_id(command.0 as usize)
        }
    }
}

impl Drop for Tray {
    fn drop(&mut self) {
        if self.added {
            let data = base_data(self.hwnd, self.icon);
            unsafe {
                let _ = Shell_NotifyIconW(NIM_DELETE, &data);
            }
            self.added = false;
        }
    }
}

fn load_icon() -> Result<HICON> {
    if let Ok(executable) = std::env::current_exe() {
        let wide_path = wide(&executable.to_string_lossy());
        let mut large = HICON(std::ptr::null_mut());
        let mut small = HICON(std::ptr::null_mut());
        let count = unsafe {
            ExtractIconExW(
                PCWSTR(wide_path.as_ptr()),
                0,
                Some(&mut large),
                Some(&mut small),
                1,
            )
        };
        if count > 0 {
            let icon = if !small.0.is_null() { small } else { large };
            if !icon.0.is_null() {
                return Ok(icon);
            }
        }
    }

    unsafe {
        LoadIconW(None, IDI_APPLICATION)
            .map_err(|error| AppError::msg(format!("加载托盘图标失败: {error}")))
    }
}

fn base_data(hwnd: HWND, icon: HICON) -> NOTIFYICONDATAW {
    let mut data: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
    data.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
    data.hWnd = hwnd;
    data.uID = TRAY_ID;
    data.hIcon = icon;
    data
}

unsafe fn append(
    menu: HMENU,
    flags: windows::Win32::UI::WindowsAndMessaging::MENU_ITEM_FLAGS,
    id: usize,
    text: &str,
) {
    unsafe {
        if text.is_empty() {
            let _ = AppendMenuW(menu, flags, id, PCWSTR::null());
        } else {
            let wide = wide(text);
            let _ = AppendMenuW(menu, flags, id, PCWSTR(wide.as_ptr()));
        }
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn copy_wide(text: &str, destination: &mut [u16]) {
    let source = wide(text);
    let length = source.len().min(destination.len());
    destination[..length].copy_from_slice(&source[..length]);
    if length == destination.len() {
        destination[length - 1] = 0;
    }
}
