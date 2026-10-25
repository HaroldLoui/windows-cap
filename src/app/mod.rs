//! 应用编排：命令分发、无 UI 采集、自检与泄漏测试。

pub mod autostart;
pub mod cli;
pub mod config;
pub mod hotkey;
pub mod single_instance;
pub mod tray;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::capture::{self, CaptureBackend, GdiBackend, RectI, VirtualScreen};
use crate::output::{clipboard, files};
use crate::platform::test_pattern::TestPattern;
use crate::spike;
use crate::util::error::{AppError, Result};
use crate::util::resources::process_resources;

/// 解析命令行并执行对应子命令。
pub fn run() -> Result<()> {
    crate::util::log::init()?;
    let parsed = cli::parse();
    if parsed.verbose {
        log::set_max_level(log::LevelFilter::Debug);
    }

    match parsed.command {
        cli::Command::Resident { config } => hotkey::run_resident(config),
        cli::Command::Once { config, delay_ms } => hotkey::run_once(config, delay_ms),
        cli::Command::Capture(options) => capture_command(options),
        cli::Command::Selftest => selftest(),
        cli::Command::Soak(iterations) => soak(iterations),
        cli::Command::Spike(name) => spike::run(&name),
        cli::Command::OverlaySelftest(iterations) => hotkey::run_overlay_selftest(iterations),
        cli::Command::Autostart(action) => autostart::run(action),
    }
}

/// 无 UI 采集一次并按选项输出。
pub(crate) fn capture_command(options: cli::CaptureOptions) -> Result<()> {
    // 步骤 1：按需延迟，并确定采集区域。
    if options.delay_ms > 0 {
        log::info!("{} ms 后采集…", options.delay_ms);
        std::thread::sleep(Duration::from_millis(options.delay_ms));
    }

    let screen = VirtualScreen::current();
    let rect = if options.last {
        let path = config::Config::path(options.config.as_deref());
        let (config, warning) = config::Config::load_or_create(&path);
        if let Some(warning) = warning {
            log::warn!("配置: {warning}");
        }
        config
            .capture
            .last_rect()
            .ok_or_else(|| AppError::msg("还没有上次选区，请先执行一次截图"))?
    } else {
        options
            .region
            .unwrap_or_else(|| RectI::new(screen.x, screen.y, screen.width, screen.height))
    };

    // 步骤 2：校验区域后采集一帧。
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
    let started = Instant::now();
    let frame = backend.capture(area)?;
    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;

    // 步骤 3：保存、复制并按格式输出结果。
    let mut output_path: Option<PathBuf> = None;
    if let Some(path) = &options.output {
        files::save_png(&frame, path)?;
        output_path = Some(path.clone());
    }

    let clipboard_ok = if options.clipboard {
        clipboard::put(&frame)?;
        true
    } else {
        false
    };

    if options.json {
        let output_json = match &output_path {
            Some(path) => format!("\"{}\"", json_escape(&path.display().to_string())),
            None => "null".to_string(),
        };
        log::info!(
            "{{\"width\":{},\"height\":{},\"x\":{},\"y\":{},\"backend\":\"{}\",\"elapsed_ms\":{:.2},\"output\":{},\"clipboard\":{}}}",
            frame.width,
            frame.height,
            frame.origin.0,
            frame.origin.1,
            backend.name(),
            elapsed_ms,
            output_json,
            clipboard_ok
        );
    } else {
        log::info!(
            "截取: {}x{} @ ({}, {}) backend={}",
            frame.width,
            frame.height,
            frame.origin.0,
            frame.origin.1,
            backend.name()
        );
        log::info!("耗时: {:.1} ms", elapsed_ms);
        if let Some(path) = &output_path {
            log::info!("已保存: {}", path.display());
        }
        if clipboard_ok {
            log::info!("已复制到剪贴板 (CF_DIB, {} 字节)", 40 + frame.bgra.len());
        }
        if output_path.is_none() && !clipboard_ok {
            log::info!("提示: 未指定输出，可加 --output 或 --clipboard");
        }
    }

    // 步骤 4：记录本次选区，供 --last 使用。
    record_last_region(&options, rect);

    Ok(())
}

