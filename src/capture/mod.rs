//! 屏幕采集：虚拟桌面、显示器枚举、GDI 后端。
//!
//! 坐标约定：全链路使用**物理像素**；`Frame.origin` 是画面左上角在虚拟桌面中的坐标，
//! 因此 `Frame` 内部像素坐标 = 屏幕坐标 - origin。

pub mod gdi;

use std::mem::size_of;

use windows::Win32::Foundation::{LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFOEXW,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{
    GetSystemMetrics, MONITORINFOF_PRIMARY, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN,
    SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
};

use crate::util::error::{AppError, Result};

/// 虚拟桌面（所有显示器拼起来的矩形，物理像素）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct VirtualScreen {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl VirtualScreen {
    pub fn current() -> Self {
        unsafe {
            Self {
                x: GetSystemMetrics(SM_XVIRTUALSCREEN),
                y: GetSystemMetrics(SM_YVIRTUALSCREEN),
                width: GetSystemMetrics(SM_CXVIRTUALSCREEN),
                height: GetSystemMetrics(SM_CYVIRTUALSCREEN),
            }
        }
    }

    pub fn right(&self) -> i32 {
        self.x + self.width
    }

    pub fn bottom(&self) -> i32 {
        self.y + self.height
    }

    pub fn contains(&self, rect: RectI) -> bool {
        rect.width > 0
            && rect.height > 0
            && rect.x >= self.x
            && rect.y >= self.y
            && rect.x + rect.width <= self.right()
            && rect.y + rect.height <= self.bottom()
    }
}

/// 屏幕/图像坐标系中的矩形。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RectI {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl RectI {
    pub const fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.width <= 0 || self.height <= 0
    }

    /// 解析 `x,y,w,h`（允许空格）。
    pub fn parse(text: &str) -> Result<Self> {
        let parts: Vec<&str> = text.split(',').map(str::trim).collect();
        if parts.len() != 4 {
            return Err(AppError::msg(format!(
                "区域格式应为 x,y,w,h（收到 {text:?}）"
            )));
        }

        let mut values = [0i32; 4];
        for (index, part) in parts.iter().enumerate() {
            values[index] = part
                .parse::<i32>()
                .map_err(|_| AppError::msg(format!("区域坐标不是整数: {part:?}")))?;
        }

        let rect = Self::new(values[0], values[1], values[2], values[3]);
        if rect.is_empty() {
            return Err(AppError::msg(format!("区域宽高必须为正: {text:?}")));
        }
        Ok(rect)
    }
}

impl std::fmt::Display for RectI {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{},{},{},{}",
            self.x, self.y, self.width, self.height
        )
    }
}

#[derive(Clone, Debug)]
pub struct MonitorInfo {
    pub device: String,
    pub rect: VirtualScreen,
    pub primary: bool,
    pub dpi_x: u32,
    pub dpi_y: u32,
}

/// 一张自顶向下的 32bpp 图像（BGRA）。
#[derive(Clone, Debug)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub origin: (i32, i32),
    pub bgra: Vec<u8>,
}

impl Frame {
    /// 按屏幕矩形裁剪；要求矩形完全落在画面内。
    pub fn crop(&self, rect: RectI) -> Result<Frame> {
        if rect.is_empty() {
            return Err(AppError::msg(format!("裁剪区域为空: {rect}")));
        }

        let local_x = rect.x - self.origin.0;
        let local_y = rect.y - self.origin.1;
        if local_x < 0
            || local_y < 0
            || local_x + rect.width > self.width as i32
            || local_y + rect.height > self.height as i32
        {
            return Err(AppError::msg(format!(
                "裁剪区域 {rect} 超出画面 {}x{} @ ({}, {})",
                self.width, self.height, self.origin.0, self.origin.1
            )));
        }

        let row_bytes = (rect.width as usize) * 4;
        let mut bgra = Vec::with_capacity(row_bytes * rect.height as usize);
        for row in 0..rect.height as usize {
            let source_row = (local_y as usize + row) * (self.width as usize) * 4;
            let start = source_row + local_x as usize * 4;
            bgra.extend_from_slice(&self.bgra[start..start + row_bytes]);
        }

        Ok(Frame {
            width: rect.width as u32,
            height: rect.height as u32,
            origin: (rect.x, rect.y),
            bgra,
        })
    }

    /// 屏幕坐标处的 BGR 值（不检查 alpha）。
    pub fn pixel(&self, x: i32, y: i32) -> Option<(u8, u8, u8)> {
        let local_x = x - self.origin.0;
        let local_y = y - self.origin.1;
        if local_x < 0
            || local_y < 0
            || local_x >= self.width as i32
            || local_y >= self.height as i32
        {
            return None;
        }
        let offset = ((local_y as usize) * (self.width as usize) + local_x as usize) * 4;
        Some((
            self.bgra[offset],
            self.bgra[offset + 1],
            self.bgra[offset + 2],
        ))
    }

    /// 非纯黑像素占比：用于判断截屏是否真的拿到了画面。
    pub fn non_black_ratio(&self) -> f64 {
        if self.bgra.is_empty() {
            return 0.0;
        }
        let lit = self
            .bgra
            .chunks_exact(4)
            .filter(|pixel| pixel[0] | pixel[1] | pixel[2] != 0)
            .count();
        lit as f64 / (self.bgra.len() / 4) as f64
    }

    /// 与另一张同尺寸画面的差异像素占比；尺寸不同视为 100%。
    pub fn pixel_diff_ratio(&self, other: &Frame) -> f64 {
        if self.width != other.width || self.height != other.height {
            return 1.0;
        }
        if self.bgra.len() != other.bgra.len() {
            return 1.0;
        }
        let mut different = 0usize;
        for (left, right) in self.bgra.chunks_exact(4).zip(other.bgra.chunks_exact(4)) {
            if left[0] != right[0] || left[1] != right[1] || left[2] != right[2] {
                different += 1;
            }
        }
        different as f64 / (self.width as usize * self.height as usize).max(1) as f64
    }
}

