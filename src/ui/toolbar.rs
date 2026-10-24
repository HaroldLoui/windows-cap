//! 标注工具栏：两行纯图标布局、外部优先放置、GDI+ 绘制。

use std::ptr::null_mut;

use windows::Win32::Foundation::{COLORREF, RECT};
use windows::Win32::Graphics::Gdi::{
    DT_CENTER, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, DrawTextW, HDC, HFONT, HGDIOBJ,
    SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
};
use windows::Win32::Graphics::GdiPlus::{
    FillModeAlternate, FlushIntentionSync, GdipAddPathArc, GdipClosePathFigure, GdipCreateFromHDC,
    GdipCreatePath, GdipCreatePen1, GdipCreateSolidFill, GdipDeleteBrush, GdipDeleteGraphics,
    GdipDeletePath, GdipDeletePen, GdipDrawEllipse, GdipDrawLine, GdipDrawLines, GdipDrawPath,
    GdipDrawPolygon, GdipDrawRectangle, GdipFillEllipse, GdipFillPath, GdipFillPolygon,
    GdipFillRectangle, GdipFlush, GdipSetSmoothingMode, GpGraphics, GpPath, GpPen, GpSolidFill,
    PointF, SmoothingModeAntiAlias, UnitPixel,
};

use crate::annotate::render;
use crate::capture::{RectI, VirtualScreen};

const ACCENT: u32 = 0xFFFF_9B2E;
const PANEL_BG: u32 = 0xF21B_1B20;
const PANEL_BORDER: u32 = 0xFF3A_3A42;
const ICON_COLOR: u32 = 0xFFE8_E8EC;
const ICON_DIM: u32 = 0xFF6E_6E76;
const HOVER_BG: u32 = 0xFF2E_2E36;
const ACTIVE_ICON: u32 = 0xFF1A_1A1A;
const CLOSE_COLOR: u32 = 0xFFFF_7870;
const CLOSE_HOVER: u32 = 0xCC7A_2A28;
const SEPARATOR: u32 = 0x40FF_FFFF;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Select,
    Pen,
    Line,
    Arrow,
    Rect,
    Ellipse,
    Highlight,
    Text,
    Mosaic,
    Step,
}

pub const TOOLS: [Tool; 10] = [
    Tool::Select,
    Tool::Pen,
    Tool::Line,
    Tool::Arrow,
    Tool::Rect,
    Tool::Ellipse,
    Tool::Highlight,
    Tool::Text,
    Tool::Mosaic,
    Tool::Step,
];

