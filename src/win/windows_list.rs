//! Snapshot of top-level windows for the Add Game dialog. Read-only queries: window
//! attributes and the owning process's image path (limited-query handle, see `ipc`).

use std::ffi::c_void;
use windows_sys::Win32::Foundation::{HWND, LPARAM};
use windows_sys::core::BOOL;
use windows_sys::Win32::Graphics::Dwm::{DWMWA_CLOAKED, DwmGetWindowAttribute};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GW_OWNER, GWL_EXSTYLE, GetWindow, GetWindowLongW, GetWindowTextLengthW, GetWindowTextW,
    GetWindowThreadProcessId, IsWindowVisible, WS_EX_TOOLWINDOW,
};

use super::ipc::process_image_path;
use crate::apps::WindowInfo;

pub fn top_level_windows() -> Vec<WindowInfo> {
    let mut out: Vec<WindowInfo> = Vec::new();
    // SAFETY: the callback only dereferences the Vec pointer passed here, which outlives the call.
    unsafe { EnumWindows(Some(collect), &mut out as *mut Vec<WindowInfo> as LPARAM) };
    out
}

unsafe extern "system" fn collect(hwnd: HWND, lparam: LPARAM) -> BOOL {
    // SAFETY: lparam is the `&mut Vec<WindowInfo>` from `top_level_windows`.
    let out = unsafe { &mut *(lparam as *mut Vec<WindowInfo>) };
    // SAFETY: plain queries on a window handle from EnumWindows; a stale handle just fails.
    unsafe {
        let visible = IsWindowVisible(hwnd) != 0;
        let has_owner = !GetWindow(hwnd, GW_OWNER).is_null();
        let tool_window = (GetWindowLongW(hwnd, GWL_EXSTYLE) as u32) & WS_EX_TOOLWINDOW != 0;
        let mut cloaked_val: u32 = 0;
        let hr = DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED as u32,
            &mut cloaked_val as *mut u32 as *mut c_void,
            std::mem::size_of::<u32>() as u32,
        );
        let cloaked = hr == 0 && cloaked_val != 0;

        let len = GetWindowTextLengthW(hwnd).max(0) as usize;
        let mut buf = vec![0u16; len + 1];
        let n = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32).max(0) as usize;
        let title = String::from_utf16_lossy(&buf[..n.min(buf.len())]);

        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        let exe_path = process_image_path(pid).unwrap_or_default();

        out.push(WindowInfo { exe_path, title, visible, has_owner, tool_window, cloaked });
    }
    1
}
