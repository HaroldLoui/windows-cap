//! 标注工具栏：布局、命中测试、绘制。
//!
//! 工具栏是遮罩的一部分，画在合成缓冲上（不在文档表面里）。

use windows::Win32::Foundation::{COLORREF, RECT};
use windows::Win32::Graphics::Gdi::{
    CreateSolidBrush, DT_CENTER, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, DeleteObject, DrawTextW,
    FillRect, FrameRect, HDC, HFONT, HGDIOBJ, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
};

use crate::capture::{RectI, VirtualScreen};

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
    pub fn label(self) -> &'static str {
        match self {
            Tool::Select => "选",
            Tool::Pen => "笔",
            Tool::Line => "线",
            Tool::Arrow => "箭",
            Tool::Rect => "矩",
            Tool::Ellipse => "圆",
            Tool::Highlight => "高",
            Tool::Text => "字",
            Tool::Mosaic => "码",
            Tool::Step => "序",
        }
    }

    /// 键盘快捷键（返回虚拟键码）。
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

struct Button {
    rect: RectI,
    action: ToolbarAction,
    label: String,
    swatch: Option<u32>,
    enabled: bool,
    active: bool,
}

pub struct Toolbar {
    pub rect: RectI,
    buttons: Vec<Button>,
}

impl Toolbar {
    pub fn layout(selection: RectI, area: VirtualScreen, scale: f32, state: &ToolbarState) -> Self {
        let scale = scale.max(1.0);
        let cell = (32.0 * scale).round() as i32;
        let gap = (4.0 * scale).round() as i32;
        let pad = (6.0 * scale).round() as i32;

        let columns = 11; // 8 色 + 减 + 宽度 + 加
        let row_width = columns * cell + (columns - 1) * gap;
        let width = row_width + pad * 2;
        let height = 3 * cell + 2 * gap + pad * 2;

        let local = RectI::new(
            selection.x - area.x,
            selection.y - area.y,
            selection.width,
            selection.height,
        );
        let x = (local.x + local.width - width - pad).clamp(0, (area.width - width).max(0));
        let y = (local.y + local.height - height - pad).clamp(0, (area.height - height).max(0));
        let panel = RectI::new(x, y, width, height);

        let mut buttons = Vec::with_capacity(22);
        let mut row_y = y + pad;

        // 第一行：工具
        for (index, tool) in TOOLS.iter().enumerate() {
            let bx = x + pad + index as i32 * (cell + gap);
            buttons.push(Button {
                rect: RectI::new(bx, row_y, cell, cell),
                action: ToolbarAction::Tool(*tool),
                label: tool.label().to_string(),
                swatch: None,
                enabled: true,
                active: state.tool == *tool,
            });
        }

        // 第二行：颜色 + 线宽
        row_y += cell + gap;
        for (index, color) in COLORS.iter().enumerate() {
            let bx = x + pad + index as i32 * (cell + gap);
            buttons.push(Button {
                rect: RectI::new(bx, row_y, cell, cell),
                action: ToolbarAction::Color(*color),
                label: String::new(),
                swatch: Some(*color),
                enabled: true,
                active: state.color == *color,
            });
        }
        let minus_x = x + pad + 8 * (cell + gap);
        buttons.push(Button {
            rect: RectI::new(minus_x, row_y, cell, cell),
            action: ToolbarAction::Width(-1),
            label: "−".to_string(),
            swatch: None,
            enabled: state.width > 1.0,
            active: false,
        });
        let width_x = minus_x + cell + gap;
        buttons.push(Button {
            rect: RectI::new(width_x, row_y, cell, cell),
            action: ToolbarAction::Noop,
            label: format!("{:.0}", state.width),
            swatch: None,
            enabled: true,
            active: false,
        });
        let plus_x = width_x + cell + gap;
        buttons.push(Button {
            rect: RectI::new(plus_x, row_y, cell, cell),
            action: ToolbarAction::Width(1),
            label: "+".to_string(),
            swatch: None,
            enabled: state.width < 40.0,
            active: false,
        });

        // 第三行：操作
        row_y += cell + gap;
        let actions = [
            ("撤销", ToolbarAction::Undo, state.can_undo),
            ("重做", ToolbarAction::Redo, state.can_redo),
            ("复制", ToolbarAction::Copy, true),
            ("保存", ToolbarAction::Save, true),
            ("关闭", ToolbarAction::Cancel, true),
        ];
        let action_width = cell * 2;
        for (index, (label, action, enabled)) in actions.into_iter().enumerate() {
            let bx = x + pad + index as i32 * (action_width + gap);
            buttons.push(Button {
                rect: RectI::new(bx, row_y, action_width, cell),
                action,
                label: label.to_string(),
                swatch: None,
                enabled,
                active: false,
            });
        }

        Self {
            rect: panel,
            buttons,
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

    pub fn paint(&self, hdc: HDC, font: HFONT) {
        unsafe {
            let accent = CreateSolidBrush(COLORREF(0x00FF_9B2E));
            let panel = CreateSolidBrush(COLORREF(0x0020_2020));
            let button = CreateSolidBrush(COLORREF(0x0035_3540));
            let white = CreateSolidBrush(COLORREF(0x00FF_FFFF));

            let panel_rect = win_rect(self.rect);
            let _ = FillRect(hdc, &panel_rect, panel);
            let _ = FrameRect(hdc, &panel_rect, accent);

            let old_font = SelectObject(hdc, HGDIOBJ(font.0));
            SetBkMode(hdc, TRANSPARENT);

            for item in &self.buttons {
                let rect = win_rect(item.rect);
                match item.swatch {
                    Some(color) => {
                        let brush = CreateSolidBrush(argb_to_colorref(color));
                        let _ = FillRect(hdc, &rect, brush);
                        let _ = DeleteObject(HGDIOBJ(brush.0));
                        let _ = FrameRect(hdc, &rect, if item.active { white } else { accent });
                    }
                    None => {
                        let _ = FillRect(
                            hdc,
                            &rect,
                            if item.active {
                                accent
                            } else if item.enabled {
                                button
                            } else {
                                panel
                            },
                        );
                        let _ = FrameRect(hdc, &rect, accent);
                        if !item.label.is_empty() {
                            let mut wide: Vec<u16> = item.label.encode_utf16().collect();
                            SetTextColor(
                                hdc,
                                COLORREF(if item.active {
                                    0x0010_1010
                                } else if item.enabled {
                                    0x00FF_FFFF
                                } else {
                                    0x0080_8080
                                }),
                            );
                            let mut text_rect = rect;
                            DrawTextW(
                                hdc,
                                &mut wide,
                                &mut text_rect,
                                DT_SINGLELINE | DT_CENTER | DT_VCENTER | DT_NOPREFIX,
                            );
                        }
                    }
                }
            }

            SelectObject(hdc, old_font);
            for brush in [accent, panel, button, white] {
                let _ = DeleteObject(HGDIOBJ(brush.0));
            }
        }
    }
}

fn win_rect(rect: RectI) -> RECT {
    RECT {
        left: rect.x,
        top: rect.y,
        right: rect.x + rect.width,
        bottom: rect.y + rect.height,
    }
}

fn argb_to_colorref(argb: u32) -> COLORREF {
    let r = (argb >> 16) & 0xFF;
    let g = (argb >> 8) & 0xFF;
    let b = argb & 0xFF;
    COLORREF((b << 16) | (g << 8) | r)
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
    fn layout_exposes_every_action() {
        let area = VirtualScreen {
            x: 0,
            y: 0,
            width: 1920,
            height: 1080,
        };
        let toolbar = Toolbar::layout(RectI::new(200, 200, 800, 500), area, 1.0, &state());

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
            assert!(
                actions.contains(&ToolbarAction::Tool(tool)),
                "缺少工具 {tool:?}"
            );
        }
        for color in COLORS {
            assert!(
                actions.contains(&ToolbarAction::Color(color)),
                "缺少颜色 {color:#010x}"
            );
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
            assert!(actions.contains(&action), "缺少动作 {action:?}");
        }
    }

    #[test]
    fn layout_is_clamped_inside_screen() {
        let area = VirtualScreen {
            x: 0,
            y: 0,
            width: 800,
            height: 600,
        };
        let toolbar = Toolbar::layout(RectI::new(0, 0, 200, 100), area, 1.0, &state());
        assert!(toolbar.rect.x >= 0 && toolbar.rect.y >= 0);
        assert!(toolbar.rect.x + toolbar.rect.width <= area.width);
        assert!(toolbar.rect.y + toolbar.rect.height <= area.height);
    }
}
