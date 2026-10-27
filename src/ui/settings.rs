//! 设置窗口：用原生控件编辑 config.toml 的常用项。

use std::ffi::c_void;
use std::mem::size_of;
use std::path::{Path, PathBuf};
use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, Ordering};

use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute};
use windows::Win32::Graphics::Gdi::{
    CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateFontW, CreatePen, CreateSolidBrush,
    DEFAULT_CHARSET, DEFAULT_PITCH, DT_CENTER, DT_SINGLELINE, DT_VCENTER, DeleteObject, DrawTextW,
    FillRect, HBRUSH, HDC, HFONT, HGDIOBJ, OUT_DEFAULT_PRECIS, PS_SOLID, RoundRect, SelectObject,
    SetBkColor, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{DRAWITEMSTRUCT, ODS_SELECTED, SetWindowTheme};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Shell::{
    BIF_NEWDIALOGSTYLE, BIF_RETURNONLYFSDIRS, BROWSEINFOW, SHBrowseForFolderW, SHGetPathFromIDListW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    BS_OWNERDRAW, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GWLP_USERDATA,
    GetClientRect, GetMessageW, GetSystemMetrics, GetWindowLongPtrW, GetWindowTextLengthW,
    GetWindowTextW, HMENU, IsDialogMessageW, MB_ICONERROR, MB_OK, MSG, MessageBoxW,
    PostQuitMessage, RegisterClassW, SM_CXSCREEN, SM_CYSCREEN, SW_SHOWNORMAL, SendMessageW,
    SetForegroundWindow, SetWindowLongPtrW, SetWindowTextW, ShowWindow, TranslateMessage, WM_CLOSE,
    WM_COMMAND, WM_CREATE, WM_CTLCOLORBTN, WM_CTLCOLOREDIT, WM_CTLCOLORSTATIC, WM_DESTROY,
    WM_DRAWITEM, WM_ERASEBKGND, WM_NCCREATE, WM_SETFONT, WNDCLASSW, WS_BORDER, WS_CAPTION,
    WS_CHILD, WS_MINIMIZEBOX, WS_OVERLAPPED, WS_SYSMENU, WS_TABSTOP, WS_VISIBLE,
};
use windows::core::{PCWSTR, w};

use crate::app::autostart;
use crate::app::config::{Config, Hotkey};
use crate::util::error::Result;

const CLASS_NAME: PCWSTR = w!("window_cap_settings");
const TITLE: PCWSTR = w!("window-cap 设置");

const ID_CAPTURE: usize = 101;
const ID_QUIT: usize = 102;
const ID_SAVE_DIR: usize = 103;
const ID_BROWSE: usize = 104;
const ID_DELAY: usize = 105;
const ID_COPY: usize = 106;
const ID_SAVE_FILE: usize = 107;
const ID_AUTOSTART: usize = 108;
const ID_SAVE: usize = 109;
const ID_CANCEL: usize = 110;

const BN_CLICKED: u16 = 0;
const BASE_WIDTH: i32 = 580;
const BASE_HEIGHT: i32 = 430;

const BG_COLOR: COLORREF = COLORREF(0x0020_1B1B);
const TEXT_COLOR: COLORREF = COLORREF(0x00EC_E8E8);
const BORDER_COLOR: COLORREF = COLORREF(0x0042_3A3A);
const HOVER_COLOR: COLORREF = COLORREF(0x0036_2E2E);
const ACCENT_COLOR: COLORREF = COLORREF(0x002E_9BFF);
const ACCENT_TEXT: COLORREF = COLORREF(0x001A_1A1A);

static OPEN: AtomicBool = AtomicBool::new(false);

struct OpenGuard;

impl Drop for OpenGuard {
    fn drop(&mut self) {
        OPEN.store(false, Ordering::SeqCst);
    }
}

struct SettingsState {
    config: Config,
    path: PathBuf,
    capture: HWND,
    quit: HWND,
    save_dir: HWND,
    delay: HWND,
    copy: HWND,
    save_file: HWND,
    autostart: HWND,
    result: Option<Config>,
    scale: f32,
    brush: HBRUSH,
    font: HFONT,
}

/// 打开设置窗口；保存时返回新配置，取消/关闭返回 None。
pub fn open(config: &Config, path: &Path) -> Result<Option<Config>> {
    if OPEN.swap(true, Ordering::SeqCst) {
        return Ok(None);
    }
    let _guard = OpenGuard;
    unsafe {
        let hinstance = HINSTANCE(GetModuleHandleW(None)?.0);
        let mut class: WNDCLASSW = std::mem::zeroed();
        class.lpfnWndProc = Some(settings_wndproc);
        class.hInstance = hinstance;
        class.lpszClassName = CLASS_NAME;
        class.hbrBackground = HBRUSH(null_mut());
        let _ = RegisterClassW(&class);

        let screen_width = GetSystemMetrics(SM_CXSCREEN);
        let screen_height = GetSystemMetrics(SM_CYSCREEN);
        let scale = GetDpiForWindow(get_desktop_window_safe()) as f32 / 96.0;
        let scale = scale.max(1.0);
        let width = (BASE_WIDTH as f32 * scale) as i32;
        let height = (BASE_HEIGHT as f32 * scale) as i32;
        let x = (screen_width - width) / 2;
        let y = (screen_height - height) / 2;
        let brush = CreateSolidBrush(BG_COLOR);
        let font = create_font(scale);

        let raw_state = Box::into_raw(Box::new(SettingsState {
            config: config.clone(),
            path: path.to_path_buf(),
            capture: HWND(null_mut()),
            quit: HWND(null_mut()),
            save_dir: HWND(null_mut()),
            delay: HWND(null_mut()),
            copy: HWND(null_mut()),
            save_file: HWND(null_mut()),
            autostart: HWND(null_mut()),
            result: None,
            scale,
            brush,
            font,
        }));

        let hwnd = match CreateWindowExW(
            windows::Win32::UI::WindowsAndMessaging::WINDOW_EX_STYLE(0),
            CLASS_NAME,
            TITLE,
            windows::Win32::UI::WindowsAndMessaging::WINDOW_STYLE(
                WS_OVERLAPPED.0 | WS_CAPTION.0 | WS_SYSMENU.0 | WS_MINIMIZEBOX.0 | WS_VISIBLE.0,
            ),
            x,
            y,
            width,
            height,
            None,
            None,
            Some(hinstance),
            Some(raw_state as *const c_void),
        ) {
            Ok(hwnd) => hwnd,
            Err(error) => {
                let _ = DeleteObject(HGDIOBJ(brush.0));
                let _ = DeleteObject(HGDIOBJ(font.0));
                drop(Box::from_raw(raw_state));
                return Err(error.into());
            }
        };
        let dark: i32 = 1;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &dark as *const i32 as *const c_void,
            size_of::<i32>() as u32,
        );
        let _ = ShowWindow(hwnd, SW_SHOWNORMAL);
        let _ = SetForegroundWindow(hwnd);

        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).0 > 0 {
            if IsDialogMessageW(hwnd, &message).0 == 0 {
                let _ = TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }

        let state = Box::from_raw(raw_state);
        let result = state.result.clone();
        let _ = DeleteObject(HGDIOBJ(state.brush.0));
        let _ = DeleteObject(HGDIOBJ(state.font.0));
        Ok(result)
    }
}

