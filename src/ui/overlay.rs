//! 全屏遮罩：冻结帧、选区、标注与工具栏。

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
    GetStockObject, HBRUSH, HDC, HFONT, HGDIOBJ, InvalidateRect, NULL_BRUSH, OUT_DEFAULT_PRECIS,
    PAINTSTRUCT, SRCCOPY, SelectObject, SetBkMode, SetTextColor, TRANSPARENT, UpdateWindow,
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
    GetWindowThreadProcessId, HCURSOR, HTCLIENT, HWND_TOPMOST, IDC_ARROW, IDC_CROSS, IDC_SIZEALL,
    IDC_SIZENESW, IDC_SIZENS, IDC_SIZENWSE, IDC_SIZEWE, IsWindow, KillTimer, LoadCursorW, MSG,
    PostThreadMessageW, RegisterClassW, SW_SHOW, SWP_SHOWWINDOW, SetCursor, SetForegroundWindow,
    SetTimer, SetWindowLongPtrW, SetWindowPos, ShowWindow, TranslateMessage, WM_APP, WM_CREATE,
    WM_CTLCOLOREDIT, WM_ERASEBKGND, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE,
    WM_NCCREATE, WM_NCHITTEST, WM_PAINT, WM_RBUTTONUP, WM_SETCURSOR, WM_TIMER, WNDCLASSW,
    WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_POPUP,
};
use windows::core::{PCWSTR, w};

use crate::annotate::{Document, Point, Shape, ShapeKind, render};
use crate::capture::{Frame, RectI, VirtualScreen};
use crate::platform::dib::Dib;
use crate::ui::selection::{EndResult, HANDLE_HIT_TOLERANCE, Handle, SelectionState};
use crate::ui::text_edit::{TextEditor, WM_TEXT_CANCEL, WM_TEXT_COMMIT};
use crate::ui::toolbar::{COLORS, TOOLS, Tool, Toolbar, ToolbarAction, ToolbarState};
use crate::util::error::Result;

const CLASS_NAME: PCWSTR = w!("window_cap_overlay");
const WM_FINISHED: u32 = WM_APP + 1;
const TIMER_SCRIPT: usize = 1;
const TIMER_TOOLTIP: usize = 2;

/// 遮罩启动选项。
#[derive(Clone, Copy, Debug, Default)]
pub struct OverlayOptions {
    /// 脚本化测试选区；设置后延时自动确认。
    pub scripted: Option<RectI>,
    /// 脚本确认前的延时（毫秒）。
    pub script_delay_ms: u32,
    /// 是否允许贴图（常驻模式为 true）。
    pub allow_pin: bool,
}

/// 输出行为枚举
#[derive(Clone, Copy, Debug)]
pub enum OutputAction {
    Copy,
    Save,
}

/// 遮罩关闭后的结果。
pub enum OverlayResult {
    /// 已确认：输出图像与对应选区。
    Confirmed {
        image: Frame,
        rect: RectI,
        action: OutputAction,
    },
    /// 已贴图：仅输出图像。
    Pinned { image: Frame },
    /// 已取消。
    Cancelled,
}

