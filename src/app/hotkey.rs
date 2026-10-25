//! 常驻模式：托盘、全局热键、单实例、配置热加载、延迟截图。

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Console::{GetConsoleProcessList, GetConsoleWindow};
use windows::Win32::System::DataExchange::COPYDATASTRUCT;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    MOD_NOREPEAT, RegisterHotKey, UnregisterHotKey, VK_Q,
};
use windows::Win32::UI::Shell::NIN_SELECT;
use windows::Win32::UI::WindowsAndMessaging::{
    CREATESTRUCTW, CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DefWindowProcW, DestroyWindow,
    DispatchMessageW, GWLP_USERDATA, GetMessageW, GetWindowLongPtrW, KillTimer, MSG,
    PostQuitMessage, RegisterClassW, SW_HIDE, SetTimer, SetWindowLongPtrW, ShowWindow,
    TranslateMessage, WINDOW_EX_STYLE, WM_CONTEXTMENU, WM_COPYDATA, WM_DESTROY, WM_HOTKEY,
    WM_LBUTTONDBLCLK, WM_LBUTTONUP, WM_NCCREATE, WM_RBUTTONUP, WM_TIMER, WNDCLASSW, WS_OVERLAPPED,
};
use windows::core::w;

use super::autostart;
use super::config::{Config, DEFAULT_CAPTURE_HOTKEY, DEFAULT_QUIT_HOTKEY, Hotkey};
use super::single_instance::{self, SingleInstance, WM_APP_ACTIVATE};
use super::tray::{Tray, TrayCommand, WM_TRAY};
use crate::capture::{CaptureBackend, GdiBackend, RectI, VirtualScreen};
use crate::output::{clipboard, files};
use crate::ui::{self, OverlayOptions, OverlayResult};
use crate::util::error::{AppError, Result};
use crate::util::resources::process_resources;

const HOTKEY_CAPTURE: i32 = 1;
const HOTKEY_QUIT: i32 = 2;
const TIMER_DELAY: usize = 1;
const TRAY_DELAY_MS: u32 = 3000;

static CAPTURING: AtomicBool = AtomicBool::new(false);

/// 常驻：托盘 + 全局热键 + 单实例。
pub fn run_resident(config_path: Option<PathBuf>) -> Result<()> {
    let single_instance = SingleInstance::acquire()?;
    if single_instance.already_running() {
        let args: Vec<String> = std::env::args().skip(1).collect();
        if !args.is_empty() && single_instance::forward_to_existing(&args) {
            println!("已转交给运行中的 window-cap");
        } else if single_instance::activate_existing() {
            println!("window-cap 已在运行");
        } else {
            eprintln!("window-cap 已在运行（未找到常驻窗口）");
        }
        return Ok(());
    }

    hide_own_console();

    let path = Config::path(config_path.as_deref());
    let (config, warning) = Config::load_or_create(&path);
    if let Some(warning) = &warning {
        eprintln!("配置: {warning}");
        ::log::warn!("{warning}");
    }

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

        let mut app = Box::new(App {
            hwnd,
            config,
            config_path: path,
            tray: None,
            capture_label: DEFAULT_CAPTURE_HOTKEY.to_string(),
            quit_label: DEFAULT_QUIT_HOTKEY.to_string(),
        });

        ::log::info!("resident window hwnd={hwnd:?}");

        SetWindowLongPtrW(hwnd, GWLP_USERDATA, (&mut *app) as *mut App as isize);

        match Tray::add(hwnd, "window-cap - 截图工具") {
            Ok(tray) => app.tray = Some(tray),
            Err(error) => {
                eprintln!("托盘不可用: {error}");
                ::log::warn!("tray unavailable: {error}");
            }
        }

        if let Some(warning) = &warning {
            app.notify(warning, true);
        }

        if app.apply_hotkeys() {
            let tooltip = format!("window-cap - {} 截图", app.capture_label);
            if let Some(tray) = &app.tray {
                tray.set_tooltip(&tooltip);
            }
            println!("window-cap 已启动");
            println!("  截图: {}", app.capture_label);
            if app.quit_label.is_empty() {
                println!("  退出: 托盘菜单");
            } else {
                println!("  退出: {}", app.quit_label);
            }
            ::log::info!(
                "resident started: capture={} quit={}",
                app.capture_label,
                app.quit_label
            );
        }

        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).0 > 0 {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }

        let _ = UnregisterHotKey(Some(hwnd), HOTKEY_CAPTURE);
        let _ = UnregisterHotKey(Some(hwnd), HOTKEY_QUIT);
        drop(app);
    }

    Ok(())
}

