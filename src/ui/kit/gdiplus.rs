//! 进程级 GDI+ 初始化。

use std::sync::Once;
use std::sync::atomic::{AtomicUsize, Ordering};

use windows::Win32::Graphics::GdiPlus::{
    GdiplusStartup, GdiplusStartupInput, GdiplusStartupOutput,
};

static INIT: Once = Once::new();
static TOKEN: AtomicUsize = AtomicUsize::new(0);

/// 初始化 GDI+；重复调用安全。
pub fn init() -> bool {
    INIT.call_once(|| unsafe {
        let mut token = 0usize;
        let input = GdiplusStartupInput {
            GdiplusVersion: 1,
            ..Default::default()
        };
        let mut output = GdiplusStartupOutput::default();
        let status = GdiplusStartup(&mut token, &input, &mut output);
        if status.0 == 0 {
            TOKEN.store(token, Ordering::SeqCst);
        }
    });
    TOKEN.load(Ordering::SeqCst) != 0
}
