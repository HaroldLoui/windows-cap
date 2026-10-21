//! 文件输出：PNG 编码、原子写、默认路径。

use std::io::Cursor;
use std::path::{Path, PathBuf};

use windows::Win32::System::SystemInformation::GetLocalTime;

use crate::capture::Frame;
use crate::util::error::{AppError, Result};

/// 把画面编码为 PNG 并原子写入目标路径（先写临时文件，再 rename）。
pub fn save_png(frame: &Frame, path: &Path) -> Result<()> {
    let mut rgba = frame.bgra.clone();
    for pixel in rgba.chunks_exact_mut(4) {
        pixel.swap(0, 2);
        pixel[3] = 255;
    }

    let image = image::RgbaImage::from_raw(frame.width, frame.height, rgba)
        .ok_or_else(|| AppError::msg("图像尺寸与像素数据不一致"))?;

    let mut bytes = Vec::new();
    image.write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)?;
    write_atomic(path, &bytes)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        std::fs::create_dir_all(parent)?;
    }

    let file_name = path
        .file_name()
        .ok_or_else(|| AppError::msg(format!("无效的输出路径: {}", path.display())))?;

    let mut temporary_name = std::ffi::OsString::from(".");
    temporary_name.push(file_name);
    temporary_name.push(".tmp");
    let temporary = path.with_file_name(temporary_name);

    std::fs::write(&temporary, bytes)?;
    std::fs::rename(&temporary, path)?;
    Ok(())
}

/// 默认截图路径：`%USERPROFILE%\Pictures\Screenshots\Screen_<时间戳>.png`。
#[allow(dead_code)] // P2 遮罩保存与 P4 托盘菜单会使用
pub fn default_path() -> PathBuf {
    let directory = std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Pictures")
        .join("Screenshots");
    path_in(&directory)
}

/// 在指定目录下生成一张新的截图路径。
pub fn path_in(directory: &Path) -> PathBuf {
    directory.join(format!("Screen_{}.png", timestamp()))
}

#[allow(dead_code)]
fn timestamp() -> String {
    let time = unsafe { GetLocalTime() };
    format!(
        "{:04}{:02}{:02}_{:02}{:02}{:02}_{:03}",
        time.wYear,
        time.wMonth,
        time.wDay,
        time.wHour,
        time.wMinute,
        time.wSecond,
        time.wMilliseconds
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_and_reload_png() {
        let frame = Frame {
            width: 2,
            height: 2,
            origin: (0, 0),
            bgra: vec![
                0, 0, 255, 0, // 红
                0, 255, 0, 0, // 绿
                255, 0, 0, 0, // 蓝
                255, 255, 255, 0, // 白
            ],
        };

        let path = std::env::temp_dir().join(format!(
            "window-cap-test-{}-{}.png",
            std::process::id(),
            timestamp()
        ));
        save_png(&frame, &path).expect("保存失败");
        assert!(path.exists());

        let reloaded = image::open(&path).expect("读回失败").to_rgba8();
        assert_eq!(reloaded.width(), 2);
        assert_eq!(reloaded.get_pixel(0, 0).0, [255, 0, 0, 255]);
        assert_eq!(reloaded.get_pixel(1, 0).0, [0, 255, 0, 255]);
        assert_eq!(reloaded.get_pixel(0, 1).0, [0, 0, 255, 255]);
        assert_eq!(reloaded.get_pixel(1, 1).0, [255, 255, 255, 255]);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn atomic_write_creates_parent_directories() {
        let directory = std::env::temp_dir().join(format!(
            "window-cap-dir-{}-{}",
            std::process::id(),
            timestamp()
        ));
        let path = directory.join("nested").join("capture.png");
        let frame = Frame {
            width: 1,
            height: 1,
            origin: (0, 0),
            bgra: vec![1, 2, 3, 0],
        };
        save_png(&frame, &path).expect("保存失败");
        assert!(path.exists());
        let _ = std::fs::remove_dir_all(&directory);
    }
}
