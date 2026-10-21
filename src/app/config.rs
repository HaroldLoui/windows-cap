//! 配置文件：读取、校验、默认值、热键解析。

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_SHIFT, MOD_WIN,
};

use crate::capture::RectI;
use crate::util::error::{AppError, Result};

pub const DEFAULT_CAPTURE_HOTKEY: &str = "Ctrl+Alt+A";
pub const DEFAULT_QUIT_HOTKEY: &str = "Ctrl+Alt+Q";

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub hotkey: HotkeyConfig,
    pub capture: CaptureConfig,
    pub output: OutputConfig,
    pub ui: UiConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HotkeyConfig {
    pub capture: String,
    pub quit: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct CaptureConfig {
    /// 截图前等待毫秒数（托盘“延迟截图”和 CLI `--delay` 使用）。
    pub delay_ms: u64,
    /// 上次确认的选区（屏幕坐标 x, y, w, h），用于“按上次选区截图”。
    pub last_region: Option<[i32; 4]>,
}

impl CaptureConfig {
    pub fn last_rect(&self) -> Option<RectI> {
        let [x, y, width, height] = self.last_region?;
        let rect = RectI::new(x, y, width, height);
        if rect.is_empty() { None } else { Some(rect) }
    }

    pub fn set_last_rect(&mut self, rect: RectI) {
        self.last_region = Some([rect.x, rect.y, rect.width, rect.height]);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct OutputConfig {
    /// 保存目录；支持 `%USERPROFILE%` 前缀，相对路径按用户目录展开。
    pub save_dir: String,
    pub copy_to_clipboard: bool,
    pub save_file: bool,
    pub format: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UiConfig {
    pub language: String,
}

impl Default for HotkeyConfig {
    fn default() -> Self {
        Self {
            capture: DEFAULT_CAPTURE_HOTKEY.to_string(),
            quit: DEFAULT_QUIT_HOTKEY.to_string(),
        }
    }
}

impl Default for OutputConfig {
    fn default() -> Self {
        Self {
            save_dir: "Pictures/Screenshots".to_string(),
            copy_to_clipboard: true,
            save_file: true,
            format: "png".to_string(),
        }
    }
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            language: "zh-CN".to_string(),
        }
    }
}

impl Config {
    /// 配置路径：显式指定，或 `%APPDATA%\window-cap\config.toml`。
    pub fn path(explicit: Option<&Path>) -> PathBuf {
        if let Some(path) = explicit {
            return path.to_path_buf();
        }
        std::env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."))
            .join("window-cap")
            .join("config.toml")
    }

    /// 读取配置；文件不存在时创建默认文件，解析失败时备份并回退默认。
    pub fn load_or_create(path: &Path) -> (Self, Option<String>) {
        match fs::read_to_string(path) {
            Ok(text) => match toml::from_str::<Config>(&text) {
                Ok(config) => (config.sanitize(), None),
                Err(error) => {
                    let backup = path.with_extension("toml.bak");
                    let _ = fs::rename(path, &backup);
                    let config = Config::default();
                    let _ = config.save(path);
                    (
                        config,
                        Some(format!(
                            "配置解析失败（{error}），已回退默认值，原文件备份为 {}",
                            backup.display()
                        )),
                    )
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let config = Config::default();
                if let Err(error) = config.save(path) {
                    return (config, Some(format!("创建默认配置失败: {error}")));
                }
                (config, None)
            }
            Err(error) => (
                Config::default(),
                Some(format!("读取配置失败（{error}），使用默认值")),
            ),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent)?;
        }
        let text = toml::to_string_pretty(self)
            .map_err(|error| AppError::msg(format!("序列化配置失败: {error}")))?;
        let temporary = path.with_extension("toml.tmp");
        fs::write(&temporary, text)?;
        fs::rename(&temporary, path)?;
        Ok(())
    }

    pub fn sanitize(mut self) -> Self {
        if !self.output.format.eq_ignore_ascii_case("png") {
            self.output.format = "png".to_string();
        }
        self.capture.delay_ms = self.capture.delay_ms.min(60_000);
        self
    }

    /// 展开后的保存目录。
    pub fn save_dir(&self) -> PathBuf {
        resolve_dir(&self.output.save_dir)
    }
}

fn resolve_dir(value: &str) -> PathBuf {
    let home = std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));

    let expanded = if let Some(rest) = value.strip_prefix("%USERPROFILE%") {
        home.join(rest.trim_start_matches(['\\', '/']))
    } else {
        PathBuf::from(value)
    };

    if expanded.is_absolute() {
        expanded
    } else {
        home.join(expanded)
    }
}

/// 一个解析后的全局热键。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hotkey {
    pub modifiers: HOT_KEY_MODIFIERS,
    pub virtual_key: u32,
}

