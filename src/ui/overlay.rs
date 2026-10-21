//! 全屏遮罩：冻结帧、选区、标注、工具栏。
//!
//! 渲染方式（P3）：所有内容先合成到一块全屏 DIB（压暗背景 + 选区内容 + 预览 +
//! chrome + 工具栏），再一次性 `BitBlt` 到窗口，避免逐层贴图造成的闪烁。
//!
//! 交互：
//! - 拖拽创建选区；松手后保留，可继续移动/缩放（画出标注后锁定选区）
//! - 工具：选择 / 画笔 / 直线 / 箭头 / 矩形 / 椭圆 / 文字 / 马赛克 / 序号
//! - `Enter` / `Ctrl+C` / `Ctrl+S` / 双击：确认（复制 + 保存）
//! - `Esc` / 右键 / 空白单击：取消
//! - `Ctrl+Z` / `Ctrl+Y`：撤销/重做
//! - 方向键移动选区，`Shift` + 方向键缩放（无标注时）
//! - 文字工具用系统 `EDIT` 子窗口，支持中文 IME（回车提交、Shift+回车换行、Esc 取消）

use std::ffi::c_void;
use std::mem::size_of;
use std::ptr::null_mut;
use std::time::Instant;

use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Dwm::{DWMWA_TRANSITIONS_FORCEDISABLED, DwmSetWindowAttribute};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateFontW, CreateSolidBrush,
    DEFAULT_CHARSET, DEFAULT_GUI_FONT, DT_CALCRECT, DT_CENTER, DT_NOPREFIX, DT_SINGLELINE,
    DT_VCENTER, DeleteObject, DrawTextW, EndPaint, FF_DONTCARE, FW_NORMAL, FillRect, FrameRect,
    GetStockObject, HBRUSH, HFONT, HGDIOBJ, InvalidateRect, OUT_DEFAULT_PRECIS, PAINTSTRUCT,
    SRCCOPY, SelectObject, SetBkMode, SetTextColor, TRANSPARENT, UpdateWindow,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::Ime::{HIMC, ImmAssociateContext, ImmGetVirtualKey};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, SetFocus, VK_CONTROL, VK_DOWN, VK_ESCAPE, VK_LEFT,
    VK_PROCESSKEY, VK_RETURN, VK_RIGHT, VK_SHIFT, VK_UP,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CS_DBLCLKS, CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, DestroyWindow,
    DispatchMessageW, GWLP_USERDATA, GetForegroundWindow, GetMessageW, GetWindowLongPtrW,
    GetWindowThreadProcessId, HCURSOR, HTCLIENT, HWND_TOPMOST, IDC_ARROW, IDC_CROSS, IsWindow,
    KillTimer, LoadCursorW, MSG, PostThreadMessageW, RegisterClassW, SW_SHOW, SWP_SHOWWINDOW,
    SetCursor, SetForegroundWindow, SetTimer, SetWindowLongPtrW, SetWindowPos, ShowWindow,
    TranslateMessage, WM_APP, WM_CREATE, WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDBLCLK,
    WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_NCCREATE, WM_NCHITTEST, WM_PAINT, WM_RBUTTONUP,
    WM_SETCURSOR, WM_TIMER, WNDCLASSW, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};
use windows::core::{PCWSTR, w};

use crate::annotate::{Document, Point, Shape, ShapeKind, render};
use crate::capture::{Frame, RectI, VirtualScreen};
use crate::platform::dib::Dib;
use crate::ui::selection::{EndResult, Handle, SelectionState};
use crate::ui::text_edit::{TextEditor, WM_TEXT_CANCEL, WM_TEXT_COMMIT};
use crate::ui::toolbar::{COLORS, TOOLS, Tool, Toolbar, ToolbarAction, ToolbarState};
use crate::util::error::Result;

const CLASS_NAME: PCWSTR = w!("window_cap_overlay");
const WM_FINISHED: u32 = WM_APP + 1;
const TIMER_SCRIPT: usize = 1;

/// 遮罩参数。
#[derive(Clone, Copy, Debug, Default)]
pub struct OverlayOptions {
    /// 脚本化测试用：直接给出一个选区，延时后自动确认。
    pub scripted: Option<RectI>,
    pub script_delay_ms: u32,
}

/// 遮罩结果。
pub enum OverlayResult {
    Confirmed { image: Frame, rect: RectI },
    Cancelled,
}