impl Tool {
    pub fn shortcut(self) -> u16 {
        match self {
            Tool::Select => b'V' as u16,
            Tool::Pen => b'P' as u16,
            Tool::Line => b'L' as u16,
            Tool::Arrow => b'A' as u16,
            Tool::Rect => b'R' as u16,
            Tool::Ellipse => b'O' as u16,
            Tool::Highlight => b'H' as u16,
            Tool::Text => b'T' as u16,
            Tool::Mosaic => b'B' as u16,
            Tool::Step => b'N' as u16,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolbarAction {
    Tool(Tool),
    Color(u32),
    Width(i32),
    Undo,
    Redo,
    Copy,
    Save,
    Cancel,
    Noop,
}

pub const COLORS: [u32; 8] = [
    0xFFFF_3B30,
    0xFFFF_9500,
    0xFFFF_CC00,
    0xFF34_C759,
    0xFF00_7AFF,
    0xFFAF_52DE,
    0xFFFF_FFFF,
    0xFF00_0000,
];

#[derive(Clone, Copy, Debug)]
pub struct ToolbarState {
    pub tool: Tool,
    pub color: u32,
    pub width: f32,
    pub can_undo: bool,
    pub can_redo: bool,
}

#[derive(Clone, Copy)]
enum Icon {
    Tool(Tool),
    Undo,
    Redo,
    Copy,
    Save,
    Close,
    Minus,
    Plus,
}

struct Button {
    rect: RectI,
    action: ToolbarAction,
    icon: Option<Icon>,
    swatch: Option<u32>,
    label: Option<String>,
    enabled: bool,
    active: bool,
}

pub struct Toolbar {
    pub rect: RectI,
    buttons: Vec<Button>,
    row1_divider: i32,
    row2_divider: i32,
    scale: f32,
}

impl Toolbar {
    pub fn layout(selection: RectI, area: VirtualScreen, scale: f32, state: &ToolbarState) -> Self {
        let scale = scale.max(1.0);
        let cell = (30.0 * scale).round() as i32;
        let gap = (4.0 * scale).round() as i32;
        let pad = (8.0 * scale).round() as i32;
        let separator_margin = (6.0 * scale).round() as i32;

        let tools_width = 10 * cell + 9 * gap;
        let actions_width = 5 * cell + 4 * gap;
        let colors_width = 8 * cell + 7 * gap;
        let width_controls = cell + gap + (40.0 * scale) as i32 + gap + cell;
        let divider_width = separator_margin * 2 + 1;

        let row1_width = tools_width + divider_width + actions_width;
        let row2_width = colors_width + divider_width + width_controls;
        let panel_width = row1_width.max(row2_width) + pad * 2;
        let panel_height = pad * 2 + cell * 2 + gap;

        let local = RectI::new(
            selection.x - area.x,
            selection.y - area.y,
            selection.width,
            selection.height,
        );
        let panel = place_panel(
            local,
            area.width,
            area.height,
            panel_width,
            panel_height,
            scale,
        );

        let mut buttons = Vec::with_capacity(25);

        // 第一行：10 个工具 + 分隔 + 撤销/重做/复制/保存/关闭
        let row1_y = panel.y + pad;
        let mut x = panel.x + pad;
        for tool in TOOLS {
            buttons.push(Button {
                rect: RectI::new(x, row1_y, cell, cell),
                action: ToolbarAction::Tool(tool),
                icon: Some(Icon::Tool(tool)),
                swatch: None,
                label: None,
                enabled: true,
                active: state.tool == tool,
            });
            x += cell + gap;
        }
        x += separator_margin;
        let row1_divider = x;
        x += separator_margin + 1;
        for (icon, action, enabled) in [
            (Icon::Undo, ToolbarAction::Undo, state.can_undo),
            (Icon::Redo, ToolbarAction::Redo, state.can_redo),
            (Icon::Copy, ToolbarAction::Copy, true),
            (Icon::Save, ToolbarAction::Save, true),
            (Icon::Close, ToolbarAction::Cancel, true),
        ] {
            buttons.push(Button {
                rect: RectI::new(x, row1_y, cell, cell),
                action,
                icon: Some(icon),
                swatch: None,
                label: None,
                enabled,
                active: false,
            });
            x += cell + gap;
        }

        // 第二行：8 色 + 分隔 + 线宽 −/数值/+
        let row2_y = panel.y + pad + cell + gap;
        let mut x = panel.x + pad;
        for color in COLORS {
            buttons.push(Button {
                rect: RectI::new(x, row2_y, cell, cell),
                action: ToolbarAction::Color(color),
                icon: None,
                swatch: Some(color),
                label: None,
                enabled: true,
                active: state.color == color,
            });
            x += cell + gap;
        }
        x += separator_margin;
        let row2_divider = x;
        x += separator_margin + 1;

        buttons.push(Button {
            rect: RectI::new(x, row2_y, cell, cell),
            action: ToolbarAction::Width(-1),
            icon: Some(Icon::Minus),
            swatch: None,
            label: None,
            enabled: state.width > 1.0,
            active: false,
        });
        x += cell + gap;
        let value_width = (40.0 * scale) as i32;
        buttons.push(Button {
            rect: RectI::new(x, row2_y, value_width, cell),
            action: ToolbarAction::Noop,
            icon: None,
            swatch: None,
            label: Some(format!("{:.0}", state.width)),
            enabled: true,
            active: false,
        });
        x += value_width + gap;
        buttons.push(Button {
            rect: RectI::new(x, row2_y, cell, cell),
            action: ToolbarAction::Width(1),
            icon: Some(Icon::Plus),
            swatch: None,
            label: None,
            enabled: state.width < 40.0,
            active: false,
        });

        Self {
            rect: panel,
            buttons,
            row1_divider,
            row2_divider,
            scale,
        }
    }

    pub fn hit(&self, x: i32, y: i32) -> Option<ToolbarAction> {
        self.buttons
            .iter()
            .find(|button| {
                x >= button.rect.x
                    && y >= button.rect.y
                    && x < button.rect.x + button.rect.width
                    && y < button.rect.y + button.rect.height
            })
            .map(|button| button.action)
    }

    pub fn paint(&self, hdc: HDC, font: HFONT, hover: Option<ToolbarAction>) {
        let _ = render::init();
        unsafe {
            let mut graphics: *mut GpGraphics = null_mut();
            if GdipCreateFromHDC(hdc, &mut graphics).0 != 0 || graphics.is_null() {
                return;
            }
            let _ = GdipSetSmoothingMode(graphics, SmoothingModeAntiAlias);
            let s = self.scale;

            let rect = self.rect;
            fill_rounded(
                graphics,
                rect.x as f32 + 2.0 * s,
                rect.y as f32 + 3.0 * s,
                rect.width as f32,
                rect.height as f32,
                13.0 * s,
                0x6600_0000,
            );
            fill_rounded(
                graphics,
                rect.x as f32,
                rect.y as f32,
                rect.width as f32,
                rect.height as f32,
                12.0 * s,
                PANEL_BG,
            );
            stroke_rounded(
                graphics,
                rect.x as f32,
                rect.y as f32,
                rect.width as f32,
                rect.height as f32,
                12.0 * s,
                PANEL_BORDER,
                1.0 * s,
            );

            let sep_top = rect.y as f32 + 8.0 * s;
            let sep_bottom = rect.y as f32 + rect.height as f32 - 8.0 * s;
            for divider in [self.row1_divider, self.row2_divider] {
                stroke_line(
                    graphics,
                    divider as f32,
                    sep_top,
                    divider as f32,
                    sep_bottom,
                    SEPARATOR,
                    1.0 * s,
                );
            }

            for button in &self.buttons {
                let hovered = button.enabled
                    && !matches!(button.action, ToolbarAction::Noop)
                    && hover == Some(button.action);
                let rect = button.rect;
                if let Some(color) = button.swatch {
                    let cx = rect.x as f32 + rect.width as f32 / 2.0;
                    let cy = rect.y as f32 + rect.height as f32 / 2.0;
                    let radius = 10.0 * s;
                    fill_ellipse(
                        graphics,
                        cx - radius,
                        cy - radius,
                        radius * 2.0,
                        radius * 2.0,
                        color,
                    );
                    stroke_ellipse(
                        graphics,
                        cx - radius - 1.0,
                        cy - radius - 1.0,
                        radius * 2.0 + 2.0,
                        radius * 2.0 + 2.0,
                        0x6600_0000,
                        1.0 * s,
                    );
                    if button.active {
                        stroke_ellipse(
                            graphics,
                            cx - radius - 2.5 * s,
                            cy - radius - 2.5 * s,
                            radius * 2.0 + 5.0 * s,
                            radius * 2.0 + 5.0 * s,
                            0xFFFF_FFFF,
                            1.6 * s,
                        );
                    }
                    continue;
                }

                if button.active {
                    fill_rounded(
                        graphics,
                        rect.x as f32,
                        rect.y as f32,
                        rect.width as f32,
                        rect.height as f32,
                        8.0 * s,
                        ACCENT,
                    );
                } else if hovered {
                    let color = if matches!(button.icon, Some(Icon::Close)) {
                        CLOSE_HOVER
                    } else {
                        HOVER_BG
                    };
                    fill_rounded(
                        graphics,
                        rect.x as f32,
                        rect.y as f32,
                        rect.width as f32,
                        rect.height as f32,
                        8.0 * s,
                        color,
                    );
                }

                let color = if !button.enabled {
                    ICON_DIM
                } else if button.active {
                    ACTIVE_ICON
                } else if matches!(button.icon, Some(Icon::Close)) {
                    CLOSE_COLOR
                } else {
                    ICON_COLOR
                };

                if let Some(icon) = button.icon {
                    let icon_size = 18.0 * s;
                    let x = rect.x as f32 + (rect.width as f32 - icon_size) / 2.0;
                    let y = rect.y as f32 + (rect.height as f32 - icon_size) / 2.0;
                    draw_icon(graphics, icon, x, y, s, color);
                }
            }

            let _ = GdipFlush(graphics, FlushIntentionSync);

            let old_font = SelectObject(hdc, HGDIOBJ(font.0));
            SetBkMode(hdc, TRANSPARENT);
            SetTextColor(hdc, COLORREF(0x00E8_E8EC));
            for button in &self.buttons {
                if let Some(label) = &button.label {
                    let mut wide: Vec<u16> = label.encode_utf16().collect();
                    let mut text_rect = RECT {
                        left: button.rect.x,
                        top: button.rect.y,
                        right: button.rect.x + button.rect.width,
                        bottom: button.rect.y + button.rect.height,
                    };
                    DrawTextW(
                        hdc,
                        &mut wide,
                        &mut text_rect,
                        DT_SINGLELINE | DT_CENTER | DT_VCENTER | DT_NOPREFIX,
                    );
                }
            }
            SelectObject(hdc, old_font);

            let _ = GdipDeleteGraphics(graphics);
        }
    }
}

fn place_panel(
    selection: RectI,
    screen_width: i32,
    screen_height: i32,
    width: i32,
    height: i32,
    scale: f32,
) -> RectI {
    let gap = (8.0 * scale).round() as i32;
    let margin = (8.0 * scale).round() as i32;
    let min_x = margin;
    let min_y = margin;
    let max_x = (screen_width - width - margin).max(min_x);
    let max_y = (screen_height - height - margin).max(min_y);
    let clamp_x = |x: i32| x.clamp(min_x, max_x);
    let clamp_y = |y: i32| y.clamp(min_y, max_y);

    let below = selection.y + selection.height + gap;
    if below + height <= screen_height - margin {
        return RectI::new(
            clamp_x(selection.x + selection.width - width),
            below,
            width,
            height,
        );
    }
    let above = selection.y - gap - height;
    if above >= margin {
        return RectI::new(
            clamp_x(selection.x + selection.width - width),
            above,
            width,
            height,
        );
    }
    let right = selection.x + selection.width + gap;
    if right + width <= screen_width - margin {
        return RectI::new(
            right,
            clamp_y(selection.y + selection.height - height),
            width,
            height,
        );
    }
    let left = selection.x - gap - width;
    if left >= margin {
        return RectI::new(
            left,
            clamp_y(selection.y + selection.height - height),
            width,
            height,
        );
    }
    RectI::new(
        clamp_x(selection.x + selection.width - width - gap),
        clamp_y(selection.y + selection.height - height - gap),
        width,
        height,
    )
}

// ---------------- GDI+ 基础绘制 ----------------

unsafe fn fill_rounded(
    graphics: *mut GpGraphics,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    radius: f32,
    color: u32,
) {
    unsafe {
        let mut path: *mut GpPath = null_mut();
        if GdipCreatePath(FillModeAlternate, &mut path).0 != 0 || path.is_null() {
            return;
        }
        add_rounded_rect(path, x, y, width, height, radius);
        let mut brush: *mut GpSolidFill = null_mut();
        if GdipCreateSolidFill(color, &mut brush).0 == 0 && !brush.is_null() {
            let _ = GdipFillPath(graphics, brush as *mut _, path);
            let _ = GdipDeleteBrush(brush as *mut _);
        }
        let _ = GdipDeletePath(path);
    }
}

#[allow(clippy::too_many_arguments)]
unsafe fn stroke_rounded(
    graphics: *mut GpGraphics,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    radius: f32,
    color: u32,
    thickness: f32,
) {
    unsafe {
        let mut path: *mut GpPath = null_mut();
        if GdipCreatePath(FillModeAlternate, &mut path).0 != 0 || path.is_null() {
            return;
        }
        add_rounded_rect(path, x, y, width, height, radius);
        let mut pen: *mut GpPen = null_mut();
        if GdipCreatePen1(color, thickness, UnitPixel, &mut pen).0 == 0 && !pen.is_null() {
            let _ = GdipDrawPath(graphics, pen, path);
            let _ = GdipDeletePen(pen);
        }
        let _ = GdipDeletePath(path);
    }
}

unsafe fn add_rounded_rect(path: *mut GpPath, x: f32, y: f32, w: f32, h: f32, r: f32) {
    unsafe {
        let d = 2.0 * r;
        let _ = GdipAddPathArc(path, x, y, d, d, 180.0, 90.0);
        let _ = GdipAddPathArc(path, x + w - d, y, d, d, 270.0, 90.0);
        let _ = GdipAddPathArc(path, x + w - d, y + h - d, d, d, 0.0, 90.0);
        let _ = GdipAddPathArc(path, x, y + h - d, d, d, 90.0, 90.0);
        let _ = GdipClosePathFigure(path);
    }
}

unsafe fn solid_fill(graphics: *mut GpGraphics, x: f32, y: f32, w: f32, h: f32, color: u32) {
    unsafe {
        let mut brush: *mut GpSolidFill = null_mut();
        if GdipCreateSolidFill(color, &mut brush).0 == 0 && !brush.is_null() {
            let _ = GdipFillRectangle(graphics, brush as *mut _, x, y, w, h);
            let _ = GdipDeleteBrush(brush as *mut _);
        }
    }
}

unsafe fn fill_ellipse(graphics: *mut GpGraphics, x: f32, y: f32, w: f32, h: f32, color: u32) {
    unsafe {
        let mut brush: *mut GpSolidFill = null_mut();
        if GdipCreateSolidFill(color, &mut brush).0 == 0 && !brush.is_null() {
            let _ = GdipFillEllipse(graphics, brush as *mut _, x, y, w, h);
            let _ = GdipDeleteBrush(brush as *mut _);
        }
    }
}

unsafe fn stroke_ellipse(
    graphics: *mut GpGraphics,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    color: u32,
    thickness: f32,
) {
    unsafe {
        let mut pen: *mut GpPen = null_mut();
        if GdipCreatePen1(color, thickness, UnitPixel, &mut pen).0 == 0 && !pen.is_null() {
            let _ = GdipDrawEllipse(graphics, pen, x, y, w, h);
            let _ = GdipDeletePen(pen);
        }
    }
}

unsafe fn stroke_line(
    graphics: *mut GpGraphics,
    x1: f32,
    y1: f32,
    x2: f32,
    y2: f32,
    color: u32,
    thickness: f32,
) {
    unsafe {
        let mut pen: *mut GpPen = null_mut();
        if GdipCreatePen1(color, thickness, UnitPixel, &mut pen).0 == 0 && !pen.is_null() {
            let _ = GdipDrawLine(graphics, pen, x1, y1, x2, y2);
            let _ = GdipDeletePen(pen);
        }
    }
}

unsafe fn stroke_lines(graphics: *mut GpGraphics, points: &[PointF], color: u32, thickness: f32) {
    unsafe {
        let mut pen: *mut GpPen = null_mut();
        if GdipCreatePen1(color, thickness, UnitPixel, &mut pen).0 == 0 && !pen.is_null() {
            let _ = GdipDrawLines(graphics, pen, points.as_ptr(), points.len() as i32);
            let _ = GdipDeletePen(pen);
        }
    }
}

unsafe fn stroke_polygon(graphics: *mut GpGraphics, points: &[PointF], color: u32, thickness: f32) {
    unsafe {
        let mut pen: *mut GpPen = null_mut();
        if GdipCreatePen1(color, thickness, UnitPixel, &mut pen).0 == 0 && !pen.is_null() {
            let _ = GdipDrawPolygon(graphics, pen, points.as_ptr(), points.len() as i32);
            let _ = GdipDeletePen(pen);
        }
    }
}

unsafe fn fill_polygon(graphics: *mut GpGraphics, points: &[PointF], color: u32) {
    unsafe {
        let mut brush: *mut GpSolidFill = null_mut();
        if GdipCreateSolidFill(color, &mut brush).0 == 0 && !brush.is_null() {
            let _ = GdipFillPolygon(
                graphics,
                brush as *mut _,
                points.as_ptr(),
                points.len() as i32,
                FillModeAlternate,
            );
            let _ = GdipDeleteBrush(brush as *mut _);
        }
    }
}

unsafe fn stroke_rect(
    graphics: *mut GpGraphics,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
    color: u32,
    thickness: f32,
) {
    unsafe {
        let mut pen: *mut GpPen = null_mut();
        if GdipCreatePen1(color, thickness, UnitPixel, &mut pen).0 == 0 && !pen.is_null() {
            let _ = GdipDrawRectangle(graphics, pen, x, y, w, h);
            let _ = GdipDeletePen(pen);
        }
    }
}

// ---------------- 图标（18 单位坐标系） ----------------

unsafe fn draw_icon(graphics: *mut GpGraphics, icon: Icon, x: f32, y: f32, s: f32, color: u32) {
    unsafe {
        let u = |v: f32| v * s;
        let p = |px: f32, py: f32| PointF {
            X: x + u(px),
            Y: y + u(py),
        };
        match icon {
            Icon::Tool(Tool::Select) => {
                stroke_polygon(
                    graphics,
                    &[
                        p(3.0, 1.0),
                        p(3.0, 15.0),
                        p(7.0, 11.0),
                        p(10.0, 17.0),
                        p(13.0, 15.0),
                        p(10.0, 9.0),
                        p(16.0, 8.0),
                    ],
                    color,
                    u(1.4),
                );
            }
            Icon::Tool(Tool::Pen) => {
                stroke_line(
                    graphics,
                    x + u(4.0),
                    y + u(14.0),
                    x + u(13.0),
                    y + u(4.0),
                    color,
                    u(2.0),
                );
                fill_polygon(graphics, &[p(2.0, 16.0), p(4.0, 12.0), p(7.0, 15.0)], color);
                stroke_line(
                    graphics,
                    x + u(12.0),
                    y + u(3.0),
                    x + u(15.0),
                    y + u(6.0),
                    color,
                    u(2.0),
                );
            }
            Icon::Tool(Tool::Line) => {
                stroke_line(
                    graphics,
                    x + u(4.0),
                    y + u(14.0),
                    x + u(14.0),
                    y + u(4.0),
                    color,
                    u(1.7),
                );
                fill_ellipse(graphics, x + u(2.0), y + u(12.0), u(4.0), u(4.0), color);
                fill_ellipse(graphics, x + u(12.0), y + u(2.0), u(4.0), u(4.0), color);
            }
            Icon::Tool(Tool::Arrow) => {
                stroke_line(
                    graphics,
                    x + u(3.0),
                    y + u(15.0),
                    x + u(14.0),
                    y + u(4.0),
                    color,
                    u(1.7),
                );
                stroke_lines(
                    graphics,
                    &[p(8.0, 3.0), p(15.0, 3.0), p(15.0, 10.0)],
                    color,
                    u(1.7),
                );
            }
            Icon::Tool(Tool::Rect) => {
                stroke_rect(
                    graphics,
                    x + u(3.0),
                    y + u(3.0),
                    u(12.0),
                    u(12.0),
                    color,
                    u(1.6),
                );
            }
            Icon::Tool(Tool::Ellipse) => {
                stroke_ellipse(
                    graphics,
                    x + u(2.0),
                    y + u(3.0),
                    u(14.0),
                    u(12.0),
                    color,
                    u(1.6),
                );
            }
            Icon::Tool(Tool::Highlight) => {
                stroke_line(
                    graphics,
                    x + u(5.0),
                    y + u(13.0),
                    x + u(14.0),
                    y + u(4.0),
                    0xA0FF_D65C,
                    u(6.0),
                );
                fill_polygon(
                    graphics,
                    &[p(2.0, 16.0), p(3.0, 11.0), p(7.0, 14.0)],
                    0xE6FF_D65C,
                );
                stroke_line(
                    graphics,
                    x + u(4.0),
                    y + u(15.0),
                    x + u(13.0),
                    y + u(6.0),
                    color,
                    u(1.0),
                );
            }
            Icon::Tool(Tool::Text) => {
                stroke_line(
                    graphics,
                    x + u(3.0),
                    y + u(3.0),
                    x + u(15.0),
                    y + u(3.0),
                    color,
                    u(1.8),
                );
                stroke_line(
                    graphics,
                    x + u(9.0),
                    y + u(3.0),
                    x + u(9.0),
                    y + u(16.0),
                    color,
                    u(1.8),
                );
            }
            Icon::Tool(Tool::Mosaic) => {
                for row in 0..3 {
                    for col in 0..3 {
                        let bx = x + u(2.0) + u(5.0) * col as f32;
                        let by = y + u(2.0) + u(5.0) * row as f32;
                        if (row + col) % 2 == 0 {
                            solid_fill(graphics, bx, by, u(4.0), u(4.0), color);
                        } else {
                            stroke_rect(graphics, bx, by, u(4.0), u(4.0), color, u(1.0));
                        }
                    }
                }
            }
            Icon::Tool(Tool::Step) => {
                stroke_ellipse(
                    graphics,
                    x + u(2.0),
                    y + u(2.0),
                    u(14.0),
                    u(14.0),
                    color,
                    u(1.6),
                );
                stroke_line(
                    graphics,
                    x + u(9.0),
                    y + u(7.0),
                    x + u(9.0),
                    y + u(13.0),
                    color,
                    u(1.5),
                );
                stroke_line(
                    graphics,
                    x + u(6.5),
                    y + u(8.5),
                    x + u(9.0),
                    y + u(7.0),
                    color,
                    u(1.5),
                );
                stroke_line(
                    graphics,
                    x + u(7.0),
                    y + u(13.0),
                    x + u(11.0),
                    y + u(13.0),
                    color,
                    u(1.5),
                );
            }
            Icon::Undo => {
                stroke_lines(
                    graphics,
                    &[
                        p(3.0, 12.0),
                        p(9.0, 12.0),
                        p(12.0, 9.0),
                        p(9.0, 6.0),
                        p(3.0, 6.0),
                    ],
                    color,
                    u(1.6),
                );
                stroke_lines(graphics, &[p(3.0, 6.0), p(7.0, 3.0)], color, u(1.6));
                stroke_lines(graphics, &[p(3.0, 6.0), p(7.0, 9.0)], color, u(1.6));
            }
            Icon::Redo => {
                stroke_lines(
                    graphics,
                    &[
                        p(15.0, 12.0),
                        p(9.0, 12.0),
                        p(6.0, 9.0),
                        p(9.0, 6.0),
                        p(15.0, 6.0),
                    ],
                    color,
                    u(1.6),
                );
                stroke_lines(graphics, &[p(15.0, 6.0), p(11.0, 3.0)], color, u(1.6));
                stroke_lines(graphics, &[p(15.0, 6.0), p(11.0, 9.0)], color, u(1.6));
            }
            Icon::Copy => {
                stroke_rect(
                    graphics,
                    x + u(4.0),
                    y + u(4.0),
                    u(9.0),
                    u(10.0),
                    color,
                    u(1.4),
                );
                stroke_rect(
                    graphics,
                    x + u(7.0),
                    y + u(2.0),
                    u(9.0),
                    u(9.0),
                    color,
                    u(1.4),
                );
            }
            Icon::Save => {
                stroke_line(
                    graphics,
                    x + u(9.0),
                    y + u(2.0),
                    x + u(9.0),
                    y + u(11.0),
                    color,
                    u(1.6),
                );
                stroke_lines(
                    graphics,
                    &[p(5.0, 8.0), p(9.0, 12.0), p(13.0, 8.0)],
                    color,
                    u(1.6),
                );
                stroke_line(
                    graphics,
                    x + u(3.0),
                    y + u(15.0),
                    x + u(15.0),
                    y + u(15.0),
                    color,
                    u(1.6),
                );
            }
            Icon::Close => {
                stroke_line(
                    graphics,
                    x + u(4.0),
                    y + u(4.0),
                    x + u(14.0),
                    y + u(14.0),
                    color,
                    u(1.7),
                );
                stroke_line(
                    graphics,
                    x + u(14.0),
                    y + u(4.0),
                    x + u(4.0),
                    y + u(14.0),
                    color,
                    u(1.7),
                );
            }
            Icon::Minus => {
                stroke_line(
                    graphics,
                    x + u(4.0),
                    y + u(9.0),
                    x + u(14.0),
                    y + u(9.0),
                    color,
                    u(1.7),
                );
            }
            Icon::Plus => {
                stroke_line(
                    graphics,
                    x + u(4.0),
                    y + u(9.0),
                    x + u(14.0),
                    y + u(9.0),
                    color,
                    u(1.7),
                );
                stroke_line(
                    graphics,
                    x + u(9.0),
                    y + u(4.0),
                    x + u(9.0),
                    y + u(14.0),
                    color,
                    u(1.7),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> ToolbarState {
        ToolbarState {
            tool: Tool::Select,
            color: COLORS[0],
            width: 4.0,
            can_undo: false,
            can_redo: false,
        }
    }

    #[test]
    fn layout_prefers_outside_below() {
        let area = VirtualScreen {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
        };
        let toolbar = Toolbar::layout(RectI::new(400, 200, 600, 400), area, 1.0, &state());
        assert!(toolbar.rect.y >= 600, "应放在选区下方");
        assert!(toolbar.rect.y + toolbar.rect.height <= 1080);
    }

    #[test]
    fn layout_moves_above_when_no_room_below() {
        let area = VirtualScreen {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
        };
        let toolbar = Toolbar::layout(RectI::new(400, 700, 600, 350), area, 1.0, &state());
        assert!(
            toolbar.rect.y + toolbar.rect.height <= 700,
            "应放在选区上方"
        );
    }

    #[test]
    fn hit_exposes_tools_and_actions() {
        let area = VirtualScreen {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
        };
        let toolbar = Toolbar::layout(RectI::new(400, 200, 600, 400), area, 1.0, &state());
        let mut actions = Vec::new();
        for y in toolbar.rect.y..toolbar.rect.y + toolbar.rect.height {
            for x in toolbar.rect.x..toolbar.rect.x + toolbar.rect.width {
                if let Some(action) = toolbar.hit(x, y)
                    && !actions.contains(&action)
                {
                    actions.push(action);
                }
            }
        }
        for tool in TOOLS {
            assert!(actions.contains(&ToolbarAction::Tool(tool)));
        }
        for color in COLORS {
            assert!(actions.contains(&ToolbarAction::Color(color)));
        }
        for action in [
            ToolbarAction::Undo,
            ToolbarAction::Redo,
            ToolbarAction::Copy,
            ToolbarAction::Save,
            ToolbarAction::Cancel,
            ToolbarAction::Width(-1),
            ToolbarAction::Width(1),
        ] {
            assert!(actions.contains(&action));
        }
    }
}
