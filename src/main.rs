#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![cfg_attr(not(windows), allow(dead_code))]

mod config;
mod ramp;
mod vibrance;

#[cfg(windows)]
fn main() {
    std::process::exit(0);
}

#[cfg(not(windows))]
fn main() {
    eprintln!("BrightMarmot runs on Windows only.");
    std::process::exit(1);
}
