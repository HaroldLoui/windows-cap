//! P0 技术验证（Spike）：GDI 采集、焦点、IME 与 GDI+ 绘制性能。

use std::mem::size_of;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, CreateFontW, DEFAULT_CHARSET, DeleteObject,
    FF_DONTCARE, FW_NORMAL, HGDIOBJ, OUT_DEFAULT_PRECIS,
};
use windows::Win32::Graphics::GdiPlus::{
    FlushIntentionSync, FontStyleRegular, GdipCreateFont, GdipCreateFontFamilyFromName,
    GdipCreateFromHDC, GdipCreatePen1, GdipCreateSolidFill, GdipCreateStringFormat,
    GdipDeleteBrush, GdipDeleteFont, GdipDeleteFontFamily, GdipDeleteGraphics, GdipDeletePen,
    GdipDeleteStringFormat, GdipDrawEllipse, GdipDrawLine, GdipDrawLines, GdipDrawRectangle,
    GdipDrawString, GdipFillEllipse, GdipFlush, GdipSetPageUnit, GdipSetSmoothingMode,
    GdipSetTextRenderingHint, GdiplusShutdown, GdiplusStartup, GdiplusStartupInput,
    GdiplusStartupOutput, GpBrush, GpFont, GpFontCollection, GpFontFamily, GpGraphics, GpPen,
    GpSolidFill, GpStringFormat, PointF, RectF, SmoothingModeAntiAlias, Status,
    TextRenderingHintAntiAliasGridFit, UnitPixel,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::Input::Ime::{
    ImmGetContext, ImmGetDefaultIMEWnd, ImmIsIME, ImmReleaseContext,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyboardLayout, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT,
    KEYEVENTF_KEYUP, SendInput, SetFocus, VIRTUAL_KEY, VK_ESCAPE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
    ES_AUTOVSCROLL, ES_MULTILINE, GetForegroundWindow, GetMessageW, GetWindowThreadProcessId,
    IsWindow, KillTimer, MSG, PostQuitMessage, RegisterClassW, SW_SHOW, SendMessageW,
    SetForegroundWindow, SetTimer, ShowWindow, TranslateMessage, WINDOW_STYLE, WM_DESTROY,
    WM_KEYDOWN, WM_SETFONT, WM_TIMER, WNDCLASSW, WS_CHILD, WS_EX_CLIENTEDGE, WS_EX_TOPMOST,
    WS_OVERLAPPEDWINDOW, WS_POPUP, WS_VISIBLE, WS_VSCROLL,
};
use windows::core::{PCWSTR, w};

use crate::capture::{self, VirtualScreen};
use crate::output;
use crate::platform::dib::Dib;
use crate::util::error::{AppError, Result};

const FOCUS_CLASS: PCWSTR = w!("window_cap_spike_focus");
const IME_CLASS: PCWSTR = w!("window_cap_spike_ime");

static ESC_RECEIVED: AtomicBool = AtomicBool::new(false);

/// 按名称运行对应的 spike。
pub fn run(name: &str) -> Result<()> {
    match name {
        "capture" | "s1" => capture_spike(),
        "focus" | "s2" => focus_spike(),
        "ime" | "s3" => ime_spike(),
        "gdiplus" | "s4" => gdiplus_spike(),
        other => Err(AppError::msg(format!(
            "未知 spike: {other}（可选 capture | focus | ime | gdiplus）"
        ))),
    }
}

/// 验证 GDI 能否截取虚拟桌面并保存 PNG。
fn capture_spike() -> Result<()> {
    let screen = VirtualScreen::current();
    println!(
        "[S1] 虚拟桌面: {}x{} @ ({}, {})",
        screen.width, screen.height, screen.x, screen.y
    );
    if screen.width <= 0 || screen.height <= 0 {
        return Err(AppError::msg("虚拟桌面尺寸无效"));
    }

    let monitors = capture::monitors();
    println!("[S1] 显示器数量: {}", monitors.len());
    for (index, monitor) in monitors.iter().enumerate() {
        println!(
            "      #{} {} {}x{} @ ({}, {}) dpi={}x{} device={}",
            index,
            if monitor.primary {
                "[主屏]"
            } else {
                "      "
            },
            monitor.rect.width,
            monitor.rect.height,
            monitor.rect.x,
            monitor.rect.y,
            monitor.dpi_x,
            monitor.dpi_y,
            monitor.device
        );
    }

    let started = Instant::now();
    let frame = capture::gdi::capture(screen)?;
    let elapsed = started.elapsed();
    println!(
        "[S1] 截屏 {}x{} @ ({}, {})，耗时 {:.1} ms，非黑像素 {:.1}%",
        frame.width,
        frame.height,
        frame.origin.0,
        frame.origin.1,
        elapsed.as_secs_f64() * 1000.0,
        frame.non_black_ratio() * 100.0
    );

    let path = std::path::Path::new("target/spike/s1-capture.png");
    output::save_png(&frame, path)?;
    println!("[S1] 已保存 {}", path.display());

    if frame.non_black_ratio() < 0.01 {
        println!("[S1] 警告: 画面几乎全黑（锁屏/无桌面/采集受限），结论需人工复核");
    }

    Ok(())
}

/// 验证 4K DIB 上 GDI+ 绘制 8 种形状与中文文本的性能。
fn gdiplus_spike() -> Result<()> {
    let dib = Dib::new(3840, 2160)?;

    // 步骤 1：启动 GDI+ 并绑定 DIB 的内存 DC。
    unsafe {
        let mut token = 0usize;
        let input = GdiplusStartupInput {
            GdiplusVersion: 1,
            ..Default::default()
        };
        let mut output = GdiplusStartupOutput::default();
        check(
            GdiplusStartup(&mut token, &input, &mut output),
            "GdiplusStartup",
        )?;

        let mut graphics: *mut GpGraphics = null_mut();
        check(
            GdipCreateFromHDC(dib.dc, &mut graphics),
            "GdipCreateFromHDC",
        )?;

        let _ = GdipSetSmoothingMode(graphics, SmoothingModeAntiAlias);
        let _ = GdipSetPageUnit(graphics, UnitPixel);
        let _ = GdipSetTextRenderingHint(graphics, TextRenderingHintAntiAliasGridFit);

        // 步骤 2：准备画笔、画刷、字体与文本格式。
        let mut pen: *mut GpPen = null_mut();
        check(
            GdipCreatePen1(0xFFFF9B2E, 8.0, UnitPixel, &mut pen),
            "GdipCreatePen1",
        )?;

        let mut fill: *mut GpSolidFill = null_mut();
        check(
            GdipCreateSolidFill(0xFF2E9BFF, &mut fill),
            "GdipCreateSolidFill",
        )?;

        let mut family: *mut GpFontFamily = null_mut();
        check(
            GdipCreateFontFamilyFromName(
                w!("Microsoft YaHei UI"),
                null_mut::<GpFontCollection>(),
                &mut family,
            ),
            "GdipCreateFontFamilyFromName",
        )?;

        let mut font: *mut GpFont = null_mut();
        check(
            GdipCreateFont(family, 72.0, FontStyleRegular.0, UnitPixel, &mut font),
            "GdipCreateFont",
        )?;

        let mut format: *mut GpStringFormat = null_mut();
        check(
            GdipCreateStringFormat(0, 0, &mut format),
            "GdipCreateStringFormat",
        )?;

        let points = [
            PointF { X: 200.0, Y: 400.0 },
            PointF {
                X: 600.0,
                Y: 1500.0,
            },
            PointF {
                X: 1200.0,
                Y: 300.0,
            },
            PointF {
                X: 1800.0,
                Y: 1500.0,
            },
            PointF {
                X: 2400.0,
                Y: 500.0,
            },
        ];

        let text: Vec<u16> = "中文标注测试 · window-cap".encode_utf16().collect();
        let text_rect = RectF {
            X: 1200.0,
            Y: 1700.0,
            Width: 1800.0,
            Height: 220.0,
        };

        // 步骤 3：重复绘制 8 种形状并计时。
        let iterations = 10u32;
        let started = Instant::now();
        for index in 0..iterations {
            let shift = index as f32 * 30.0;
            let _ = GdipDrawLine(graphics, pen, 100.0 + shift, 100.0, 1200.0 + shift, 700.0);
            let _ = GdipDrawRectangle(graphics, pen, 300.0 + shift, 200.0, 600.0, 400.0);
            let _ = GdipDrawEllipse(graphics, pen, 1000.0 + shift, 200.0, 500.0, 300.0);
            let _ = GdipFillEllipse(
                graphics,
                fill as *mut GpBrush,
                2000.0 + shift,
                300.0,
                400.0,
                400.0,
            );
            let _ = GdipDrawLines(graphics, pen, points.as_ptr(), points.len() as i32);
            let _ = GdipDrawLine(graphics, pen, 2600.0 + shift, 300.0, 3200.0 + shift, 600.0);
            let _ = GdipDrawLine(graphics, pen, 3200.0 + shift, 600.0, 3100.0 + shift, 520.0);
            let _ = GdipDrawString(
                graphics,
                PCWSTR::from_raw(text.as_ptr()),
                text.len() as i32,
                font,
                &text_rect,
                format,
                fill as *const GpBrush,
            );
        }
        let elapsed = started.elapsed();
        // 步骤 4：刷新输出、统计文字像素并释放 GDI+ 资源。
        let _ = GdipFlush(graphics, FlushIntentionSync);

        let lit = count_lit_region(dib.pixels(), 3840, 1200, 1700, 3000, 1920);

        println!(
            "[S4] 4K DIB（3840x2160）8 形状 × {iterations} 次: 总 {:.1} ms，平均 {:.2} ms/帧",
            elapsed.as_secs_f64() * 1000.0,
            elapsed.as_secs_f64() * 1000.0 / iterations as f64
        );
        println!(
            "[S4] 文字区域非黑像素: {lit}（{}）",
            if lit > 500 {
                "CJK 渲染正常"
            } else {
                "可能未渲染，需要检查字体"
            }
        );

        let _ = GdipDeleteStringFormat(format);
        let _ = GdipDeleteFont(font);
        let _ = GdipDeleteFontFamily(family);
        let _ = GdipDeleteBrush(fill as *mut GpBrush);
        let _ = GdipDeletePen(pen);
        let _ = GdipDeleteGraphics(graphics);
        GdiplusShutdown(token);
    }

    Ok(())
}

/// 验证全屏置顶窗口能否取得焦点并接收键盘输入。
fn focus_spike() -> Result<()> {
    let screen = VirtualScreen::current();

    // 步骤 1：注册并创建与虚拟桌面等大的置顶窗口。
    unsafe {
        let hinstance = HINSTANCE(GetModuleHandleW(None)?.0);
        let mut window_class: WNDCLASSW = std::mem::zeroed();
        window_class.style = CS_HREDRAW | CS_VREDRAW;
        window_class.lpfnWndProc = Some(spike_wndproc);
        window_class.hInstance = hinstance;
        window_class.lpszClassName = FOCUS_CLASS;
        let _ = RegisterClassW(&window_class);

        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST,
            FOCUS_CLASS,
            w!("window-cap spike: focus"),
            WS_POPUP | WS_VISIBLE,
            screen.x,
            screen.y,
            screen.width,
            screen.height,
            None,
            None,
            Some(hinstance),
            None,
        )?;

        let _ = ShowWindow(hwnd, SW_SHOW);
        // 步骤 2：直接抢前台焦点，失败时用 AttachThreadInput 再试。
        let direct = SetForegroundWindow(hwnd).0 != 0;
        let mut focused = GetForegroundWindow() == hwnd;

        let mut attached = false;
        let mut after_attach = false;
        if !focused {
            let foreground = GetForegroundWindow();
            let foreground_thread = GetWindowThreadProcessId(foreground, None);
            let our_thread = GetCurrentThreadId();
            attached = AttachThreadInput(our_thread, foreground_thread, true).0 != 0;
            after_attach = SetForegroundWindow(hwnd).0 != 0;
            if attached {
                let _ = AttachThreadInput(our_thread, foreground_thread, false);
            }
            focused = GetForegroundWindow() == hwnd;
        }
        let _ = SetFocus(Some(hwnd));

        SetTimer(Some(hwnd), 1, 4000, None);
        println!(
            "[S2] 全屏遮罩已显示: {}x{} @ ({}, {})",
            screen.width, screen.height, screen.x, screen.y
        );
        println!(
            "[S2] SetForegroundWindow 直接结果: {direct}; AttachThreadInput: {attached} -> {after_attach}; 最终前台: {focused}"
        );

        // 步骤 3：注入 Esc 并运行消息循环（4 秒后由定时器退出）。
        if focused {
            std::thread::sleep(Duration::from_millis(250));
            send_key(VK_ESCAPE, KEYBD_EVENT_FLAGS(0));
            send_key(VK_ESCAPE, KEYEVENTF_KEYUP);
        }

        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).0 > 0 {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }

        // 步骤 4：清理窗口并输出结论。
        let _ = KillTimer(Some(hwnd), 1);
        if IsWindow(Some(hwnd)).0 != 0 {
            let _ = DestroyWindow(hwnd);
        }

        let received = ESC_RECEIVED.load(Ordering::SeqCst);
        println!("[S2] 注入 Esc 后窗口收到 WM_KEYDOWN: {received}");
        println!(
            "[S2] 结论: {}",
            if focused && received {
                "通过：可抢焦点并接收键盘"
            } else if focused {
                "部分通过：焦点成功，但按键未收到（需检查 SetFocus）"
            } else {
                "未通过：无法取得前台焦点（CLI 启动场景；热键场景需 P2 复测）"
            }
        );
    }

    Ok(())
}

