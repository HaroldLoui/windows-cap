//! 标注工具栏：两行图标布局、外部优先定位与 GDI+ 绘制。

use std::ptr::null_mut;

use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Gdi::{HDC, HFONT};
use windows::Win32::Graphics::GdiPlus::{
    FlushIntentionSync, GdipCreateFromHDC, GdipDeleteGraphics, GdipFlush, GdipSetSmoothingMode,
    GpGraphics, PointF, SmoothingModeAntiAlias,
};

use crate::annotate::render;
use crate::capture::{RectI, VirtualScreen};
use crate::ui::kit::shapes::{
    fill_ellipse, fill_polygon, fill_rounded, solid_fill, stroke_ellipse, stroke_line,
    stroke_lines, stroke_polygon, stroke_rect, stroke_rounded,
};
use crate::ui::kit::text as kit_text;
use crate::ui::kit::theme::{
    ACCENT, ACTIVE_ICON, CLOSE_COLOR, CLOSE_HOVER, HOVER_BG, ICON_COLOR, ICON_COLORREF, ICON_DIM,
    PANEL_BG, PANEL_BORDER, SEPARATOR,
};

pub const PIN_ACTIONS_WIDTH: i32 = 110;
pub const PIN_ACTIONS_HEIGHT: i32 = 42;

/// 标注工具。
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
    /// 返回工具快捷键。
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

    /// 返回工具中文名。
    pub fn name(self) -> &'static str {
        match self {
            Tool::Select => "选择",
            Tool::Pen => "画笔",
            Tool::Line => "直线",
            Tool::Arrow => "箭头",
            Tool::Rect => "矩形",
            Tool::Ellipse => "椭圆",
            Tool::Highlight => "高亮",
            Tool::Text => "文字",
            Tool::Mosaic => "马赛克",
            Tool::Step => "序号",
        }
    }
}

/// 工具栏可触发的动作。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolbarAction {
    Tool(Tool),
    Color(u32),
    Width(i32),
    Undo,
    Redo,
    Pin,
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

/// 绘制工具栏所需的当前状态。
#[derive(Clone, Copy, Debug)]
pub struct ToolbarState {
    pub tool: Tool,
    pub color: u32,
    pub width: f32,
    pub can_undo: bool,
    pub can_redo: bool,
    pub can_pin: bool,
}

/// 内置矢量图标。
#[derive(Clone, Copy)]
enum Icon {
    Tool(Tool),
    Undo,
    Redo,
    Pin,
    Copy,
    Save,
    Close,
    Minus,
    Plus,
}

/// 工具栏中的一个可点击按钮。
struct Button {
    rect: RectI,
    action: ToolbarAction,
    icon: Option<Icon>,
    swatch: Option<u32>,
    label: Option<String>,
    enabled: bool,
    active: bool,
}

/// 布局完成的两行工具栏。
pub struct Toolbar {
    pub rect: RectI,
    buttons: Vec<Button>,
    row1_divider: i32,
    row2_divider: i32,
    row1_y: i32,
    row2_y: i32,
    cell: i32,
    scale: f32,
}

impl Toolbar {
    /// 按 DPI、选区与屏幕范围计算工具栏布局。
    pub fn layout(selection: RectI, area: VirtualScreen, scale: f32, state: &ToolbarState) -> Self {
        // 步骤 1：按 DPI 取整计算单元格、间距与两行宽度。
        let scale = scale.max(1.0);
        let cell = (30.0 * scale).round() as i32;
        let gap = (4.0 * scale).round() as i32;
        let pad = (8.0 * scale).round() as i32;
        let separator_margin = (6.0 * scale).round() as i32;

        let tools_width = 10 * cell + 9 * gap;
        let mut pin_actions: Vec<(Icon, ToolbarAction, bool)> = vec![
            (Icon::Undo, ToolbarAction::Undo, state.can_undo),
            (Icon::Redo, ToolbarAction::Redo, state.can_redo),
        ];
        if state.can_pin {
            pin_actions.push((Icon::Pin, ToolbarAction::Pin, true));
        }
        pin_actions.push((Icon::Copy, ToolbarAction::Copy, true));
        pin_actions.push((Icon::Save, ToolbarAction::Save, true));
        pin_actions.push((Icon::Close, ToolbarAction::Cancel, true));
        let actions_width = pin_actions.len() as i32 * cell + (pin_actions.len() as i32 - 1) * gap;
        let colors_width = 8 * cell + 7 * gap;
        let width_controls = cell + gap + (40.0 * scale) as i32 + gap + cell;
        let divider_width = separator_margin * 2 + 1;

        let row1_width = tools_width + divider_width + actions_width;
        let row2_width = colors_width + divider_width + width_controls;
        let panel_width = row1_width.max(row2_width) + pad * 2;
        let panel_height = pad * 2 + cell * 2 + gap;

        // 步骤 2：优先放在选区外，并夹在屏幕范围内。
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

        // 步骤 3：生成第一行工具与操作按钮。
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
        for (icon, action, enabled) in pin_actions {
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

        // 步骤 4：生成第二行颜色与线宽按钮。
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
            row1_y,
            row2_y,
            cell,
            scale,
        }
    }