unsafe fn state(hwnd: HWND) -> Option<&'static mut SettingsState> {
    unsafe {
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut SettingsState;
        if pointer.is_null() {
            None
        } else {
            Some(&mut *pointer)
        }
    }
}

fn s(value: f32, scale: f32) -> i32 {
    (value * scale).round() as i32
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

unsafe fn create_font(scale: f32) -> HFONT {
    unsafe {
        CreateFontW(
            -((15.0 * scale).round() as i32),
            0,
            0,
            0,
            400,
            0,
            0,
            0,
            DEFAULT_CHARSET,
            OUT_DEFAULT_PRECIS,
            CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY,
            DEFAULT_PITCH.0 as u32,
            w!("Microsoft YaHei UI"),
        )
    }
}

#[allow(clippy::too_many_arguments)]
unsafe fn create_control(
    parent: HWND,
    class: PCWSTR,
    text: PCWSTR,
    style: u32,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    id: usize,
) -> HWND {
    unsafe {
        let hinstance = HINSTANCE(
            GetModuleHandleW(None)
                .map(|module| module.0)
                .unwrap_or(null_mut()),
        );
        let hwnd = CreateWindowExW(
            windows::Win32::UI::WindowsAndMessaging::WINDOW_EX_STYLE(0),
            class,
            text,
            windows::Win32::UI::WindowsAndMessaging::WINDOW_STYLE(style),
            x,
            y,
            width,
            height,
            Some(parent),
            Some(HMENU(id as isize as *mut c_void)),
            Some(hinstance),
            None,
        );
        if let Ok(hwnd) = hwnd {
            let font = state(parent).map(|s| s.font).unwrap_or(HFONT(null_mut()));
            let _ = SendMessageW(
                hwnd,
                WM_SETFONT,
                Some(WPARAM(font.0 as usize)),
                Some(LPARAM(1)),
            );
            let _ = SetWindowTheme(hwnd, w!("DarkMode_Explorer"), PCWSTR::null());
            hwnd
        } else {
            HWND(null_mut())
        }
    }
}

unsafe fn create_controls(hwnd: HWND) {
    unsafe {
        let Some(settings) = state(hwnd) else {
            return;
        };
        let scale = settings.scale;
        let margin = s(28.0, scale);
        let label_width = s(110.0, scale);
        let field_x = margin + label_width + s(8.0, scale);
        let field_width = s(300.0, scale);
        let row_height = s(28.0, scale);
        let row_gap = s(12.0, scale);
        let mut row = margin;

        let static_style = WS_CHILD.0 | WS_VISIBLE.0;
        let edit_style = WS_CHILD.0 | WS_VISIBLE.0 | WS_BORDER.0 | WS_TABSTOP.0 | 0x0080; // ES_AUTOHSCROLL
        let owner_style = WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | BS_OWNERDRAW as u32;
        let check_style = WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | 0x0003; // BS_AUTOCHECKBOX

        let label = |text: &str, y: i32| {
            let text = wide(text);
            create_control(
                hwnd,
                w!("STATIC"),
                PCWSTR(text.as_ptr()),
                static_style,
                margin,
                y,
                label_width,
                row_height,
                0,
            )
        };

        label("截图热键", row);
        settings.capture = create_control(
            hwnd,
            w!("EDIT"),
            PCWSTR::null(),
            edit_style,
            field_x,
            row,
            field_width,
            row_height,
            ID_CAPTURE,
        );
        row += row_height + row_gap;

        label("退出热键", row);
        settings.quit = create_control(
            hwnd,
            w!("EDIT"),
            PCWSTR::null(),
            edit_style,
            field_x,
            row,
            field_width,
            row_height,
            ID_QUIT,
        );
        row += row_height + row_gap;

        label("保存目录", row);
        settings.save_dir = create_control(
            hwnd,
            w!("EDIT"),
            PCWSTR::null(),
            edit_style,
            field_x,
            row,
            field_width - s(90.0, scale),
            row_height,
            ID_SAVE_DIR,
        );
        create_control(
            hwnd,
            w!("BUTTON"),
            w!("浏览..."),
            owner_style,
            field_x + field_width - s(80.0, scale),
            row,
            s(80.0, scale),
            row_height,
            ID_BROWSE,
        );
        row += row_height + row_gap;

        label("延迟截图 (ms)", row);
        settings.delay = create_control(
            hwnd,
            w!("EDIT"),
            PCWSTR::null(),
            edit_style,
            field_x,
            row,
            s(120.0, scale),
            row_height,
            ID_DELAY,
        );
        row += row_height + s(18.0, scale);

        settings.copy = create_control(
            hwnd,
            w!("BUTTON"),
            w!("截图后复制到剪贴板"),
            check_style,
            margin,
            row,
            s(400.0, scale),
            row_height,
            ID_COPY,
        );
        row += row_height + s(4.0, scale);
        settings.save_file = create_control(
            hwnd,
            w!("BUTTON"),
            w!("截图后保存 PNG 文件"),
            check_style,
            margin,
            row,
            s(400.0, scale),
            row_height,
            ID_SAVE_FILE,
        );
        row += row_height + s(4.0, scale);
        settings.autostart = create_control(
            hwnd,
            w!("BUTTON"),
            w!("开机自动启动"),
            check_style,
            margin,
            row,
            s(400.0, scale),
            row_height,
            ID_AUTOSTART,
        );
        row += row_height + s(8.0, scale);

        let hint = wide("保存目录支持 %USERPROFILE% 或相对路径；输出格式固定为 PNG。");
        create_control(
            hwnd,
            w!("STATIC"),
            PCWSTR(hint.as_ptr()),
            static_style,
            margin,
            row,
            s(520.0, scale),
            row_height,
            0,
        );

        let button_y = s(BASE_HEIGHT as f32, scale) - s(80.0, scale);
        create_control(
            hwnd,
            w!("BUTTON"),
            w!("保存"),
            owner_style,
            s(BASE_WIDTH as f32, scale) - s(200.0, scale),
            button_y,
            s(80.0, scale),
            s(32.0, scale),
            ID_SAVE,
        );
        create_control(
            hwnd,
            w!("BUTTON"),
            w!("取消"),
            owner_style,
            s(BASE_WIDTH as f32, scale) - s(110.0, scale),
            button_y,
            s(80.0, scale),
            s(32.0, scale),
            ID_CANCEL,
        );

        let capture = wide(&settings.config.hotkey.capture);
        let quit = wide(&settings.config.hotkey.quit);
        let save_dir = wide(&settings.config.output.save_dir);
        let delay = wide(&settings.config.capture.delay_ms.to_string());
        let _ = SetWindowTextW(settings.capture, PCWSTR(capture.as_ptr()));
        let _ = SetWindowTextW(settings.quit, PCWSTR(quit.as_ptr()));
        let _ = SetWindowTextW(settings.save_dir, PCWSTR(save_dir.as_ptr()));
        let _ = SetWindowTextW(settings.delay, PCWSTR(delay.as_ptr()));
        set_check(settings.copy, settings.config.output.copy_to_clipboard);
        set_check(settings.save_file, settings.config.output.save_file);
        set_check(settings.autostart, autostart::is_enabled());
    }
}

unsafe fn set_check(hwnd: HWND, checked: bool) {
    unsafe {
        let _ = SendMessageW(
            hwnd,
            windows::Win32::UI::WindowsAndMessaging::BM_SETCHECK,
            Some(WPARAM(if checked { 1 } else { 0 })),
            Some(LPARAM(0)),
        );
    }
}

unsafe fn check_state(hwnd: HWND) -> bool {
    unsafe {
        SendMessageW(
            hwnd,
            windows::Win32::UI::WindowsAndMessaging::BM_GETCHECK,
            Some(WPARAM(0)),
            Some(LPARAM(0)),
        )
        .0 == 1
    }
}

unsafe fn read_text(hwnd: HWND) -> String {
    unsafe {
        let length = GetWindowTextLengthW(hwnd);
        if length <= 0 {
            return String::new();
        }
        let mut buffer = vec![0u16; length as usize + 1];
        let copied = GetWindowTextW(hwnd, &mut buffer);
        String::from_utf16_lossy(&buffer[..copied.max(0) as usize])
    }
}

unsafe fn draw_owner_button(hwnd: HWND, item: &DRAWITEMSTRUCT) {
    unsafe {
        let Some(settings) = state(hwnd) else {
            return;
        };
        let save = item.CtlID == ID_SAVE as u32;
        let browse = item.CtlID == ID_BROWSE as u32;
        let pressed = item.itemState.0 & ODS_SELECTED.0 != 0;
        let (background, border, text) = if save {
            (
                if pressed {
                    COLORREF(0x002A_8AE6)
                } else {
                    ACCENT_COLOR
                },
                ACCENT_COLOR,
                ACCENT_TEXT,
            )
        } else {
            (
                if pressed { HOVER_COLOR } else { BG_COLOR },
                BORDER_COLOR,
                TEXT_COLOR,
            )
        };

        let brush = CreateSolidBrush(background);
        let pen = CreatePen(PS_SOLID, 1, border);
        let old_brush = SelectObject(item.hDC, HGDIOBJ(brush.0));
        let old_pen = SelectObject(item.hDC, HGDIOBJ(pen.0));
        let old_font = SelectObject(item.hDC, HGDIOBJ(settings.font.0));
        SetBkMode(item.hDC, TRANSPARENT);
        SetTextColor(item.hDC, text);

        let mut rect = item.rcItem;
        let radius = s(8.0, settings.scale);
        let _ = RoundRect(
            item.hDC,
            rect.left,
            rect.top,
            rect.right,
            rect.bottom,
            radius,
            radius,
        );
        let mut label = wide(if save {
            "保存"
        } else if browse {
            "浏览..."
        } else {
            "取消"
        });
        DrawTextW(
            item.hDC,
            &mut label,
            &mut rect,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE,
        );
        SelectObject(item.hDC, old_font);
        SelectObject(item.hDC, old_pen);
        SelectObject(item.hDC, old_brush);
        let _ = DeleteObject(HGDIOBJ(pen.0));
        let _ = DeleteObject(HGDIOBJ(brush.0));
    }
}

unsafe fn message_box(hwnd: HWND, text: &str) {
    unsafe {
        let text = wide(text);
        let title = wide("window-cap 设置");
        let _ = MessageBoxW(
            Some(hwnd),
            PCWSTR(text.as_ptr()),
            PCWSTR(title.as_ptr()),
            MB_OK | MB_ICONERROR,
        );
    }
}

unsafe fn browse_folder(hwnd: HWND) -> Option<PathBuf> {
    unsafe {
        let mut display = [0u16; 260];
        let title = wide("选择保存目录");
        let info = BROWSEINFOW {
            hwndOwner: hwnd,
            pidlRoot: null_mut(),
            pszDisplayName: windows::core::PWSTR(display.as_mut_ptr()),
            lpszTitle: PCWSTR(title.as_ptr()),
            ulFlags: BIF_RETURNONLYFSDIRS | BIF_NEWDIALOGSTYLE,
            lpfn: None,
            lParam: LPARAM(0),
            iImage: 0,
        };
        let pidl = SHBrowseForFolderW(&info);
        if pidl.is_null() {
            return None;
        }
        let mut path = [0u16; 260];
        let ok = SHGetPathFromIDListW(pidl, &mut path).as_bool();
        CoTaskMemFree(Some(pidl as *const c_void));
        if !ok {
            return None;
        }
        let length = path.iter().position(|&ch| ch == 0).unwrap_or(path.len());
        Some(PathBuf::from(String::from_utf16_lossy(&path[..length])))
    }
}

unsafe fn save_settings(hwnd: HWND) -> bool {
    unsafe {
        let Some(settings) = state(hwnd) else {
            return false;
        };
        let capture = read_text(settings.capture).trim().to_string();
        let quit = read_text(settings.quit).trim().to_string();
        if Hotkey::parse(&capture).is_err() {
            message_box(hwnd, "截图热键格式无效，例如 Ctrl+Alt+A");
            return false;
        }
        if Hotkey::parse(&quit).is_err() {
            message_box(hwnd, "退出热键格式无效，例如 Ctrl+Alt+Q");
            return false;
        }
        let save_dir = read_text(settings.save_dir).trim().to_string();
        if save_dir.is_empty() {
            message_box(hwnd, "保存目录不能为空");
            return false;
        }
        let Ok(delay) = read_text(settings.delay).trim().parse::<u64>() else {
            message_box(hwnd, "延迟截图必须是毫秒数（0-60000）");
            return false;
        };
        if delay > 60_000 {
            message_box(hwnd, "延迟截图不能超过 60000 ms");
            return false;
        }

        let autostart_checked = check_state(settings.autostart);
        if autostart_checked != autostart::is_enabled()
            && let Err(error) = autostart::set_enabled(autostart_checked)
        {
            message_box(hwnd, &format!("设置开机自启失败: {error}"));
            return false;
        }

        let mut config = settings.config.clone();
        config.hotkey.capture = capture;
        config.hotkey.quit = quit;
        config.output.save_dir = save_dir;
        config.capture.delay_ms = delay;
        config.output.copy_to_clipboard = check_state(settings.copy);
        config.output.save_file = check_state(settings.save_file);
        let config = config.sanitize();
        if let Err(error) = config.save(&settings.path) {
            message_box(hwnd, &format!("保存配置失败: {error}"));
            return false;
        }
        settings.result = Some(config);
        true
    }
}

unsafe extern "system" fn settings_wndproc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        match message {
            WM_ERASEBKGND => {
                let hdc = HDC(wparam.0 as *mut c_void);
                if let Some(settings) = state(hwnd) {
                    let mut rect = RECT::default();
                    let _ = GetClientRect(hwnd, &mut rect);
                    let _ = FillRect(hdc, &rect, settings.brush);
                }
                LRESULT(1)
            }
            WM_CTLCOLORSTATIC => {
                let hdc = HDC(wparam.0 as *mut c_void);
                if let Some(settings) = state(hwnd) {
                    let _ = SetTextColor(hdc, TEXT_COLOR);
                    let _ = SetBkMode(hdc, TRANSPARENT);
                    return LRESULT(settings.brush.0 as isize);
                }
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
            WM_CTLCOLOREDIT | WM_CTLCOLORBTN => {
                let hdc = HDC(wparam.0 as *mut c_void);
                if let Some(settings) = state(hwnd) {
                    let _ = SetTextColor(hdc, TEXT_COLOR);
                    let _ = SetBkColor(hdc, BG_COLOR);
                    return LRESULT(settings.brush.0 as isize);
                }
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
            WM_DRAWITEM => {
                let item = lparam.0 as *const DRAWITEMSTRUCT;
                if !item.is_null() {
                    draw_owner_button(hwnd, &*item);
                }
                LRESULT(1)
            }
            WM_NCCREATE => {
                let create =
                    lparam.0 as *const windows::Win32::UI::WindowsAndMessaging::CREATESTRUCTW;
                if !create.is_null() {
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, (*create).lpCreateParams as isize);
                }
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
            WM_CREATE => {
                create_controls(hwnd);
                LRESULT(0)
            }
            WM_COMMAND => {
                let id = wparam.0 & 0xFFFF;
                let code = ((wparam.0 >> 16) & 0xFFFF) as u16;
                if code == BN_CLICKED {
                    match id {
                        ID_BROWSE => {
                            if let Some(settings) = state(hwnd)
                                && let Some(path) = browse_folder(hwnd)
                            {
                                let text = wide(&path.to_string_lossy());
                                let _ = SetWindowTextW(settings.save_dir, PCWSTR(text.as_ptr()));
                            }
                        }
                        ID_SAVE => {
                            if save_settings(hwnd) {
                                let _ = DestroyWindow(hwnd);
                            }
                        }
                        ID_CANCEL => {
                            let _ = DestroyWindow(hwnd);
                        }
                        _ => {}
                    }
                }
                LRESULT(0)
            }
            WM_CLOSE => {
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, message, wparam, lparam),
        }
    }
}

/// 无窗口时用桌面窗口取 DPI。
fn get_desktop_window_safe() -> HWND {
    unsafe {
        use windows::Win32::UI::WindowsAndMessaging::GetDesktopWindow;
        GetDesktopWindow()
    }
}
