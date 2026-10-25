//! 命令行解析（clap）。

use std::path::PathBuf;

use clap::{Parser, ValueEnum};

use crate::capture::RectI;
use crate::util::error::{AppError, Result};

/// 顶层命令行参数。
#[derive(Parser, Debug)]
#[command(
    name = "window-cap",
    version,
    about = "Windows 截图与标注工具（Flameshot 风格）",
    long_about = "用法：\n  window-cap                 常驻：托盘 + 全局热键\n  window-cap --capture ...   无 UI 采集\n  window-cap --selftest      自检\n\n配置：%APPDATA%\\window-cap\\config.toml"
)]
pub struct Cli {
    /// 常驻模式：全局热键 + 托盘（默认）
    #[arg(long)]
    pub hotkey: bool,

    /// 立即执行一次截图流程后退出（测试用）
    #[arg(long)]
    pub once: bool,

    /// 环境、采集、坐标、剪贴板与落盘自检
    #[arg(long)]
    pub selftest: bool,

    /// 无 UI 采集整个虚拟桌面
    #[arg(long)]
    pub capture: bool,

    /// 配合 --capture：x,y,w,h
    #[arg(long, value_parser = parse_region)]
    pub region: Option<RectI>,

    /// 配合 --capture：保存为 PNG
    #[arg(long)]
    pub output: Option<PathBuf>,

    /// 配合 --capture：复制到剪贴板（CF_DIB）
    #[arg(long)]
    pub clipboard: bool,

    /// 配合 --capture：使用上次选区（无需遮罩）
    #[arg(long)]
    pub last: bool,

    /// 配合 --capture：以 JSON 输出结果
    #[arg(long)]
    pub json: bool,

    /// 截图前延迟毫秒数（--once / --capture）
    #[arg(long, default_value_t = 0)]
    pub delay: u64,

    /// 配置文件路径（默认 %APPDATA%\window-cap\config.toml）
    #[arg(long)]
    pub config: Option<PathBuf>,

    /// 连续采集 N 次，检查句柄/内存泄漏
    #[arg(long)]
    pub soak: Option<u32>,

    /// 技术验证：capture | focus | ime | gdiplus
    #[arg(long)]
    pub spike: Option<String>,

    /// 遮罩窗口压力测试次数（脚本化选区）
    #[arg(long)]
    pub overlay_selftest: Option<u32>,

    /// 开机自启：on | off | status
    #[arg(long)]
    pub autostart: Option<AutoStartAction>,

    /// 输出调试日志
    #[arg(long, short)]
    pub verbose: bool,
}

/// 开机自启动作。
#[derive(Clone, Copy, Debug, ValueEnum)]
pub enum AutoStartAction {
    On,
    Off,
    Status,
}

/// 解析后的参数。
pub struct Parsed {
    pub verbose: bool,
    pub command: Command,
}

/// 本次运行的命令。
pub enum Command {
    Resident {
        config: Option<PathBuf>,
    },
    Once {
        config: Option<PathBuf>,
        delay_ms: u64,
    },
    Capture(CaptureOptions),
    Selftest,
    Soak(u32),
    Spike(String),
    OverlaySelftest(u32),
    Autostart(AutoStartAction),
}

/// 无 UI 采集选项。
#[derive(Default, Clone)]
pub struct CaptureOptions {
    pub region: Option<RectI>,
    pub output: Option<PathBuf>,
    pub clipboard: bool,
    pub json: bool,
    pub delay_ms: u64,
    pub last: bool,
    pub config: Option<PathBuf>,
}

/// 解析命令行；`--help` / `--version` 由 clap 直接打印并退出。
pub fn parse() -> Parsed {
    let cli = match Cli::try_parse_from(std::env::args_os()) {
        Ok(cli) => cli,
        Err(error) => {
            let code = if error.use_stderr() { 2 } else { 0 };
            print!("{error}");
            std::process::exit(code);
        }
    };

    let verbose = cli.verbose;
    let command = cli.into_command().unwrap_or_else(|error| {
        eprintln!("参数错误: {error}");
        std::process::exit(2);
    });
    Parsed { verbose, command }
}