pub fn select(frame: Frame, area: VirtualScreen, options: OverlayOptions) -> Result<OverlayResult> {
    let dimmed = dim_bgra(&frame.bgra);
    let dim = Dib::from_bgra(frame.width, frame.height, &dimmed)?;
    let composed = Dib::new(area.width as u32, area.height as u32)?;

    let mut selection = SelectionState::new();
    let mut document = None;
    if let Some(rect) = options.scripted {
        selection.set_selection(rect);
        document = Some(Document::new(&frame, rect)?);
    }

    unsafe {
        let hinstance = HINSTANCE(GetModuleHandleW(None)?.0);
        let mut window_class: WNDCLASSW = std::mem::zeroed();
        window_class.style = CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS;
        window_class.lpfnWndProc = Some(overlay_wndproc);
        window_class.hInstance = hinstance;
        window_class.lpszClassName = CLASS_NAME;
        let _ = RegisterClassW(&window_class);

        let cross = LoadCursorW(None, IDC_CROSS).unwrap_or(HCURSOR(null_mut()));
        let arrow = LoadCursorW(None, IDC_ARROW).unwrap_or(HCURSOR(null_mut()));
        let previous_foreground = GetForegroundWindow();

        let mut overlay = Box::new(Overlay {
            hwnd: HWND(null_mut()),
            area,
            frame,
            dim,
            composed,
            document,
            selection,
            tool: Tool::Select,
            color: COLORS[0],
            width: 4.0,
            drag: None,
            text_editor: None,
            toolbar: None,
            result: None,
            cross,
            arrow,
            font: HFONT(null_mut()),
            accent: CreateSolidBrush(COLORREF(0x00FF_9B2E)),
            white: CreateSolidBrush(COLORREF(0x00FF_FFFF)),
            dark: CreateSolidBrush(COLORREF(0x0020_2020)),
            ui_scale: 1.0,
            started: Instant::now(),
            first_paint_ms: None,
            paints: 0,
            paint_total_ms: 0.0,
            cursor: (area.x, area.y),
            previous_foreground,
        });

        let pointer = (&mut *overlay) as *mut Overlay;
        let hwnd = match CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            CLASS_NAME,
            w!("window-cap"),
            WS_POPUP,
            area.x,
            area.y,
            area.width,
            area.height,
            None,
            None,
            Some(hinstance),
            Some(pointer as *const c_void),
        ) {
            Ok(hwnd) => hwnd,
            Err(error) => {
                drop(overlay);
                return Err(error.into());
            }
        };
        overlay.hwnd = hwnd;
        overlay.ui_scale = (GetDpiForWindow(hwnd) as f32 / 96.0).max(1.0);
        overlay.font = create_font(overlay.ui_scale);
        overlay.refresh_toolbar();

        // 遮罩本身不需要输入法：禁用它，否则按键会变成 VK_PROCESSKEY。
        // 文字工具会为自己的 EDIT 单独创建 IME 上下文。
        let _ = ImmAssociateContext(hwnd, HIMC(null_mut()));

        let transitions_disabled: i32 = 1;
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_TRANSITIONS_FORCEDISABLED,
            &transitions_disabled as *const i32 as *const c_void,
            size_of::<i32>() as u32,
        );

        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            area.x,
            area.y,
            area.width,
            area.height,
            SWP_SHOWWINDOW,
        );
        let _ = ShowWindow(hwnd, SW_SHOW);
        force_foreground(hwnd);
        let _ = SetFocus(Some(hwnd));
        let _ = SetCursor(Some(cross));
        let _ = UpdateWindow(hwnd);

        if options.scripted.is_some() {
            SetTimer(
                Some(hwnd),
                TIMER_SCRIPT,
                options.script_delay_ms.max(10),
                None,
            );
        }

        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).0 > 0 {
            if message.message == WM_FINISHED {
                break;
            }
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }

        if IsWindow(Some(hwnd)).0 != 0 {
            let _ = DestroyWindow(hwnd);
        }

        let result = overlay.result.take().unwrap_or(OverlayResult::Cancelled);
        let previous = overlay.previous_foreground;
        let paints = overlay.paints;
        let average_paint_ms = if paints > 0 {
            overlay.paint_total_ms / paints as f64
        } else {
            0.0
        };
        if !previous.0.is_null() && IsWindow(Some(previous)).0 != 0 {
            let _ = SetForegroundWindow(previous);
        }

        ::log::info!(
            "overlay closed: paints={} first_paint_ms={:?} avg_paint_ms={:.1}",
            paints,
            overlay.first_paint_ms,
            average_paint_ms
        );

        drop(overlay);
        Ok(result)
    }
}

struct ShapeDrag {
    start: Point,
    current: Point,
    points: Vec<Point>,
}

