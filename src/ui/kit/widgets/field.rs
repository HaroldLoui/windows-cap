//! 输入框的圆角底板与焦点边框（原生 EDIT 叠加在其上）。

use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Gdi::{
    CreatePen, CreateSolidBrush, DeleteObject, FillRect, HDC, HGDIOBJ, PS_SOLID, RoundRect,
    SelectObject,
};

use crate::ui::kit::dpi::s;
use crate::ui::kit::theme::{ACCENT_COLORREF, FIELD_BG, WINDOW_BORDER};

/// 绘制输入框底板；`focused` 时使用强调色边框。
pub unsafe fn draw(hdc: HDC, rect: RECT, focused: bool, scale: f32) {
    unsafe {
        let brush = CreateSolidBrush(FIELD_BG);
        let border = if focused {
            ACCENT_COLORREF
        } else {
            WINDOW_BORDER
        };
        let pen = CreatePen(PS_SOLID, if focused { 2 } else { 1 }, border);
        let old_brush = SelectObject(hdc, HGDIOBJ(brush.0));
        let old_pen = SelectObject(hdc, HGDIOBJ(pen.0));
        let _ = FillRect(hdc, &rect, brush);
        let radius = s(7.0, scale);
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
