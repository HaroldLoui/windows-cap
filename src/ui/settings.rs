//! 设置窗口：卡片布局、暗色主题、热键录制与配置保存。

use std::ffi::c_void;
use std::mem::size_of;
use std::path::{Path, PathBuf};
use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, Ordering};

use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DWMWA_CAPTION_COLOR, DWMWA_USE_IMMERSIVE_DARK_MODE, DWMWA_WINDOW_CORNER_PREFERENCE,
    DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{
    CreateSolidBrush, DeleteObject, FillRect, HBRUSH, HFONT, HGDIOBJ, SetBkColor, SetBkMode,
    SetTextColor, TRANSPARENT,
};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{DRAWITEMSTRUCT, ODS_FOCUS, SetWindowTheme};
use windows::Win32::UI::Shell::{
    BIF_NEWDIALOGSTYLE, BIF_RETURNONLYFSDIRS, BROWSEINFOW, SHBrowseForFolderW, SHGetPathFromIDListW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    BS_OWNERDRAW, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GW_CHILD,
    GW_HWNDNEXT, GWLP_USERDATA, GetClientRect, GetMessageW, GetSystemMetrics, GetWindow,
    GetWindowTextLengthW, GetWindowTextW, HMENU, IsDialogMessageW, MB_ICONERROR, MB_OK, MSG,
    MessageBoxW, PostQuitMessage, RegisterClassW, SM_CXSCREEN, SM_CYSCREEN, SW_SHOWNORMAL,
    SendMessageW, SetForegroundWindow, SetWindowLongPtrW, SetWindowPos, SetWindowTextW, ShowWindow,
    TranslateMessage, WM_CLOSE, WM_COMMAND, WM_CREATE, WM_CTLCOLORBTN, WM_CTLCOLOREDIT,
    WM_CTLCOLORSTATIC, WM_DESTROY, WM_DPICHANGED, WM_DRAWITEM, WM_ERASEBKGND, WM_NCCREATE,
    WM_PAINT, WM_SETFONT, WNDCLASSW, WS_CAPTION, WS_CHILD, WS_MINIMIZEBOX, WS_OVERLAPPED,
    WS_SYSMENU, WS_TABSTOP, WS_VISIBLE,
};
use windows::core::{PCWSTR, w};

use crate::app::autostart;
use crate::app::config::{Config, Hotkey};
use crate::ui::kit::dpi::{s, scale_from_dpi, scale_from_hwnd};
use crate::ui::kit::text::{create_font, wide};
use crate::ui::kit::theme::{WINDOW_BG, WINDOW_TEXT};
use crate::ui::kit::widgets::{button, card, field, hotkey, toggle};
use crate::util::error::Result;

const CLASS_NAME: PCWSTR = w!("window_cap_settings");
const TITLE: PCWSTR = w!("window-cap 设置");

const ID_CAPTURE: usize = 101;
const ID_QUIT: usize = 102;
const ID_SAVE_DIR: usize = 103;
const ID_BROWSE: usize = 104;
const ID_DELAY: usize = 105;
const ID_AUTOSTART: usize = 106;
const ID_SAVE: usize = 107;
const ID_CANCEL: usize = 108;

const BN_CLICKED: u16 = 0;
const EN_SETFOCUS: u16 = 0x0100;
const EN_KILLFOCUS: u16 = 0x0200;

const BASE_WIDTH: i32 = 620;
const MARGIN: i32 = 26;

static OPEN: AtomicBool = AtomicBool::new(false);

struct OpenGuard;

impl Drop for OpenGuard {
    fn drop(&mut self) {
        OPEN.store(false, Ordering::SeqCst);
    }
}

#[derive(Clone, Copy)]
struct Layout {
    title_hotkey: RECT,
    title_output: RECT,
    title_startup: RECT,
    card_hotkey: RECT,
    card_output: RECT,
    card_startup: RECT,
    capture_field: RECT,
    quit_field: RECT,
    save_dir_field: RECT,
    delay_field: RECT,
    toggle_rect: RECT,
    hint: RECT,
    save_button: RECT,
    cancel_button: RECT,
    client_height: i32,
}

