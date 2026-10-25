//! 进程资源统计：GDI/USER 对象数与工作集。

use std::mem::size_of;

use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows::Win32::System::Threading::{
    GR_GDIOBJECTS, GR_USEROBJECTS, GetCurrentProcess, GetGuiResources,
};

use crate::util::error::Result;

/// 返回 `(GDI 对象数, USER 对象数, 工作集字节数)`。
pub fn process_resources() -> Result<(u32, u32, usize)> {
    unsafe {
        let process = GetCurrentProcess();
        let gdi = GetGuiResources(process, GR_GDIOBJECTS);
        let user = GetGuiResources(process, GR_USEROBJECTS);

        let mut counters = PROCESS_MEMORY_COUNTERS {
            cb: size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
            ..Default::default()
        };
        GetProcessMemoryInfo(process, &mut counters, counters.cb)?;

        Ok((gdi, user, counters.WorkingSetSize))
    }
}
