//! 贴图窗口：置顶、带阴影的悬浮卡片；可拖动、Ctrl+滚轮缩放、复制/保存/关闭。

use std::ffi::c_void;
use std::mem::size_of;
use std::ptr::null_mut;

use windows::Win32::Foundation::{
    COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    AC_SRC_ALPHA, AC_SRC_OVER, BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION, BeginPaint,
    CreateCompatibleDC, CreateDIBSection, CreateSolidBrush, DEFAULT_GUI_FONT, DIB_RGB_COLORS,
    DT_CALCRECT, DT_CENTER, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, DeleteDC, DeleteObject,
    DrawTextW, EndPaint, FillRect, GetDC, GetStockObject, HGDIOBJ, InvalidateRect, PAINTSTRUCT,
    ReleaseDC, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, SetFocus, VK_CONTROL, VK_ESCAPE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CREATESTRUCTW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu,
    DestroyWindow, GWLP_USERDATA, GetClientRect, GetCursorPos, GetWindowLongPtrW, GetWindowRect,
    GetWindowTextW, IsWindow, MF_SEPARATOR, MF_STRING, PostMessageW, RegisterClassW,
    SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, SWP_SHOWWINDOW,
    SetForegroundWindow, SetTimer, SetWindowLongPtrW, SetWindowPos, SetWindowTextW, ShowWindow,
    TPM_LEFTALIGN, TPM_RETURNCMD, TPM_RIGHTBUTTON, TPM_TOPALIGN, TrackPopupMenu, ULW_ALPHA,
    UpdateLayeredWindow, WM_DESTROY, WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN,
    WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCCREATE, WM_NULL, WM_PAINT, WM_RBUTTONUP,
    WM_TIMER, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_POPUP, WS_VISIBLE,
};
use windows::core::{PCWSTR, w};

use crate::capture::Frame;
use crate::output::{clipboard, files};
use crate::util::error::Result;

const PIN_CLASS: PCWSTR = w!("window_cap_pin");
const HINT_CLASS: PCWSTR = w!("window_cap_pin_hint");
const ZOOM_CLASS: PCWSTR = w!("window_cap_pin_zoom");
const ACTIONS_CLASS: PCWSTR = w!("window_cap_pin_actions");
const SHADOW: i32 = 18;
const MIN_SIZE: i32 = 40;
const MAX_SIZE: i32 = 8000;
const HINT_TIMER: usize = 1;
const ZOOM_TIMER: usize = 1;
const ZOOM_HIDE_MS: u32 = 900;
const ACTIONS_WIDTH: i32 = 150;
const ACTIONS_HEIGHT: i32 = 30;
const HINT_TEXT: &str = "已贴图 · 拖动移动 · Ctrl+滚轮缩放 · Esc/双击关闭";

#[derive(Clone, Copy, PartialEq, Eq)]
enum HintPlacement {
    Right,
    Above,
}

struct DragState {
    cursor: (i32, i32),
    window: (i32, i32),
}

struct PinState {
    image: Frame,
    drag: Option<DragState>,
    hint: HWND,
    zoom: HWND,
    actions: HWND,
}

struct ActionsState {
    pin: HWND,
}

