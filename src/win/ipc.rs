//! Single-instance mutexes, registered messages to the tray window, focusing the
//! Settings window, and relaunching this exe.

use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::{CloseHandle, ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HWND};
use windows_sys::Win32::System::Threading::{
    CreateMutexW, OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    FindWindowExW, FindWindowW, GetWindowThreadProcessId, IsIconic, PostMessageW, RegisterWindowMessageW, SW_RESTORE,
    SetForegroundWindow, ShowWindow,
};

use super::wide;
use crate::config::exe_name_from_path;

pub const TRAY_CLASS: &str = "BrightMarmot.TrayWindow";
pub const SETTINGS_TITLE: &str = "BrightMarmot";
pub const MSG_CONFIG_CHANGED: &str = "BrightMarmot.ConfigChanged";
pub const MSG_OPEN_SETTINGS: &str = "BrightMarmot.OpenSettings";
pub const MUTEX_TRAY: &str = "Local\\BrightMarmot.Tray";
pub const MUTEX_SETTINGS: &str = "Local\\BrightMarmot.Settings";

/// Holds a named mutex for the life of the process; closes it on drop.
pub struct InstanceGuard(HANDLE);

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: handle came from CreateMutexW and is closed once.
            unsafe { CloseHandle(self.0) };
        }
    }
}

/// `None` if another process already holds the mutex `name`.
pub fn single_instance(name: &str) -> Option<InstanceGuard> {
    let name = wide(name);
    // SAFETY: valid NUL-terminated name; GetLastError is read straight after the call.
    let (handle, err) = unsafe {
        let h = CreateMutexW(null(), 0, name.as_ptr());
        (h, GetLastError())
    };
    if err == ERROR_ALREADY_EXISTS || (handle.is_null() && err == ERROR_ACCESS_DENIED) {
        drop(InstanceGuard(handle));
        return None;
    }
    // If the mutex could not be created at all, run anyway rather than refuse to start.
    Some(InstanceGuard(handle))
}

pub fn registered_message(name: &str) -> u32 {
    let name = wide(name);
    // SAFETY: valid NUL-terminated string.
    unsafe { RegisterWindowMessageW(name.as_ptr()) }
}

/// Posts the registered message `msg_name` to the tray process's hidden window.
pub fn notify_tray(msg_name: &str) -> bool {
    let class = wide(TRAY_CLASS);
    // SAFETY: valid class string; null title matches any.
    let hwnd = unsafe { FindWindowW(class.as_ptr(), null()) };
    if hwnd.is_null() {
        return false;
    }
    let msg = registered_message(msg_name);
    // SAFETY: hwnd came from FindWindowW; a stale handle only makes the post fail.
    msg != 0 && unsafe { PostMessageW(hwnd, msg, 0, 0) } != 0
}

/// Restores and brings to front the top-level window titled `title` that belongs to a
/// process running an exe with this exe's file name (so e.g. an Explorer window on a
/// `BrightMarmot` folder is never picked).
pub fn focus_window_titled(title: &str) -> bool {
    let title = wide(title);
    let mut hwnd: HWND = null_mut();
    loop {
        // SAFETY: null parent = top-level windows; continues after the previous match.
        hwnd = unsafe { FindWindowExW(null_mut(), hwnd, null(), title.as_ptr()) };
        if hwnd.is_null() {
            return false;
        }
        if window_runs_this_exe(hwnd) {
            break;
        }
    }
    // SAFETY: hwnd is a window handle from FindWindowExW.
    unsafe {
        if IsIconic(hwnd) != 0 {
            ShowWindow(hwnd, SW_RESTORE);
        }
        SetForegroundWindow(hwnd) != 0
    }
}

fn window_runs_this_exe(hwnd: HWND) -> bool {
    let Some(own) = std::env::current_exe().ok().map(|p| exe_name_from_path(&p.to_string_lossy())) else {
        return false;
    };
    let mut pid = 0u32;
    // SAFETY: valid out pointer.
    unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
    process_image_path(pid).is_some_and(|path| exe_name_from_path(&path).eq_ignore_ascii_case(&own))
}

/// Full exe path of process `pid`, via a limited-query handle only. `None` if it cannot be opened.
pub fn process_image_path(pid: u32) -> Option<String> {
    if pid == 0 {
        return None;
    }
    // SAFETY: limited query access only; handle closed below.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return None;
    }
    let mut buf = [0u16; 1024];
    let mut len = buf.len() as u32;
    // SAFETY: buf holds len UTF-16 units.
    let ok = unsafe { QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len) } != 0;
    // SAFETY: handle from OpenProcess, closed once.
    unsafe { CloseHandle(process) };
    let len = (len as usize).min(buf.len());
    ok.then(|| String::from_utf16_lossy(&buf[..len]))
}

/// Starts this exe again with one argument.
pub fn spawn_self(arg: &str) -> bool {
    std::env::current_exe()
        .and_then(|exe| std::process::Command::new(exe).arg(arg).spawn())
        .is_ok()
}