impl Hotkey {
    /// 解析 `Ctrl+Alt+A` / `Shift+F1` 之类写法。
    pub fn parse(text: &str) -> Result<Self> {
        let mut modifiers = HOT_KEY_MODIFIERS(0);
        let mut key = None;

        for token in text
            .split('+')
            .map(|token| token.trim().to_ascii_lowercase())
        {
            match token.as_str() {
                "ctrl" | "control" => modifiers |= MOD_CONTROL,
                "alt" => modifiers |= MOD_ALT,
                "shift" => modifiers |= MOD_SHIFT,
                "win" | "windows" | "super" => modifiers |= MOD_WIN,
                "" => {
                    return Err(AppError::msg(format!("热键格式错误: {text}")));
                }
                token => {
                    if key.is_some() {
                        return Err(AppError::msg(format!("热键只能有一个主键: {text}")));
                    }
                    key = Some(parse_key(token)?);
                }
            }
        }

        let virtual_key = key.ok_or_else(|| AppError::msg(format!("热键缺少主键: {text}")))?;
        Ok(Self {
            modifiers,
            virtual_key,
        })
    }

    pub fn label(&self) -> String {
        let mut parts = Vec::new();
        if self.modifiers.0 & MOD_CONTROL.0 != 0 {
            parts.push("Ctrl".to_string());
        }
        if self.modifiers.0 & MOD_ALT.0 != 0 {
            parts.push("Alt".to_string());
        }
        if self.modifiers.0 & MOD_SHIFT.0 != 0 {
            parts.push("Shift".to_string());
        }
        if self.modifiers.0 & MOD_WIN.0 != 0 {
            parts.push("Win".to_string());
        }
        parts.push(key_label(self.virtual_key));
        parts.join("+")
    }
}

fn parse_key(token: &str) -> Result<u32> {
    let upper = token.to_ascii_uppercase();
    if upper.len() == 1 {
        let byte = upper.as_bytes()[0];
        if byte.is_ascii_uppercase() || byte.is_ascii_digit() {
            return Ok(byte as u32);
        }
    }

    if let Some(number) = upper.strip_prefix('F')
        && let Ok(index) = number.parse::<u32>()
        && (1..=24).contains(&index)
    {
        return Ok(0x70 + index - 1);
    }

    match upper.as_str() {
        "PRINTSCREEN" | "PRTSC" => Ok(0x2C),
        "SPACE" => Ok(0x20),
        "TAB" => Ok(0x09),
        "ESC" | "ESCAPE" => Ok(0x1B),
        _ => Err(AppError::msg(format!("无法识别的按键: {token}"))),
    }
}

fn key_label(virtual_key: u32) -> String {
    if (b'A' as u32..=b'Z' as u32).contains(&virtual_key) {
        return char::from_u32(virtual_key).unwrap_or('?').to_string();
    }
    if (b'0' as u32..=b'9' as u32).contains(&virtual_key) {
        return char::from_u32(virtual_key).unwrap_or('?').to_string();
    }
    if (0x70..=0x87).contains(&virtual_key) {
        return format!("F{}", virtual_key - 0x70 + 1);
    }
    match virtual_key {
        0x2C => "PrintScreen".to_string(),
        0x20 => "Space".to_string(),
        0x09 => "Tab".to_string(),
        0x1B => "Esc".to_string(),
        other => format!("0x{other:02X}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_hotkeys() {
        let hotkey = Hotkey::parse("Ctrl+Alt+A").expect("解析失败");
        assert_eq!(hotkey.virtual_key, b'A' as u32);
        assert_eq!(hotkey.label(), "Ctrl+Alt+A");

        let hotkey = Hotkey::parse("shift + f12").expect("解析失败");
        assert_eq!(hotkey.virtual_key, 0x7B);
        assert_eq!(hotkey.label(), "Shift+F12");

        assert!(Hotkey::parse("Ctrl+Alt+").is_err());
        assert!(Hotkey::parse("Ctrl+Nope").is_err());
        assert!(Hotkey::parse("Ctrl+A+B").is_err());
    }

    #[test]
    fn config_roundtrip_and_defaults() {
        let directory = std::env::temp_dir().join(format!(
            "window-cap-config-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|duration| duration.as_millis())
                .unwrap_or(0)
        ));
        let path = directory.join("config.toml");

        let (config, warning) = Config::load_or_create(&path);
        assert!(warning.is_none());
        assert!(path.exists());
        assert_eq!(config.hotkey.capture, DEFAULT_CAPTURE_HOTKEY);

        fs::write(&path, "not = [valid").expect("写坏配置失败");
        let (config, warning) = Config::load_or_create(&path);
        assert!(warning.is_some());
        assert!(path.with_extension("toml.bak").exists());
        assert_eq!(config.output.save_dir, "Pictures/Screenshots");

        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn expands_save_directory() {
        let config = Config::default();
        let directory = config.save_dir();
        assert!(directory.is_absolute());
        assert!(
            directory.ends_with("Pictures/Screenshots")
                || directory.ends_with("Pictures\\Screenshots")
        );
    }
}
