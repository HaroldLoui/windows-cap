//! 静态四色块测试窗口：验证坐标映射与采集一致性。

use std::time::Duration;

use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, DeleteObject, EndPaint, FillRect, HBRUSH, HDC, HGDIOBJ,
    PAINTSTRUCT, UpdateWindow,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, DestroyWindow, GetClientRect,
    GetWindowRect, RegisterClassW, SW_SHOW, ShowWindow, WM_ERASEBKGND, WM_PAINT, WNDCLASSW,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP, WS_VISIBLE,
};

use crate::capture::{RectI, VirtualScreen};
use crate::util::error::{AppError, Result};

const CLASS_NAME: windows::core::PCWSTR = windows::core::w!("window_cap_test_pattern");
const PATTERN_WIDTH: i32 = 400;
const PATTERN_HEIGHT: i32 = 300;

/// 静态测试窗口，Drop 时自动销毁。
pub struct TestPattern {
    hwnd: HWND,
}

impl TestPattern {
    /// 在虚拟桌面中央显示 400x300 的四色块窗口。
    pub fn show() -> Result<Self> {
        let screen = VirtualScreen::current();
        if screen.width < PATTERN_WIDTH || screen.height < PATTERN_HEIGHT {
            return Err(AppError::msg(format!(
                "屏幕太小，无法显示 {}x{} 测试窗",
                PATTERN_WIDTH, PATTERN_HEIGHT
            )));
        }

        let x = screen.x + (screen.width - PATTERN_WIDTH) / 2;
        let y = screen.y + (screen.height - PATTERN_HEIGHT) / 2;

        unsafe {
            let hinstance = HINSTANCE(GetModuleHandleW(None)?.0);
            let mut window_class: WNDCLASSW = std::mem::zeroed();
            window_class.style = CS_HREDRAW | CS_VREDRAW;
            window_class.lpfnWndProc = Some(pattern_wndproc);
            window_class.hInstance = hinstance;
            window_class.lpszClassName = CLASS_NAME;
            let _ = RegisterClassW(&window_class);

            let hwnd = CreateWindowExW(
                WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
                CLASS_NAME,
                windows::core::w!("window-cap test pattern"),
                WS_POPUP | WS_VISIBLE,
                x,
                y,
                PATTERN_WIDTH,
                PATTERN_HEIGHT,
                None,
                None,
                Some(hinstance),
                None,
            )?;

            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = UpdateWindow(hwnd);
            // 等 DWM 完成合成，避免抓到旧内容。
            std::thread::sleep(Duration::from_millis(150));

            Ok(Self { hwnd })
        }
    }

    /// 返回窗口的屏幕矩形。
    pub fn rect(&self) -> RectI {
        let mut rect = RECT::default();
        unsafe {
            let _ = GetWindowRect(self.hwnd, &mut rect);
        }
        RectI::new(
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
        )
    }

    /// 关闭并销毁窗口。
    pub fn close(self) {
        drop(self);
    }
}

impl Drop for TestPattern {
    fn drop(&mut self) {
        unsafe {
            if !self.hwnd.0.is_null() {
                let _ = DestroyWindow(self.hwnd);
            }
        }
    }
}

/// 窗口过程：擦除背景并按象限绘制四色块。
unsafe extern "system" fn pattern_wndproc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        match message {
            WM_ERASEBKGND => LRESULT(1),
            WM_PAINT => {
                let mut paint = PAINTSTRUCT::default();
                let hdc = BeginPaint(hwnd, &mut paint);

                // 步骤 1：把客户区均分为四个象限。
                let mut client = RECT::default();
                let _ = GetClientRect(hwnd, &mut client);
                let half_width = (client.right - client.left) / 2;
                let half_height = (client.bottom - client.top) / 2;

                // 步骤 2：按象限填充红、绿、蓝、白。
                fill(
                    hdc,
                    RECT {
                        left: 0,
                        top: 0,
                        right: half_width,
                        bottom: half_height,
                    },
                    0x000000FF,
                );
                fill(
                    hdc,
                    RECT {
                        left: half_width,
                        top: 0,
                        right: client.right,
                        bottom: half_height,
                    },
                    0x0000FF00,
                );
                fill(
                    hdc,
                    RECT {
                        left: 0,
                        top: half_height,
                        right: half_width,
                        bottom: client.bottom,
                    },
                    0x00FF0000,
                );
                fill(
                    hdc,
                    RECT {
                        left: half_width,
                        top: half_height,
                        right: client.right,
                        bottom: client.bottom,
                    },
                    0x00FFFFFF,
                );

                let _ = EndPaint(hwnd, &paint);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, message, wparam, lparam),
        }
    }
}

/// 用纯色画刷填充矩形，并释放画刷。
unsafe fn fill(hdc: HDC, rect: RECT, color: u32) {
    unsafe {
        let brush: HBRUSH = CreateSolidBrush(COLORREF(color));
        let _ = FillRect(hdc, &rect, brush);
        let _ = DeleteObject(HGDIOBJ(brush.0));
    }
}
