//! 文本与字体相关的基础工具。

use windows::Win32::Graphics::Gdi::{
    CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateFontW, DEFAULT_CHARSET, DEFAULT_GUI_FONT,
    FF_DONTCARE, FW_NORMAL, GetStockObject, HFONT, OUT_DEFAULT_PRECIS,
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
