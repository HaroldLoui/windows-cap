//! 剪贴板输出：把 `Frame` 写成 Windows 标准的 `CF_DIB`。

use windows::Win32::Foundation::{GlobalFree, HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
    SetClipboardData,
};
use windows::Win32::System::Memory::{
    GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock,
};

use crate::capture::Frame;
use crate::util::error::{AppError, Result};

/// Windows 标准剪贴板格式 `CF_DIB`（值为 8）。
const CF_DIB: u32 = 8;
/// `BITMAPINFOHEADER` 的字节数。
const DIB_HEADER_BYTES: usize = 40;

/// 把画面以 `CF_DIB` 放入剪贴板。
pub fn put(frame: &Frame) -> Result<()> {
    let dib = encode_dib(frame);
    let mut last_error: Option<windows::core::Error> = None;

    // 剪贴板可能被其他程序短暂占用，最多重试 3 次。
    for attempt in 0..3 {
        match unsafe { OpenClipboard(None) } {
            Ok(()) => {
                let result = unsafe {
                    (|| -> Result<()> {
                        EmptyClipboard()?;

                        let hglobal = GlobalAlloc(GMEM_MOVEABLE, dib.len())?;
                        let pointer = GlobalLock(hglobal);
                        if pointer.is_null() {
                            let _ = GlobalFree(Some(hglobal));
                            return Err(AppError::msg("GlobalLock 失败"));
                        }
                        // 写入剪贴板
                        std::ptr::copy_nonoverlapping(dib.as_ptr(), pointer as *mut u8, dib.len());
                        let _ = GlobalUnlock(hglobal);

                        // 设置成功后所有权归系统；失败路径自行 GlobalFree。
                        match SetClipboardData(CF_DIB, Some(HANDLE(hglobal.0))) {
                            Ok(_) => Ok(()),
                            Err(error) => {
                                let _ = GlobalFree(Some(hglobal));
                                Err(error.into())
                            }
                        }
                    })()
                };
                let _ = unsafe { CloseClipboard() };
                result?;
                return Ok(());
            }
            Err(error) => {
                last_error = Some(error);
                if attempt < 2 {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
            }
        }
    }

    Err(last_error
        .map(AppError::from)
        .unwrap_or_else(|| AppError::msg("剪贴板被其他程序占用")))
}

/// 读回剪贴板中 `CF_DIB` 的字节数；没有该格式时返回 `None`。
pub fn probe_dib() -> Result<Option<usize>> {
    unsafe {
        OpenClipboard(None)?;
        let result = (|| -> windows::core::Result<Option<usize>> {
            if IsClipboardFormatAvailable(CF_DIB).is_err() {
                return Ok(None);
            }
            let handle = GetClipboardData(CF_DIB)?;
            if handle.0.is_null() {
                return Ok(None);
            }
            Ok(Some(GlobalSize(HGLOBAL(handle.0))))
        })();
        let _ = CloseClipboard();
        result.map_err(AppError::from)
    }
}

/// 编码为 `BITMAPINFOHEADER` + 自底向上 BGRA（32bpp，alpha=255）。
fn encode_dib(frame: &Frame) -> Vec<u8> {
    let row_bytes = frame.width as usize * 4;
    let pixel_bytes = row_bytes * frame.height as usize;
    let mut out = vec![0u8; DIB_HEADER_BYTES + pixel_bytes];

    // 头部字段均为小端。
    out[0..4].copy_from_slice(&(DIB_HEADER_BYTES as u32).to_le_bytes());
    out[4..8].copy_from_slice(&(frame.width as i32).to_le_bytes());
    out[8..12].copy_from_slice(&(frame.height as i32).to_le_bytes());
    out[12..14].copy_from_slice(&1u16.to_le_bytes());
    out[14..16].copy_from_slice(&32u16.to_le_bytes());
    out[16..20].copy_from_slice(&0u32.to_le_bytes()); // BI_RGB
    out[20..24].copy_from_slice(&(pixel_bytes as u32).to_le_bytes());

    for row in 0..frame.height as usize {
        let source_row = (frame.height as usize - 1 - row) * row_bytes;
        let target_row = DIB_HEADER_BYTES + row * row_bytes;
        for column in 0..frame.width as usize {
            let source = source_row + column * 4;
            let target = target_row + column * 4;
            out[target] = frame.bgra[source];
            out[target + 1] = frame.bgra[source + 1];
            out[target + 2] = frame.bgra[source + 2];
            out[target + 3] = 255;
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dib_header_and_bottom_up_order() {
        let frame = Frame {
            width: 2,
            height: 2,
            origin: (0, 0),
            bgra: vec![
                0, 0, 255, 0, // 顶行左：红
                0, 255, 0, 0, // 顶行右：绿
                255, 0, 0, 0, // 底行左：蓝
                255, 255, 255, 0, // 底行右：白
            ],
        };

        let dib = encode_dib(&frame);
        assert_eq!(dib.len(), DIB_HEADER_BYTES + 16);
        assert_eq!(u32::from_le_bytes(dib[0..4].try_into().unwrap()), 40);
        assert_eq!(i32::from_le_bytes(dib[4..8].try_into().unwrap()), 2);
        assert_eq!(i32::from_le_bytes(dib[8..12].try_into().unwrap()), 2);
        assert_eq!(u16::from_le_bytes(dib[12..14].try_into().unwrap()), 1);
        assert_eq!(u16::from_le_bytes(dib[14..16].try_into().unwrap()), 32);
        assert_eq!(u32::from_le_bytes(dib[16..20].try_into().unwrap()), 0);
        assert_eq!(u32::from_le_bytes(dib[20..24].try_into().unwrap()), 16);

        // 第一行数据应来自图像最后一行；alpha 统一为 255。
        assert_eq!(&dib[40..44], &[255, 0, 0, 255]);
        assert_eq!(&dib[44..48], &[255, 255, 255, 255]);
        assert_eq!(&dib[48..52], &[0, 0, 255, 255]);
        assert_eq!(&dib[52..56], &[0, 255, 0, 255]);
    }
}