/// 验证 EDIT 子窗口是否具备 IME 上下文。
fn ime_spike() -> Result<()> {
    // 步骤 1：创建宿主窗口和 EDIT 子窗口。
    unsafe {
        let hinstance = HINSTANCE(GetModuleHandleW(None)?.0);
        let mut window_class: WNDCLASSW = std::mem::zeroed();
        window_class.style = CS_HREDRAW | CS_VREDRAW;
        window_class.lpfnWndProc = Some(spike_wndproc);
        window_class.hInstance = hinstance;
        window_class.lpszClassName = IME_CLASS;
        let _ = RegisterClassW(&window_class);

        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST,
            IME_CLASS,
            w!("window-cap spike: IME"),
            WS_OVERLAPPEDWINDOW,
            240,
            240,
            680,
            360,
            None,
            None,
            Some(hinstance),
            None,
        )?;

        let edit = CreateWindowExW(
            WS_EX_CLIENTEDGE,
            w!("EDIT"),
            w!(""),
            WS_CHILD
                | WS_VISIBLE
                | WINDOW_STYLE((ES_MULTILINE | ES_AUTOVSCROLL) as u32)
                | WS_VSCROLL,
            16,
            16,
            630,
            280,
            Some(hwnd),
            None,
            Some(hinstance),
            None,
        )?;

        let font = CreateFontW(
            -20,
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
        if !font.0.is_null() {
            SendMessageW(
                edit,
                WM_SETFONT,
                Some(WPARAM(font.0 as usize)),
                Some(LPARAM(1)),
            );
        }

        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
        let _ = SetFocus(Some(edit));

        // 步骤 2：检查键盘布局与 EDIT 的 IME 上下文。
        let keyboard_layout = GetKeyboardLayout(0);
        let is_ime = ImmIsIME(keyboard_layout).0 != 0;
        let ime_context = ImmGetContext(edit);
        let ime_window = ImmGetDefaultIMEWnd(hwnd);

        println!("[S3] 键盘布局: {:?}，是否 IME: {is_ime}", keyboard_layout.0);
        println!("[S3] EDIT 的 IME 上下文: {}", !ime_context.0.is_null());
        println!("[S3] 默认 IME 窗口: {}", !ime_window.0.is_null());
        println!("[S3] 请在弹出的编辑框里用中文输入法输入几个字（2 秒后自动关闭）");

        if !ime_context.0.is_null() {
            let _ = ImmReleaseContext(edit, ime_context);
        }

        // 步骤 3：2 秒后自动关闭，并清理窗口与字体。
        SetTimer(Some(hwnd), 2, 2000, None);
        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).0 > 0 {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }

        if IsWindow(Some(hwnd)).0 != 0 {
            let _ = DestroyWindow(hwnd);
        }
        if !font.0.is_null() {
            let _ = DeleteObject(HGDIOBJ(font.0));
        }
    }

    Ok(())
}

