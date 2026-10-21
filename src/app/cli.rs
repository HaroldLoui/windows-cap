use std::path::PathBuf;

use crate::capture::RectI;
use crate::util::error::{AppError, Result};

pub struct Cli {
    pub verbose: bool,
    pub command: Command,
}

pub enum Command {
    Help,
    Version,
    Selftest,
    Capture(CaptureOptions),
    Soak(u32),
    Spike(String),
    Resident,
    Once,
    OverlaySelftest(u32),
}

#[derive(Default, Clone)]
pub struct CaptureOptions {
    pub region: Option<RectI>,
    pub output: Option<PathBuf>,
    pub clipboard: bool,
    pub json: bool,
}

enum Mode {
    Help,
    Version,
    Selftest,
    Capture,
    Soak(u32),
    Spike(String),
    Resident,
    Once,
    OverlaySelftest(u32),
}

pub fn parse() -> Result<Cli> {
    let mut verbose = false;
    let mut mode: Option<Mode> = None;
    let mut capture: Option<CaptureOptions> = None;
    let mut args = std::env::args().skip(1);

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => set_mode(&mut mode, Mode::Help)?,
            "-V" | "--version" => set_mode(&mut mode, Mode::Version)?,
            "-v" | "--verbose" => verbose = true,
            "--hotkey" => set_mode(&mut mode, Mode::Resident)?,
            "--once" => set_mode(&mut mode, Mode::Once)?,
            "--selftest" => set_mode(&mut mode, Mode::Selftest)?,
            "--overlay-selftest" => {
                let value = args.next().ok_or_else(|| {
                    AppError::msg("--overlay-selftest 需要次数，例如 --overlay-selftest 50")
                })?;
                let iterations: u32 = value.parse().map_err(|_| {
                    AppError::msg(format!("--overlay-selftest 次数无效: {value:?}"))
                })?;
                if iterations == 0 {
                    return Err(AppError::msg("--overlay-selftest 次数必须大于 0"));
                }
                set_mode(&mut mode, Mode::OverlaySelftest(iterations))?;
            }
            "--capture" => {
                set_mode(&mut mode, Mode::Capture)?;
                capture.get_or_insert_with(CaptureOptions::default);
            }
            "--soak" => {
                let value = args
                    .next()
                    .ok_or_else(|| AppError::msg("--soak 需要次数，例如 --soak 100"))?;
                let iterations: u32 = value
                    .parse()
                    .map_err(|_| AppError::msg(format!("--soak 次数无效: {value:?}")))?;
                if iterations == 0 {
                    return Err(AppError::msg("--soak 次数必须大于 0"));
                }
                set_mode(&mut mode, Mode::Soak(iterations))?;
            }
            "--spike" => {
                let name = args.next().ok_or_else(|| {
                    AppError::msg("--spike 需要参数: capture | focus | ime | gdiplus")
                })?;
                set_mode(&mut mode, Mode::Spike(name))?;
            }
            "--region" => {
                let value = args
                    .next()
                    .ok_or_else(|| AppError::msg("--region 需要 x,y,w,h"))?;
                let region = RectI::parse(&value)?;
                capture_options(&mut mode, &mut capture)?.region = Some(region);
            }
            "--output" => {
                let value = args
                    .next()
                    .ok_or_else(|| AppError::msg("--output 需要文件路径"))?;
                capture_options(&mut mode, &mut capture)?.output = Some(PathBuf::from(value));
            }
            "--clipboard" => {
                capture_options(&mut mode, &mut capture)?.clipboard = true;
            }
            "--json" => {
                capture_options(&mut mode, &mut capture)?.json = true;
            }
            other => {
                return Err(AppError::msg(format!("未知参数: {other}\n\n{HELP}")));
            }
        }
    }

    let command = match mode.unwrap_or(Mode::Resident) {
        Mode::Help => Command::Help,
        Mode::Version => Command::Version,
        Mode::Selftest => Command::Selftest,
        Mode::Capture => Command::Capture(capture.unwrap_or_default()),
        Mode::Soak(iterations) => Command::Soak(iterations),
        Mode::Spike(name) => Command::Spike(name),
        Mode::Resident => Command::Resident,
        Mode::Once => Command::Once,
        Mode::OverlaySelftest(iterations) => Command::OverlaySelftest(iterations),
    };

    Ok(Cli { verbose, command })
}

fn set_mode(slot: &mut Option<Mode>, mode: Mode) -> Result<()> {
    if slot.is_some() {
        return Err(AppError::msg(
            "只能指定一个运行模式：--help / --version / --hotkey / --once / --selftest / --capture / --soak / --spike / --overlay-selftest",
        ));
    }
    *slot = Some(mode);
    Ok(())
}

fn capture_options<'a>(
    mode: &mut Option<Mode>,
    capture: &'a mut Option<CaptureOptions>,
) -> Result<&'a mut CaptureOptions> {
    if mode.is_none() {
        *mode = Some(Mode::Capture);
    }
    if !matches!(mode, Some(Mode::Capture)) {
        return Err(AppError::msg(
            "--region / --output / --clipboard / --json 只能与 --capture 一起使用",
        ));
    }
    Ok(capture.get_or_insert_with(CaptureOptions::default))
}

pub const HELP: &str = "\
window-cap - Windows 截图与标注工具

用法:
  window-cap                 常驻运行：Ctrl+Alt+A 截图，Ctrl+Alt+Q 退出
  window-cap [选项]

运行模式:
  --hotkey                   常驻模式（默认）
  --once                     立即执行一次截图流程后退出（测试用）
  --selftest                 环境、采集、坐标、剪贴板与落盘自检
  --capture                  无 UI 采集整个虚拟桌面
  --region x,y,w,h           配合 --capture：只截取指定区域
  --output <路径>            配合 --capture：保存为 PNG
  --clipboard                配合 --capture：复制到剪贴板（CF_DIB）
  --json                     配合 --capture：以 JSON 输出结果
  --soak <次数>              连续采集 N 次，检查句柄/内存泄漏

开发:
  --overlay-selftest <次数>  遮罩窗口压力测试（脚本化选区）
  --spike <名称>             技术验证: capture | focus | ime | gdiplus

其它:
  -h, --help                 显示帮助
  -V, --version              显示版本
  -v, --verbose              输出调试日志

阶段状态:
  P0 工程骨架与 Spike（完成）
  P1 采集与输出（完成）
  P2 遮罩与框选（当前）
  P3 标注引擎
  P4 系统集成
  P5 兼容性与 Beta
";

pub fn print_help() {
    println!("{HELP}");
}