/// 打开全屏遮罩，等待用户确认、贴图或取消。
pub fn select(frame: Frame, area: VirtualScreen, options: OverlayOptions) -> Result<OverlayResult> {
    // 步骤 1：准备压暗帧、合成缓冲与初始选区。
    let dimmed = dim_bgra(&frame.bgra);
    let dim = Dib::from_bgra(frame.width, frame.height, &dimmed)?;
    let composed = Dib::new(area.width as u32, area.height as u32)?;

    let mut selection = SelectionState::new();
    let mut document = None;
    if let Some(rect) = options.scripted {
        selection.set_selection(rect);
        document = Some(Document::from_frame(&frame, rect)?);
    }

    unsafe {
        // 步骤 2：注册窗口类并创建置顶遮罩窗口。
        let hinstance = HINSTANCE(GetModuleHandleW(None)?.0);
        let mut window_class: WNDCLASSW = std::mem::zeroed();
        window_class.style = CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS;
        window_class.lpfnWndProc = Some(overlay_wndproc);
        window_class.hInstance = hinstance;
        window_class.lpszClassName = CLASS_NAME;
        let _ = RegisterClassW(&window_class);

        let cross = LoadCursorW(None, IDC_CROSS).unwrap_or(HCURSOR(null_mut()));
        let arrow = LoadCursorW(None, IDC_ARROW).unwrap_or(HCURSOR(null_mut()));
        let size_nwse = LoadCursorW(None, IDC_SIZENWSE).unwrap_or(HCURSOR(null_mut()));
        let size_nesw = LoadCursorW(None, IDC_SIZENESW).unwrap_or(HCURSOR(null_mut()));
        let size_ns = LoadCursorW(None, IDC_SIZENS).unwrap_or(HCURSOR(null_mut()));
        let size_we = LoadCursorW(None, IDC_SIZEWE).unwrap_or(HCURSOR(null_mut()));
        let size_all = LoadCursorW(None, IDC_SIZEALL).unwrap_or(HCURSOR(null_mut()));
        ::log::debug!(
            "overlay cursors: cross={:?} arrow={:?} nwse={:?} all={:?}",
            cross,
            arrow,
            size_nwse,
            size_all
        );
        let previous_foreground = GetForegroundWindow();

        let mut overlay = Box::new(Overlay {
            hwnd: HWND(null_mut()),
            area,
            allow_pin: options.allow_pin,
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
            text_drag: None,
            toolbar: None,
            result: None,
            cross,
            arrow,
            size_nwse,
            size_nesw,
            size_ns,
            size_we,
            size_all,
            font: HFONT(null_mut()),
            accent: CreateSolidBrush(COLORREF(0x00FF_9B2E)),
            white: CreateSolidBrush(COLORREF(0x00FF_FFFF)),
            dark: CreateSolidBrush(COLORREF(0x0020_2020)),
            ui_scale: 1.0,
            started: Instant::now(),
            first_paint_ms: None,
            paints: 0,
            paint_total_ms: 0.0,
            toolbar_hover: None,
            tooltip_action: None,
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
        // 步骤 3：初始化 DPI、字体与工具栏，显示并聚焦窗口。
        overlay.hwnd = hwnd;
        overlay.ui_scale = (GetDpiForWindow(hwnd) as f32 / 96.0).max(1.0);
        overlay.font = create_font(overlay.ui_scale);
        overlay.refresh_toolbar();

        // 遮罩自身禁用 IME，避免按键变为 VK_PROCESSKEY；文字编辑框会另建上下文。
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

        // 步骤 4：运行消息循环，直到窗口结束。
        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).0 > 0 {
            if message.message == WM_FINISHED {
                break;
            }
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }

        // 步骤 5：销毁窗口、恢复前台窗口并汇总结果。
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

/// 标注拖拽状态：文档坐标下的起点、当前点与画笔轨迹。
struct ShapeDrag {
    start: Point,
    current: Point,
    points: Vec<Point>,
}

/// 文字输入框的移动/缩放拖拽，坐标为遮罩客户区。
#[derive(Clone, Copy)]
enum TextDrag {
    Move {
        start: (i32, i32),
        origin: (i32, i32),
    },
    Resize {
        start: (i32, i32),
        origin: (i32, i32),
    },
}

/// 遮罩窗口的运行时状态与 GDI 资源。
struct Overlay {
    hwnd: HWND,
    area: VirtualScreen,
    allow_pin: bool,
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
    text_drag: Option<TextDrag>,
    toolbar: Option<Toolbar>,
    toolbar_hover: Option<ToolbarAction>,
    tooltip_action: Option<ToolbarAction>,
    result: Option<OverlayResult>,
    cross: HCURSOR,
    arrow: HCURSOR,
    size_nwse: HCURSOR,
    size_nesw: HCURSOR,
    size_ns: HCURSOR,
    size_we: HCURSOR,
    size_all: HCURSOR,
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
    /// 整个虚拟屏幕的矩形（屏幕坐标）。
    fn area_rect(&self) -> RectI {
        RectI::new(self.area.x, self.area.y, self.area.width, self.area.height)
    }

    /// 屏幕坐标矩形转遮罩客户区坐标。
    fn to_local(&self, rect: RectI) -> RectI {
        RectI::new(
            rect.x - self.area.x,
            rect.y - self.area.y,
            rect.width,
            rect.height,
        )
    }

    /// 文档中是否已有标注。
    fn has_shapes(&self) -> bool {
        self.document
            .as_ref()
            .map(|document| document.has_shapes())
            .unwrap_or(false)
    }

    /// 按当前选区创建或复用标注文档；已有标注时不重建。
    fn setup_document(&mut self) -> Result<()> {
        let Some(rect) = self.selection.rect() else {
            self.document = None;
            return Ok(());
        };

        let same = self
            .document
            .as_ref()
            .map(|document| {
                document.width() == rect.width as u32
                    && document.height() == rect.height as u32
                    && document.origin() == (rect.x, rect.y)
            })
            .unwrap_or(false);
        if same {
            return Ok(());
        }
        if self.has_shapes() {
            return Ok(());
        }
        self.document = Some(Document::from_frame(&self.frame, rect)?);
        Ok(())
    }

    /// 把冻结帧中选区像素直接拷入合成缓冲，不重建文档。
    fn copy_frame_region(&mut self, current: RectI) {
        let area_width = self.area.width as usize;
        let area_height = self.area.height as usize;
        let x = (current.x - self.area.x) as usize;
        let y = (current.y - self.area.y) as usize;
        let width = current.width as usize;
        let height = current.height as usize;
        if width == 0 || height == 0 || x + width > area_width || y + height > area_height {
            return;
        }

        let frame = &self.frame.bgra;
        let composed = self.composed.pixels_mut();
        if frame.len() != composed.len() {
            return;
        }

        let row_bytes = width * 4;
        for row in 0..height {
            let offset = ((y + row) * area_width + x) * 4;
            composed[offset..offset + row_bytes]
                .copy_from_slice(&frame[offset..offset + row_bytes]);
        }
    }

    /// 按当前选区、工具与撤销状态重排工具栏。
    fn refresh_toolbar(&mut self) {
        let layout_rect = if self.selection.rect().is_some() {
            self.selection.current_rect()
        } else {
            None
        };
        self.toolbar = layout_rect.map(|rect| {
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
                can_pin: self.allow_pin,
            };
            Toolbar::layout(rect, self.area, self.ui_scale, &state)
        });
        self.toolbar_hover = None;
    }

    /// 按选区导出图像；无选区时导出整个虚拟屏幕。
    fn export_image(&self) -> Option<Frame> {
        let selection = self.selection.rect();
        let rect = selection.unwrap_or_else(|| self.area_rect());
        match (&self.document, selection) {
            (Some(document), Some(_)) => Some(document.export_frame((rect.x, rect.y))),
            _ => match self.frame.crop(rect) {
                Ok(image) => Some(image),
                Err(error) => {
                    ::log::error!("导出失败: {error}");
                    None
                }
            },
        }
    }

    /// 确认当前结果并关闭遮罩。
    fn confirm(&mut self, action: OutputAction) {
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
        let Some(image) = self.export_image() else {
            self.cancel();
            return;
        };
        self.finish(OverlayResult::Confirmed {
            image,
            rect,
            action,
        });
    }

    /// 将当前图像作为贴图返回。
    fn pin_image(&mut self) {
        if self.result.is_some() {
            return;
        }
        self.close_text_editor();
        let Some(image) = self.export_image() else {
            self.cancel();
            return;
        };
        self.finish(OverlayResult::Pinned { image });
    }

    /// 取消并关闭遮罩。
    fn cancel(&mut self) {
        self.close_text_editor();
        self.finish(OverlayResult::Cancelled);
    }

    /// 记录结果、销毁窗口并结束消息循环。
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

    /// 关闭文字输入框并把焦点交回遮罩。
    fn close_text_editor(&mut self) {
        self.text_drag = None;
        if let Some(editor) = self.text_editor.take() {
            drop(editor);
            unsafe {
                let _ = SetFocus(Some(self.hwnd));
            }
        }
    }

    /// 关闭输入框，并把非空文字提交为标注。
    fn commit_text(&mut self) {
        // 步骤 1：读取文字、锚点与宽度后销毁输入框。
        self.text_drag = None;
        let Some(editor) = self.text_editor.take() else {
            return;
        };
        let text = editor.text();
        let position = editor.doc_position;
        let (editor_width, _) = editor.size();
        drop(editor);

        // 步骤 2：非空文字转为文档坐标下的标注，并恢复遮罩焦点。
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            let max_width = self
                .selection
                .rect()
                .map(|rect| (rect.width - position.0).clamp(40, editor_width) as f32)
                .unwrap_or(editor_width as f32);
            let shape = Shape::new(
                ShapeKind::Text {
                    pos: (position.0, position.1),
                    text,
                    size: 24.0 * self.ui_scale,
                    max_width,
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

    /// 放弃当前文字输入并关闭输入框。
    fn cancel_text(&mut self) {
        self.close_text_editor();
        self.invalidate();
    }

    /// 将背景、选区、预览与界面合成到后备 DIB。
    fn compose(&mut self) {
        unsafe {
            // 步骤 1：铺压暗的全屏背景。
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

            // 步骤 2：无选区时只绘制操作提示。
            let Some(current) = self.selection.current_rect() else {
                self.draw_hint(hdc);
                return;
            };
            let local = self.to_local(current);

            // 步骤 3：贴入选区亮区；有标注时用文档表面，拖动/缩放时无需重建文档。
            let mut blitted_from_document = false;
            if self.has_shapes()
                && let Some(document) = &self.document
                && document.origin() == (current.x, current.y)
                && document.width() == current.width as u32
                && document.height() == current.height as u32
            {
                let _ = BitBlt(
                    hdc,
                    local.x,
                    local.y,
                    current.width,
                    current.height,
                    Some(document.surface().dc),
                    0,
                    0,
                    SRCCOPY,
                );
                blitted_from_document = true;
            }
            if !blitted_from_document {
                self.copy_frame_region(current);
            }

            // 步骤 4：叠加正在拖拽的标注预览。
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

            // 步骤 5：绘制选区边框、工具栏、提示与文字输入框。
            self.draw_chrome(hdc, current);
            if let Some(toolbar) = &self.toolbar {
                toolbar.paint(hdc, self.font, self.toolbar_hover);
            }
            self.draw_tooltip(hdc);
            self.draw_text_frame(hdc);
        }
    }

    /// 响应 WM_PAINT：合成后一次 BitBlt 上屏。
    fn paint(&mut self) {
        unsafe {
            let mut paint = PAINTSTRUCT::default();
            let hdc = BeginPaint(self.hwnd, &mut paint);
            if hdc.0.is_null() {
                return;
            }
            let started = Instant::now();

            // 步骤 1：合成完整画面到后备 DIB。
            self.compose();
            // 步骤 2：一次 BitBlt 上屏，避免逐层绘制闪烁。
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

            // 步骤 3：记录首帧与平均绘制耗时。
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

    /// 绘制选区边框、控制点与尺寸标签。
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

    /// 绘制空选区时的操作提示。
    unsafe fn draw_hint(&self, hdc: windows::Win32::Graphics::Gdi::HDC) {
        unsafe {
            let center_x = self.area.width / 2;
            let y = ((30.0 * self.ui_scale) as i32).max(20);
            self.draw_chip(hdc, "拖拽选择区域    Enter 复制    Esc 取消", center_x, y);
        }
    }

    /// 在指定中心绘制带背景的提示文字。
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

    /// 绘制工具栏悬停提示。
    unsafe fn draw_tooltip(&self, hdc: windows::Win32::Graphics::Gdi::HDC) {
        let Some(action) = self.tooltip_action else {
            return;
        };
        let Some(toolbar) = &self.toolbar else {
            return;
        };
        let text = tooltip_text(action);
        if text.is_empty() {
            return;
        }

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
            let padding_x = (9.0 * self.ui_scale).round() as i32;
            let padding_y = (4.0 * self.ui_scale).round() as i32;
            let width = (measure.right - measure.left).max(0) + padding_x * 2;
            let height = (measure.bottom - measure.top).max(0) + padding_y * 2;

            let gap = (6.0 * self.ui_scale).round() as i32;
            let mut x = toolbar.rect.x + (toolbar.rect.width - width) / 2;
            let mut y = toolbar.rect.y + toolbar.rect.height + gap;
            if y + height > self.area.height - 4 {
                y = toolbar.rect.y - height - gap;
            }
            x = x.clamp(4, (self.area.width - width - 4).max(4));
            y = y.clamp(4, (self.area.height - height - 4).max(4));

            let rect = windows::Win32::Foundation::RECT {
                left: x,
                top: y,
                right: x + width,
                bottom: y + height,
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

    /// 绘制文字输入框边框、四角与缩放手柄。
    unsafe fn draw_text_frame(&self, hdc: windows::Win32::Graphics::Gdi::HDC) {
        let Some(editor) = &self.text_editor else {
            return;
        };
        let rect = editor.client_rect();
        let frame = RectI::new(rect.x - 4, rect.y - 4, rect.width + 8, rect.height + 8);
        let frame_rect = windows::Win32::Foundation::RECT {
            left: frame.x,
            top: frame.y,
            right: frame.x + frame.width,
            bottom: frame.y + frame.height,
        };
        unsafe {
            let _ = FrameRect(hdc, &frame_rect, self.accent);
            for (x, y) in [
                (frame.x, frame.y),
                (frame.x + frame.width - 5, frame.y),
                (frame.x, frame.y + frame.height - 5),
                (frame.x + frame.width - 5, frame.y + frame.height - 5),
            ] {
                let dot = windows::Win32::Foundation::RECT {
                    left: x,
                    top: y,
                    right: x + 5,
                    bottom: y + 5,
                };
                let _ = FillRect(hdc, &dot, self.white);
                let _ = FrameRect(hdc, &dot, self.accent);
            }
            let grip_rect = windows::Win32::Foundation::RECT {
                left: rect.x + rect.width + 2,
                top: rect.y + rect.height + 2,
                right: rect.x + rect.width + 14,
                bottom: rect.y + rect.height + 14,
            };
            let _ = FillRect(hdc, &grip_rect, self.white);
            let _ = FrameRect(hdc, &grip_rect, self.accent);
        }
    }

    /// 鼠标客户区坐标转屏幕坐标。
    fn screen_point(&self, lparam: LPARAM) -> (i32, i32) {
        (
            loword(lparam.0) + self.area.x,
            hiword(lparam.0) + self.area.y,
        )
    }

    /// 屏幕坐标转文档局部坐标。
    fn doc_point(&self, screen: (i32, i32)) -> Option<Point> {
        let rect = self.selection.rect()?;
        Some((screen.0 - rect.x, screen.1 - rect.y))
    }

    /// 根据当前拖拽与工具生成预览标注。
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

    /// 在文档坐标开始标注拖拽；文字工具改为打开输入框。
    fn start_drag(&mut self, screen: (i32, i32)) {
        let Some(position) = self.doc_point(screen) else {
            return;
        };

        if self.tool == Tool::Text {
            let client = (screen.0 - self.area.x, screen.1 - self.area.y);
            // 步骤 1：文字工具先合成当前画面，作为透明输入框的背景。
            self.compose();
            match TextEditor::open(
                self.hwnd,
                client,
                position,
                self.ui_scale,
                24.0 * self.ui_scale,
                &self.composed,
            ) {
                Ok(editor) => {
                    let edit_hwnd = editor.hwnd;
                    self.text_editor = Some(editor);
                    // 步骤 2：把键盘焦点交给 EDIT，后续输入由其处理。
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

    /// 更新拖拽终点及画笔采样点。
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

    /// 结束拖拽并把预览标注提交到文档。
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

    /// 执行工具栏动作并刷新界面。
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
            ToolbarAction::Copy => {
                self.confirm(OutputAction::Copy);
                return;
            }
            ToolbarAction::Save => {
                self.confirm(OutputAction::Save);
                return;
            }
            ToolbarAction::Pin => {
                self.pin_image();
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

    /// 处理左键按下：文字框操作、工具栏、选区与标注。
    fn on_mouse_down(&mut self, screen: (i32, i32)) {
        // 步骤 1：关闭提示，并优先处理文字框的缩放手柄或移动边框。
        self.tooltip_action = None;
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_TOOLTIP);
        }

        if let Some(editor) = &self.text_editor {
            let client = (screen.0 - self.area.x, screen.1 - self.area.y);
            let rect = editor.client_rect();
            let frame = RectI::new(rect.x - 4, rect.y - 4, rect.width + 8, rect.height + 8);
            let grip = RectI::new(rect.x + rect.width + 2, rect.y + rect.height + 2, 12, 12);
            if inside(grip, client) {
                self.text_drag = Some(TextDrag::Resize {
                    start: client,
                    origin: (rect.width, rect.height),
                });
                unsafe {
                    SetCapture(self.hwnd);
                }
                return;
            }
            if inside(frame, client) && !inside(rect, client) {
                self.text_drag = Some(TextDrag::Move {
                    start: client,
                    origin: (rect.x, rect.y),
                });
                unsafe {
                    SetCapture(self.hwnd);
                }
            }
            return;
        }
        ::log::debug!(
            "mouse down {screen:?} tool={:?} selection={:?} shapes={}",
            self.tool,
            self.selection.rect(),
            self.has_shapes()
        );

        // 步骤 2：命中工具栏则执行对应动作。
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

        // 步骤 3：按命中位置开始标注、移动/缩放选区或重新框选。
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

    /// 处理鼠标移动：拖拽、框选、工具栏悬停与光标。
    fn on_mouse_move(&mut self, screen: (i32, i32)) {
        self.cursor = screen;
        let mut needs_paint = false;

        // 步骤 1：文字框拖拽优先，处理移动或缩放。
        if let Some(drag) = self.text_drag {
            let client = (screen.0 - self.area.x, screen.1 - self.area.y);
            let area = self.area;
            let scale = self.ui_scale;
            if let Some(editor) = self.text_editor.as_mut() {
                match drag {
                    TextDrag::Move { start, origin } => {
                        let (width, height) = editor.size();
                        let x =
                            (origin.0 + client.0 - start.0).clamp(0, (area.width - width).max(0));
                        let y =
                            (origin.1 + client.1 - start.1).clamp(0, (area.height - height).max(0));
                        editor.move_to((x, y));
                    }
                    TextDrag::Resize { start, origin } => {
                        let (position_x, position_y) = editor.position();
                        let min_width = (120.0 * scale) as i32;
                        let min_height = (48.0 * scale) as i32;
                        let width = (origin.0 + client.0 - start.0)
                            .clamp(min_width, (area.width - position_x).max(min_width));
                        let height = (origin.1 + client.1 - start.1)
                            .clamp(min_height, (area.height - position_y).max(min_height));
                        editor.resize_to((width, height));
                    }
                }
            }
            self.invalidate();
            self.update_cursor();
            return;
        }

        // 步骤 2：更新标注拖拽或选区框选。
        if self.drag.is_some() {
            self.update_drag(screen);
            needs_paint = true;
        } else if self.selection.is_active() {
            self.selection.update(screen.0, screen.1, self.area);
            self.refresh_toolbar();
            needs_paint = true;
        }

        // 步骤 3：更新工具栏悬停状态与提示定时器。
        let client = (screen.0 - self.area.x, screen.1 - self.area.y);
        let hover = self
            .toolbar
            .as_ref()
            .and_then(|toolbar| toolbar.hit(client.0, client.1));
        if hover != self.toolbar_hover {
            self.toolbar_hover = hover;
            self.tooltip_action = None;
            needs_paint = true;
            unsafe {
                let _ = KillTimer(Some(self.hwnd), TIMER_TOOLTIP);
                if hover.is_some() && !matches!(hover, Some(ToolbarAction::Noop)) {
                    SetTimer(Some(self.hwnd), TIMER_TOOLTIP, 600, None);
                }
            }
        }
        if needs_paint {
            self.invalidate();
        }

        // 步骤 4：按最新状态刷新光标。
        self.update_cursor();
    }

    /// 处理左键释放：结束文字拖拽、标注或框选。
    fn on_mouse_up(&mut self, screen: (i32, i32)) {
        // 步骤 1：结束文字框拖拽，并把焦点还给 EDIT。
        if self.text_drag.take().is_some() {
            unsafe {
                let _ = ReleaseCapture();
            }
            if let Some(editor) = &self.text_editor {
                unsafe {
                    let _ = SetFocus(Some(editor.hwnd));
                }
            }
            self.invalidate();
            return;
        }

        // 步骤 2：落下标注，或结束选区框选并创建文档。
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

    /// 处理遮罩键盘快捷键。
    fn handle_key(&mut self, wparam: WPARAM) {
        // 步骤 1：文字输入期间按键归 EDIT，遮罩不处理。
        if self.text_editor.is_some() {
            return;
        }

        // 步骤 2：IME 的 VK_PROCESSKEY 用 ImmGetVirtualKey 还原真实按键。
        let mut key = wparam.0 as u16;
        if key == VK_PROCESSKEY.0 {
            key = unsafe { ImmGetVirtualKey(self.hwnd) } as u16;
        }
        let ctrl = unsafe { GetKeyState(VK_CONTROL.0 as i32) } < 0;
        let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0;
        ::log::debug!("key down {key:#06x} ctrl={ctrl} shift={shift}");

        // 步骤 3：Ctrl 快捷键：确认、撤销/重做。
        if ctrl {
            match key {
                key if key == b'C' as u16 => self.confirm(OutputAction::Copy),
                key if key == b'S' as u16 => self.confirm(OutputAction::Save),
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

        // 步骤 4：普通快捷键：取消、确认、方向键、线宽与工具。
        match key {
            key if key == VK_ESCAPE.0 => self.cancel(),
            key if key == VK_RETURN.0 => self.confirm(OutputAction::Copy),
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

    /// 方向键移动选区；resize 为 true 时缩放。
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

    /// 按当前命中区域更新鼠标光标。
    fn update_cursor(&self) {
        let client = (self.cursor.0 - self.area.x, self.cursor.1 - self.area.y);

        if let Some(editor) = &self.text_editor {
            let rect = editor.client_rect();
            let frame = RectI::new(rect.x - 4, rect.y - 4, rect.width + 8, rect.height + 8);
            let grip = RectI::new(rect.x + rect.width + 2, rect.y + rect.height + 2, 12, 12);
            let cursor = if inside(grip, client) {
                self.size_nwse
            } else if inside(frame, client) && !inside(rect, client) {
                self.size_all
            } else {
                self.arrow
            };
            unsafe {
                let _ = SetCursor(Some(cursor));
            }
            return;
        }

        let over_toolbar = self
            .toolbar
            .as_ref()
            .map(|toolbar| toolbar.hit(client.0, client.1).is_some())
            .unwrap_or(false);

        let cursor = if over_toolbar {
            self.arrow
        } else {
            match self.selection.rect() {
                Some(rect) if !self.has_shapes() => {
                    let handle = self.selection.active_handle().or_else(|| {
                        SelectionState::handle_at(
                            rect,
                            self.cursor.0,
                            self.cursor.1,
                            HANDLE_HIT_TOLERANCE,
                        )
                    });
                    match handle {
                        Some(Handle::NorthWest | Handle::SouthEast) => self.size_nwse,
                        Some(Handle::NorthEast | Handle::SouthWest) => self.size_nesw,
                        Some(Handle::North | Handle::South) => self.size_ns,
                        Some(Handle::East | Handle::West) => self.size_we,
                        None => {
                            if self.selection.is_moving()
                                || (self.tool == Tool::Select && inside(rect, self.cursor))
                            {
                                self.size_all
                            } else {
                                self.cross
                            }
                        }
                    }
                }
                Some(rect) if self.has_shapes() => {
                    if inside(rect, self.cursor) {
                        self.cross
                    } else {
                        self.arrow
                    }
                }
                _ => self.cross,
            }
        };

        unsafe {
            let _ = SetCursor(Some(cursor));
        }
    }

    /// 请求重绘整个遮罩。
    fn invalidate(&self) {
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd), None, false);
        }
    }
}

impl Drop for Overlay {
    fn drop(&mut self) {
        // 字体与画刷由 Overlay 持有，统一在此释放。
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

/// 生成压暗且不透明的 BGRA 副本。
fn dim_bgra(bgra: &[u8]) -> Vec<u8> {
    const FACTOR: u16 = 115;
    let mut dimmed = bgra.to_vec();
    for pixel in dimmed.as_chunks_mut::<4>().0 {
        for channel in &mut pixel[..3] {
            *channel = ((*channel as u16 * FACTOR) / 255) as u8;
        }
        pixel[3] = 255;
    }
    dimmed
}

/// 按 DPI 创建界面字体；失败时退回系统默认字体。
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

/// 尽力把窗口置前；必要时临时附加前台线程输入队列。
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

/// 用画刷填充客户区矩形。
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

/// 判断点是否位于矩形内（右、下边界不含）。
fn inside(rect: RectI, point: (i32, i32)) -> bool {
    point.0 >= rect.x
        && point.1 >= rect.y
        && point.0 < rect.x + rect.width
        && point.1 < rect.y + rect.height
}

/// ARGB 转 COLORREF（GDI 的 0x00BBGGRR）。
fn argb_to_colorref(argb: u32) -> COLORREF {
    let r = (argb >> 16) & 0xFF;
    let g = (argb >> 8) & 0xFF;
    let b = argb & 0xFF;
    COLORREF((b << 16) | (g << 8) | r)
}

/// 返回工具栏动作的提示文字。
fn tooltip_text(action: ToolbarAction) -> String {
    match action {
        ToolbarAction::Tool(tool) => format!(
            "{} ({})",
            tool.name(),
            char::from_u32(tool.shortcut() as u32).unwrap_or('?')
        ),
        ToolbarAction::Color(_) => "颜色".to_string(),
        ToolbarAction::Width(delta) => {
            if delta < 0 {
                "线宽 −".to_string()
            } else {
                "线宽 +".to_string()
            }
        }
        ToolbarAction::Undo => "撤销 (Ctrl+Z)".to_string(),
        ToolbarAction::Redo => "重做 (Ctrl+Y)".to_string(),
        ToolbarAction::Copy => "复制 (Ctrl+C)".to_string(),
        ToolbarAction::Pin => "贴图".to_string(),
        ToolbarAction::Save => "保存 (Ctrl+S)".to_string(),
        ToolbarAction::Cancel => "关闭 (Esc)".to_string(),
        ToolbarAction::Noop => String::new(),
    }
}

/// 取 LPARAM 低 16 位的带符号值。
fn loword(value: isize) -> i32 {
    ((value as usize) & 0xFFFF) as u16 as i16 as i32
}

/// 取 LPARAM 高 16 位的带符号值。
fn hiword(value: isize) -> i32 {
    (((value as usize) >> 16) & 0xFFFF) as u16 as i16 as i32
}

/// 取窗口绑定的 Overlay 状态（仅窗口存续期内有效）。
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

/// 遮罩窗口过程：转发绘制、输入与文字编辑消息。
unsafe extern "system" fn overlay_wndproc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: Overlay 指针由 WM_NCCREATE 写入，窗口销毁前保持有效。
    unsafe {
        // 步骤 1：创建时把 Overlay 指针绑定到 GWLP_USERDATA。
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
            // 步骤 2：绘制、鼠标、键盘与光标消息。
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
            // 步骤 3：定时器与文字编辑消息。
            WM_TIMER => {
                if let Some(overlay) = state(hwnd) {
                    match wparam.0 {
                        TIMER_SCRIPT => overlay.confirm(OutputAction::Copy),
                        TIMER_TOOLTIP => {
                            let _ = KillTimer(Some(hwnd), TIMER_TOOLTIP);
                            if overlay.toolbar_hover.is_some() {
                                overlay.tooltip_action = overlay.toolbar_hover;
                                overlay.invalidate();
                            }
                        }
                        _ => {}
                    }
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
            // 步骤 4：EDIT 文字颜色跟随当前色，背景保持透明。
            WM_CTLCOLOREDIT => {
                if let Some(overlay) = state(hwnd) {
                    let hdc = HDC(wparam.0 as *mut c_void);
                    let _ = SetTextColor(hdc, argb_to_colorref(overlay.color));
                    let _ = SetBkMode(hdc, TRANSPARENT);
                    return LRESULT(GetStockObject(NULL_BRUSH).0 as isize);
                }
                LRESULT(0)
            }
            WM_NCHITTEST => LRESULT(HTCLIENT as isize),
            _ => DefWindowProcW(hwnd, message, wparam, lparam),
        }
    }
}
