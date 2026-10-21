use std::ffi::c_void;
use std::mem::size_of;
use std::ptr::null_mut;

use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS,
    DeleteDC, DeleteObject, HBITMAP, HDC, HGDIOBJ, SelectObject,
};

use crate::util::error::{AppError, Result};

/// 一块可读写的 32bpp 自顶向下 DIB，配套一个内存 DC。
pub struct Dib {
    pub dc: HDC,
    hbm: HBITMAP,
    old: HGDIOBJ,
    bits: *mut u8,
    len: usize,
}

impl Dib {
    pub fn new(width: u32, height: u32) -> Result<Self> {
        if width == 0 || height == 0 {
            return Err(AppError::msg("DIB 尺寸不能为 0"));
        }

        unsafe {
            let dc = CreateCompatibleDC(None);
            if dc.0.is_null() {
                return Err(AppError::msg("CreateCompatibleDC 失败"));
            }

            let mut bmi = BITMAPINFO::default();
            bmi.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
            bmi.bmiHeader.biWidth = width as i32;
            // 负高度 = 自顶向下，行序与内存一致。
            bmi.bmiHeader.biHeight = -(height as i32);
            bmi.bmiHeader.biPlanes = 1;
            bmi.bmiHeader.biBitCount = 32;
            bmi.bmiHeader.biCompression = BI_RGB.0;

            let mut bits: *mut c_void = null_mut();
            let hbm = match CreateDIBSection(Some(dc), &bmi, DIB_RGB_COLORS, &mut bits, None, 0) {
                Ok(hbm) => hbm,
                Err(error) => {
                    let _ = DeleteDC(dc);
                    return Err(error.into());
                }
            };

            if bits.is_null() {
                let _ = DeleteObject(HGDIOBJ(hbm.0));
                let _ = DeleteDC(dc);
                return Err(AppError::msg("CreateDIBSection 返回空像素指针"));
            }

            let old = SelectObject(dc, HGDIOBJ(hbm.0));
            Ok(Self {
                dc,
                hbm,
                old,
                bits: bits as *mut u8,
                len: (width as usize) * (height as usize) * 4,
            })
        }
    }

    /// 从自顶向下 BGRA 数据创建 DIB。
    pub fn from_bgra(width: u32, height: u32, data: &[u8]) -> Result<Self> {
        let mut dib = Self::new(width, height)?;
        let pixels = dib.pixels_mut();
        if data.len() != pixels.len() {
            return Err(AppError::msg(format!(
                "像素数据长度不符: 期望 {} 实际 {}",
                pixels.len(),
                data.len()
            )));
        }
        pixels.copy_from_slice(data);
        Ok(dib)
    }

    pub fn pixels(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.bits, self.len) }
    }

    #[allow(dead_code)] // P3 的马赛克/撤销会直接改像素
    pub fn pixels_mut(&mut self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.bits, self.len) }
    }
}

impl Drop for Dib {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old);
            let _ = DeleteObject(HGDIOBJ(self.hbm.0));
            let _ = DeleteDC(self.dc);
        }
    }
}
