//! 文字标注输入：一个位于遮罩上的 `EDIT` 子窗口。
//!
//! 用系统 EDIT 承载输入，天然支持中文 IME；回车提交、Shift+回车换行、Esc 取消。

use std::sync::atomic::{AtomicIsize, Ordering};

use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateFontW, DEFAULT_CHARSET, DeleteObject,
    FF_DONTCARE, FW_NORMAL, HGDIOBJ, OUT_DEFAULT_PRECIS,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::Ime::{
    HIMC, ImmAssociateContext, ImmCreateContext, ImmDestroyContext,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, SetFocus, VK_ESCAPE, VK_RETURN, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallWindowProcW, CreateWindowExW, DestroyWindow, ES_AUTOVSCROLL, ES_MULTILINE, GWLP_WNDPROC,
    GetParent, GetWindowTextLengthW, GetWindowTextW, PostMessageW, SendMessageW, SetWindowLongPtrW,
    WINDOW_EX_STYLE, WINDOW_STYLE, WM_APP, WM_KEYDOWN, WM_SETFONT, WNDPROC, WS_CHILD, WS_VISIBLE,
};
use windows::core::w;

use crate::util::error::Result;

pub const WM_TEXT_COMMIT: u32 = WM_APP + 2;
pub const WM_TEXT_CANCEL: u32 = WM_APP + 3;

static OLD_PROC: AtomicIsize = AtomicIsize::new(0);

pub struct TextEditor {
    pub hwnd: HWND,
    /// 文字锚点（document 本地坐标）。
    pub doc_position: (i32, i32),
    font: HGDIOBJ,
    ime_context: HIMC,
}

impl TextEditor {
    /// 在遮罩客户端坐标 `client_position` 处创建输入框，字号与最终文字一致。
    pub fn open(
        parent: HWND,
        client_position: (i32, i32),
        doc_position: (i32, i32),
        scale: f32,
        font_size: f32,
    ) -> Result<Self> {
        let scale = scale.max(1.0);
        let width = ((320.0 * scale).round() as i32).max(160);
        let height = ((font_size * 1.8).round() as i32).max(28);
        let font_height = -((font_size).round() as i32).max(10);

        unsafe {
            let hinstance = HINSTANCE(GetModuleHandleW(None)?.0);
            let edit = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("EDIT"),
                w!(""),
                WS_CHILD | WS_VISIBLE | WINDOW_STYLE((ES_MULTILINE | ES_AUTOVSCROLL) as u32),
                client_position.0,
                client_position.1,
                width,
                height,
                Some(parent),
                None,
                Some(hinstance),
                None,
            )?;

            let font = CreateFontW(
                font_height,
                0,
                0,
                0,
                FW_NORMAL.0 as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET,
                OUT_DEFAULT_PRECIS,
                CLIP_DEFAULT_PRECIS,
                CLEARTYPE_QUALITY,
                FF_DONTCARE.0 as u32,
                w!("Microsoft YaHei UI"),
            );
            if !font.0.is_null() {
                SendMessageW(
                    edit,
                    WM_SETFONT,
                    Some(WPARAM(font.0 as usize)),
                    Some(LPARAM(1)),
                );
            }

            let previous =
                SetWindowLongPtrW(edit, GWLP_WNDPROC, edit_wndproc as *const () as isize);
            OLD_PROC.store(previous, Ordering::SeqCst);

            // 遮罩本身禁用了 IME，这里给输入框单独建一个上下文。
            let ime_context = ImmCreateContext();
            if !ime_context.0.is_null() {
                let _ = ImmAssociateContext(edit, ime_context);
            }
            let _ = SetFocus(Some(edit));

            Ok(Self {
                hwnd: edit,
                doc_position,
                font: HGDIOBJ(font.0),
                ime_context,
            })
        }
    }

    pub fn text(&self) -> String {
        unsafe {
            let length = GetWindowTextLengthW(self.hwnd);
            if length <= 0 {
                return String::new();
            }
            let mut buffer = vec![0u16; length as usize + 1];
            let copied = GetWindowTextW(self.hwnd, &mut buffer);
            String::from_utf16_lossy(&buffer[..copied.max(0) as usize])
        }
    }
}

impl Drop for TextEditor {
    fn drop(&mut self) {
        unsafe {
            let _ = ImmAssociateContext(self.hwnd, HIMC(std::ptr::null_mut()));
            let _ = DestroyWindow(self.hwnd);
            if !self.ime_context.0.is_null() {
                let _ = ImmDestroyContext(self.ime_context);
            }
            if !self.font.0.is_null() {
                let _ = DeleteObject(self.font);
            }
        }
    }
}

unsafe extern "system" fn edit_wndproc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        if message == WM_KEYDOWN {
            let key = wparam.0 as u16;
            let shift = GetKeyState(VK_SHIFT.0 as i32) < 0;
            if key == VK_RETURN.0 && !shift {
                if let Ok(parent) = GetParent(hwnd) {
                    let _ = PostMessageW(Some(parent), WM_TEXT_COMMIT, WPARAM(0), LPARAM(0));
                }
                return LRESULT(0);
            }
            if key == VK_ESCAPE.0 {
                if let Ok(parent) = GetParent(hwnd) {
                    let _ = PostMessageW(Some(parent), WM_TEXT_CANCEL, WPARAM(0), LPARAM(0));
                }
                return LRESULT(0);
            }
        }

        let previous: WNDPROC = std::mem::transmute(OLD_PROC.load(Ordering::SeqCst));
        CallWindowProcW(previous, hwnd, message, wparam, lparam)
    }
}
