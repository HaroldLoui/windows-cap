//! 全屏遮罩窗口：冻结帧 + 压暗 + 选区 + 手柄 + 尺寸标签。
//!
//! 交互（P2）：
//! - 拖拽创建选区；松手后保留，可继续移动/缩放
//! - `Enter` / `Ctrl+C` / 双击：确认（复制 + 保存）
//! - `Esc` / 右键：取消
//! - 方向键移动 1px，`Shift` + 方向键缩放
//! - 空白处单击（未拖动且无选区）：取消遮罩
//!
//! P3 会在此基础上加入标注层与工具栏。

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
    GetStockObject, HBRUSH, HFONT, HGDIOBJ, IntersectClipRect, InvalidateRect, OUT_DEFAULT_PRECIS,
    PAINTSTRUCT, RestoreDC, SRCCOPY, SaveDC, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
    UpdateWindow,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, ReleaseCapture, SetCapture, SetFocus, VK_CONTROL, VK_DOWN, VK_ESCAPE, VK_LEFT,
    VK_RETURN, VK_RIGHT, VK_SHIFT, VK_UP,
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

use crate::capture::{Frame, RectI, VirtualScreen};
use crate::platform::dib::Dib;
use crate::ui::selection::{EndResult, Handle, SelectionState};
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OverlayResult {
    Confirmed(RectI),
    Cancelled,
}

pub fn select(
    frame: &Frame,
    area: VirtualScreen,
    options: OverlayOptions,
) -> Result<OverlayResult> {
    let dimmed = dim_bgra(&frame.bgra);
    let bright = Dib::from_bgra(frame.width, frame.height, &frame.bgra)?;
    let dim = Dib::from_bgra(frame.width, frame.height, &dimmed)?;

    let mut state = SelectionState::new();
    if let Some(rect) = options.scripted {
        state.set_selection(rect);
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
            bright,
            dim,
            state,
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

        // 禁用 DWM 淡入，避免遮罩出现时闪一下。
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
            "overlay closed: result={:?} first_paint_ms={:?} paints={} avg_paint_ms={:.1}",
            result,
            overlay.first_paint_ms,
            paints,
            average_paint_ms
        );

        drop(overlay);
        Ok(result)
    }
}

struct Overlay {
    hwnd: HWND,
    area: VirtualScreen,
    bright: Dib,
    dim: Dib,
    state: SelectionState,
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

    fn confirm(&mut self) {
        if self.result.is_some() {
            return;
        }
        let rect = self.state.rect().unwrap_or_else(|| self.area_rect());
        ::log::debug!("overlay confirm rect={rect:?}");
        if rect.is_empty() {
            return;
        }
        self.finish(OverlayResult::Confirmed(rect));
    }

    fn cancel(&mut self) {
        self.finish(OverlayResult::Cancelled);
    }

    fn finish(&mut self, result: OverlayResult) {
        if self.result.is_some() {
            return;
        }
        self.result = Some(result);
        unsafe {
            let _ = KillTimer(Some(self.hwnd), TIMER_SCRIPT);
            let _ = DestroyWindow(self.hwnd);
            let _ = PostThreadMessageW(GetCurrentThreadId(), WM_FINISHED, WPARAM(0), LPARAM(0));
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
            let width = self.area.width;
            let height = self.area.height;

            let saved = SaveDC(hdc);
            let _ = BitBlt(hdc, 0, 0, width, height, Some(self.dim.dc), 0, 0, SRCCOPY);

            if let Some(rect) = self.state.current_rect() {
                let local = self.to_local(rect);
                let _ = IntersectClipRect(
                    hdc,
                    local.x,
                    local.y,
                    local.x + local.width,
                    local.y + local.height,
                );
                let _ = BitBlt(
                    hdc,
                    0,
                    0,
                    width,
                    height,
                    Some(self.bright.dc),
                    0,
                    0,
                    SRCCOPY,
                );
            }
            let _ = RestoreDC(hdc, saved);

            match self.state.current_rect() {
                Some(rect) => self.draw_frame(hdc, rect),
                None => self.draw_hint(hdc),
            }

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

    unsafe fn draw_frame(&self, hdc: windows::Win32::Graphics::Gdi::HDC, rect: RectI) {
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

    fn handle_key(&mut self, wparam: WPARAM) {
        let key = wparam.0 as u16;
        let ctrl = unsafe { GetKeyState(VK_CONTROL.0 as i32) } < 0;
        let shift = unsafe { GetKeyState(VK_SHIFT.0 as i32) } < 0;

        let mut changed = false;
        match key {
            key if key == VK_ESCAPE.0 => self.cancel(),
            key if key == VK_RETURN.0 => self.confirm(),
            key if key == b'C' as u16 && ctrl => self.confirm(),
            key if key == b'S' as u16 && ctrl => self.confirm(),
            key if key == VK_LEFT.0 => {
                self.arrow(-1, 0, shift);
                changed = true;
            }
            key if key == VK_RIGHT.0 => {
                self.arrow(1, 0, shift);
                changed = true;
            }
            key if key == VK_UP.0 => {
                self.arrow(0, -1, shift);
                changed = true;
            }
            key if key == VK_DOWN.0 => {
                self.arrow(0, 1, shift);
                changed = true;
            }
            _ => {}
        }

        if changed {
            unsafe {
                let _ = InvalidateRect(Some(self.hwnd), None, false);
            }
        }
    }

    fn arrow(&mut self, dx: i32, dy: i32, resize: bool) {
        if resize {
            self.state.resize_by(dx, dy, self.area);
        } else {
            self.state.move_by(dx, dy, self.area);
        }
    }

    fn update_cursor(&self) {
        let over_selection = match self.state.rect() {
            Some(rect) => {
                self.cursor.0 >= rect.x
                    && self.cursor.1 >= rect.y
                    && self.cursor.0 < rect.x + rect.width
                    && self.cursor.1 < rect.y + rect.height
            }
            None => false,
        };
        let cursor = if over_selection {
            self.arrow
        } else {
            self.cross
        };
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
    const FACTOR: u16 = 115; // 0.45
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
                    overlay.cursor = point;
                    overlay.state.update(point.0, point.1, overlay.area);
                    overlay.update_cursor();
                    if overlay.state.is_active() {
                        overlay.invalidate();
                    }
                }
                LRESULT(0)
            }
            WM_LBUTTONDOWN => {
                if let Some(overlay) = state(hwnd) {
                    let point = overlay.screen_point(lparam);
                    ::log::debug!(
                        "overlay mouse down at {point:?}, rect={:?}",
                        overlay.state.rect()
                    );
                    overlay.cursor = point;
                    overlay.state.begin(point.0, point.1, overlay.area);
                    SetCapture(hwnd);
                    overlay.invalidate();
                }
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                if let Some(overlay) = state(hwnd) {
                    let point = overlay.screen_point(lparam);
                    ::log::debug!("overlay mouse up at {point:?}");
                    overlay.cursor = point;
                    overlay.state.update(point.0, point.1, overlay.area);
                    let _ = ReleaseCapture();
                    match overlay.state.end(overlay.area) {
                        EndResult::Cancelled => overlay.cancel(),
                        _ => overlay.invalidate(),
                    }
                }
                LRESULT(0)
            }
            WM_LBUTTONDBLCLK => {
                if let Some(overlay) = state(hwnd) {
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
            WM_NCHITTEST => LRESULT(HTCLIENT as isize),
            _ => DefWindowProcW(hwnd, message, wparam, lparam),
        }
    }
}
