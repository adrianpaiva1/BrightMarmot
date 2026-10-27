#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![cfg_attr(not(windows), allow(dead_code))]

mod apps;
mod config;
mod engine;
mod icon;
mod ramp;
mod restore;
#[cfg(windows)]
mod settings;
mod vibrance;
#[cfg(windows)]
mod win;

#[cfg(windows)]
fn main() {
    #[cfg(debug_assertions)]
    if std::env::args().nth(1).as_deref() == Some("--probe") {
        std::process::exit(win::probe::run());
    }
    if std::env::args().nth(1).as_deref() == Some("--settings") {
        std::process::exit(settings::run());
    }
    if std::env::args().nth(1).as_deref() == Some("--set-gamma-range") {
        std::process::exit(if win::registry::write_gamma_range() { 0 } else { 1 });
    }
    std::process::exit(0);
}

#[cfg(not(windows))]
fn main() {
    eprintln!("BrightMarmot runs on Windows only.");
    std::process::exit(1);
}
