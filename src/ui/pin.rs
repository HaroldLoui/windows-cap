//! 贴图窗口：把截图固定到屏幕最上层，可拖动、Ctrl+滚轮缩放、复制/保存/关闭。

use std::ffi::c_void;
use std::mem::size_of;

use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BeginPaint, DIB_RGB_COLORS, EndPaint, HALFTONE,
    PAINTSTRUCT, SRCCOPY, SetStretchBltMode, StretchDIBits,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, VK_CONTROL, VK_ESCAPE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CREATESTRUCTW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu,
    DestroyWindow, GWLP_USERDATA, GetClientRect, GetCursorPos, GetWindowLongPtrW, GetWindowRect,
    MF_SEPARATOR, MF_STRING, PostMessageW, RegisterClassW, SWP_NOACTIVATE, SWP_NOSIZE,
    SWP_NOZORDER, SWP_SHOWWINDOW, SetForegroundWindow, SetWindowLongPtrW, SetWindowPos,
    TPM_LEFTALIGN, TPM_RETURNCMD, TPM_RIGHTBUTTON, TPM_TOPALIGN, TrackPopupMenu, WM_DESTROY,
    WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
    WM_MOUSEWHEEL, WM_NCCREATE, WM_NULL, WM_PAINT, WM_RBUTTONUP, WNDCLASSW, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_POPUP, WS_VISIBLE,
};
use windows::core::{PCWSTR, w};

use crate::capture::Frame;
use crate::output::{clipboard, files};
use crate::util::error::Result;

const CLASS_NAME: PCWSTR = w!("window_cap_pin");
const MIN_SIZE: i32 = 40;
const MAX_SIZE: i32 = 8000;

struct DragState {
    cursor: (i32, i32),
    window: (i32, i32),
}

struct PinState {
    image: Frame,
    drag: Option<DragState>,
}

/// 创建一个贴图窗口（不阻塞，常驻消息循环继续处理）。
pub fn show(image: Frame) -> Result<()> {
    unsafe {
        let hinstance = HINSTANCE(GetModuleHandleW(None)?.0);
        let mut window_class: WNDCLASSW = std::mem::zeroed();
        window_class.lpfnWndProc = Some(pin_wndproc);
        window_class.hInstance = hinstance;
        window_class.lpszClassName = CLASS_NAME;
        let _ = RegisterClassW(&window_class);

        let state = Box::into_raw(Box::new(PinState { image, drag: None }));
        let width = (*state).image.width as i32;
        let height = (*state).image.height as i32;
        let x = (*state).image.origin.0;
        let y = (*state).image.origin.1;

        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            CLASS_NAME,
            w!("window-cap pin"),
            WS_POPUP | WS_VISIBLE,
            x,
            y,
            width,
            height,
            None,
            None,
            Some(hinstance),
            Some(state as *const c_void),
        );

        match hwnd {
            Ok(hwnd) => {
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    x,
                    y,
                    width,
                    height,
                    SWP_SHOWWINDOW | SWP_NOACTIVATE,
                );
                let _ = SetForegroundWindow(hwnd);
                ::log::info!(
                    "pinned window created: {}x{} at ({}, {})",
                    width,
                    height,
                    x,
                    y
                );
                Ok(())
            }
            Err(error) => {
                drop(Box::from_raw(state));
                Err(error.into())
            }
        }
    }
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

unsafe fn state(hwnd: HWND) -> Option<&'static mut PinState> {
    unsafe {
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut PinState;
        if pointer.is_null() {
            None
        } else {
            Some(&mut *pointer)
        }
    }
}

unsafe fn paint(hwnd: HWND) {
    unsafe {
        let mut paint = PAINTSTRUCT::default();
        let hdc = BeginPaint(hwnd, &mut paint);
        if hdc.0.is_null() {
            return;
        }
        let Some(pin) = state(hwnd) else {
            let _ = EndPaint(hwnd, &paint);
            return;
        };

        let mut rect = RECT::default();
        let _ = GetClientRect(hwnd, &mut rect);
        let mut info = BITMAPINFO::default();
        info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = pin.image.width as i32;
        info.bmiHeader.biHeight = -(pin.image.height as i32);
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        info.bmiHeader.biCompression = BI_RGB.0;

        let _ = SetStretchBltMode(hdc, HALFTONE);
        let _ = StretchDIBits(
            hdc,
            0,
            0,
            rect.right,
            rect.bottom,
            0,
            0,
            pin.image.width as i32,
            pin.image.height as i32,
            Some(pin.image.bgra.as_ptr() as *const c_void),
            &info,
            DIB_RGB_COLORS,
            SRCCOPY,
        );
        let _ = EndPaint(hwnd, &paint);
    }
}

