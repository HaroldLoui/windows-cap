//! DPI 缩放工具：统一按 96 DPI 为 1.0。

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::HiDpi::GetDpiForWindow;

/// 以 96 DPI 为基准的缩放系数，最小 1.0。
pub fn scale_from_dpi(dpi: u32) -> f32 {
    (dpi as f32 / 96.0).max(1.0)
}

/// 读取窗口所在显示器的 DPI 缩放系数。
pub unsafe fn scale_from_hwnd(hwnd: HWND) -> f32 {
    unsafe { scale_from_dpi(GetDpiForWindow(hwnd)) }
}

/// 按缩放系数换算逻辑像素。
pub fn s(value: f32, scale: f32) -> i32 {
    (value * scale).round() as i32
}