struct Overlay {
    hwnd: HWND,
    area: VirtualScreen,
    frame: Frame,
    dim: Dib,
    composed: Dib,
    document: Option<Document>,
    selection: SelectionState,
    tool: Tool,
    color: u32,
    width: f32,
    drag: Option<ShapeDrag>,
    text_editor: Option<TextEditor>,
    toolbar: Option<Toolbar>,
    result: Option<OverlayResult>,
    cross: HCURSOR,
    arrow: HCURSOR,
    font: HFONT,
    accent: HBRUSH,
    white: HBRUSH,
    dark: HBRUSH,
    ui_scale: f32,
    started: Instant,
    first_paint_ms: Option<f64>,
    paints: u32,
    paint_total_ms: f64,
    cursor: (i32, i32),
    previous_foreground: HWND,
}

impl Overlay {
    fn area_rect(&self) -> RectI {
        RectI::new(self.area.x, self.area.y, self.area.width, self.area.height)
    }

    fn to_local(&self, rect: RectI) -> RectI {
        RectI::new(
            rect.x - self.area.x,
            rect.y - self.area.y,
            rect.width,
            rect.height,
        )
    }

    fn has_shapes(&self) -> bool {
        self.document
            .as_ref()
            .map(|document| document.has_shapes())
            .unwrap_or(false)
    }

    fn setup_document(&mut self) -> Result<()> {
        let Some(rect) = self.selection.rect() else {
            self.document = None;
            return Ok(());
        };

        let same_size = self
            .document
            .as_ref()
            .map(|document| {
                document.width() == rect.width as u32 && document.height() == rect.height as u32
            })
            .unwrap_or(false);
        if !same_size {
            if self.has_shapes() {
                return Ok(());
            }
            self.document = Some(Document::new(&self.frame, rect)?);
        }
        Ok(())
    }

    fn refresh_toolbar(&mut self) {
        self.toolbar = self.selection.rect().map(|rect| {
            let state = ToolbarState {
                tool: self.tool,
                color: self.color,
                width: self.width,
                can_undo: self
                    .document
                    .as_ref()
                    .map(|document| document.can_undo())
                    .unwrap_or(false),
                can_redo: self
                    .document
                    .as_ref()
                    .map(|document| document.can_redo())
                    .unwrap_or(false),
            };
            Toolbar::layout(rect, self.area, self.ui_scale, &state)
        });
    }

    fn confirm(&mut self) {
        if self.result.is_some() {
            return;
        }
        self.close_text_editor();

        let selection = self.selection.rect();
        let rect = selection.unwrap_or_else(|| self.area_rect());
        ::log::debug!(
            "confirm rect={rect:?} has_document={}",
            self.document.is_some()
        );
        let image = match (&self.document, selection) {
            (Some(document), Some(_)) => document.export_frame((rect.x, rect.y)),
            _ => match self.frame.crop(rect) {
                Ok(image) => image,
                Err(error) => {
                    ::log::error!("导出失败: {error}");
                    self.cancel();
                    return;
                }
            },
        };
        self.finish(OverlayResult::Confirmed { image, rect });
    }

    fn cancel(&mut self) {
        self.close_text_editor();
        self.finish(OverlayResult::Cancelled);
    }

