//! 全局热键与常驻模式（P2：无托盘，命令行启动）。
//!
//! - `Ctrl+Alt+A` 截图
//! - `Ctrl+Alt+Q` 退出
//!
//! `--once` 用于测试：不起热键，直接走一次截图流程。

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, RegisterHotKey,
    UnregisterHotKey, VK_A, VK_Q, VK_X,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
    GetMessageW, MSG, PostQuitMessage, RegisterClassW, TranslateMessage, WINDOW_EX_STYLE,
    WM_DESTROY, WM_HOTKEY, WNDCLASSW, WS_OVERLAPPED,
};
use windows::core::w;

use crate::capture::{CaptureBackend, GdiBackend, RectI, VirtualScreen};
use crate::output::{clipboard, files};
use crate::ui::{self, OverlayOptions, OverlayResult};
use crate::util::error::{AppError, Result};
use crate::util::resources::process_resources;

const HOTKEY_CAPTURE: i32 = 1;
const HOTKEY_QUIT: i32 = 2;

static CAPTURING: AtomicBool = AtomicBool::new(false);

/// 常驻：注册全局热键并进入消息循环。
pub fn run_resident() -> Result<()> {
    unsafe {
        let hinstance = HINSTANCE(GetModuleHandleW(None)?.0);
        let mut window_class: WNDCLASSW = std::mem::zeroed();
        window_class.style = CS_HREDRAW | CS_VREDRAW;
        window_class.lpfnWndProc = Some(hotkey_wndproc);
        window_class.hInstance = hinstance;
        window_class.lpszClassName = w!("window_cap_hotkey");
        let _ = RegisterClassW(&window_class);

        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("window_cap_hotkey"),
            w!("window-cap"),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(hinstance),
            None,
        )?;

        // 默认 Ctrl+Alt+A 常被 IM 占用，失败时自动退避到其它组合。
        let candidates = [
            ("Ctrl+Alt+A", MOD_CONTROL | MOD_ALT | MOD_NOREPEAT, VK_A),
            ("Ctrl+Shift+A", MOD_CONTROL | MOD_SHIFT | MOD_NOREPEAT, VK_A),
            ("Ctrl+Alt+X", MOD_CONTROL | MOD_ALT | MOD_NOREPEAT, VK_X),
        ];

        let mut registered: Option<(&str, HOT_KEY_MODIFIERS)> = None;
        let mut last_error = None;
        for (label, modifiers, virtual_key) in candidates {
            match RegisterHotKey(Some(hwnd), HOTKEY_CAPTURE, modifiers, virtual_key.0 as u32) {
                Ok(()) => {
                    registered = Some((label, modifiers));
                    break;
                }
                Err(error) => last_error = Some((label, error)),
            }
        }

        let Some((capture_label, modifiers)) = registered else {
            let detail = last_error
                .map(|(label, error)| format!("{label}: {error}"))
                .unwrap_or_default();
            return Err(AppError::msg(format!(
                "截图热键全部被占用（Ctrl+Alt+A / Ctrl+Shift+A / Ctrl+Alt+X）: {detail}"
            )));
        };
        let quit_label = format!("{}Q", &capture_label[..capture_label.len() - 1]);

        if let Err(error) = RegisterHotKey(Some(hwnd), HOTKEY_QUIT, modifiers, VK_Q.0 as u32) {
            let _ = UnregisterHotKey(Some(hwnd), HOTKEY_CAPTURE);
            return Err(AppError::msg(format!(
                "注册退出热键 {quit_label} 失败: {error}"
            )));
        }

        if capture_label != "Ctrl+Alt+A" {
            println!("提示: Ctrl+Alt+A 已被其他程序占用，改用 {capture_label}");
        }
        println!("window-cap 已启动");
        println!("  截图: {capture_label}");
        println!("  退出: {quit_label}（或关闭控制台）");
        ::log::info!("resident started, hotkey {capture_label}");

        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).0 > 0 {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }

        let _ = UnregisterHotKey(Some(hwnd), HOTKEY_CAPTURE);
        let _ = UnregisterHotKey(Some(hwnd), HOTKEY_QUIT);
    }

    Ok(())
}

/// 不走热键，直接执行一次截图流程（测试/脚本用）。
pub fn run_once() -> Result<()> {
    capture_flow(None).map(|_| ())
}

