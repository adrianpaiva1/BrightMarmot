//! Thin wrappers over the GDI per-monitor gamma ramp.

use crate::ramp::GammaRamp;
use std::ffi::c_void;
use std::ptr::null;
use windows_sys::Win32::Foundation::{HWND, LPARAM, RECT, TRUE};
use windows_sys::Win32::Graphics::Gdi::{
    CreateDCW, DeleteDC, EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR,
    MONITOR_DEFAULTTONEAREST, MONITORINFOEXW, MonitorFromWindow,
};
use windows_sys::Win32::UI::ColorSystem::{GetDeviceGammaRamp, SetDeviceGammaRamp};

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn device_name(mon: HMONITOR) -> Option<String> {
    // SAFETY: MONITORINFOEXW is plain data; cbSize is set as the API requires.
    unsafe {
        let mut mi: MONITORINFOEXW = std::mem::zeroed();
        mi.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        if GetMonitorInfoW(mon, &mut mi as *mut MONITORINFOEXW as *mut _) == 0 {
            return None;
        }
        let len = mi.szDevice.iter().position(|&c| c == 0).unwrap_or(mi.szDevice.len());
        Some(String::from_utf16_lossy(&mi.szDevice[..len]))
    }
}

unsafe extern "system" fn enum_cb(mon: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> i32 {
    // SAFETY: `data` is the `&mut Vec<String>` passed by `monitors()`, alive for the call.
    let out = unsafe { &mut *(data as *mut Vec<String>) };
    if let Some(name) = device_name(mon) {
        out.push(name);
    }
    TRUE
}

/// Device names (e.g. `\\.\DISPLAY1`) of all attached monitors.
pub fn monitors() -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    // SAFETY: callback only dereferences `data`, which points at `out` for the whole call.
    unsafe {
        EnumDisplayMonitors(
            std::ptr::null_mut(),
            null(),
            Some(enum_cb),
            &mut out as *mut Vec<String> as LPARAM,
        );
    }
    out
}

/// Device name of the monitor nearest to `hwnd`.
pub fn monitor_of_window(hwnd: HWND) -> Option<String> {
    // SAFETY: MonitorFromWindow tolerates any handle value with DEFAULTTONEAREST.
    let mon = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
    if mon.is_null() { None } else { device_name(mon) }
}

/// Run `f` with a DC for `device`; the DC is always deleted.
fn with_dc<T>(device: &str, f: impl FnOnce(HDC) -> T) -> Option<T> {
    let dev = wide(device);
    // SAFETY: `dev` is NUL-terminated and outlives the call.
    let dc = unsafe { CreateDCW(dev.as_ptr(), null(), null(), null()) };
    if dc.is_null() {
        return None;
    }
    let r = f(dc);
    // SAFETY: `dc` was created above and is not used afterwards.
    unsafe { DeleteDC(dc) };
    Some(r)
}

pub fn get_ramp(device: &str) -> Option<GammaRamp> {
    let mut ramp: GammaRamp = [[0; 256]; 3];
    // SAFETY: GammaRamp is exactly the 3x256 u16 buffer GetDeviceGammaRamp fills.
    let ok = with_dc(device, |dc| unsafe {
        GetDeviceGammaRamp(dc, ramp.as_mut_ptr() as *mut c_void)
    })?;
    (ok != 0).then_some(ramp)
}

pub fn set_ramp(device: &str, ramp: &GammaRamp) -> bool {
    // SAFETY: GammaRamp is exactly the 3x256 u16 buffer SetDeviceGammaRamp reads.
    with_dc(device, |dc| unsafe {
        SetDeviceGammaRamp(dc, ramp.as_ptr() as *const c_void)
    })
    .is_some_and(|ok| ok != 0)
}