    fn finish(&mut self, result: OverlayResult) {
        if self.result.is_some() {
            return;
        }
        if let Some(editor) = self.text_editor.take() {
            drop(editor);
        }
        self.result = Some(result);
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_SCRIPT);
            let _ = DestroyWindow(self.hwnd);
            let _ = PostThreadMessageW(GetCurrentThreadId(), WM_FINISHED, WPARAM(0), LPARAM(0));
        }
    }

    fn close_text_editor(&mut self) {
        if let Some(editor) = self.text_editor.take() {
            drop(editor);
            unsafe {
                let _ = SetFocus(Some(self.hwnd));
            }
        }
    }

    fn commit_text(&mut self) {
        let Some(editor) = self.text_editor.take() else {
            return;
        };
        let text = editor.text();
        let position = editor.doc_position;
        drop(editor);

        let trimmed = text.trim();
        if !trimmed.is_empty() {
            let shape = Shape::new(
                ShapeKind::Text {
                    pos: (position.0, position.1),
                    text,
                    size: 24.0 * self.ui_scale,
                },
                self.color,
                self.width,
            );
            if let Some(document) = self.document.as_mut()
                && let Err(error) = document.add(shape)
            {
                ::log::error!("添加文字失败: {error}");
            }
        }

        unsafe {
            let _ = SetFocus(Some(self.hwnd));
        }
        self.refresh_toolbar();
        self.invalidate();
    }

    fn cancel_text(&mut self) {
        self.close_text_editor();
        self.invalidate();
    }

    fn compose(&self) {
        unsafe {
            let hdc = self.composed.dc;
            let _ = BitBlt(
                hdc,
                0,
                0,
                self.area.width,
                self.area.height,
                Some(self.dim.dc),
                0,
                0,
                SRCCOPY,
            );

            let Some(current) = self.selection.current_rect() else {
                self.draw_hint(hdc);
                return;
            };
            let local = self.to_local(current);

            if let (Some(rect), Some(document)) = (self.selection.rect(), &self.document) {
                let selection_local = self.to_local(rect);
                let _ = BitBlt(
                    hdc,
                    selection_local.x,
                    selection_local.y,
                    rect.width,
                    rect.height,
                    Some(document.surface().dc),
                    0,
                    0,
                    SRCCOPY,
                );
            }

            if let Some(drag) = &self.drag
                && let Some(shape) = self.preview_shape(drag)
            {
                match &shape.kind {
                    ShapeKind::Mosaic { .. } => {
                        let outline = Shape::new(
                            ShapeKind::Rect {
                                a: drag.start,
                                b: drag.current,
                            },
                            0xFFFF_FFFF,
                            self.width.max(1.0),
                        );
                        let _ = render::draw_vector(hdc, &outline, (local.x, local.y));
                    }
                    _ => {
                        let _ = render::draw_vector(hdc, &shape, (local.x, local.y));
                    }
                }
            }

            self.draw_chrome(hdc, current);
            if let Some(toolbar) = &self.toolbar {
                toolbar.paint(hdc, self.font);
            }
        }
    }

    fn paint(&mut self) {
        unsafe {
            let mut paint = PAINTSTRUCT::default();
            let hdc = BeginPaint(self.hwnd, &mut paint);
            if hdc.0.is_null() {
                return;
            }
            let started = Instant::now();

            self.compose();
            let _ = BitBlt(
                hdc,
                0,
                0,
                self.area.width,
                self.area.height,
                Some(self.composed.dc),
                0,
                0,
                SRCCOPY,
            );

            let _ = EndPaint(self.hwnd, &paint);

            self.paints += 1;
            self.paint_total_ms += started.elapsed().as_secs_f64() * 1000.0;
            if self.first_paint_ms.is_none() {
                self.first_paint_ms = Some(self.started.elapsed().as_secs_f64() * 1000.0);
                ::log::info!(
                    "overlay first paint in {:.1} ms",
                    self.first_paint_ms.unwrap_or(0.0)
                );
            }
        }
    }

    unsafe fn draw_chrome(&self, hdc: windows::Win32::Graphics::Gdi::HDC, rect: RectI) {
        unsafe {
            let local = self.to_local(rect);
            let thickness = ((2.0 * self.ui_scale).round() as i32).max(1);

            fill(hdc, local.x, local.y, local.width, thickness, self.accent);
            fill(
                hdc,
                local.x,
                local.y + local.height - thickness,
                local.width,
                thickness,
                self.accent,
            );
            fill(hdc, local.x, local.y, thickness, local.height, self.accent);
            fill(
                hdc,
                local.x + local.width - thickness,
                local.y,
                thickness,
                local.height,
                self.accent,
            );

            if !self.has_shapes() {
                let half = ((3.5 * self.ui_scale).round() as i32).max(2);
                for handle in Handle::ALL {
                    let (x, y) = handle.position(rect);
                    let (x, y) = (x - self.area.x, y - self.area.y);
                    let handle_rect = windows::Win32::Foundation::RECT {
                        left: x - half,
                        top: y - half,
                        right: x + half,
                        bottom: y + half,
                    };
                    let _ = FillRect(hdc, &handle_rect, self.white);
                    let _ = FrameRect(hdc, &handle_rect, self.accent);
                }
            }

            let label = format!("{} × {}", rect.width, rect.height);
            let center_x = local.x + local.width / 2;
            let label_y = if local.y > (40.0 * self.ui_scale) as i32 {
                local.y - ((18.0 * self.ui_scale) as i32)
            } else {
                local.y + local.height + ((18.0 * self.ui_scale) as i32)
            };
            self.draw_chip(hdc, &label, center_x, label_y);
        }
    }

    unsafe fn draw_hint(&self, hdc: windows::Win32::Graphics::Gdi::HDC) {
        unsafe {
            let center_x = self.area.width / 2;
            let y = ((30.0 * self.ui_scale) as i32).max(20);
            self.draw_chip(hdc, "拖拽选择区域    Enter 复制    Esc 取消", center_x, y);
        }
    }

    unsafe fn draw_chip(
        &self,
        hdc: windows::Win32::Graphics::Gdi::HDC,
        text: &str,
        center_x: i32,
        center_y: i32,
    ) {
        unsafe {
            let mut wide: Vec<u16> = text.encode_utf16().collect();
            let old_font = SelectObject(hdc, HGDIOBJ(self.font.0));
            SetBkMode(hdc, TRANSPARENT);
            SetTextColor(hdc, COLORREF(0x00FF_FFFF));

            let mut measure = windows::Win32::Foundation::RECT::default();
            DrawTextW(
                hdc,
                &mut wide,
                &mut measure,
                DT_SINGLELINE | DT_CALCRECT | DT_NOPREFIX,
            );

            let padding_x = (10.0 * self.ui_scale) as i32;
            let padding_y = (5.0 * self.ui_scale) as i32;
            let width = (measure.right - measure.left).max(0) + padding_x * 2;
            let height = (measure.bottom - measure.top).max(0) + padding_y * 2;
            let left = (center_x - width / 2).clamp(0, (self.area.width - width).max(0));
            let top = (center_y - height / 2).clamp(0, (self.area.height - height).max(0));

            let rect = windows::Win32::Foundation::RECT {
                left,
                top,
                right: left + width,
                bottom: top + height,
            };
            let _ = FillRect(hdc, &rect, self.dark);
            let _ = FrameRect(hdc, &rect, self.accent);

            let mut text_rect = rect;
            DrawTextW(
                hdc,
                &mut wide,
                &mut text_rect,
                DT_SINGLELINE | DT_CENTER | DT_VCENTER | DT_NOPREFIX,
            );
            SelectObject(hdc, old_font);
        }
    }

    fn screen_point(&self, lparam: LPARAM) -> (i32, i32) {
        (
            loword(lparam.0) + self.area.x,
            hiword(lparam.0) + self.area.y,
        )
    }

    fn doc_point(&self, screen: (i32, i32)) -> Option<Point> {
        let rect = self.selection.rect()?;
        Some((screen.0 - rect.x, screen.1 - rect.y))
    }

    fn preview_shape(&self, drag: &ShapeDrag) -> Option<Shape> {
        let moved =
            (drag.current.0 - drag.start.0).abs() + (drag.current.1 - drag.start.1).abs() >= 2;

        let kind = match self.tool {
            Tool::Pen => {
                if drag.points.len() >= 2 {
                    Some(ShapeKind::Pen {
                        points: drag.points.clone(),
                    })
                } else {
                    None
                }
            }
            Tool::Highlight => {
                if drag.points.len() >= 2 {
                    Some(ShapeKind::Highlight {
                        points: drag.points.clone(),
                    })
                } else {
                    None
                }
            }
            Tool::Line if moved => Some(ShapeKind::Line {
                a: drag.start,
                b: drag.current,
            }),
            Tool::Arrow if moved => Some(ShapeKind::Arrow {
                a: drag.start,
                b: drag.current,
            }),
            Tool::Rect if moved => Some(ShapeKind::Rect {
                a: drag.start,
                b: drag.current,
            }),
            Tool::Ellipse if moved => Some(ShapeKind::Ellipse {
                a: drag.start,
                b: drag.current,
            }),
            Tool::Mosaic if moved => Some(ShapeKind::Mosaic {
                a: drag.start,
                b: drag.current,
                block: 12,
            }),
            Tool::Step => Some(ShapeKind::Step {
                center: drag.current,
                index: self
                    .document
                    .as_ref()
                    .map(|document| document.next_step_index())
                    .unwrap_or(1),
            }),
            _ => None,
        }?;

        let width = match self.tool {
            Tool::Highlight => self.width * 4.0,
            Tool::Mosaic => 1.0,
            _ => self.width,
        };
        let color = if matches!(self.tool, Tool::Highlight) {
            (self.color & 0x00FF_FFFF) | 0x80_000000
        } else {
            self.color
        };
        Some(Shape::new(kind, color, width))
    }

    fn start_drag(&mut self, screen: (i32, i32)) {
        let Some(position) = self.doc_point(screen) else {
            return;
        };

        if self.tool == Tool::Text {
            let client = (screen.0 - self.area.x, screen.1 - self.area.y);
            match TextEditor::open(self.hwnd, client, position, self.ui_scale) {
                Ok(editor) => {
                    let edit_hwnd = editor.hwnd;
                    self.text_editor = Some(editor);
                    unsafe {
                        let _ = SetFocus(Some(edit_hwnd));
                    }
                }
                Err(error) => {
                    ::log::error!("创建文字输入框失败: {error}");
                }
            }
            return;
        }

        self.drag = Some(ShapeDrag {
            start: position,
            current: position,
            points: vec![position],
        });
        ::log::debug!("start drag tool={:?} at {position:?}", self.tool);
        unsafe {
            SetCapture(self.hwnd);
        }
        self.invalidate();
    }

    fn update_drag(&mut self, screen: (i32, i32)) {
        let Some(rect) = self.selection.rect() else {
            return;
        };
        let Some(drag) = self.drag.as_mut() else {
            return;
        };

        drag.current = (screen.0 - rect.x, screen.1 - rect.y);
        if matches!(self.tool, Tool::Pen | Tool::Highlight) {
            let should_push = drag
                .points
                .last()
                .map(|last| (last.0 - drag.current.0).abs() + (last.1 - drag.current.1).abs() >= 2)
                .unwrap_or(true);
            if should_push {
                drag.points.push(drag.current);
            }
        }
    }

    fn finish_drag(&mut self) {
        let Some(drag) = self.drag.take() else {
            return;
        };
        unsafe {
            let _ = ReleaseCapture();
        }

        if let Some(shape) = self.preview_shape(&drag)
            && let Some(document) = self.document.as_mut()
            && let Err(error) = document.add(shape)
        {
            ::log::error!("添加标注失败: {error}");
        }
        ::log::debug!("finish drag tool={:?}", self.tool);

        self.refresh_toolbar();
        self.invalidate();
    }

    fn apply_action(&mut self, action: ToolbarAction) {
        match action {
            ToolbarAction::Tool(tool) => self.tool = tool,
            ToolbarAction::Color(color) => self.color = color,
            ToolbarAction::Width(delta) => {
                self.width = (self.width + delta as f32).clamp(1.0, 40.0);
            }
            ToolbarAction::Undo => {
                if let Some(document) = self.document.as_mut() {
                    document.undo();
                }
            }
            ToolbarAction::Redo => {
                if let Some(document) = self.document.as_mut() {
                    document.redo();
                }
            }
            ToolbarAction::Copy | ToolbarAction::Save => {
                self.confirm();
                return;
            }
            ToolbarAction::Cancel => {
                self.cancel();
                return;
            }
            ToolbarAction::Noop => {}
        }

        self.refresh_toolbar();
        self.invalidate();
    }

    fn on_mouse_down(&mut self, screen: (i32, i32)) {
        if self.text_editor.is_some() {
            return;
        }
        ::log::debug!(
            "mouse down {screen:?} tool={:?} selection={:?} shapes={}",
            self.tool,
            self.selection.rect(),
            self.has_shapes()
        );

        let client = (screen.0 - self.area.x, screen.1 - self.area.y);
        let action = self
            .toolbar
            .as_ref()
            .and_then(|toolbar| toolbar.hit(client.0, client.1));
        if let Some(action) = action {
            ::log::debug!("toolbar action {action:?}");
            self.apply_action(action);
            return;
        }

        if let Some(rect) = self.selection.rect() {
            if inside(rect, screen) {
                if self.has_shapes() || self.tool != Tool::Select {
                    self.start_drag(screen);
                } else {
                    self.selection.begin(screen.0, screen.1, self.area);
                    unsafe {
                        SetCapture(self.hwnd);
                    }
                    self.invalidate();
                }
                return;
            }

            if !self.has_shapes() {
                self.selection.begin(screen.0, screen.1, self.area);
                unsafe {
                    SetCapture(self.hwnd);
                }
                self.invalidate();
            }
            return;
        }

        self.selection.begin(screen.0, screen.1, self.area);
        unsafe {
            SetCapture(self.hwnd);
        }
        self.invalidate();
    }

    fn on_mouse_move(&mut self, screen: (i32, i32)) {
        self.cursor = screen;

        if self.drag.is_some() {
            self.update_drag(screen);
            self.invalidate();
        } else if self.selection.is_active() {
            self.selection.update(screen.0, screen.1, self.area);
            self.invalidate();
        }

        if let Some(rect) = self.selection.rect()
            && self.has_shapes()
        {
            let cursor = if inside(rect, screen) {
                self.cross
            } else {
                self.arrow
            };
            unsafe {
                let _ = SetCursor(Some(cursor));
            }
            return;
        }
        self.update_cursor();
    }

    fn on_mouse_up(&mut self, screen: (i32, i32)) {
        if self.drag.is_some() {
            self.update_drag(screen);
            self.finish_drag();
            return;
        }

        if self.selection.is_active() {
            self.selection.update(screen.0, screen.1, self.area);
            unsafe {
                let _ = ReleaseCapture();
            }
            let end = self.selection.end(self.area);
            ::log::debug!(
                "mouse up {screen:?} end={end:?} rect={:?}",
                self.selection.rect()
            );
            match end {
                EndResult::Cancelled => self.cancel(),
                _ => {
                    if let Err(error) = self.setup_document() {
                        ::log::error!("创建标注文档失败: {error}");
                    }
                    self.refresh_toolbar();
                    self.invalidate();
                }
            }
        }
    }

    fn handle_key(&mut self, wparam: WPARAM) {
        if self.text_editor.is_some() {
            return;
        }

        let mut key = wparam.0 as u16;
        // 有 IME 时 WM_KEYDOWN 的键码是 VK_PROCESSKEY，用 ImmGetVirtualKey 还原真实按键。
        if key == VK_PROCESSKEY.0 {
            key = unsafe { ImmGetVirtualKey(self.hwnd) } as u16;
        }
        let ctrl = unsafe { GetKeyState(VK_CONTROL.0 as i32) } < 0;
        let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0;
        ::log::debug!("key down {key:#06x} ctrl={ctrl} shift={shift}");

        if ctrl {
            match key {
                key if key == b'C' as u16 || key == b'S' as u16 => self.confirm(),
                key if key == b'Z' as u16 => {
                    if shift {
                        if let Some(document) = self.document.as_mut() {
                            document.redo();
                        }
                    } else if let Some(document) = self.document.as_mut() {
                        document.undo();
                    }
                    self.refresh_toolbar();
                    self.invalidate();
                }
                key if key == b'Y' as u16 => {
                    if let Some(document) = self.document.as_mut() {
                        document.redo();
                    }
                    self.refresh_toolbar();
                    self.invalidate();
                }
                _ => {}
            }
            return;
        }

        match key {
            key if key == VK_ESCAPE.0 => self.cancel(),
            key if key == VK_RETURN.0 => self.confirm(),
            key if key == VK_LEFT.0 => self.nudge(-1, 0, shift),
            key if key == VK_RIGHT.0 => self.nudge(1, 0, shift),
            key if key == VK_UP.0 => self.nudge(0, -1, shift),
            key if key == VK_DOWN.0 => self.nudge(0, 1, shift),
            key if key == b'[' as u16 => {
                self.width = (self.width - 1.0).max(1.0);
                self.refresh_toolbar();
                self.invalidate();
            }
            key if key == b']' as u16 => {
                self.width = (self.width + 1.0).min(40.0);
                self.refresh_toolbar();
                self.invalidate();
            }
            key if TOOLS.iter().any(|tool| tool.shortcut() == key) => {
                if let Some(tool) = TOOLS.iter().find(|tool| tool.shortcut() == key) {
                    self.tool = *tool;
                    self.refresh_toolbar();
                    self.invalidate();
                }
            }
            _ => {}
        }
    }

    fn nudge(&mut self, dx: i32, dy: i32, resize: bool) {
        if self.has_shapes() {
            return;
        }
        if resize {
            self.selection.resize_by(dx, dy, self.area);
        } else {
            self.selection.move_by(dx, dy, self.area);
        }
        self.invalidate();
    }

    fn update_cursor(&self) {
        let client = (self.cursor.0 - self.area.x, self.cursor.1 - self.area.y);
        let over_toolbar = self
            .toolbar
            .as_ref()
            .map(|toolbar| toolbar.hit(client.0, client.1).is_some())
            .unwrap_or(false);
        let cursor = if over_toolbar { self.arrow } else { self.cross };
        unsafe {
            let _ = SetCursor(Some(cursor));
        }
    }

    fn invalidate(&self) {
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd), None, false);
        }
    }
}

