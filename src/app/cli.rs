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
            "--selftest" => set_mode(&mut mode, Mode::Selftest)?,
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

    let command = match mode.unwrap_or(Mode::Help) {
        Mode::Help => Command::Help,
        Mode::Version => Command::Version,
        Mode::Selftest => Command::Selftest,
        Mode::Capture => Command::Capture(capture.unwrap_or_default()),
        Mode::Soak(iterations) => Command::Soak(iterations),
        Mode::Spike(name) => Command::Spike(name),
    };

    Ok(Cli { verbose, command })
}

fn set_mode(slot: &mut Option<Mode>, mode: Mode) -> Result<()> {
    if slot.is_some() {
        return Err(AppError::msg(
            "只能指定一个运行模式：--help / --version / --selftest / --capture / --soak / --spike",
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
  window-cap [选项]

基础:
  -h, --help         显示帮助
  -V, --version      显示版本
  -v, --verbose      输出调试日志

采集 (P1):
  --capture              截取整个虚拟桌面（可与其他采集参数组合）
  --region x,y,w,h       只截取指定区域（虚拟桌面物理像素，可含负坐标）
  --output <路径>        保存为 PNG（自动创建目录，原子写）
  --clipboard            复制到剪贴板（CF_DIB）
  --json                 以 JSON 输出结果

自检:
  --selftest             环境、采集、坐标、剪贴板与落盘自检

压力测试:
  --soak <次数>          连续采集 N 次，检查 GDI/USER 对象是否泄漏

开发:
  --spike <名称>         运行技术验证: capture | focus | ime | gdiplus

阶段状态:
  P0 工程骨架与 Spike（完成）
  P1 采集与输出（当前）
  P2 遮罩与框选
  P3 标注引擎
  P4 系统集成
  P5 兼容性与 Beta
";

pub fn print_help() {
    println!("{HELP}");
}
