//! Debug-only `--probe`: verifies gamma ramp and NvAPI vibrance/hue access on the real machine.

use super::gamma::{get_ramp, monitors, set_ramp};
use super::nvapi::NvApi;
use crate::ramp::{CurveParams, layered};
use std::time::Duration;

pub fn run() -> i32 {
    let devices = monitors();
    if devices.is_empty() {
        println!("no monitors found");
        return 1;
    }
    let mut failed = false;
    for dev in devices {
        let Some(normal) = get_ramp(&dev) else {
            println!("{dev}: read failed");
            failed = true;
            continue;
        };
        println!("{dev}: read ok");
        let mut applied = false;
        for gamma in [1.5, 1.2] {
            let bright = layered(&normal, &CurveParams { gamma, ..CurveParams::NEUTRAL });
            if set_ramp(&dev, &bright) {
                println!("{dev}: set ok at {gamma}");
                applied = true;
                break;
            }
            println!("{dev}: set rejected at {gamma}");
        }
        if !applied {
            failed = true;
        }
        std::thread::sleep(Duration::from_secs(2));
        if set_ramp(&dev, &normal) {
            println!("{dev}: restored ok");
        } else {
            println!("{dev}: restore failed");
            failed = true;
        }
    }
    if probe_nvapi() {
        failed = true;
    }
    i32::from(failed)
}

const HOLD: Duration = Duration::from_secs(2);

/// Returns true when something failed. A machine without NvAPI is not a failure.
fn probe_nvapi() -> bool {
    let Some(nv) = NvApi::load() else {
        println!("nvapi: not loaded");
        return false;
    };
    println!("nvapi: loaded");
    println!("vibrance supported: {}", if nv.vibrance_supported() { "yes" } else { "no" });
    let mut failed = false;
    for dev in monitors() {
        let vib = nv.get_vibrance(&dev);
        match vib {
            Some(v) => {
                println!("{dev}: vibrance current/min/max/default {}/{}/{}/{}", v.current, v.min, v.max, v.default);
                if !(v.min <= v.default && v.default <= v.max) {
                    println!("{dev}: vibrance range not sensible");
                    failed = true;
                }
            }
            None => println!("{dev}: vibrance unavailable"),
        }
        let hue = nv.get_hue(&dev);
        match hue {
            Some(h) => println!("{dev}: hue current {h}"),
            None => println!("{dev}: hue unavailable"),
        }
        if let Some(v) = vib {
            failed |= exercise(&dev, "vibrance", v.current, v.max, |x| nv.set_vibrance(&dev, x), || {
                nv.get_vibrance(&dev).map(|r| r.current)
            });
        }
        if let Some(h) = hue {
            failed |= exercise(&dev, "hue", h, (h + 30) % 360, |x| nv.set_hue(&dev, x), || nv.get_hue(&dev));
        }
    }
    if let Some(status) = nv.failed_status() {
        println!("nvapi: vibrance/hue switched off after status {status}");
        failed = true;
    }
    failed
}

/// Sets `what` to `target`, reads it back, holds it visibly, then restores `original`.
/// Returns true when something failed.
fn exercise<T: Copy + PartialEq + std::fmt::Display>(
    dev: &str,
    what: &str,
    original: T,
    target: T,
    set: impl Fn(T) -> bool,
    get: impl Fn() -> Option<T>,
) -> bool {
    if !set(target) {
        println!("{dev}: {what} set rejected");
        return true;
    }
    let mut failed = false;
    match get() {
        Some(r) => {
            println!("{dev}: {what} set ok (readback {r})");
            if r != target {
                println!("{dev}: {what} readback mismatch (wanted {target})");
                failed = true;
            }
        }
        None => {
            println!("{dev}: {what} set ok (readback failed)");
            failed = true;
        }
    }
    std::thread::sleep(HOLD);
    if !set(original) {
        println!("{dev}: {what} restore failed");
        return true;
    }
    match get() {
        Some(r) => {
            println!("{dev}: {what} restored (readback {r})");
            if r != original {
                println!("{dev}: {what} restore mismatch (wanted {original})");
                failed = true;
            }
        }
        None => {
            println!("{dev}: {what} restored (readback failed)");
            failed = true;
        }
    }
    failed
}