/// 创建一个贴图窗口（不阻塞，常驻消息循环继续处理）。
pub fn show(image: Frame) -> Result<()> {
    unsafe {
        let hinstance = HINSTANCE(GetModuleHandleW(None)?.0);
        register_class(hinstance, PIN_CLASS, pin_wndproc);
        register_class(hinstance, HINT_CLASS, hint_wndproc);
        register_class(hinstance, ZOOM_CLASS, zoom_wndproc);
        register_class(hinstance, ACTIONS_CLASS, actions_wndproc);

        let raw_state = Box::into_raw(Box::new(PinState {
            image,
            drag: None,
            hint: HWND(null_mut()),
            zoom: HWND(null_mut()),
            actions: HWND(null_mut()),
        }));
        let width = (*raw_state).image.width as i32;
        let height = (*raw_state).image.height as i32;
        let window_width = width + SHADOW * 2;
        let window_height = height + SHADOW * 2;
        let x = (*raw_state).image.origin.0 - SHADOW;
        let y = (*raw_state).image.origin.1 - SHADOW;

        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_LAYERED,
            PIN_CLASS,
            w!("window-cap pin"),
            WS_POPUP | WS_VISIBLE,
            x,
            y,
            window_width,
            window_height,
            None,
            None,
            Some(hinstance),
            Some(raw_state as *const c_void),
        );

        match hwnd {
            Ok(hwnd) => {
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    x,
                    y,
                    window_width,
                    window_height,
                    SWP_SHOWWINDOW | SWP_NOACTIVATE,
                );
                update_layer(hwnd);
                let hint = show_hint(hwnd, HintPlacement::Right, HINT_TEXT, 3200);
                let actions = create_actions(hwnd);
                if let Some(pin) = state(hwnd) {
                    pin.hint = hint;
                    pin.actions = actions;
                }
                let _ = SetForegroundWindow(hwnd);
                let _ = SetFocus(Some(hwnd));
                ::log::info!(
                    "pinned window created: {}x{} at ({}, {})",
                    width,
                    height,
                    x + SHADOW,
                    y + SHADOW
                );
                Ok(())
            }
            Err(error) => {
                drop(Box::from_raw(raw_state));
                Err(error.into())
            }
        }
    }
}

