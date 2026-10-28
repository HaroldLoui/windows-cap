//! 窗口通用辅助：从 GWLP_USERDATA 取状态指针。

use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::HBRUSH;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CREATESTRUCTW, GWLP_USERDATA, GetWindowLongPtrW, RegisterClassW, SetWindowLongPtrW,
    WNDCLASS_STYLES, WNDCLASSW,
};
use windows::core::PCWSTR;

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

/// 把控件/窗口状态指针写入 GWLP_USERDATA。
pub unsafe fn bind(hwnd: HWND, pointer: *const std::ffi::c_void) {
    unsafe {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, pointer as isize);
    }
}

/// 从 WM_NCCREATE 的 lparam 取出创建参数并绑定状态。
pub unsafe fn bind_from_create(hwnd: HWND, lparam: LPARAM) {
    unsafe {
        if lparam.0 != 0 {
            let create = lparam.0 as *const CREATESTRUCTW;
            bind(hwnd, (*create).lpCreateParams);
        }
    }
}

/// 清空 GWLP_USERDATA。
pub unsafe fn unbind(hwnd: HWND) {
    unsafe {
        SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
    }
}

/// 注册窗口类；重复注册忽略。
pub unsafe fn register_class(
    name: PCWSTR,
    wndproc: unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT,
    style: WNDCLASS_STYLES,
) {
    unsafe {
        let hinstance = HINSTANCE(
            GetModuleHandleW(None)
                .map(|module| module.0)
                .unwrap_or(std::ptr::null_mut()),
        );
        let mut class: WNDCLASSW = std::mem::zeroed();
        class.style = style;
        class.lpfnWndProc = Some(wndproc);
        class.hInstance = hinstance;
        class.lpszClassName = name;
        class.hbrBackground = HBRUSH(std::ptr::null_mut());
        let _ = RegisterClassW(&class);
    }
}

/// 取出状态指针并清空 GWLP_USERDATA；调用方负责释放。
pub unsafe fn take_state<T>(hwnd: HWND) -> Option<Box<T>> {
    unsafe {
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut T;
        if pointer.is_null() {
            None
        } else {
            unbind(hwnd);
            Some(Box::from_raw(pointer))
        }
    }
}