fn record_last_region(options: &cli::CaptureOptions, rect: RectI) {
    let path = config::Config::path(options.config.as_deref());
    let (mut config, _) = config::Config::load_or_create(&path);
    config.capture.set_last_rect(rect);
    if let Err(error) = config.save(&path) {
        ::log::warn!("save last region failed: {error}");
    }
}

fn json_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn soak(iterations: u32) -> Result<()> {
    let screen = VirtualScreen::current();
    if screen.width <= 0 || screen.height <= 0 {
        return Err(AppError::msg("虚拟桌面尺寸无效"));
    }

    let mut backend = GdiBackend;

    // 预热到分配器稳态（每帧约 14.7MB），避免把保留内存误判为泄漏。
    let warmup = 10.min(iterations);
    for _ in 0..warmup {
        backend.capture(screen)?;
    }

    let (gdi_before, user_before, memory_before) = process_resources()?;
    log::info!(
        "soak 开始: {iterations} 次（已预热 {warmup} 次），初始 GDI={gdi_before} USER={user_before} 内存={:.1} MB",
        memory_before as f64 / 1024.0 / 1024.0
    );

    for index in 0..iterations {
        backend.capture(screen)?;
        if index % 10 == 0 || index + 1 == iterations {
            log::info!("  进度: {}/{}", index + 1, iterations);
        }
    }

    let (gdi_after, user_after, memory_after) = process_resources()?;
    let gdi_delta = gdi_after as i64 - gdi_before as i64;
    let user_delta = user_after as i64 - user_before as i64;
    let memory_delta = memory_after as i64 - memory_before as i64;

    log::info!(
        "soak 结束: GDI={gdi_after} ({gdi_delta:+}) USER={user_after} ({user_delta:+}) 内存={:.1} MB ({:+.1} MB)",
        memory_after as f64 / 1024.0 / 1024.0,
        memory_delta as f64 / 1024.0 / 1024.0
    );

    if gdi_delta > 2 || user_delta > 2 {
        return Err(AppError::msg(format!(
            "句柄泄漏: GDI {gdi_delta:+}, USER {user_delta:+}（阈值 ±2）"
        )));
    }

    if memory_delta > 32 * 1024 * 1024 {
        return Err(AppError::msg(format!(
            "内存持续增长: {:+.1} MB（阈值 32MB）",
            memory_delta as f64 / 1024.0 / 1024.0
        )));
    }

    Ok(())
}