/// 不走热键，直接执行一次截图流程（测试/脚本用）。
pub fn run_once(config_path: Option<PathBuf>, delay_ms: u64) -> Result<()> {
    let path = Config::path(config_path.as_deref());
    let (mut config, warning) = Config::load_or_create(&path);
    if let Some(warning) = warning {
        eprintln!("配置: {warning}");
    }

    if delay_ms > 0 {
        println!("{delay_ms} ms 后截图…");
        std::thread::sleep(Duration::from_millis(delay_ms));
    }

    match capture_flow(None, &config, false)? {
        Some(outcome) => {
            config.capture.set_last_rect(outcome.rect);
            if let Err(error) = config.save(&path) {
                eprintln!("保存上次选区失败: {error}");
            }
            println!("{}", outcome.message());
            Ok(())
        }
        None => {
            println!("已取消");
            Ok(())
        }
    }
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
                allow_pin: false,
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
                allow_pin: false,
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

struct CaptureOutcome {
    width: u32,
    height: u32,
    rect: RectI,
    path: Option<PathBuf>,
    copied: bool,
    pinned: bool,
}

impl CaptureOutcome {
    fn message(&self) -> String {
        if self.pinned {
            return format!(
                "已贴图 {}×{}（拖动移动，Ctrl+滚轮缩放，Esc/双击关闭）",
                self.width, self.height
            );
        }
        match (&self.path, self.copied) {
            (Some(path), true) => format!(
                "已复制 {}×{} 并保存到 {}",
                self.width,
                self.height,
                path.display()
            ),
            (Some(path), false) => format!(
                "已保存 {}×{} 到 {}",
                self.width,
                self.height,
                path.display()
            ),
            (None, true) => format!("已复制 {}×{} 到剪贴板", self.width, self.height),
            (None, false) => format!(
                "截图完成 {}×{}（未保存，请检查配置）",
                self.width, self.height
            ),
        }
    }
}

fn capture_flow(
    scripted: Option<RectI>,
    config: &Config,
    allow_pin: bool,
) -> Result<Option<CaptureOutcome>> {
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
            allow_pin,
        },
    )?;

    match result {
        OverlayResult::Confirmed { image, rect } => {
            let (copied, path) = apply_outputs(&image, config)?;

            ::log::info!(
                "capture ok {}x{} capture_ms={:.1} copied={} saved={}",
                image.width,
                image.height,
                capture_ms,
                copied,
                path.is_some()
            );

            Ok(Some(CaptureOutcome {
                width: image.width,
                height: image.height,
                rect,
                path,
                copied,
                pinned: false,
            }))
        }
        OverlayResult::Pinned { image } => {
            let width = image.width;
            let height = image.height;
            let origin = image.origin;
            ui::pin::show(image)?;
            ::log::info!("pinned capture {width}x{height}");
            Ok(Some(CaptureOutcome {
                width,
                height,
                rect: RectI::new(origin.0, origin.1, width as i32, height as i32),
                path: None,
                copied: false,
                pinned: true,
            }))
        }
        OverlayResult::Cancelled => {
            ::log::info!("capture cancelled");
            Ok(None)
        }
    }
}

fn apply_outputs(
    image: &crate::capture::Frame,
    config: &Config,
) -> Result<(bool, Option<PathBuf>)> {
    let mut copied = false;
    if config.output.copy_to_clipboard {
        clipboard::put(image)?;
        copied = true;
    }

    let mut path = None;
    if config.output.save_file {
        let target = files::path_in(&config.save_dir());
        files::save_png(image, &target)?;
        path = Some(target);
    }

    Ok((copied, path))
}

