//! Foreground-window hook and the `Foreground` (exe name + monitor) of a window.
//! Anti-cheat rules: out-of-context hook only, limited-query process handle only.

use std::ptr::null_mut;
use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::Accessibility::{SetWinEventHook, WINEVENTPROC};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EVENT_SYSTEM_FOREGROUND, GetWindowThreadProcessId, WINEVENT_OUTOFCONTEXT,
};

use super::gamma::monitor_of_window;
use super::ipc::process_image_path;
use crate::config::exe_name_from_path;
use crate::engine::Foreground;

/// Calls `callback` on this thread (from its message loop) whenever the foreground window changes.
/// The hook lives until the thread exits.
pub fn install_hook(callback: WINEVENTPROC) -> bool {
    // SAFETY: out-of-context hook (no DLL is loaded into other processes); all processes and threads.
    let hook = unsafe {
        SetWinEventHook(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND, null_mut(), callback, 0, 0, WINEVENT_OUTOFCONTEXT)
    };
    !hook.is_null()
}

/// The exe name and monitor of `hwnd`. `None` for a null window, a process that cannot be opened
/// with limited-query access, or an empty image path.
pub fn foreground_info(hwnd: HWND) -> Option<Foreground> {
    if hwnd.is_null() {
        return None;
    }
    let mut pid = 0u32;
    // SAFETY: valid out pointer; a stale handle just leaves pid at 0.
    unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
    let path = process_image_path(pid).filter(|p| !p.is_empty())?;
    let monitor = monitor_of_window(hwnd)?;
    Some(Foreground { exe_name: exe_name_from_path(&path), monitor })
}