/// 采集后端的统一接口；P2/P6 会增加 WGC 等实现。
pub trait CaptureBackend {
    fn name(&self) -> &'static str;
    fn capture(&mut self, area: VirtualScreen) -> Result<Frame>;
}

/// GDI `BitBlt` 后端（默认）。
#[derive(Default)]
pub struct GdiBackend;

impl CaptureBackend for GdiBackend {
    fn name(&self) -> &'static str {
        "gdi"
    }

    fn capture(&mut self, area: VirtualScreen) -> Result<Frame> {
        gdi::capture(area)
    }
}

pub fn monitors() -> Vec<MonitorInfo> {
    let mut list: Vec<MonitorInfo> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(monitor_proc),
            LPARAM(&mut list as *mut Vec<MonitorInfo> as isize),
        );
    }
    list
}

unsafe extern "system" fn monitor_proc(
    monitor: HMONITOR,
    _hdc: HDC,
    _rect: *mut RECT,
    data: LPARAM,
) -> windows::core::BOOL {
    unsafe {
        let list = &mut *(data.0 as *mut Vec<MonitorInfo>);

        let mut info = MONITORINFOEXW::default();
        info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;

        if GetMonitorInfoW(monitor, &mut info.monitorInfo as *mut _).0 != 0 {
            let len = info
                .szDevice
                .iter()
                .position(|value| *value == 0)
                .unwrap_or(info.szDevice.len());

            let mut dpi_x = 96;
            let mut dpi_y = 96;
            let _ = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);

            let rc = info.monitorInfo.rcMonitor;
            list.push(MonitorInfo {
                device: String::from_utf16_lossy(&info.szDevice[..len]),
                rect: VirtualScreen {
                    x: rc.left,
                    y: rc.top,
                    width: rc.right - rc.left,
                    height: rc.bottom - rc.top,
                },
                primary: info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
                dpi_x,
                dpi_y,
            });
        }

        windows::core::BOOL(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::test_pattern::TestPattern;

    fn synthetic_frame() -> Frame {
        // 2x2 画面，origin (10, 20)：TL 红、TR 绿、BL 蓝、BR 白。
        Frame {
            width: 2,
            height: 2,
            origin: (10, 20),
            bgra: vec![
                0, 0, 255, 255, // 红
                0, 255, 0, 255, // 绿
                255, 0, 0, 255, // 蓝
                255, 255, 255, 255, // 白
            ],
        }
    }

    #[test]
    fn rect_parse_and_validation() {
        assert_eq!(
            RectI::parse("10, 20, 30, 40").unwrap(),
            RectI::new(10, 20, 30, 40)
        );
        assert!(RectI::parse("10,20,0,40").is_err());
        assert!(RectI::parse("10,20,30").is_err());
        assert!(RectI::parse("a,b,c,d").is_err());
    }

    #[test]
    fn crop_respects_origin_and_bounds() {
        let frame = synthetic_frame();

        let red = frame.crop(RectI::new(10, 20, 1, 1)).unwrap();
        assert_eq!(red.pixel(10, 20), Some((0, 0, 255)));

        let bottom_right = frame.crop(RectI::new(11, 21, 1, 1)).unwrap();
        assert_eq!(bottom_right.pixel(11, 21), Some((255, 255, 255)));

        let whole = frame.crop(RectI::new(10, 20, 2, 2)).unwrap();
        assert_eq!(whole.width, 2);
        assert_eq!(whole.height, 2);

        assert!(frame.crop(RectI::new(9, 20, 1, 1)).is_err());
        assert!(frame.crop(RectI::new(11, 21, 2, 1)).is_err());
        assert!(frame.crop(RectI::new(10, 20, 0, 1)).is_err());
    }

    #[test]
    fn diff_ratio_is_zero_for_identical_frames() {
        let frame = synthetic_frame();
        assert_eq!(frame.pixel_diff_ratio(&frame), 0.0);
        let other = Frame {
            bgra: vec![0; 16],
            ..frame.clone()
        };
        assert!(frame.pixel_diff_ratio(&other) > 0.9);
    }

    /// 端到端：先显示静态测试窗，再分别整屏抓取和按区域抓取，两者必须逐像素一致。
    #[test]
    fn region_capture_equals_full_crop() {
        let screen = VirtualScreen::current();
        if screen.width <= 0 || screen.height <= 0 {
            eprintln!("跳过：没有可用的桌面会话");
            return;
        }

        let pattern = match TestPattern::show() {
            Ok(pattern) => pattern,
            Err(error) => {
                eprintln!("跳过：无法创建测试窗口: {error}");
                return;
            }
        };

        let mut backend = GdiBackend;
        let full = match backend.capture(screen) {
            Ok(frame) => frame,
            Err(error) => {
                eprintln!("跳过：采集失败: {error}");
                pattern.close();
                return;
            }
        };
        if full.non_black_ratio() < 0.01 {
            eprintln!("跳过：采集画面几乎全黑");
            pattern.close();
            return;
        }

        let region = pattern.rect();
        let region_screen = VirtualScreen {
            x: region.x,
            y: region.y,
            width: region.width,
            height: region.height,
        };
        let region_frame = backend.capture(region_screen).expect("区域采集失败");
        let cropped = full.crop(region).expect("裁剪失败");
        let diff = region_frame.pixel_diff_ratio(&cropped);

        pattern.close();

        assert!(
            diff <= 0.001,
            "区域截图与整屏裁剪不一致: diff={diff:.6} region={region}"
        );
    }
}