/// 从指定参数解析（第二实例转发时复用）。
pub fn parse_from_args<I, T>(args: I) -> std::result::Result<Parsed, String>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    let cli = Cli::try_parse_from(args).map_err(|error| error.to_string())?;
    let verbose = cli.verbose;
    let command = cli.into_command().map_err(|error| error.to_string())?;
    Ok(Parsed { verbose, command })
}

impl Cli {
    fn into_command(self) -> Result<Command> {
        // 步骤 1：校验模式与采集选项组合。
        let capture_options_present = self.region.is_some()
            || self.output.is_some()
            || self.clipboard
            || self.json
            || self.last;

        let modes = [
            self.hotkey,
            self.once,
            self.selftest,
            self.capture,
            self.soak.is_some(),
            self.spike.is_some(),
            self.overlay_selftest.is_some(),
            self.autostart.is_some(),
        ]
        .iter()
        .filter(|present| **present)
        .count();

        if modes > 1 {
            return Err(AppError::msg(
                "只能指定一个运行模式：--hotkey / --once / --selftest / --capture / --soak / --spike / --overlay-selftest / --autostart",
            ));
        }

        if capture_options_present && !self.capture && modes > 0 {
            return Err(AppError::msg(
                "--region / --output / --clipboard / --json 只能与 --capture 一起使用",
            ));
        }

        if self.last && self.region.is_some() {
            return Err(AppError::msg("--last 与 --region 不能同时使用"));
        }

        // 步骤 2：按优先级选择命令。
        if let Some(action) = self.autostart {
            return Ok(Command::Autostart(action));
        }
        if let Some(iterations) = self.soak {
            if iterations == 0 {
                return Err(AppError::msg("--soak 次数必须大于 0"));
            }
            return Ok(Command::Soak(iterations));
        }
        if let Some(name) = self.spike {
            return Ok(Command::Spike(name));
        }
        if let Some(iterations) = self.overlay_selftest {
            if iterations == 0 {
                return Err(AppError::msg("--overlay-selftest 次数必须大于 0"));
            }
            return Ok(Command::OverlaySelftest(iterations));
        }
        if self.selftest {
            return Ok(Command::Selftest);
        }
        if self.once {
            return Ok(Command::Once {
                config: self.config,
                delay_ms: self.delay,
            });
        }
        if self.capture || capture_options_present {
            return Ok(Command::Capture(CaptureOptions {
                region: self.region,
                output: self.output,
                clipboard: self.clipboard,
                json: self.json,
                delay_ms: self.delay,
                last: self.last,
                config: self.config,
            }));
        }
        if self.delay > 0 {
            return Err(AppError::msg("--delay 需要配合 --once 或 --capture 使用"));
        }

        Ok(Command::Resident {
            config: self.config,
        })
    }
}

fn parse_region(value: &str) -> std::result::Result<RectI, String> {
    RectI::parse(value).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(args: &[&str]) -> Result<Command> {
        Cli::try_parse_from(std::iter::once("window-cap").chain(args.iter().copied()))
            .map_err(|error| AppError::msg(error.to_string()))?
            .into_command()
    }

    #[test]
    fn parses_modes() {
        assert!(matches!(parsed(&[]), Ok(Command::Resident { .. })));
        assert!(matches!(parsed(&["--once"]), Ok(Command::Once { .. })));
        assert!(matches!(
            parsed(&["--capture", "--region", "10,20,30,40"]),
            Ok(Command::Capture(_))
        ));
        assert!(matches!(
            parsed(&["--autostart", "status"]),
            Ok(Command::Autostart(AutoStartAction::Status))
        ));
        assert!(parsed(&["--once", "--selftest"]).is_err());
        assert!(parsed(&["--region", "1,2,3,4"]).is_ok());
        assert!(parsed(&["--capture", "--last"]).is_ok());
        assert!(parsed(&["--last", "--region", "1,2,3,4"]).is_err());
    }
}
