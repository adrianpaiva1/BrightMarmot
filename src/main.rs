#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![cfg_attr(not(windows), allow(dead_code))]

mod config;
mod engine;
mod ramp;
mod restore;
mod vibrance;
#[cfg(windows)]
mod win;

#[cfg(windows)]
fn main() {
    #[cfg(debug_assertions)]
    if std::env::args().nth(1).as_deref() == Some("--probe") {
        std::process::exit(win::probe::run());
    }
    std::process::exit(0);
}

#[cfg(not(windows))]
fn main() {
    eprintln!("BrightMarmot runs on Windows only.");
    std::process::exit(1);
}