/// 直接按给定区域截图（不走遮罩），用于“按上次选区截图”。
fn capture_region(rect: RectI, config: &Config) -> Result<CaptureOutcome> {
    let screen = VirtualScreen::current();
    if !screen.contains(rect) {
        return Err(AppError::msg(format!(
            "区域 {rect} 超出虚拟桌面 {}x{} @ ({}, {})",
            screen.width, screen.height, screen.x, screen.y
        )));
    }

    let area = VirtualScreen {
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
    };
    let mut backend = GdiBackend;
    let frame = backend.capture(area)?;
    let (copied, path) = apply_outputs(&frame, config)?;
    ::log::info!(
        "capture region {}x{} at ({}, {}) copied={} saved={}",
        frame.width,
        frame.height,
        rect.x,
        rect.y,
        copied,
        path.is_some()
    );

    Ok(CaptureOutcome {
        width: frame.width,
        height: frame.height,
        rect,
        path,
        copied,
        pinned: false,
    })
}

struct App {
    hwnd: HWND,
    config: Config,
    config_path: PathBuf,
    tray: Option<Tray>,
    capture_label: String,
    quit_label: String,
}

impl App {
    fn notify(&self, message: &str, is_error: bool) {
        if let Some(tray) = &self.tray {
            tray.notify(message, is_error);
        }
        if is_error {
            eprintln!("{message}");
        } else {
            println!("{message}");
        }
    }

    fn apply_hotkeys(&mut self) -> bool {
        let (capture, quit) = unsafe {
            let _ = UnregisterHotKey(Some(self.hwnd), HOTKEY_CAPTURE);
            let _ = UnregisterHotKey(Some(self.hwnd), HOTKEY_QUIT);
            register_hotkeys(self.hwnd, &self.config)
        };

        let Some(capture) = capture else {
            self.notify(
                "截图热键全部被占用（配置热键 / Ctrl+Alt+A / Ctrl+Shift+A / Ctrl+Alt+X）",
                true,
            );
            return false;
        };

        self.capture_label = capture.label();
        match quit {
            Some(quit) => {
                self.quit_label = quit.label();
            }
            None => {
                self.quit_label.clear();
                self.notify("退出热键被占用，可用托盘菜单退出", true);
            }
        }

        if Hotkey::parse(&self.config.hotkey.capture).ok() != Some(capture) {
            self.notify(
                &format!(
                    "配置热键 {} 不可用，已改用 {}",
                    self.config.hotkey.capture, self.capture_label
                ),
                false,
            );
        }

        true
    }

    fn start_capture(&mut self, delayed: bool) {
        if CAPTURING.swap(true, Ordering::SeqCst) {
            return;
        }

        if delayed {
            unsafe {
                SetTimer(Some(self.hwnd), TIMER_DELAY, TRAY_DELAY_MS, None);
            }
            return;
        }

        self.do_capture();
        CAPTURING.store(false, Ordering::SeqCst);
    }

    fn do_capture(&mut self) {
        match capture_flow(None, &self.config, true) {
            Ok(Some(outcome)) => {
                self.remember_region(outcome.rect);
                self.notify(&outcome.message(), false);
            }
            Ok(None) => {
                ::log::info!("capture cancelled by user");
            }
            Err(error) => {
                ::log::error!("capture failed: {error}");
                self.notify(&format!("截图失败: {error}"), true);
            }
        }
    }

    fn remember_region(&mut self, rect: RectI) {
        self.config.capture.set_last_rect(rect);
        if let Err(error) = self.config.save(&self.config_path) {
            ::log::warn!("save last region failed: {error}");
        }
    }

    fn capture_last_region(&mut self) {
        let Some(rect) = self.config.capture.last_rect() else {
            self.notify("还没有上次选区，先截图一次", true);
            return;
        };

        match capture_region(rect, &self.config) {
            Ok(outcome) => {
                self.remember_region(outcome.rect);
                self.notify(&outcome.message(), false);
            }
            Err(error) => {
                ::log::error!("last region capture failed: {error}");
                self.notify(&format!("截图失败: {error}"), true);
            }
        }
    }

    fn on_copy_data(&mut self, lparam: LPARAM) {
        if lparam.0 == 0 {
            return;
        }
        let data = lparam.0 as *const COPYDATASTRUCT;
        let (magic, length, pointer) = unsafe {
            (
                (*data).dwData,
                (*data).cbData as usize,
                (*data).lpData as *const u16,
            )
        };
        if magic != single_instance::FORWARD_MAGIC || length < 2 || pointer.is_null() {
            return;
        }

        let words = length / 2;
        let slice = unsafe { std::slice::from_raw_parts(pointer, words) };
        let text = String::from_utf16_lossy(slice);
        let text = text.trim_end_matches('\0');
        let args: Vec<String> = text.split('\u{1f}').map(str::to_string).collect();
        self.handle_forwarded(args);
    }

