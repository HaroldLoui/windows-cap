//! 文本与字体相关的基础工具。

use windows::Win32::Foundation::{COLORREF, RECT};
use windows::Win32::Graphics::Gdi::{
    CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateFontW, DEFAULT_CHARSET, DEFAULT_GUI_FONT,
    DT_CALCRECT, DT_CENTER, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, DrawTextW, FF_DONTCARE,
    FW_NORMAL, GetStockObject, HDC, HFONT, HGDIOBJ, OUT_DEFAULT_PRECIS, SelectObject, SetBkMode,
    SetTextColor, TRANSPARENT,
};
use windows::core::w;

/// UTF-16 字符串，含结尾 NUL。
pub fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// 按 DPI 创建界面字体；失败时退回系统默认字体。
pub unsafe fn create_font(scale: f32) -> HFONT {
    unsafe {
        let height = -((15.0 * scale).round() as i32).max(10);
        let font = CreateFontW(
            height,
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
        if font.0.is_null() {
            HFONT(GetStockObject(DEFAULT_GUI_FONT).0)
        } else {
            font
        }
    }
}

/// 在矩形内居中绘制单行文字；返回文字宽高。
pub unsafe fn text_size(hdc: HDC, text: &str, font: HFONT) -> (i32, i32) {
    unsafe {
        let old_font = SelectObject(hdc, HGDIOBJ(font.0));
        let mut wide = wide(text);
        let mut rect = RECT::default();
        DrawTextW(
            hdc,
            &mut wide,
            &mut rect,
            DT_CALCRECT | DT_SINGLELINE | DT_NOPREFIX,
        );
        SelectObject(hdc, old_font);
        (
            (rect.right - rect.left).max(0),
            (rect.bottom - rect.top).max(0),
        )
    }
}

/// 在矩形内居中绘制单行文字。
pub unsafe fn draw_text_centered(
    hdc: HDC,
    text: &str,
    rect: &mut RECT,
    font: HFONT,
    color: COLORREF,
) {
    unsafe {
        let old_font = SelectObject(hdc, HGDIOBJ(font.0));
        SetBkMode(hdc, TRANSPARENT);
        SetTextColor(hdc, color);
        let mut wide = wide(text);
        DrawTextW(
            hdc,
            &mut wide,
            rect,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
        );
        SelectObject(hdc, old_font);
    }
}
