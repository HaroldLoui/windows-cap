//! 文字标注输入：一个位于遮罩上的 `EDIT` 子窗口。
//!
//! 背景透明方式：子类化 EDIT，在 `WM_ERASEBKGND` 里把遮罩合成缓冲中
//! 该位置的像素原位 BitBlt 进来，文字由系统 EDIT 以透明背景绘制。
//! 这样既保留原生输入/中文 IME，又不会出现不透明输入框。

use std::sync::atomic::{AtomicI32, AtomicIsize, Ordering};

use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BitBlt, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateFontW, DEFAULT_CHARSET, DeleteObject,
    FF_DONTCARE, FW_NORMAL, GetClipBox, HDC, HGDIOBJ, OUT_DEFAULT_PRECIS, SRCCOPY,
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
    GetParent, GetWindowTextLengthW, GetWindowTextW, PostMessageW, SWP_NOACTIVATE, SWP_NOMOVE,
    SWP_NOSIZE, SWP_NOZORDER, SendMessageW, SetWindowLongPtrW, SetWindowPos, WINDOW_EX_STYLE,
    WINDOW_STYLE, WM_APP, WM_ERASEBKGND, WM_KEYDOWN, WM_SETFONT, WNDPROC, WS_CHILD, WS_VISIBLE,
};
use windows::core::w;

use crate::capture::RectI;
use crate::platform::dib::Dib;
use crate::util::error::Result;

pub const WM_TEXT_COMMIT: u32 = WM_APP + 2;
pub const WM_TEXT_CANCEL: u32 = WM_APP + 3;

static OLD_PROC: AtomicIsize = AtomicIsize::new(0);
static BACKGROUND_DC: AtomicIsize = AtomicIsize::new(0);
static BACKGROUND_X: AtomicI32 = AtomicI32::new(0);
static BACKGROUND_Y: AtomicI32 = AtomicI32::new(0);

pub struct TextEditor {
    pub hwnd: HWND,
    /// 文字锚点（document 本地坐标）。
    pub doc_position: (i32, i32),
    position: (i32, i32),
    size: (i32, i32),
    font: HGDIOBJ,
    ime_context: HIMC,
}

impl TextEditor {
    /// 在遮罩客户端坐标 `client_position` 处创建输入框，背景取自 `background` 的对应区域。
    pub fn open(
        parent: HWND,
        client_position: (i32, i32),
        doc_position: (i32, i32),
        scale: f32,
        font_size: f32,
        background: &Dib,
    ) -> Result<Self> {
        let scale = scale.max(1.0);
        let width = ((320.0 * scale).round() as i32).max(160);
        let height = ((font_size * 1.7 * 3.0).round() as i32).max(64);
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

            BACKGROUND_DC.store(background.dc.0 as isize, Ordering::SeqCst);
            BACKGROUND_X.store(client_position.0, Ordering::SeqCst);
            BACKGROUND_Y.store(client_position.1, Ordering::SeqCst);

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
                position: client_position,
                size: (width, height),
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

    pub fn position(&self) -> (i32, i32) {
        self.position
    }

    pub fn client_rect(&self) -> RectI {
        RectI::new(self.position.0, self.position.1, self.size.0, self.size.1)
    }

    pub fn size(&self) -> (i32, i32) {
        self.size
    }

    /// 移动输入框；文字锚点同步平移。
    pub fn move_to(&mut self, position: (i32, i32)) {
        let dx = position.0 - self.position.0;
        let dy = position.1 - self.position.1;
        if dx == 0 && dy == 0 {
            return;
        }
        self.position = position;
        self.doc_position = (self.doc_position.0 + dx, self.doc_position.1 + dy);
        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                None,
                position.0,
                position.1,
                0,
                0,
                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
            BACKGROUND_X.store(position.0, Ordering::SeqCst);
            BACKGROUND_Y.store(position.1, Ordering::SeqCst);
        }
    }

    /// 调整输入框大小（保持左上角位置）。
    pub fn resize_to(&mut self, size: (i32, i32)) {
        if size == self.size {
            return;
        }
        self.size = size;
        unsafe {
            let _ = SetWindowPos(
                self.hwnd,
                None,
                0,
                0,
                size.0,
                size.1,
                SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
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
            BACKGROUND_DC.store(0, Ordering::SeqCst);
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
        match message {
            WM_ERASEBKGND => {
                let background = HDC(BACKGROUND_DC.load(Ordering::SeqCst) as *mut _);
                if !background.0.is_null() {
                    let hdc = HDC(wparam.0 as *mut _);
                    // 只重绘失效区域；整块重绘会把编辑框里已有的文字擦掉。
                    let mut clip = RECT::default();
                    if GetClipBox(hdc, &mut clip).0 > 1 {
                        let width = clip.right - clip.left;
                        let height = clip.bottom - clip.top;
                        if width > 0 && height > 0 {
                            let _ = BitBlt(
                                hdc,
                                clip.left,
                                clip.top,
                                width,
                                height,
                                Some(background),
                                BACKGROUND_X.load(Ordering::SeqCst) + clip.left,
                                BACKGROUND_Y.load(Ordering::SeqCst) + clip.top,
                                SRCCOPY,
                            );
                        }
                    }
                }
                return LRESULT(1);
            }
            WM_KEYDOWN => {
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
            _ => {}
        }

        let previous: WNDPROC = std::mem::transmute(OLD_PROC.load(Ordering::SeqCst));
        CallWindowProcW(previous, hwnd, message, wparam, lparam)
    }
}