    fn handle_forwarded(&mut self, args: Vec<String>) {
        ::log::info!("forwarded command: {args:?}");
        let mut full = vec!["window-cap".to_string()];
        full.extend(args);

        match crate::app::cli::parse_from_args(full) {
            Ok(parsed) => match parsed.command {
                crate::app::cli::Command::Resident { .. } => {
                    self.notify("window-cap 已在运行", false);
                }
                crate::app::cli::Command::Capture(options) => {
                    if CAPTURING.swap(true, Ordering::SeqCst) {
                        self.notify("截图进行中，已忽略转发请求", true);
                        return;
                    }
                    ::log::info!("forwarded capture command");
                    let result = crate::app::capture_command(options);
                    CAPTURING.store(false, Ordering::SeqCst);
                    match result {
                        Ok(()) => self.notify("已执行转发的截图请求", false),
                        Err(error) => {
                            ::log::error!("forwarded capture failed: {error}");
                            self.notify(&format!("转发的截图失败: {error}"), true);
                        }
                    }
                }
                _ => self.notify("只支持转发 --capture 命令", true),
            },
            Err(error) => self.notify(&format!("转发命令解析失败: {error}"), true),
        }
    }

    fn on_tray(&mut self, lparam: LPARAM) {
        let event = ((lparam.0 as usize) & 0xFFFF) as u32;
        match event {
            // V4 通知：左键给 NIN_SELECT / NIN_KEYSELECT，右键给 WM_CONTEXTMENU。
            NIN_SELECT | 1025 | WM_LBUTTONUP | WM_LBUTTONDBLCLK => self.start_capture(false),
            WM_RBUTTONUP | WM_CONTEXTMENU => self.show_menu(),
            _ => {}
        }
    }

    fn show_menu(&mut self) {
        let autostart_enabled = autostart::is_enabled();
        let last_region = self.config.capture.last_rect();
        let command = self
            .tray
            .as_ref()
            .and_then(|tray| tray.show_menu(autostart_enabled, &self.capture_label, last_region));

        let Some(command) = command else {
            return;
        };

        match command {
            TrayCommand::Capture => self.start_capture(false),
            TrayCommand::DelayedCapture => self.start_capture(true),
            TrayCommand::LastRegion => self.capture_last_region(),
            TrayCommand::ReloadConfig => self.reload_config(),
            TrayCommand::ToggleAutostart => self.toggle_autostart(),
            TrayCommand::OpenConfig => self.open_config(),
            TrayCommand::Quit => unsafe {
                let _ = DestroyWindow(self.hwnd);
            },
        }
    }

    fn reload_config(&mut self) {
        let (config, warning) = Config::load_or_create(&self.config_path);
        if let Some(warning) = warning {
            self.notify(&warning, true);
        }
        self.config = config;
        if self.apply_hotkeys() {
            let tooltip = format!("window-cap - {} 截图", self.capture_label);
            if let Some(tray) = &self.tray {
                tray.set_tooltip(&tooltip);
            }
            ::log::info!(
                "config reloaded from {}: capture={} quit={}",
                self.config_path.display(),
                self.capture_label,
                self.quit_label
            );
            self.notify(
                &format!("配置已重新加载，截图热键 {}", self.capture_label),
                false,
            );
        }
    }

    fn toggle_autostart(&mut self) {
        let enabled = autostart::is_enabled();
        match autostart::set_enabled(!enabled) {
            Ok(()) => {
                if enabled {
                    self.notify("已关闭开机自启", false);
                } else {
                    self.notify("已开启开机自启", false);
                }
            }
            Err(error) => self.notify(&format!("设置开机自启失败: {error}"), true),
        }
    }

    fn open_config(&mut self) {
        if let Err(error) = std::process::Command::new("notepad")
            .arg(&self.config_path)
            .spawn()
        {
            self.notify(&format!("打开配置文件失败: {error}"), true);
        }
    }
}