unsafe fn register_class(
    hinstance: HINSTANCE,
    name: PCWSTR,
    wndproc: unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT,
) {
    unsafe {
        let mut window_class: WNDCLASSW = std::mem::zeroed();
        window_class.lpfnWndProc = Some(wndproc);
        window_class.hInstance = hinstance;
        window_class.lpszClassName = name;
        let _ = RegisterClassW(&window_class);
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

/// 生成带阴影的分层窗口位图并提交。
unsafe fn update_layer(hwnd: HWND) {
    unsafe {
        let Some(pin) = state(hwnd) else {
            return;
        };
        let mut rect = RECT::default();
        if GetClientRect(hwnd, &mut rect).is_err() {
            return;
        }
        let width = rect.right.max(1);
        let height = rect.bottom.max(1);
        let content_width = (width - SHADOW * 2).max(1);
        let content_height = (height - SHADOW * 2).max(1);
        let mut buffer = vec![0u8; (width * height * 4) as usize];

        for y in 0..height {
            for x in 0..width {
                let index = ((y * width + x) * 4) as usize;
                if x >= SHADOW && x < width - SHADOW && y >= SHADOW && y < height - SHADOW {
                    let local_x = x - SHADOW;
                    let local_y = y - SHADOW;
                    let source_x = local_x * pin.image.width as i32 / content_width;
                    let source_y = local_y * pin.image.height as i32 / content_height;
                    let source = ((source_y * pin.image.width as i32 + source_x) * 4) as usize;
                    buffer[index] = pin.image.bgra[source];
                    buffer[index + 1] = pin.image.bgra[source + 1];
                    buffer[index + 2] = pin.image.bgra[source + 2];
                    buffer[index + 3] = 255;
                } else {
                    let dx = ((SHADOW - x).max(x - (width - SHADOW - 1))).max(0) as f32;
                    let dy = ((SHADOW - y).max(y - (height - SHADOW - 1))).max(0) as f32;
                    let distance = (dx * dx + dy * dy).sqrt();
                    let falloff = (1.0 - distance / SHADOW as f32).clamp(0.0, 1.0);
                    let alpha = (130.0 * falloff.powf(1.5)) as u8;
                    buffer[index] = 0;
                    buffer[index + 1] = 0;
                    buffer[index + 2] = 0;
                    buffer[index + 3] = alpha;
                }
            }
        }

        let mut info = BITMAPINFO::default();
        info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
        info.bmiHeader.biWidth = width;
        info.bmiHeader.biHeight = -height;
        info.bmiHeader.biPlanes = 1;
        info.bmiHeader.biBitCount = 32;
        info.bmiHeader.biCompression = 0;
        let memory_dc = CreateCompatibleDC(None);
        let mut bits: *mut c_void = null_mut();
        let Ok(bitmap) =
            CreateDIBSection(Some(memory_dc), &info, DIB_RGB_COLORS, &mut bits, None, 0)
        else {
            let _ = DeleteDC(memory_dc);
            return;
        };
        if !bits.is_null() {
            std::ptr::copy_nonoverlapping(buffer.as_ptr(), bits as *mut u8, buffer.len());
        }
        let old = SelectObject(memory_dc, HGDIOBJ(bitmap.0));

        let screen = GetDC(None);
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        let size = SIZE {
            cx: width,
            cy: height,
        };
        let _ = UpdateLayeredWindow(
            hwnd,
            Some(screen),
            None,
            Some(&size),
            Some(memory_dc),
            Some(&POINT { x: 0, y: 0 }),
            COLORREF(0),
            Some(&blend),
            ULW_ALPHA,
        );
        let _ = ReleaseDC(None, screen);
        SelectObject(memory_dc, old);
        let _ = DeleteObject(HGDIOBJ(bitmap.0));
        let _ = DeleteDC(memory_dc);
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
                let mut paint = PAINTSTRUCT::default();
                let _ = BeginPaint(hwnd, &mut paint);
                let _ = EndPaint(hwnd, &paint);
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
                let _ = SetFocus(Some(hwnd));
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
                        reposition_actions(hwnd);
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
                        let old_content_w = (old_w - SHADOW * 2).max(1);
                        let old_content_h = (old_h - SHADOW * 2).max(1);
                        let new_content_w = ((old_content_w as f32 * factor) as i32)
                            .clamp(MIN_SIZE, MAX_SIZE.min(pin.image.width as i32 * 8));
                        let new_content_h = ((old_content_h as f32 * factor) as i32)
                            .clamp(MIN_SIZE, MAX_SIZE.min(pin.image.height as i32 * 8));
                        let new_w = new_content_w + SHADOW * 2;
                        let new_h = new_content_h + SHADOW * 2;
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
                        update_layer(hwnd);
                        reposition_actions(hwnd);
                        let percent = ((new_content_w as f32 / pin.image.width as f32) * 100.0)
                            .round() as i32;
                        show_zoom(hwnd, percent);
                    }
                }
                LRESULT(0)
            }
            WM_KEYDOWN => {
                let key = wparam.0 as u16;
                let ctrl = GetKeyState(VK_CONTROL.0 as i32) < 0;
                if key == VK_ESCAPE.0 {
                    let _ = DestroyWindow(hwnd);
                } else if key == 0x0D || (ctrl && key == b'S' as u16) {
                    pin_save(hwnd);
                } else if ctrl && key == b'C' as u16 {
                    pin_copy(hwnd);
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
                    let state = Box::from_raw(pointer);
                    if IsWindow(Some(state.hint)).as_bool() {
                        let _ = DestroyWindow(state.hint);
                    }
                    if IsWindow(Some(state.zoom)).as_bool() {
                        let _ = DestroyWindow(state.zoom);
                    }
                    if IsWindow(Some(state.actions)).as_bool() {
                        let _ = DestroyWindow(state.actions);
                    }
                    drop(state);
                }
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, message, wparam, lparam),
        }
    }
}

