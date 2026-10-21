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