impl Drop for Overlay {
    fn drop(&mut self) {
        unsafe {
            if !self.font.0.is_null() {
                let _ = DeleteObject(HGDIOBJ(self.font.0));
            }
            for brush in [self.accent, self.white, self.dark] {
                let _ = DeleteObject(HGDIOBJ(brush.0));
            }
        }
    }
}

fn dim_bgra(bgra: &[u8]) -> Vec<u8> {
    const FACTOR: u16 = 115;
    let mut dimmed = bgra.to_vec();
    for pixel in dimmed.chunks_exact_mut(4) {
        for channel in &mut pixel[..3] {
            *channel = ((*channel as u16 * FACTOR) / 255) as u8;
        }
        pixel[3] = 255;
    }
    dimmed
}

fn create_font(scale: f32) -> HFONT {
    let height = -((15.0 * scale).round() as i32).max(10);
    unsafe {
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

fn force_foreground(hwnd: HWND) -> bool {
    unsafe {
        if SetForegroundWindow(hwnd).0 != 0 {
            return true;
        }
        let foreground = GetForegroundWindow();
        if foreground.0.is_null() {
            return false;
        }
        let foreground_thread = GetWindowThreadProcessId(foreground, None);
        let our_thread = GetCurrentThreadId();
        if AttachThreadInput(our_thread, foreground_thread, true).0 == 0 {
            return false;
        }
        let result = SetForegroundWindow(hwnd).0 != 0;
        let _ = AttachThreadInput(our_thread, foreground_thread, false);
        result
    }
}

unsafe fn fill(
    hdc: windows::Win32::Graphics::Gdi::HDC,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    brush: HBRUSH,
) {
    unsafe {
        let rect = windows::Win32::Foundation::RECT {
            left: x,
            top: y,
            right: x + width,
            bottom: y + height,
        };
        let _ = FillRect(hdc, &rect, brush);
    }
}

fn inside(rect: RectI, point: (i32, i32)) -> bool {
    point.0 >= rect.x
        && point.1 >= rect.y
        && point.0 < rect.x + rect.width
        && point.1 < rect.y + rect.height
}

fn loword(value: isize) -> i32 {
    ((value as usize) & 0xFFFF) as u16 as i16 as i32
}

fn hiword(value: isize) -> i32 {
    (((value as usize) >> 16) & 0xFFFF) as u16 as i16 as i32
}

unsafe fn state(hwnd: HWND) -> Option<&'static mut Overlay> {
    unsafe {
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut Overlay;
        if pointer.is_null() {
            None
        } else {
            Some(&mut *pointer)
        }
    }
}

unsafe extern "system" fn overlay_wndproc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        match message {
            WM_NCCREATE => {
                let create =
                    lparam.0 as *const windows::Win32::UI::WindowsAndMessaging::CREATESTRUCTW;
                if !create.is_null() {
                    let pointer = (*create).lpCreateParams as *mut Overlay;
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, pointer as isize);
                }
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
            WM_CREATE => {
                if let Some(overlay) = state(hwnd) {
                    overlay.hwnd = hwnd;
                }
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_PAINT => {
                if let Some(overlay) = state(hwnd) {
                    overlay.paint();
                }
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                if let Some(overlay) = state(hwnd) {
                    let point = overlay.screen_point(lparam);
                    overlay.on_mouse_move(point);
                }
                LRESULT(0)
            }
            WM_LBUTTONDOWN => {
                if let Some(overlay) = state(hwnd) {
                    let point = overlay.screen_point(lparam);
                    overlay.on_mouse_down(point);
                }
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                if let Some(overlay) = state(hwnd) {
                    let point = overlay.screen_point(lparam);
                    overlay.on_mouse_up(point);
                }
                LRESULT(0)
            }
            WM_LBUTTONDBLCLK => {
                if let Some(overlay) = state(hwnd)
                    && overlay.text_editor.is_none()
                {
                    overlay.confirm();
                }
                LRESULT(0)
            }
            WM_RBUTTONUP => {
                if let Some(overlay) = state(hwnd) {
                    overlay.cancel();
                }
                LRESULT(0)
            }
            WM_KEYDOWN => {
                if let Some(overlay) = state(hwnd) {
                    overlay.handle_key(wparam);
                }
                LRESULT(0)
            }
            WM_SETCURSOR => {
                if let Some(overlay) = state(hwnd) {
                    overlay.update_cursor();
                }
                LRESULT(1)
            }
            WM_TIMER => {
                if let Some(overlay) = state(hwnd)
                    && wparam.0 == TIMER_SCRIPT
                {
                    overlay.confirm();
                }
                LRESULT(0)
            }
            WM_TEXT_COMMIT => {
                if let Some(overlay) = state(hwnd) {
                    overlay.commit_text();
                }
                LRESULT(0)
            }
            WM_TEXT_CANCEL => {
                if let Some(overlay) = state(hwnd) {
                    overlay.cancel_text();
                }
                LRESULT(0)
            }
            WM_NCHITTEST => LRESULT(HTCLIENT as isize),
            _ => DefWindowProcW(hwnd, message, wparam, lparam),
        }
    }
}