    /// 返回坐标处命中的动作。
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

    /// 绘制工具栏面板、按钮与文字。
    pub fn paint(&self, hdc: HDC, font: HFONT, hover: Option<ToolbarAction>) {
        // 步骤 1：初始化 GDI+ 并创建绘图对象。
        let _ = render::init();
        unsafe {
            let mut graphics: *mut GpGraphics = null_mut();
            if GdipCreateFromHDC(hdc, &mut graphics).0 != 0 || graphics.is_null() {
                return;
            }
            let _ = GdipSetSmoothingMode(graphics, SmoothingModeAntiAlias);
            let s = self.scale;

            // 步骤 2：绘制面板阴影、背景与行分隔线。
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

            let inset = (5.0 * s).round() as i32;
            for (divider, row_y) in [
                (self.row1_divider, self.row1_y),
                (self.row2_divider, self.row2_y),
            ] {
                stroke_line(
                    graphics,
                    divider as f32,
                    (row_y + inset) as f32,
                    divider as f32,
                    (row_y + self.cell - inset) as f32,
                    SEPARATOR,
                    1.0 * s,
                );
            }

            // 步骤 3：按状态绘制按钮底色与图标。
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

            // 步骤 4：刷出 GDI+ 结果后用 GDI 绘制文字，再释放绘图对象。
            let _ = GdipFlush(graphics, FlushIntentionSync);

            for button in &self.buttons {
                if let Some(label) = &button.label {
                    let mut text_rect = RECT {
                        left: button.rect.x,
                        top: button.rect.y,
                        right: button.rect.x + button.rect.width,
                        bottom: button.rect.y + button.rect.height,
                    };
                    kit_text::draw_text_centered(hdc, label, &mut text_rect, font, ICON_COLORREF);
                }
            }

            let _ = GdipDeleteGraphics(graphics);
        }
    }
}

/// 在选区外选择面板位置，并夹在屏幕内。
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

// 图标：统一使用 18 单位坐标系。

/// 绘制贴图窗口外侧的复制、保存与关闭操作条。
pub fn paint_pin_actions(hdc: HDC, hover: Option<usize>) {
    let _ = render::init();
    unsafe {
        let mut graphics: *mut GpGraphics = null_mut();
        if GdipCreateFromHDC(hdc, &mut graphics).0 != 0 || graphics.is_null() {
            return;
        }
        let _ = GdipSetSmoothingMode(graphics, SmoothingModeAntiAlias);
        let s = 1.0;
        let width = PIN_ACTIONS_WIDTH as f32;
        let height = PIN_ACTIONS_HEIGHT as f32;
        fill_rounded(
            graphics,
            s,
            s,
            width - 2.0 * s,
            height - 2.0 * s,
            10.0 * s,
            PANEL_BG,
        );
        stroke_rounded(
            graphics,
            s,
            s,
            width - 2.0 * s,
            height - 2.0 * s,
            10.0 * s,
            PANEL_BORDER,
            s,
        );
        let icons = [Icon::Copy, Icon::Save, Icon::Close];
        for (index, icon) in icons.iter().enumerate() {
            let left = 6.0 * s + index as f32 * 34.0 * s;
            let top = 6.0 * s;
            if hover == Some(index) {
                let background = if index == 2 { CLOSE_HOVER } else { HOVER_BG };
                fill_rounded(graphics, left, top, 30.0 * s, 30.0 * s, 7.0 * s, background);
            }
            let color = if index == 2 { CLOSE_COLOR } else { ICON_COLOR };
            let (offset_x, offset_y) = match index {
                0 => (5.0, 7.0),
                1 => (6.0, 6.5),
                _ => (6.0, 6.0),
            };
            draw_icon(
                graphics,
                *icon,
                left + offset_x * s,
                top + offset_y * s,
                s,
                color,
            );
        }
        let _ = GdipFlush(graphics, FlushIntentionSync);
        let _ = GdipDeleteGraphics(graphics);
    }
}

/// 在 18×18 坐标系中绘制指定图标。
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
            Icon::Pin => {
                stroke_ellipse(
                    graphics,
                    x + u(6.5),
                    y + u(2.0),
                    u(5.0),
                    u(5.0),
                    color,
                    u(1.3),
                );
                stroke_line(
                    graphics,
                    x + u(9.0),
                    y + u(6.2),
                    x + u(9.0),
                    y + u(11.0),
                    color,
                    u(1.6),
                );
                stroke_line(
                    graphics,
                    x + u(4.5),
                    y + u(11.0),
                    x + u(13.5),
                    y + u(11.0),
                    color,
                    u(1.6),
                );
                stroke_line(
                    graphics,
                    x + u(9.0),
                    y + u(11.0),
                    x + u(9.0),
                    y + u(16.0),
                    color,
                    u(1.6),
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
            can_pin: true,
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
            ToolbarAction::Pin,
            ToolbarAction::Save,
            ToolbarAction::Cancel,
            ToolbarAction::Width(-1),
            ToolbarAction::Width(1),
        ] {
            assert!(actions.contains(&action));
        }
    }
}
