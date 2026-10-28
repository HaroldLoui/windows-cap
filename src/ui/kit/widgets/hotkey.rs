//! 热键录制控件：点击进入捕获态，只接受“修饰键 + 主键”组合。

use std::ffi::c_void;
use std::ptr::null_mut;
use std::sync::Once;

use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, DT_LEFT, DT_SINGLELINE, DT_VCENTER, DeleteObject, DrawTextW, EndPaint, FillRect,
    HFONT, HGDIOBJ, PAINTSTRUCT, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetFocus, GetKeyState, MOD_ALT, MOD_CONTROL, MOD_SHIFT, MOD_WIN, RegisterHotKey, SetFocus,
    UnregisterHotKey, VK_CONTROL, VK_ESCAPE, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CS_DBLCLKS, CreateWindowExW, DLGC_WANTALLKEYS, DefWindowProcW, ShowWindow, WS_CHILD,
    WS_TABSTOP, WS_VISIBLE,
};
use windows::core::{PCWSTR, w};

use crate::ui::kit::dpi::s;
use crate::ui::kit::text::wide;
use crate::ui::kit::theme::{ACCENT_COLORREF, WINDOW_BG, WINDOW_TEXT};
use crate::ui::kit::widgets::field;

const CLASS_NAME: PCWSTR = w!("window_cap_hotkey_field");
static REGISTER: Once = Once::new();

struct HotkeyState {
    value: String,
    capturing: bool,
    hint: Option<String>,
    font: HFONT,
    scale: f32,
    registered: Option<(u32, u32)>,
}

/// 创建热键录制控件。
#[allow(clippy::too_many_arguments)]
pub unsafe fn create(
    parent: HWND,
    id: usize,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    font: HFONT,
    scale: f32,
    initial: &str,
) -> HWND {
    unsafe {
        REGISTER.call_once(|| {
            crate::ui::kit::window::register_class(CLASS_NAME, hotkey_wndproc, CS_DBLCLKS);
        });
        let hinstance = HINSTANCE(GetModuleHandleW(None).map(|m| m.0).unwrap_or(null_mut()));
        let state = Box::into_raw(Box::new(HotkeyState {
            value: initial.to_string(),
            capturing: false,
            hint: None,
            font,
            scale,
            registered: None,
        }));
        let hwnd = CreateWindowExW(
            windows::Win32::UI::WindowsAndMessaging::WINDOW_EX_STYLE(0),
            CLASS_NAME,
            PCWSTR::null(),
            windows::Win32::UI::WindowsAndMessaging::WINDOW_STYLE(
                WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0,
            ),
            x,
            y,
            width,
            height,
            Some(parent),
            Some(windows::Win32::UI::WindowsAndMessaging::HMENU(
                id as isize as *mut c_void,
            )),
            Some(hinstance),
            Some(state as *const c_void),
        );
        match hwnd {
            Ok(hwnd) => {
                let _ = ShowWindow(hwnd, windows::Win32::UI::WindowsAndMessaging::SW_SHOW);
                hwnd
            }
            Err(_) => {
                drop(Box::from_raw(state));
                HWND(null_mut())
            }
        }
    }
}

/// 读取当前热键字符串。
pub unsafe fn value(hwnd: HWND) -> String {
    unsafe {
        state(hwnd)
            .map(|state| state.value.clone())
            .unwrap_or_default()
    }
}

unsafe fn state(hwnd: HWND) -> Option<&'static mut HotkeyState> {
    unsafe { crate::ui::kit::window::state(hwnd) }
}

unsafe fn redraw(hwnd: HWND) {
    unsafe {
        let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd), None, false);
    }
}

fn is_modifier(key: u16) -> bool {
    key == VK_CONTROL.0
        || key == VK_MENU.0
        || key == VK_SHIFT.0
        || key == VK_LWIN.0
        || key == VK_RWIN.0
}

