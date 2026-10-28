//! 自绘开关（owner-draw 按钮）。

use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Gdi::{
    CreatePen, CreateSolidBrush, DeleteObject, Ellipse, GetStockObject, HDC, HGDIOBJ, NULL_BRUSH,
    PS_SOLID, RoundRect, SelectObject,
};

use crate::ui::kit::dpi::s;
use crate::ui::kit::theme::{ACCENT_COLORREF, TOGGLE_KNOB, TOGGLE_OFF, WINDOW_BORDER, WINDOW_TEXT};

/// 在给定矩形内画一个开关；`on` 决定滑轨颜色与滑块位置。
pub unsafe fn draw(hdc: HDC, rect: RECT, on: bool, focused: bool, scale: f32) {
    unsafe {
        let height = s(20.0, scale).min(rect.bottom - rect.top);
        let width = s(40.0, scale).min(rect.right - rect.left);
        let left = rect.left + ((rect.right - rect.left) - width) / 2;
        let top = rect.top + ((rect.bottom - rect.top) - height) / 2;
        let right = left + width;
        let bottom = top + height;
        let radius = height;

        let track = if on { ACCENT_COLORREF } else { TOGGLE_OFF };
        let brush = CreateSolidBrush(track);
        let pen = CreatePen(PS_SOLID, 1, track);
        let old_brush = SelectObject(hdc, HGDIOBJ(brush.0));
        let old_pen = SelectObject(hdc, HGDIOBJ(pen.0));
        let _ = RoundRect(hdc, left, top, right, bottom, radius, radius);
        SelectObject(hdc, old_pen);
        SelectObject(hdc, old_brush);
        let _ = DeleteObject(HGDIOBJ(pen.0));
        let _ = DeleteObject(HGDIOBJ(brush.0));

        let knob_size = height - s(4.0, scale);
        let knob_left = if on {
            right - knob_size - s(2.0, scale)
        } else {
            left + s(2.0, scale)
        };
        let knob_top = top + s(2.0, scale);
        let knob_brush = CreateSolidBrush(TOGGLE_KNOB);
        let knob_pen = CreatePen(PS_SOLID, 1, WINDOW_BORDER);
        let old_brush = SelectObject(hdc, HGDIOBJ(knob_brush.0));
        let old_pen = SelectObject(hdc, HGDIOBJ(knob_pen.0));
        let _ = Ellipse(
            hdc,
            knob_left,
            knob_top,
            knob_left + knob_size,
            knob_top + knob_size,
        );
        SelectObject(hdc, old_pen);
        SelectObject(hdc, old_brush);
        let _ = DeleteObject(HGDIOBJ(knob_pen.0));
        let _ = DeleteObject(HGDIOBJ(knob_brush.0));

        if focused {
            let focus = RECT {
                left: left - s(2.0, scale),
                top: top - s(2.0, scale),
                right: right + s(2.0, scale),
                bottom: bottom + s(2.0, scale),
            };
            let focus_pen = CreatePen(PS_SOLID, 1, WINDOW_TEXT);
            let old_pen = SelectObject(hdc, HGDIOBJ(focus_pen.0));
            let old_brush = SelectObject(hdc, GetStockObject(NULL_BRUSH));
            let _ = RoundRect(
                hdc,
                focus.left,
                focus.top,
                focus.right,
                focus.bottom,
                radius + s(4.0, scale),
                radius + s(4.0, scale),
            );
            SelectObject(hdc, old_brush);
            SelectObject(hdc, old_pen);
            let _ = DeleteObject(HGDIOBJ(focus_pen.0));
        }
    }
}
