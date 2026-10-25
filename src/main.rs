//! window-cap 可执行入口：声明模块并运行应用。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod annotate;
mod app;
mod capture;
mod output;
mod platform;
mod spike;
mod ui;
mod util;

/// release 无控制台：把 stdout/stderr 重定向到 CLI 日志文件。
#[cfg(not(debug_assertions))]
fn init_cli_output() {
    use std::io::Write;
    use std::os::windows::io::AsRawHandle;

    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::Console::{STD_ERROR_HANDLE, STD_OUTPUT_HANDLE, SetStdHandle};

    let Some(path) = crate::util::log::cli_log_file() else {
        return;
    };
    let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    else {
        return;
    };

    let started = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0);
    if file.metadata().map(|meta| meta.len() == 0).unwrap_or(false) {
        let _ = file.write_all(&[0xEF, 0xBB, 0xBF]);
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    let _ = writeln!(
        file,
        "\n===== window-cap cli @ {started} args={args:?} ====="
    );
    let _ = file.flush();

    let handle = HANDLE(file.as_raw_handle() as _);
    unsafe {
        let _ = SetStdHandle(STD_OUTPUT_HANDLE, handle);
        let _ = SetStdHandle(STD_ERROR_HANDLE, handle);
    }
    std::mem::forget(file);
}

#[cfg(debug_assertions)]
fn init_cli_output() {}

fn main() {
    init_cli_output();
    if let Err(error) = app::run() {
        eprintln!("window-cap: {error}");
        std::process::exit(1);
    }
}