fn key_name(key: u16) -> Option<String> {
    let name = match key {
        0x41..=0x5A => ((b'A' + (key as u8 - 0x41)) as char).to_string(),
        0x30..=0x39 => ((b'0' + (key as u8 - 0x30)) as char).to_string(),
        0x70..=0x87 => format!("F{}", key - 0x70 + 1),
        0x20 => "Space".to_string(),
        0x0D => "Enter".to_string(),
        0x09 => "Tab".to_string(),
        0x25 => "Left".to_string(),
        0x26 => "Up".to_string(),
        0x27 => "Right".to_string(),
        0x28 => "Down".to_string(),
        0x24 => "Home".to_string(),
        0x23 => "End".to_string(),
        0x21 => "PageUp".to_string(),
        0x22 => "PageDown".to_string(),
        0x2D => "Insert".to_string(),
        0x2E => "Delete".to_string(),
        0x08 => "Backspace".to_string(),
        _ => return None,
    };
    Some(name)
}

unsafe fn current_modifiers() -> u32 {
    unsafe {
        let mut modifiers = 0u32;
        if GetKeyState(VK_CONTROL.0 as i32) < 0 {
            modifiers |= MOD_CONTROL.0;
        }
        if GetKeyState(VK_MENU.0 as i32) < 0 {
            modifiers |= MOD_ALT.0;
        }
        if GetKeyState(VK_SHIFT.0 as i32) < 0 {
            modifiers |= MOD_SHIFT.0;
        }
        if GetKeyState(VK_LWIN.0 as i32) < 0 || GetKeyState(VK_RWIN.0 as i32) < 0 {
            modifiers |= MOD_WIN.0;
        }
        modifiers
    }
}

fn modifier_prefix(modifiers: u32) -> String {
    let mut parts = Vec::new();
    if modifiers & MOD_CONTROL.0 != 0 {
        parts.push("Ctrl");
    }
    if modifiers & MOD_ALT.0 != 0 {
        parts.push("Alt");
    }
    if modifiers & MOD_SHIFT.0 != 0 {
        parts.push("Shift");
    }
    if modifiers & MOD_WIN.0 != 0 {
        parts.push("Win");
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!("{}+", parts.join("+"))
    }
}

unsafe fn try_register(hwnd: HWND, modifiers: u32, key: u16) -> bool {
    unsafe {
        let Some(state) = state(hwnd) else {
            return false;
        };
        if let Some((old_modifiers, old_key)) = state.registered.take() {
            let _ = UnregisterHotKey(Some(hwnd), 1);
            let _ = (old_modifiers, old_key);
        }
        if RegisterHotKey(
            Some(hwnd),
            1,
            windows::Win32::UI::Input::KeyboardAndMouse::HOT_KEY_MODIFIERS(modifiers),
            key as u32,
        )
        .is_ok()
        {
            state.registered = Some((modifiers, key as u32));
            true
        } else {
            false
        }
    }
}

unsafe extern "system" fn hotkey_wndproc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        match message {
            windows::Win32::UI::WindowsAndMessaging::WM_NCCREATE => {
                crate::ui::kit::window::bind_from_create(hwnd, lparam);
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
            windows::Win32::UI::WindowsAndMessaging::WM_PAINT => {
                paint(hwnd);
                LRESULT(0)
            }
            windows::Win32::UI::WindowsAndMessaging::WM_LBUTTONDOWN => {
                if let Some(state) = state(hwnd) {
                    state.capturing = true;
                    state.hint = None;
                    let _ = SetFocus(Some(hwnd));
                    redraw(hwnd);
                }
                LRESULT(0)
            }
            windows::Win32::UI::WindowsAndMessaging::WM_KEYDOWN
            | windows::Win32::UI::WindowsAndMessaging::WM_SYSKEYDOWN => {
                key_down(hwnd, wparam.0 as u16);
                LRESULT(0)
            }
            windows::Win32::UI::WindowsAndMessaging::WM_KILLFOCUS => {
                if let Some(state) = state(hwnd) {
                    state.capturing = false;
                }
                redraw(hwnd);
                LRESULT(0)
            }
            windows::Win32::UI::WindowsAndMessaging::WM_GETDLGCODE => {
                LRESULT(DLGC_WANTALLKEYS as isize)
            }
            windows::Win32::UI::WindowsAndMessaging::WM_ERASEBKGND => LRESULT(1),
            windows::Win32::UI::WindowsAndMessaging::WM_DESTROY => {
                if let Some(state) = crate::ui::kit::window::take_state::<HotkeyState>(hwnd) {
                    if state.registered.is_some() {
                        let _ = UnregisterHotKey(Some(hwnd), 1);
                    }
                    drop(state);
                }
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, message, wparam, lparam),
        }
    }
}

