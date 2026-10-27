//! The real `DisplayBackend`: GDI gamma ramps plus NVIDIA vibrance/hue when available.

use super::gamma;
use super::nvapi::NvApi;
use crate::engine::DisplayBackend;
use crate::ramp::GammaRamp;
use crate::vibrance::VibranceInfo;

pub struct WinDisplay {
    nv: Option<NvApi>,
}

impl WinDisplay {
    pub fn new() -> WinDisplay {
        WinDisplay { nv: NvApi::load() }
    }
}

impl Default for WinDisplay {
    fn default() -> WinDisplay {
        WinDisplay::new()
    }
}

impl DisplayBackend for WinDisplay {
    fn get_ramp(&mut self, monitor: &str) -> Option<GammaRamp> {
        gamma::get_ramp(monitor)
    }

    fn set_ramp(&mut self, monitor: &str, ramp: &GammaRamp) -> bool {
        gamma::set_ramp(monitor, ramp)
    }

    fn get_vibrance(&mut self, monitor: &str) -> Option<VibranceInfo> {
        self.nv.as_ref()?.get_vibrance(monitor)
    }

    fn set_vibrance(&mut self, monitor: &str, raw: i32) -> bool {
        self.nv.as_ref().is_some_and(|nv| nv.set_vibrance(monitor, raw))
    }

    fn get_hue(&mut self, monitor: &str) -> Option<u32> {
        self.nv.as_ref()?.get_hue(monitor)
    }

    fn set_hue(&mut self, monitor: &str, degrees: u32) -> bool {
        self.nv.as_ref().is_some_and(|nv| nv.set_hue(monitor, degrees))
    }
}

/// True when NvAPI loads, its vibrance/hue calls exist, and at least one monitor reports vibrance.
pub fn nvidia_colour_available() -> bool {
    NvApi::load().is_some_and(|nv| {
        nv.vibrance_supported() && gamma::monitors().iter().any(|m| nv.get_vibrance(m).is_some())
    })
}