/// 在贴图上方显示缩放百分比，短暂无操作后自动消失。
unsafe fn show_zoom(pin: HWND, percent: i32) {
    unsafe {
        let Some(pin_state) = state(pin) else {
            return;
        };
        let mut rect = RECT::default();
        if GetWindowRect(pin, &mut rect).is_err() {
            return;
        }
        let content_width = (rect.right - rect.left - SHADOW * 2).max(1);
        let width = 78;
        let height = 30;
        let x = rect.left + SHADOW + (content_width - width) / 2;
        let mut y = rect.top + SHADOW - height - 8;
        if y < 8 {
            y = rect.top + SHADOW + 8;
        }
        let text = format!("{percent}%");
        let wide_text = wide(&text);
        ::log::debug!(
            "show_zoom percent={percent} anchor=({x},{y}) existing={}",
            IsWindow(Some(pin_state.zoom)).as_bool()
        );

        if IsWindow(Some(pin_state.zoom)).as_bool() {
            let _ = SetWindowTextW(pin_state.zoom, PCWSTR(wide_text.as_ptr()));
            let _ = SetWindowPos(
                pin_state.zoom,
                None,
                x,
                y,
                width,
                height,
                SWP_NOZORDER | SWP_NOACTIVATE | SWP_SHOWWINDOW,
            );
            let _ = InvalidateRect(Some(pin_state.zoom), None, true);
            SetTimer(Some(pin_state.zoom), ZOOM_TIMER, ZOOM_HIDE_MS, None);
            return;
        }

        let Ok(hinstance) = GetModuleHandleW(None).map(|module| HINSTANCE(module.0)) else {
            return;
        };
        let zoom = match CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            ZOOM_CLASS,
            PCWSTR(wide_text.as_ptr()),
            WS_POPUP | WS_VISIBLE,
            x,
            y,
            width,
            height,
            None,
            None,
            Some(hinstance),
            None,
        ) {
            Ok(zoom) => zoom,
            Err(error) => {
                ::log::warn!("zoom window create failed: {error}");
                return;
            }
        };
        let _ = ShowWindow(zoom, SW_SHOWNOACTIVATE);
        SetTimer(Some(zoom), ZOOM_TIMER, ZOOM_HIDE_MS, None);
        pin_state.zoom = zoom;
        ::log::debug!("zoom window created: {zoom:?}");
    }
}

unsafe extern "system" fn zoom_wndproc(
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
                let mut rect = RECT::default();
                let _ = GetClientRect(hwnd, &mut rect);
                let background = CreateSolidBrush(COLORREF(0x0020_2020));
                FillRect(hdc, &rect, background);
                let _ = DeleteObject(HGDIOBJ(background.0));
                let border = CreateSolidBrush(COLORREF(0x0080_8080));
                let _ = windows::Win32::Graphics::Gdi::FrameRect(hdc, &rect, border);
                let _ = DeleteObject(HGDIOBJ(border.0));

                SetBkMode(hdc, TRANSPARENT);
                SetTextColor(hdc, COLORREF(0x00FF_FFFF));
                let old_font = SelectObject(hdc, GetStockObject(DEFAULT_GUI_FONT));
                let mut buffer = [0u16; 32];
                let length = GetWindowTextW(hwnd, &mut buffer);
                let text = String::from_utf16_lossy(&buffer[..length.max(0) as usize]);
                let mut wide_text = wide(&text);
                DrawTextW(
                    hdc,
                    &mut wide_text,
                    &mut rect,
                    DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
                );
                SelectObject(hdc, old_font);
                let _ = EndPaint(hwnd, &paint);
                LRESULT(0)
            }
            WM_TIMER => {
                if wparam.0 == ZOOM_TIMER {
                    let _ = DestroyWindow(hwnd);
                }
                LRESULT(0)
            }
            WM_LBUTTONDOWN => {
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, message, wparam, lparam),
        }
    }
}

unsafe fn pin_copy(pin: HWND) {
    unsafe {
        if let Some(state) = state(pin) {
            let _ = clipboard::put(&state.image);
        }
        show_toast(pin, "已复制");
    }
}

unsafe fn pin_save(pin: HWND) {
    unsafe {
        let mut saved = false;
        if let Some(state) = state(pin) {
            let path = files::default_path();
            match files::save_png(&state.image, &path) {
                Ok(_) => saved = true,
                Err(error) => ::log::warn!("pin save failed: {error}"),
            }
        }
        show_toast(
            pin,
            if saved {
                "已保存 PNG"
            } else {
                "保存失败"
            },
        );
    }
}

unsafe fn show_toast(pin: HWND, text: &str) {
    unsafe {
        if let Some(state) = state(pin)
            && IsWindow(Some(state.hint)).as_bool()
        {
            let _ = DestroyWindow(state.hint);
            state.hint = HWND(null_mut());
        }
        let hint = show_hint(pin, HintPlacement::Above, text, 1800);
        if let Some(state) = state(pin) {
            state.hint = hint;
        }
    }
}