unsafe fn key_down(hwnd: HWND, key: u16) {
    unsafe {
        let Some(state) = state(hwnd) else {
            return;
        };
        if !state.capturing {
            if key == 0x0D || key == 0x20 {
                state.capturing = true;
                redraw(hwnd);
            }
            return;
        }
        if key == VK_ESCAPE.0 {
            state.capturing = false;
            state.hint = None;
            redraw(hwnd);
            return;
        }
        let modifiers = current_modifiers();
        if is_modifier(key) {
            redraw(hwnd);
            return;
        }
        let Some(name) = key_name(key) else {
            return;
        };
        if modifiers == 0 {
            state.hint = Some("需要修饰键 + 主键".to_string());
            redraw(hwnd);
            return;
        }
        let combo = format!("{}{}", modifier_prefix(modifiers), name);
        if try_register(hwnd, modifiers, key) {
            state.value = combo;
            state.capturing = false;
            state.hint = None;
        } else {
            state.hint = Some("快捷键被占用".to_string());
        }
        redraw(hwnd);
    }
}

unsafe fn paint(hwnd: HWND) {
    unsafe {
        let mut paint = PAINTSTRUCT::default();
        let hdc = BeginPaint(hwnd, &mut paint);
        let mut rect = RECT::default();
        let _ = windows::Win32::UI::WindowsAndMessaging::GetClientRect(hwnd, &mut rect);
        let background = windows::Win32::Graphics::Gdi::CreateSolidBrush(WINDOW_BG);
        let _ = FillRect(hdc, &rect, background);
        let _ = DeleteObject(HGDIOBJ(background.0));

        let Some(state) = state(hwnd) else {
            let _ = EndPaint(hwnd, &paint);
            return;
        };
        let focused = GetFocus() == hwnd;
        field::draw(hdc, rect, focused, state.scale);
        let old_font = SelectObject(hdc, HGDIOBJ(state.font.0));
        SetBkMode(hdc, TRANSPARENT);
        let text = if state.capturing {
            "按下新快捷键…".to_string()
        } else {
            state.value.clone()
        };
        SetTextColor(hdc, WINDOW_TEXT);
        let mut text16 = wide(&text);
        let mut text_rect = RECT {
            left: rect.left + s(10.0, state.scale),
            top: rect.top,
            right: rect.right,
            bottom: rect.bottom,
        };
        DrawTextW(
            hdc,
            &mut text16,
            &mut text_rect,
            DT_LEFT | DT_VCENTER | DT_SINGLELINE,
        );
        if let Some(hint) = &state.hint {
            let mut hint16 = wide(hint);
            let mut hint_rect = RECT {
                left: rect.left + s(10.0, state.scale),
                top: rect.top,
                right: rect.right - s(10.0, state.scale),
                bottom: rect.bottom,
            };
            SetTextColor(hdc, ACCENT_COLORREF);
            DrawTextW(
                hdc,
                &mut hint16,
                &mut hint_rect,
                DT_LEFT | DT_VCENTER | DT_SINGLELINE,
            );
        }
        SelectObject(hdc, old_font);
        let _ = EndPaint(hwnd, &paint);
    }
}