unsafe fn show_menu(hwnd: HWND) {
    unsafe {
        let Some(pin) = state(hwnd) else {
            return;
        };
        let Ok(menu) = CreatePopupMenu() else {
            return;
        };
        let copy = wide("复制");
        let save = wide("保存 PNG");
        let close = wide("关闭");
        let _ = AppendMenuW(menu, MF_STRING, 1, PCWSTR(copy.as_ptr()));
        let _ = AppendMenuW(menu, MF_STRING, 2, PCWSTR(save.as_ptr()));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let _ = AppendMenuW(menu, MF_STRING, 3, PCWSTR(close.as_ptr()));

        let mut point = POINT::default();
        let _ = GetCursorPos(&mut point);
        let _ = SetForegroundWindow(hwnd);
        let command = TrackPopupMenu(
            menu,
            TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_LEFTALIGN | TPM_TOPALIGN,
            point.x,
            point.y,
            Some(0),
            hwnd,
            None,
        );
        let _ = DestroyMenu(menu);
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));

        match command.0 {
            1 => {
                let _ = clipboard::put(&pin.image);
            }
            2 => {
                let path = files::default_path();
                if let Err(error) = files::save_png(&pin.image, &path) {
                    ::log::warn!("pin save failed: {error}");
                }
            }
            3 => {
                let _ = DestroyWindow(hwnd);
            }
            _ => {}
        }
    }
}

unsafe extern "system" fn pin_wndproc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        match message {
            WM_NCCREATE => {
                let create = lparam.0 as *const CREATESTRUCTW;
                if !create.is_null() {
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, (*create).lpCreateParams as isize);
                }
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_PAINT => {
                paint(hwnd);
                LRESULT(0)
            }
            WM_LBUTTONDOWN => {
                if let Some(pin) = state(hwnd) {
                    let mut cursor = POINT::default();
                    let mut rect = RECT::default();
                    if GetCursorPos(&mut cursor).is_ok() && GetWindowRect(hwnd, &mut rect).is_ok() {
                        pin.drag = Some(DragState {
                            cursor: (cursor.x, cursor.y),
                            window: (rect.left, rect.top),
                        });
                        SetCapture(hwnd);
                    }
                }
                let _ = SetForegroundWindow(hwnd);
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                if let Some(pin) = state(hwnd)
                    && let Some(drag) = &pin.drag
                {
                    let mut cursor = POINT::default();
                    if GetCursorPos(&mut cursor).is_ok() {
                        let x = drag.window.0 + cursor.x - drag.cursor.0;
                        let y = drag.window.1 + cursor.y - drag.cursor.1;
                        let _ = SetWindowPos(
                            hwnd,
                            None,
                            x,
                            y,
                            0,
                            0,
                            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                        );
                    }
                }
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                let _ = ReleaseCapture();
                if let Some(pin) = state(hwnd) {
                    pin.drag = None;
                }
                LRESULT(0)
            }
            WM_LBUTTONDBLCLK => {
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_MOUSEWHEEL => {
                let ctrl = GetKeyState(VK_CONTROL.0 as i32) < 0;
                if ctrl && let Some(pin) = state(hwnd) {
                    let delta = ((wparam.0 >> 16) & 0xFFFF) as u16 as i16;
                    let factor = if delta > 0 { 1.1 } else { 1.0 / 1.1 };
                    let mut rect = RECT::default();
                    let mut cursor = POINT::default();
                    if GetWindowRect(hwnd, &mut rect).is_ok() && GetCursorPos(&mut cursor).is_ok() {
                        let old_w = (rect.right - rect.left).max(1);
                        let old_h = (rect.bottom - rect.top).max(1);
                        let new_w = ((old_w as f32 * factor) as i32)
                            .clamp(MIN_SIZE, MAX_SIZE.min(pin.image.width as i32 * 8));
                        let new_h = ((old_h as f32 * factor) as i32)
                            .clamp(MIN_SIZE, MAX_SIZE.min(pin.image.height as i32 * 8));
                        let ratio_x = new_w as f32 / old_w as f32;
                        let ratio_y = new_h as f32 / old_h as f32;
                        let x = cursor.x - ((cursor.x - rect.left) as f32 * ratio_x) as i32;
                        let y = cursor.y - ((cursor.y - rect.top) as f32 * ratio_y) as i32;
                        let _ = SetWindowPos(
                            hwnd,
                            None,
                            x,
                            y,
                            new_w,
                            new_h,
                            SWP_NOZORDER | SWP_NOACTIVATE,
                        );
                        let _ =
                            windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd), None, false);
                    }
                }
                LRESULT(0)
            }
            WM_KEYDOWN => {
                let key = wparam.0 as u16;
                let ctrl = GetKeyState(VK_CONTROL.0 as i32) < 0;
                if key == VK_ESCAPE.0 {
                    let _ = DestroyWindow(hwnd);
                } else if ctrl
                    && key == b'C' as u16
                    && let Some(pin) = state(hwnd)
                {
                    let _ = clipboard::put(&pin.image);
                }
                LRESULT(0)
            }
            WM_RBUTTONUP => {
                show_menu(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut PinState;
                if !pointer.is_null() {
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                    drop(Box::from_raw(pointer));
                }
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, message, wparam, lparam),
        }
    }
}