/// 注册截图/退出热键，失败时按候选列表退避。
unsafe fn register_hotkeys(hwnd: HWND, config: &Config) -> (Option<Hotkey>, Option<Hotkey>) {
    unsafe {
        let mut capture = None;
        for text in [
            config.hotkey.capture.as_str(),
            DEFAULT_CAPTURE_HOTKEY,
            "Ctrl+Shift+A",
            "Ctrl+Alt+X",
        ] {
            let Ok(hotkey) = Hotkey::parse(text) else {
                continue;
            };
            if RegisterHotKey(
                Some(hwnd),
                HOTKEY_CAPTURE,
                hotkey.modifiers | MOD_NOREPEAT,
                hotkey.virtual_key,
            )
            .is_ok()
            {
                capture = Some(hotkey);
                break;
            }
        }

        let Some(capture) = capture else {
            return (None, None);
        };

        let same_modifiers = Hotkey {
            modifiers: capture.modifiers,
            virtual_key: VK_Q.0 as u32,
        };

        let mut quit_candidates = vec![
            config.hotkey.quit.clone(),
            same_modifiers.label(),
            DEFAULT_QUIT_HOTKEY.to_string(),
        ];
        quit_candidates.dedup();

        let mut quit = None;
        for text in quit_candidates {
            let Ok(hotkey) = Hotkey::parse(&text) else {
                continue;
            };
            if hotkey == capture {
                continue;
            }
            if RegisterHotKey(
                Some(hwnd),
                HOTKEY_QUIT,
                hotkey.modifiers | MOD_NOREPEAT,
                hotkey.virtual_key,
            )
            .is_ok()
            {
                quit = Some(hotkey);
                break;
            }
        }

        (Some(capture), quit)
    }
}

unsafe fn state(hwnd: HWND) -> Option<&'static mut App> {
    unsafe {
        let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut App;
        if pointer.is_null() {
            None
        } else {
            Some(&mut *pointer)
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
            WM_NCCREATE => {
                let create = lparam.0 as *const CREATESTRUCTW;
                if !create.is_null() {
                    let app = (*create).lpCreateParams as *mut App;
                    if !app.is_null() {
                        SetWindowLongPtrW(hwnd, GWLP_USERDATA, app as isize);
                    }
                }
                DefWindowProcW(hwnd, message, wparam, lparam)
            }
            WM_HOTKEY => {
                if let Some(app) = state(hwnd) {
                    match wparam.0 as i32 {
                        HOTKEY_CAPTURE => app.start_capture(false),
                        HOTKEY_QUIT => {
                            let _ = DestroyWindow(hwnd);
                        }
                        _ => {}
                    }
                }
                LRESULT(0)
            }
            WM_TRAY => {
                if let Some(app) = state(hwnd) {
                    app.on_tray(lparam);
                }
                LRESULT(0)
            }
            WM_APP_ACTIVATE => {
                if let Some(app) = state(hwnd) {
                    app.notify(
                        &format!("window-cap 已在运行（{} 截图）", app.capture_label),
                        false,
                    );
                }
                LRESULT(0)
            }
            WM_TIMER => {
                if wparam.0 == TIMER_DELAY
                    && let Some(app) = state(hwnd)
                {
                    let _ = KillTimer(Some(hwnd), TIMER_DELAY);
                    app.do_capture();
                    CAPTURING.store(false, Ordering::SeqCst);
                }
                LRESULT(0)
            }
            WM_COPYDATA => {
                if let Some(app) = state(hwnd) {
                    app.on_copy_data(lparam);
                }
                LRESULT(1)
            }
            WM_DESTROY => {
                // 先移除托盘图标，再退出消息循环。
                if let Some(app) = state(hwnd) {
                    app.tray = None;
                }
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, message, wparam, lparam),
        }
    }
}

/// 双击启动（控制台只属于本进程）时隐藏控制台；从终端启动时保留。
fn hide_own_console() {
    unsafe {
        let console = GetConsoleWindow();
        if console.0.is_null() {
            return;
        }
        let mut processes = [0u32; 2];
        let count = GetConsoleProcessList(&mut processes);
        if count == 1 {
            let _ = ShowWindow(console, SW_HIDE);
        }
    }
}