/// 创建提示条（可放在贴图右侧或上方），到时自动消失。
unsafe fn show_hint(pin: HWND, placement: HintPlacement, text: &str, duration: u32) -> HWND {
    unsafe {
        let Ok(hinstance) = GetModuleHandleW(None).map(|module| HINSTANCE(module.0)) else {
            return HWND(null_mut());
        };
        let screen_width = windows::Win32::UI::WindowsAndMessaging::GetSystemMetrics(
            windows::Win32::UI::WindowsAndMessaging::SM_CXSCREEN,
        );
        let screen_height = windows::Win32::UI::WindowsAndMessaging::GetSystemMetrics(
            windows::Win32::UI::WindowsAndMessaging::SM_CYSCREEN,
        );
        let mut pin_rect = RECT::default();
        if GetWindowRect(pin, &mut pin_rect).is_err() {
            return HWND(null_mut());
        }
        let content_left = pin_rect.left + SHADOW;
        let content_top = pin_rect.top + SHADOW;
        let content_bottom = pin_rect.bottom - SHADOW;
        let content_width = (pin_rect.right - pin_rect.left - SHADOW * 2).max(1);

        let measure_dc = GetDC(None);
        let mut measured = RECT::default();
        let mut measured_text = wide(text);
        let old_font = SelectObject(measure_dc, GetStockObject(DEFAULT_GUI_FONT));
        DrawTextW(
            measure_dc,
            &mut measured_text,
            &mut measured,
            DT_CALCRECT | DT_SINGLELINE | DT_NOPREFIX,
        );
        SelectObject(measure_dc, old_font);
        let _ = ReleaseDC(None, measure_dc);

        let width = (measured.right - measured.left) + 24;
        let height = (measured.bottom - measured.top) + 14;
        let (mut x, mut y) = match placement {
            HintPlacement::Right => (content_left + content_width + 12, content_top),
            HintPlacement::Above => (
                content_left + (content_width - width) / 2,
                content_top - height - 8,
            ),
        };
        if x + width > screen_width - 8 {
            x = content_left - width - 12;
        }
        if x < 8 {
            x = 8;
        }
        if y < 8 {
            y = content_bottom + 8;
        }
        if y + height > screen_height - 8 {
            y = screen_height - height - 8;
        }

        let title = wide(text);
        let hint = match CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            HINT_CLASS,
            PCWSTR(title.as_ptr()),
            WS_POPUP | WS_VISIBLE,
            x,
            y,
            width,
            height,
            None,
            None,
            Some(hinstance),
            None,
        ) {
            Ok(hint) => hint,
            Err(_) => return HWND(null_mut()),
        };
        let _ = ShowWindow(hint, SW_SHOWNOACTIVATE);
        SetTimer(Some(hint), HINT_TIMER, duration, None);
        hint
    }
}

fn actions_position(pin_rect: &RECT) -> (i32, i32) {
    let content_top = pin_rect.top + SHADOW;
    let content_bottom = pin_rect.bottom - SHADOW;
    let content_right = pin_rect.right - SHADOW;
    let x = (content_right - ACTIONS_WIDTH).max(8);
    let mut y = content_top - ACTIONS_HEIGHT - 8;
    if y < 8 {
        y = content_bottom + 8;
    }
    (x, y)
}

unsafe fn create_actions(pin: HWND) -> HWND {
    unsafe {
        let Ok(hinstance) = GetModuleHandleW(None).map(|module| HINSTANCE(module.0)) else {
            return HWND(null_mut());
        };
        let mut pin_rect = RECT::default();
        if GetWindowRect(pin, &mut pin_rect).is_err() {
            return HWND(null_mut());
        }
        let (x, y) = actions_position(&pin_rect);
        let raw_state = Box::into_raw(Box::new(ActionsState { pin }));
        let actions = match CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            ACTIONS_CLASS,
            w!("window-cap pin actions"),
            WS_POPUP | WS_VISIBLE,
            x,
            y,
            ACTIONS_WIDTH,
            ACTIONS_HEIGHT,
            None,
            None,
            Some(hinstance),
            Some(raw_state as *const c_void),
        ) {
            Ok(actions) => actions,
            Err(_) => {
                drop(Box::from_raw(raw_state));
                return HWND(null_mut());
            }
        };
        let _ = ShowWindow(actions, SW_SHOWNOACTIVATE);
        actions
    }
}

