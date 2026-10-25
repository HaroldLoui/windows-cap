//! window-cap 可执行入口：声明模块并运行应用。

mod annotate;
mod app;
mod capture;
mod output;
mod platform;
mod spike;
mod ui;
mod util;

fn main() {
    if let Err(error) = app::run() {
        eprintln!("window-cap: {error}");
        std::process::exit(1);
    }
}