/// 把 GDI+ 状态码转成带上下文的错误。
fn check(status: Status, what: &str) -> Result<()> {
    if status.0 == 0 {
        Ok(())
    } else {
        Err(AppError::msg(format!(
            "GDI+ {what} 失败: status={}",
            status.0
        )))
    }
}

/// 注入一次键盘事件。
fn send_key(virtual_key: VIRTUAL_KEY, flags: KEYBD_EVENT_FLAGS) {
    let input = INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: virtual_key,
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    unsafe {
        SendInput(&[input], size_of::<INPUT>() as i32);
    }
}

/// 统计区域内非黑像素数量。
fn count_lit_region(pixels: &[u8], width: u32, x0: u32, y0: u32, x1: u32, y1: u32) -> usize {
    let mut count = 0;
    for y in y0..y1 {
        let row = (y * width) as usize * 4;
        for x in x0..x1 {
            let offset = row + (x as usize) * 4;
            if pixels[offset] | pixels[offset + 1] | pixels[offset + 2] != 0 {
                count += 1;
            }
        }
    }
    count
}

/// spike 窗口过程：Esc 或定时器触发退出。
unsafe extern "system" fn spike_wndproc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        match message {
            WM_KEYDOWN if wparam.0 as u16 == VK_ESCAPE.0 => {
                ESC_RECEIVED.store(true, Ordering::SeqCst);
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_TIMER => {
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, message, wparam, lparam),
        }
    }
}