unsafe fn reposition_actions(pin: HWND) {
    unsafe {
        let Some(state) = state(pin) else {
            return;
        };
        if !IsWindow(Some(state.actions)).as_bool() {
            return;
        }
        let mut pin_rect = RECT::default();
        if GetWindowRect(pin, &mut pin_rect).is_err() {
            return;
        }
        let (x, y) = actions_position(&pin_rect);
        let _ = SetWindowPos(
            state.actions,
            None,
            x,
            y,
            0,
            0,
            SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

unsafe extern "system" fn actions_wndproc(
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
                let mut paint = PAINTSTRUCT::default();
                let hdc = BeginPaint(hwnd, &mut paint);
                let background = CreateSolidBrush(COLORREF(0x0020_2020));
                let rect = RECT {
                    left: 0,
                    top: 0,
                    right: ACTIONS_WIDTH,
                    bottom: ACTIONS_HEIGHT,
                };
                FillRect(hdc, &rect, background);
                let _ = DeleteObject(HGDIOBJ(background.0));

                let labels = ["复制", "保存", "关闭"];
                let cell_width = ACTIONS_WIDTH / 3;
                SetBkMode(hdc, TRANSPARENT);
                let old_font = SelectObject(hdc, GetStockObject(DEFAULT_GUI_FONT));
                for (index, label) in labels.iter().enumerate() {
                    let left = cell_width * index as i32;
                    let mut cell = RECT {
                        left,
                        top: 0,
                        right: left + cell_width,
                        bottom: ACTIONS_HEIGHT,
                    };
                    SetTextColor(hdc, COLORREF(0x00E0_E0E0));
                    let mut wide_label = wide(label);
                    DrawTextW(
                        hdc,
                        &mut wide_label,
                        &mut cell,
                        DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
                    );
                }
                SelectObject(hdc, old_font);
                let _ = EndPaint(hwnd, &paint);
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                let Some(actions) = actions_state(hwnd) else {
                    return LRESULT(0);
                };
                let _ = SetForegroundWindow(actions.pin);
                let _ = SetFocus(Some(actions.pin));
                let x = (lparam.0 & 0xFFFF) as i16 as i32;
                if x >= 0 {
                    match (x / (ACTIONS_WIDTH / 3)).clamp(0, 2) {
                        0 => pin_copy(actions.pin),
                        1 => pin_save(actions.pin),
                        2 => {
                            let _ = DestroyWindow(actions.pin);
                        }
                        _ => {}
                    }
                }
                LRESULT(0)
            }
            WM_DESTROY => {
                let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut ActionsState;
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

unsafe fn actions_state(hwnd: HWND) -> Option<&'static mut ActionsState> {
    unsafe {
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut ActionsState;
        if pointer.is_null() {
            None
        } else {
            Some(&mut *pointer)
        }
    }
}

unsafe extern "system" fn hint_wndproc(
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
                let mut rect = RECT::default();
                let _ = GetClientRect(hwnd, &mut rect);
                let background = CreateSolidBrush(COLORREF(0x0020_2020));
                FillRect(hdc, &rect, background);
                let _ = DeleteObject(HGDIOBJ(background.0));

                SetBkMode(hdc, TRANSPARENT);
                SetTextColor(hdc, COLORREF(0x00FF_FFFF));
                let old_font = SelectObject(hdc, GetStockObject(DEFAULT_GUI_FONT));
                let mut buffer = [0u16; 256];
                let length = GetWindowTextW(hwnd, &mut buffer);
                let text = String::from_utf16_lossy(&buffer[..length.max(0) as usize]);
                let mut text = wide(&text);
                DrawTextW(
                    hdc,
                    &mut text,
                    &mut rect,
                    DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
                );
                SelectObject(hdc, old_font);
                let _ = EndPaint(hwnd, &paint);
                LRESULT(0)
            }
            WM_TIMER => {
                if wparam.0 == HINT_TIMER {
                    let _ = DestroyWindow(hwnd);
                }
                LRESULT(0)
            }
            WM_KEYDOWN | WM_LBUTTONDOWN => {
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, message, wparam, lparam),
        }
    }
}
