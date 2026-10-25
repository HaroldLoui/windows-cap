//! GDI `BitBlt` 采集实现。

use windows::Win32::Graphics::Gdi::{BitBlt, CAPTUREBLT, GetDC, ROP_CODE, ReleaseDC, SRCCOPY};

use crate::capture::{Frame, VirtualScreen};
use crate::platform::dib::Dib;
use crate::util::error::{AppError, Result};

/// 用 GDI `BitBlt` 抓取虚拟桌面的指定区域。
pub fn capture(area: VirtualScreen) -> Result<Frame> {
    if area.width <= 0 || area.height <= 0 {
        return Err(AppError::msg(format!(
            "无效的截取区域: {}x{}",
            area.width, area.height
        )));
    }

    // SAFETY: GetDC/ReleaseDC 成对；DIB 在闭包结束时由 Drop 释放。
    unsafe {
        let screen = GetDC(None);
        if screen.0.is_null() {
            return Err(AppError::msg("GetDC(NULL) 失败"));
        }

        let result = (|| -> Result<Frame> {
            let dib = Dib::new(area.width as u32, area.height as u32)?;
            // CAPTUREBLT 让分层窗口也参与采集。
            BitBlt(
                dib.dc,
                0,
                0,
                area.width,
                area.height,
                Some(screen),
                area.x,
                area.y,
                ROP_CODE(SRCCOPY.0 | CAPTUREBLT.0),
            )?;
            Ok(Frame {
                width: area.width as u32,
                height: area.height as u32,
                origin: (area.x, area.y),
                bgra: dib.pixels().to_vec(),
            })
        })();

        ReleaseDC(None, screen);
        result
    }
}