fn selftest() -> Result<()> {
    let version = env!("CARGO_PKG_VERSION");
    let mut failures: Vec<String> = Vec::new();

    log::info!("window-cap {version} selftest");

    // 步骤 1：检查虚拟桌面与显示器。
    let screen = VirtualScreen::current();
    log::info!(
        "[1/6] 虚拟桌面: {}x{} @ ({}, {})",
        screen.width,
        screen.height,
        screen.x,
        screen.y
    );
    let monitors = capture::monitors();
    log::info!("      显示器数量: {}", monitors.len());
    for (index, monitor) in monitors.iter().enumerate() {
        log::info!(
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
    if screen.width <= 0 || screen.height <= 0 {
        return Err(AppError::msg("虚拟桌面尺寸无效"));
    }

    // 步骤 2：整屏采集并校验尺寸。
    let mut backend = GdiBackend;
    let started = Instant::now();
    let full = backend.capture(screen)?;
    let elapsed_ms = started.elapsed().as_secs_f64() * 1000.0;
    let ratio = full.non_black_ratio();
    log::info!(
        "[2/6] 整屏采集: {}x{}，{:.1} ms，非黑像素 {:.1}%",
        full.width,
        full.height,
        elapsed_ms,
        ratio * 100.0
    );
    if full.width != screen.width as u32 || full.height != screen.height as u32 {
        failures.push(format!(
            "整屏尺寸不符: 期望 {}x{} 实际 {}x{}",
            screen.width, screen.height, full.width, full.height
        ));
    }
    if ratio < 0.01 {
        log::info!("      警告: 画面几乎全黑（锁屏/无桌面/采集受限），跳过视觉校验");
    }
    files::save_png(&full, Path::new("target/selftest/full.png"))?;

    // 步骤 3：用测试图案校验四角色块。
    let mut probe_frame: Option<FrameSource> = None;
    if ratio >= 0.01 {
        match TestPattern::show() {
            Ok(pattern) => {
                let rect = pattern.rect();
                log::info!("[3/6] 测试图案窗口: {rect}");

                let with_pattern = backend.capture(screen)?;
                for (name, expected, x_step, y_step) in [
                    ("左上", (0u8, 0u8, 255u8), 1, 1),
                    ("右上", (0, 255, 0), 3, 1),
                    ("左下", (255, 0, 0), 1, 3),
                    ("右下", (255, 255, 255), 3, 3),
                ] {
                    let x = rect.x + rect.width * x_step / 4;
                    let y = rect.y + rect.height * y_step / 4;
                    match with_pattern.pixel(x, y) {
                        Some(actual) if actual == expected => {}
                        Some(actual) => failures.push(format!(
                            "{name} 色块颜色不符: 期望 {expected:?} 实际 {actual:?}"
                        )),
                        None => failures.push(format!("{name} 色块坐标 ({x}, {y}) 不在画面内")),
                    }
                }

                // 步骤 4：区域截图应与整屏裁剪逐像素一致。
                let region_frame = backend.capture(VirtualScreen {
                    x: rect.x,
                    y: rect.y,
                    width: rect.width,
                    height: rect.height,
                })?;
                let cropped = with_pattern.crop(rect)?;
                let diff = region_frame.pixel_diff_ratio(&cropped);
                log::info!("[4/6] 区域截图 vs 整屏裁剪: 差异像素 {:.4}%", diff * 100.0);
                if diff > 0.001 {
                    failures.push(format!("区域截图与整屏裁剪不一致: diff={diff:.6}"));
                }

                files::save_png(&region_frame, Path::new("target/selftest/region.png"))?;
                probe_frame = Some(FrameSource {
                    frame: region_frame,
                    expected_size: 40 + rect.width as usize * rect.height as usize * 4,
                });

                pattern.close();
            }
            Err(error) => {
                failures.push(format!("测试图案窗口创建失败: {error}"));
            }
        }
    }

    // 步骤 5：校验剪贴板 CF_DIB。
    match &probe_frame {
        Some(probe) => {
            clipboard::put(&probe.frame)?;
            match clipboard::probe_dib()? {
                Some(size)
                    if size >= probe.expected_size && size <= probe.expected_size + 65536 =>
                {
                    log::info!(
                        "[5/6] 剪贴板 CF_DIB: {size} 字节（期望 ≥ {}）",
                        probe.expected_size
                    );
                }
                Some(size) => failures.push(format!(
                    "剪贴板 CF_DIB 大小异常: 期望约 {} 实际 {size}",
                    probe.expected_size
                )),
                None => failures.push("剪贴板中没有 CF_DIB".to_string()),
            }
        }
        None => {
            clipboard::put(&full)?;
            match clipboard::probe_dib()? {
                Some(size) => log::info!("[5/6] 剪贴板 CF_DIB: {size} 字节（整屏）"),
                None => failures.push("剪贴板中没有 CF_DIB".to_string()),
            }
        }
    }

    // 步骤 6：校验 Unicode 路径落盘。
    let unicode_path = Path::new("target/selftest/中文路径 测试.png");
    files::save_png(&full, unicode_path)?;
    if unicode_path.exists() {
        log::info!("[6/6] Unicode 路径落盘: {}", unicode_path.display());
    } else {
        failures.push("Unicode 路径落盘失败".to_string());
    }

    // 步骤 7：汇总结果。
    if failures.is_empty() {
        log::info!("自检通过");
        ::log::info!("selftest ok");
        Ok(())
    } else {
        for failure in &failures {
            log::error!("FAIL: {failure}");
        }
        ::log::error!("selftest failed: {} 项", failures.len());
        Err(AppError::msg(format!("自检失败: {} 项", failures.len())))
    }
}

struct FrameSource {
    frame: capture::Frame,
    expected_size: usize,
}
