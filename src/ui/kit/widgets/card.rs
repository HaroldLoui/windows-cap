//! 圆角卡片面板。

use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Gdi::{
    CreatePen, CreateSolidBrush, DeleteObject, FillRect, HDC, HGDIOBJ, PS_SOLID, RoundRect,
    SelectObject,
};

use crate::ui::kit::dpi::s;
use crate::ui::kit::theme::{CARD_BG, CARD_BORDER};

/// 绘制卡片底色与细边框。
pub unsafe fn draw(hdc: HDC, rect: RECT, scale: f32) {
    unsafe {
        let brush = CreateSolidBrush(CARD_BG);
        let pen = CreatePen(PS_SOLID, 1, CARD_BORDER);
        let old_brush = SelectObject(hdc, HGDIOBJ(brush.0));
        let old_pen = SelectObject(hdc, HGDIOBJ(pen.0));
        let _ = FillRect(hdc, &rect, brush);
        let radius = s(10.0, scale);
        let _ = RoundRect(
            hdc,
            rect.left,
            rect.top,
            rect.right,
            rect.bottom,
            radius,
            radius,
        );
        SelectObject(hdc, old_pen);
        SelectObject(hdc, old_brush);
        let _ = DeleteObject(HGDIOBJ(pen.0));
        let _ = DeleteObject(HGDIOBJ(brush.0));
    }
}