struct SettingsState {
    config: Config,
    path: PathBuf,
    capture: HWND,
    quit: HWND,
    save_dir: HWND,
    delay: HWND,
    browse: HWND,
    toggle: HWND,
    save_button: HWND,
    cancel_button: HWND,
    autostart_on: bool,
    result: Option<Config>,
    scale: f32,
    brush: HBRUSH,
    font: HFONT,
    layout: Layout,
    focused: usize,
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
        let scale = scale_from_hwnd(get_desktop_window_safe());
        let layout = build_layout(scale);
        let width = BASE_WIDTH;
        let height = layout.client_height + s(42.0, scale);
        let x = (screen_width - width) / 2;
        let y = (screen_height - height) / 2;

        let brush = CreateSolidBrush(WINDOW_BG);
        let font = create_font(scale);
        let raw_state = Box::into_raw(Box::new(SettingsState {
            config: config.clone(),
            path: path.to_path_buf(),
            capture: HWND(null_mut()),
            quit: HWND(null_mut()),
            save_dir: HWND(null_mut()),
            delay: HWND(null_mut()),
            browse: HWND(null_mut()),
            toggle: HWND(null_mut()),
            save_button: HWND(null_mut()),
            cancel_button: HWND(null_mut()),
            autostart_on: autostart::is_enabled(),
            result: None,
            scale,
            brush,
            font,
            layout,
            focused: 0,
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

        // Win11 圆角/标题栏同色；Win10 上调用失败会自动忽略。
        let dark: i32 = 1;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            &dark as *const i32 as *const c_void,
            size_of::<i32>() as u32,
        );
        let caption = WINDOW_BG.0;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_CAPTION_COLOR,
            &caption as *const u32 as *const c_void,
            size_of::<u32>() as u32,
        );
        let corner: i32 = 2;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &corner as *const i32 as *const c_void,
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

fn build_layout(scale: f32) -> Layout {
    let margin = s(MARGIN as f32, scale);
    let card_left = margin;
    let card_width = BASE_WIDTH - margin * 2;
    let pad = s(16.0, scale);
    let title_height = s(18.0, scale);
    let label_height = s(17.0, scale);
    let field_height = s(34.0, scale);
    let button_height = s(34.0, scale);
    let button_width = s(88.0, scale);
    let button_gap = s(10.0, scale);

    let mut y = margin;
    let title_hotkey = RECT {
        left: card_left,
        top: y,
        right: card_left + card_width,
        bottom: y + title_height,
    };
    y += title_height + s(6.0, scale);
    let card_hotkey_top = y;
    let label_capture = RECT {
        left: card_left + pad,
        top: card_hotkey_top + pad,
        right: card_left + card_width - pad,
        bottom: card_hotkey_top + pad + label_height,
    };
    let capture_field = RECT {
        left: label_capture.left,
        top: label_capture.bottom + s(4.0, scale),
        right: label_capture.right,
        bottom: label_capture.bottom + s(4.0, scale) + field_height,
    };
    let label_quit = RECT {
        left: label_capture.left,
        top: capture_field.bottom + s(10.0, scale),
        right: label_capture.right,
        bottom: capture_field.bottom + s(10.0, scale) + label_height,
    };
    let quit_field = RECT {
        left: label_quit.left,
        top: label_quit.bottom + s(4.0, scale),
        right: label_quit.right,
        bottom: label_quit.bottom + s(4.0, scale) + field_height,
    };
    let card_hotkey = RECT {
        left: card_left,
        top: card_hotkey_top,
        right: card_left + card_width,
        bottom: quit_field.bottom + pad,
    };

    y = card_hotkey.bottom + s(16.0, scale);
    let title_output = RECT {
        left: card_left,
        top: y,
        right: card_left + card_width,
        bottom: y + title_height,
    };
    y += title_height + s(6.0, scale);
    let card_output_top = y;
    let label_save_dir = RECT {
        left: card_left + pad,
        top: card_output_top + pad,
        right: card_left + card_width - pad - s(90.0, scale),
        bottom: card_output_top + pad + label_height,
    };
    let save_dir_field = RECT {
        left: label_save_dir.left,
        top: label_save_dir.bottom + s(4.0, scale),
        right: card_left + card_width - pad,
        bottom: label_save_dir.bottom + s(4.0, scale) + field_height,
    };
    let label_delay = RECT {
        left: label_save_dir.left,
        top: save_dir_field.bottom + s(10.0, scale),
        right: label_save_dir.right,
        bottom: save_dir_field.bottom + s(10.0, scale) + label_height,
    };
    let delay_field = RECT {
        left: label_delay.left,
        top: label_delay.bottom + s(4.0, scale),
        right: label_delay.left + s(140.0, scale),
        bottom: label_delay.bottom + s(4.0, scale) + field_height,
    };
    let card_output = RECT {
        left: card_left,
        top: card_output_top,
        right: card_left + card_width,
        bottom: delay_field.bottom + pad,
    };

    y = card_output.bottom + s(16.0, scale);
    let title_startup = RECT {
        left: card_left,
        top: y,
        right: card_left + card_width,
        bottom: y + title_height,
    };
    y += title_height + s(6.0, scale);
    let card_startup = RECT {
        left: card_left,
        top: y,
        right: card_left + card_width,
        bottom: y + pad + s(28.0, scale) + pad,
    };
    let toggle_rect = RECT {
        left: card_startup.right - pad - s(76.0, scale),
        top: card_startup.top + pad,
        right: card_startup.right - pad,
        bottom: card_startup.top + pad + s(28.0, scale),
    };

    y = card_startup.bottom + s(14.0, scale);
    let hint = RECT {
        left: card_left,
        top: y,
        right: card_left + card_width,
        bottom: y + s(20.0, scale),
    };
    y = hint.bottom + s(16.0, scale);
    let save_button = RECT {
        left: card_left + card_width - button_width,
        top: y,
        right: card_left + card_width,
        bottom: y + button_height,
    };
    let cancel_button = RECT {
        left: save_button.left - button_gap - button_width,
        top: y,
        right: save_button.left - button_gap,
        bottom: y + button_height,
    };
    let client_height = save_button.bottom + margin;

    Layout {
        title_hotkey,
        title_output,
        title_startup,
        card_hotkey,
        card_output,
        card_startup,
        capture_field,
        quit_field,
        save_dir_field,
        delay_field,
        toggle_rect,
        hint,
        save_button,
        cancel_button,
        client_height,
    }
}

unsafe fn state(hwnd: HWND) -> Option<&'static mut SettingsState> {
    unsafe { crate::ui::kit::window::state(hwnd) }
}

unsafe fn create_control(
    parent: HWND,
    class: PCWSTR,
    text: PCWSTR,
    style: u32,
    rect: RECT,
    id: usize,
    font: HFONT,
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
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
            Some(parent),
            Some(HMENU(id as isize as *mut c_void)),
            Some(hinstance),
            None,
        );
        if let Ok(hwnd) = hwnd {
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

unsafe fn create_label(parent: HWND, text: &str, rect: RECT, font: HFONT) -> HWND {
    unsafe {
        let text = wide(text);
        create_control(
            parent,
            w!("STATIC"),
            PCWSTR(text.as_ptr()),
            WS_CHILD.0 | WS_VISIBLE.0,
            rect,
            0,
            font,
        )
    }
}

unsafe fn create_edit(parent: HWND, id: usize, rect: RECT, value: &str, font: HFONT) -> HWND {
    unsafe {
        let inset = RECT {
            left: rect.left + 2,
            top: rect.top + 2,
            right: rect.right - 2,
            bottom: rect.bottom - 2,
        };
        let style = WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | 0x0080; // ES_AUTOHSCROLL
        let hwnd = create_control(parent, w!("EDIT"), PCWSTR::null(), style, inset, id, font);
        let value = wide(value);
        let _ = SetWindowTextW(hwnd, PCWSTR(value.as_ptr()));
        hwnd
    }
}

unsafe fn create_owner_button(
    parent: HWND,
    id: usize,
    rect: RECT,
    text: &str,
    font: HFONT,
) -> HWND {
    unsafe {
        let text = wide(text);
        create_control(
            parent,
            w!("BUTTON"),
            PCWSTR(text.as_ptr()),
            WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | BS_OWNERDRAW as u32,
            rect,
            id,
            font,
        )
    }
}

unsafe fn create_controls(hwnd: HWND) {
    unsafe {
        let Some(settings) = state(hwnd) else {
            return;
        };
        let layout = settings.layout;
        let font = settings.font;
        let scale = settings.scale;

        create_label(hwnd, "快捷键", layout.title_hotkey, font);
        create_label(
            hwnd,
            "截图热键",
            RECT {
                left: layout.card_hotkey.left + s(16.0, scale),
                top: layout.card_hotkey.top + s(16.0, scale),
                right: layout.card_hotkey.right - s(16.0, scale),
                bottom: layout.card_hotkey.top + s(33.0, scale),
            },
            font,
        );
        settings.capture = hotkey::create(
            hwnd,
            ID_CAPTURE,
            layout.capture_field.left,
            layout.capture_field.top,
            layout.capture_field.right - layout.capture_field.left,
            layout.capture_field.bottom - layout.capture_field.top,
            font,
            scale,
            &settings.config.hotkey.capture,
        );
        create_label(
            hwnd,
            "退出热键",
            RECT {
                left: layout.card_hotkey.left + s(16.0, scale),
                top: layout.capture_field.bottom + s(10.0, scale),
                right: layout.card_hotkey.right - s(16.0, scale),
                bottom: layout.capture_field.bottom + s(27.0, scale),
            },
            font,
        );
        settings.quit = hotkey::create(
            hwnd,
            ID_QUIT,
            layout.quit_field.left,
            layout.quit_field.top,
            layout.quit_field.right - layout.quit_field.left,
            layout.quit_field.bottom - layout.quit_field.top,
            font,
            scale,
            &settings.config.hotkey.quit,
        );

        create_label(hwnd, "输出", layout.title_output, font);
        create_label(
            hwnd,
            "保存目录",
            RECT {
                left: layout.card_output.left + s(16.0, scale),
                top: layout.card_output.top + s(16.0, scale),
                right: layout.card_output.right - s(16.0, scale),
                bottom: layout.card_output.top + s(33.0, scale),
            },
            font,
        );
        settings.save_dir = create_edit(
            hwnd,
            ID_SAVE_DIR,
            layout.save_dir_field,
            &settings.config.output.save_dir,
            font,
        );
        let browse_rect = RECT {
            left: layout.save_dir_field.right - s(80.0, scale),
            top: layout.card_output.top + s(14.0, scale),
            right: layout.save_dir_field.right,
            bottom: layout.card_output.top + s(40.0, scale),
        };
        settings.browse = create_owner_button(hwnd, ID_BROWSE, browse_rect, "浏览...", font);
        create_label(
            hwnd,
            "延迟截图 (ms)",
            RECT {
                left: layout.card_output.left + s(16.0, scale),
                top: layout.save_dir_field.bottom + s(10.0, scale),
                right: layout.card_output.right - s(16.0, scale),
                bottom: layout.save_dir_field.bottom + s(27.0, scale),
            },
            font,
        );
        settings.delay = create_edit(
            hwnd,
            ID_DELAY,
            layout.delay_field,
            &settings.config.capture.delay_ms.to_string(),
            font,
        );

        create_label(hwnd, "启动", layout.title_startup, font);
        create_label(
            hwnd,
            "开机自动启动",
            RECT {
                left: layout.card_startup.left + s(16.0, scale),
                top: layout.card_startup.top + s(18.0, scale),
                right: layout.toggle_rect.left - s(8.0, scale),
                bottom: layout.card_startup.top + s(38.0, scale),
            },
            font,
        );
        settings.toggle = create_owner_button(hwnd, ID_AUTOSTART, layout.toggle_rect, "", font);

        let hint = wide("保存目录支持 %USERPROFILE% 或相对路径；输出格式固定为 PNG。");
        create_control(
            hwnd,
            w!("STATIC"),
            PCWSTR(hint.as_ptr()),
            WS_CHILD.0 | WS_VISIBLE.0,
            layout.hint,
            0,
            font,
        );

        settings.save_button = create_owner_button(hwnd, ID_SAVE, layout.save_button, "保存", font);
        settings.cancel_button =
            create_owner_button(hwnd, ID_CANCEL, layout.cancel_button, "取消", font);
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
        let capture = hotkey::value(settings.capture);
        let quit = hotkey::value(settings.quit);
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

        if settings.autostart_on != autostart::is_enabled()
            && let Err(error) = autostart::set_enabled(settings.autostart_on)
        {
            message_box(hwnd, &format!("设置开机自启失败: {error}"));
            return false;
        }

        let mut config = settings.config.clone();
        config.hotkey.capture = capture;
        config.hotkey.quit = quit;
        config.output.save_dir = save_dir;
        config.capture.delay_ms = delay;
        let config = config.sanitize();
        if let Err(error) = config.save(&settings.path) {
            message_box(hwnd, &format!("保存配置失败: {error}"));
            return false;
        }
        settings.result = Some(config);
        true
    }
}

unsafe fn draw_item(hwnd: HWND, item: &DRAWITEMSTRUCT) {
    unsafe {
        let Some(settings) = state(hwnd) else {
            return;
        };
        let focused = item.itemState.0 & ODS_FOCUS.0 != 0;
        let pressed = item.itemState.0 & windows::Win32::UI::Controls::ODS_SELECTED.0 != 0;
        match item.CtlID as usize {
            ID_SAVE => button::draw(
                item.hDC,
                item.rcItem,
                "保存",
                button::ButtonKind::Accent,
                pressed,
                focused,
                settings.scale,
                settings.font,
            ),
            ID_CANCEL => button::draw(
                item.hDC,
                item.rcItem,
                "取消",
                button::ButtonKind::Normal,
                pressed,
                focused,
                settings.scale,
                settings.font,
            ),
            ID_BROWSE => button::draw(
                item.hDC,
                item.rcItem,
                "浏览...",
                button::ButtonKind::Normal,
                pressed,
                focused,
                settings.scale,
                settings.font,
            ),
            ID_AUTOSTART => toggle::draw(
                item.hDC,
                item.rcItem,
                settings.autostart_on,
                focused,
                settings.scale,
            ),
            _ => {}
        }
    }
}

unsafe fn rebuild_controls(hwnd: HWND, new_scale: f32, suggested: &RECT) {
    unsafe {
        let Some(settings) = state(hwnd) else {
            return;
        };
        let capture = hotkey::value(settings.capture);
        let quit = hotkey::value(settings.quit);
        let save_dir = read_text(settings.save_dir);
        let delay = read_text(settings.delay);

        let mut child = GetWindow(hwnd, GW_CHILD).unwrap_or_default();
        while !child.is_invalid() {
            let next = GetWindow(child, GW_HWNDNEXT).unwrap_or_default();
            let _ = DestroyWindow(child);
            child = next;
        }
        settings.capture = HWND(null_mut());
        settings.quit = HWND(null_mut());
        settings.save_dir = HWND(null_mut());
        settings.delay = HWND(null_mut());
        settings.browse = HWND(null_mut());
        settings.toggle = HWND(null_mut());
        settings.save_button = HWND(null_mut());
        settings.cancel_button = HWND(null_mut());

        settings.config.hotkey.capture = capture;
        settings.config.hotkey.quit = quit;
        if !save_dir.trim().is_empty() {
            settings.config.output.save_dir = save_dir;
        }
        if let Ok(delay) = delay.trim().parse::<u64>() {
            settings.config.capture.delay_ms = delay;
        }

        let old_font = settings.font;
        settings.font = create_font(new_scale);
        let _ = DeleteObject(HGDIOBJ(old_font.0));
        settings.scale = new_scale;
        settings.layout = build_layout(new_scale);
        let _ = SetWindowPos(
            hwnd,
            None,
            suggested.left,
            suggested.top,
            suggested.right - suggested.left,
            suggested.bottom - suggested.top,
            windows::Win32::UI::WindowsAndMessaging::SWP_NOZORDER
                | windows::Win32::UI::WindowsAndMessaging::SWP_NOACTIVATE,
        );
        create_controls(hwnd);
        let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd), None, false);
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
            WM_ERASEBKGND => {
                let hdc = windows::Win32::Graphics::Gdi::HDC(wparam.0 as *mut c_void);
                if let Some(settings) = state(hwnd) {
                    let mut rect = RECT::default();
                    let _ = GetClientRect(hwnd, &mut rect);
                    let _ = FillRect(hdc, &rect, settings.brush);
                }
                LRESULT(1)
            }
            WM_PAINT => {
                let mut paint = windows::Win32::Graphics::Gdi::PAINTSTRUCT::default();
                let hdc = windows::Win32::Graphics::Gdi::BeginPaint(hwnd, &mut paint);
                if let Some(settings) = state(hwnd) {
                    let mut rect = RECT::default();
                    let _ = GetClientRect(hwnd, &mut rect);
                    let _ = FillRect(hdc, &rect, settings.brush);
                    card::draw(hdc, settings.layout.card_hotkey, settings.scale);
                    card::draw(hdc, settings.layout.card_output, settings.scale);
                    card::draw(hdc, settings.layout.card_startup, settings.scale);
                    for (id, rect) in [
                        (ID_CAPTURE, settings.layout.capture_field),
                        (ID_QUIT, settings.layout.quit_field),
                        (ID_SAVE_DIR, settings.layout.save_dir_field),
                        (ID_DELAY, settings.layout.delay_field),
                    ] {
                        field::draw(hdc, rect, settings.focused == id, settings.scale);
                    }
                }
                let _ = windows::Win32::Graphics::Gdi::EndPaint(hwnd, &paint);
                LRESULT(0)
            }
            WM_CTLCOLORSTATIC => {
                let hdc = windows::Win32::Graphics::Gdi::HDC(wparam.0 as *mut c_void);
                if let Some(settings) = state(hwnd) {
                    let _ = SetTextColor(hdc, WINDOW_TEXT);
                    let _ = SetBkMode(hdc, TRANSPARENT);
                    return LRESULT(settings.brush.0 as isize);
                }
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
            WM_CTLCOLOREDIT | WM_CTLCOLORBTN => {
                let hdc = windows::Win32::Graphics::Gdi::HDC(wparam.0 as *mut c_void);
                if let Some(settings) = state(hwnd) {
                    let _ = SetTextColor(hdc, WINDOW_TEXT);
                    let _ = SetBkColor(hdc, WINDOW_BG);
                    return LRESULT(settings.brush.0 as isize);
                }
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
            WM_DRAWITEM => {
                let item = lparam.0 as *const DRAWITEMSTRUCT;
                if !item.is_null() {
                    draw_item(hwnd, &*item);
                }
                LRESULT(1)
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
                        ID_AUTOSTART => {
                            if let Some(settings) = state(hwnd) {
                                settings.autostart_on = !settings.autostart_on;
                                let _ = windows::Win32::Graphics::Gdi::InvalidateRect(
                                    Some(settings.toggle),
                                    None,
                                    false,
                                );
                            }
                        }
                        _ => {}
                    }
                } else if code == EN_SETFOCUS
                    && let Some(settings) = state(hwnd)
                {
                    settings.focused = id;
                    let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd), None, false);
                } else if code == EN_KILLFOCUS
                    && let Some(settings) = state(hwnd)
                {
                    settings.focused = 0;
                    let _ = windows::Win32::Graphics::Gdi::InvalidateRect(Some(hwnd), None, false);
                }
                LRESULT(0)
            }
            WM_DPICHANGED => {
                if state(hwnd).is_some() {
                    let scale = scale_from_dpi((wparam.0 & 0xFFFF) as u32);
                    let suggested = &*(lparam.0 as *const RECT);
                    rebuild_controls(hwnd, scale, suggested);
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

fn get_desktop_window_safe() -> HWND {
    unsafe {
        use windows::Win32::UI::WindowsAndMessaging::GetDesktopWindow;
        GetDesktopWindow()
    }
}
