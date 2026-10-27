//! Debug-only `--probe`: verifies gamma ramp access on the real machine.

use super::gamma::{get_ramp, monitors, set_ramp};
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
    i32::from(failed)
}