/// 遮罩压力测试：脚本化选区，反复创建/绘制/确认遮罩，检查句柄泄漏。
pub fn run_overlay_selftest(iterations: u32) -> Result<()> {
    let screen = VirtualScreen::current();
    if screen.width <= 0 || screen.height <= 0 {
        return Err(AppError::msg("虚拟桌面尺寸无效"));
    }

    let width = 320.min(screen.width);
    let height = 200.min(screen.height);
    let rect = RectI::new(
        screen.x + (screen.width - width) / 2,
        screen.y + (screen.height - height) / 2,
        width,
        height,
    );

    let mut backend = GdiBackend;

    // GDI/USER 子系统会懒分配一批对象，先预热再取基线，避免误判泄漏。
    let warmup = 3.min(iterations);
    for _ in 0..warmup {
        let frame = backend.capture(screen)?;
        let _ = ui::select(
            frame,
            screen,
            OverlayOptions {
                scripted: Some(rect),
                script_delay_ms: 40,
            },
        )?;
    }

    let (gdi_before, user_before, memory_before) = process_resources()?;
    let started = Instant::now();
    let mut confirmed = 0u32;

    for index in 0..iterations {
        let frame = backend.capture(screen)?;
        match ui::select(
            frame,
            screen,
            OverlayOptions {
                scripted: Some(rect),
                script_delay_ms: 40,
            },
        )? {
            OverlayResult::Confirmed {
                image,
                rect: selected,
            } if selected == rect
                && image.width == rect.width as u32
                && image.height == rect.height as u32 =>
            {
                confirmed += 1;
            }
            _ => {
                return Err(AppError::msg(format!(
                    "第 {} 次脚本化选区结果异常",
                    index + 1
                )));
            }
        }
    }

    let elapsed = started.elapsed();
    let (gdi_after, user_after, memory_after) = process_resources()?;
    let gdi_delta = gdi_after as i64 - gdi_before as i64;
    let user_delta = user_after as i64 - user_before as i64;

    println!(
        "overlay selftest: {confirmed}/{iterations} 次确认（预热 {warmup} 次），平均 {:.1} ms/次",
        elapsed.as_secs_f64() * 1000.0 / iterations as f64
    );
    println!(
        "GDI {} -> {} ({:+})，USER {} -> {} ({:+})，内存 {:.1} -> {:.1} MB",
        gdi_before,
        gdi_after,
        gdi_delta,
        user_before,
        user_after,
        user_delta,
        memory_before as f64 / 1024.0 / 1024.0,
        memory_after as f64 / 1024.0 / 1024.0
    );

    if gdi_delta > 2 || user_delta > 2 {
        return Err(AppError::msg(format!(
            "遮罩窗口句柄泄漏: GDI {gdi_delta:+}, USER {user_delta:+}"
        )));
    }

    Ok(())
}

fn capture_flow(scripted: Option<RectI>) -> Result<Option<RectI>> {
    let screen = VirtualScreen::current();
    if screen.width <= 0 || screen.height <= 0 {
        return Err(AppError::msg("虚拟桌面尺寸无效"));
    }

    let mut backend = GdiBackend;
    let started = Instant::now();
    let frame = backend.capture(screen)?;
    let capture_ms = started.elapsed().as_secs_f64() * 1000.0;

    let result = ui::select(
        frame,
        screen,
        OverlayOptions {
            scripted,
            script_delay_ms: 80,
        },
    )?;

    match result {
        OverlayResult::Confirmed { image, rect } => {
            clipboard::put(&image)?;
            let path = files::default_path();
            files::save_png(&image, &path)?;
            println!(
                "已复制 {}×{} 到剪贴板，保存到 {}",
                image.width,
                image.height,
                path.display()
            );
            ::log::info!(
                "capture ok {}x{} capture_ms={:.1}",
                image.width,
                image.height,
                capture_ms
            );
            Ok(Some(rect))
        }
        OverlayResult::Cancelled => {
            println!("已取消");
            ::log::info!("capture cancelled");
            Ok(None)
        }
    }
}

unsafe extern "system" fn hotkey_wndproc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        match message {
            WM_HOTKEY => {
                match wparam.0 as i32 {
                    HOTKEY_CAPTURE => {
                        if !CAPTURING.swap(true, Ordering::SeqCst) {
                            if let Err(error) = capture_flow(None) {
                                eprintln!("截图失败: {error}");
                                ::log::error!("capture failed: {error}");
                            }
                            CAPTURING.store(false, Ordering::SeqCst);
                        }
                    }
                    HOTKEY_QUIT => {
                        let _ = DestroyWindow(hwnd);
                    }
                    _ => {}
                }
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
