//! 圆角按钮绘制。

use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Gdi::{
    CreatePen, CreateSolidBrush, DT_CENTER, DT_SINGLELINE, DT_VCENTER, DeleteObject, DrawTextW,
    FillRect, HDC, HFONT, HGDIOBJ, PS_SOLID, RoundRect, SelectObject, SetBkMode, SetTextColor,
    TRANSPARENT,
};

use crate::ui::kit::dpi::s;
use crate::ui::kit::text::wide;
use crate::ui::kit::theme::{
    ACCENT_COLORREF, WINDOW_ACCENT_TEXT, WINDOW_BG, WINDOW_BORDER, WINDOW_HOVER, WINDOW_TEXT,
};

#[derive(Clone, Copy)]
pub enum ButtonKind {
    Accent,
    Normal,
}

/// 绘制一个圆角按钮。
#[allow(clippy::too_many_arguments)]
pub unsafe fn draw(
    hdc: HDC,
    rect: RECT,
    label: &str,
    kind: ButtonKind,
    pressed: bool,
    focused: bool,
    scale: f32,
    font: HFONT,
) {
    unsafe {
        let (background, border, text) = match kind {
            ButtonKind::Accent => (ACCENT_COLORREF, ACCENT_COLORREF, WINDOW_ACCENT_TEXT),
            ButtonKind::Normal => (
                if pressed { WINDOW_HOVER } else { WINDOW_BG },
                WINDOW_BORDER,
                WINDOW_TEXT,
            ),
        };
        let brush = CreateSolidBrush(background);
        let pen = CreatePen(PS_SOLID, 1, border);
        let old_brush = SelectObject(hdc, HGDIOBJ(brush.0));
        let old_pen = SelectObject(hdc, HGDIOBJ(pen.0));
        let old_font = SelectObject(hdc, HGDIOBJ(font.0));
        let _ = FillRect(hdc, &rect, brush);
        let radius = s(8.0, scale);
        let _ = RoundRect(
            hdc,
            rect.left,
            rect.top,
            rect.right,
            rect.bottom,
            radius,
            radius,
        );
        if focused {
            let focus = RECT {
                left: rect.left + s(2.0, scale),
                top: rect.top + s(2.0, scale),
                right: rect.right - s(2.0, scale),
                bottom: rect.bottom - s(2.0, scale),
            };
            let focus_pen = CreatePen(PS_SOLID, 1, WINDOW_ACCENT_TEXT);
            let old = SelectObject(hdc, HGDIOBJ(focus_pen.0));
            let _ = RoundRect(
                hdc,
                focus.left,
                focus.top,
                focus.right,
                focus.bottom,
                radius,
                radius,
            );
            SelectObject(hdc, old);
            let _ = DeleteObject(HGDIOBJ(focus_pen.0));
        }
        SetBkMode(hdc, TRANSPARENT);
        SetTextColor(hdc, text);
        let mut text16 = wide(label);
        let mut text_rect = rect;
        DrawTextW(
            hdc,
            &mut text16,
            &mut text_rect,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE,
        );
        SelectObject(hdc, old_font);
        SelectObject(hdc, old_pen);
        SelectObject(hdc, old_brush);
        let _ = DeleteObject(HGDIOBJ(pen.0));
        let _ = DeleteObject(HGDIOBJ(brush.0));
    }
}
