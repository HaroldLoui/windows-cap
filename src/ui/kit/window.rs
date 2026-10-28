//! 窗口通用辅助：从 GWLP_USERDATA 取状态指针。

use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{GWLP_USERDATA, GetWindowLongPtrW};

/// 读取窗口创建时存入的 `Box<T>` 指针。
pub unsafe fn state<T>(hwnd: HWND) -> Option<&'static mut T> {
    unsafe {
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut T;
        if pointer.is_null() {
            None
        } else {
            Some(&mut *pointer)
        }
    }
}
