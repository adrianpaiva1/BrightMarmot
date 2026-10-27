//! HKCU Run-key helpers, the HKLM `GdiIcmGammaRange` value, and the elevated relaunch.

use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::{CloseHandle, ERROR_SUCCESS, WAIT_OBJECT_0};
use windows_sys::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_QUERY_VALUE, KEY_READ, KEY_SET_VALUE, KEY_WOW64_64KEY,
    REG_DWORD, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegOpenKeyExW,
    RegQueryValueExW, RegSetValueExW,
};
use windows_sys::Win32::System::Threading::{GetExitCodeProcess, INFINITE, WaitForSingleObject};
use windows_sys::Win32::UI::Shell::{SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW};
use windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE;

use super::wide;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "BrightMarmot";
const ICM_KEY: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion\ICM";
const GAMMA_RANGE_VALUE: &str = "GdiIcmGammaRange";
const GAMMA_RANGE_FULL: u32 = 256;

/// Opens (optionally creating) a key; closes it on drop.
struct Key(HKEY);

impl Key {
    fn open(root: HKEY, subkey: &str, access: u32) -> Option<Key> {
        let mut h: HKEY = null_mut();
        // SAFETY: valid NUL-terminated string and out pointer.
        let rc = unsafe { RegOpenKeyExW(root, wide(subkey).as_ptr(), 0, access, &mut h) };
        (rc == ERROR_SUCCESS).then_some(Key(h))
    }

    fn create(root: HKEY, subkey: &str, access: u32) -> Option<Key> {
        let mut h: HKEY = null_mut();
        // SAFETY: valid NUL-terminated string and out pointers.
        let rc = unsafe {
            RegCreateKeyExW(root, wide(subkey).as_ptr(), 0, null(), REG_OPTION_NON_VOLATILE, access, null(), &mut h, null_mut())
        };
        (rc == ERROR_SUCCESS).then_some(Key(h))
    }
}

impl Drop for Key {
    fn drop(&mut self) {
        // SAFETY: handle came from a successful open/create and is closed once.
        unsafe { RegCloseKey(self.0) };
    }
}

pub fn get_run_entry_in(subkey: &str, value: &str) -> Option<String> {
    let key = Key::open(HKEY_CURRENT_USER, subkey, KEY_QUERY_VALUE)?;
    let name = wide(value);
    let mut ty = 0u32;
    let mut len = 0u32;
    // SAFETY: size query with null data.
    let rc = unsafe { RegQueryValueExW(key.0, name.as_ptr(), null(), &mut ty, null_mut(), &mut len) };
    if rc != ERROR_SUCCESS || ty != REG_SZ {
        return None;
    }
    let mut buf = vec![0u16; (len as usize).div_ceil(2) + 1];
    let mut len2 = (buf.len() * 2) as u32;
    // SAFETY: buf holds len2 bytes.
    let rc = unsafe { RegQueryValueExW(key.0, name.as_ptr(), null(), &mut ty, buf.as_mut_ptr().cast(), &mut len2) };
    if rc != ERROR_SUCCESS || ty != REG_SZ {
        return None;
    }
    let units = (len2 as usize / 2).min(buf.len());
    let end = buf[..units].iter().position(|&c| c == 0).unwrap_or(units);
    Some(String::from_utf16_lossy(&buf[..end]))
}

pub fn set_run_entry_in(subkey: &str, value: &str, exe_path: &str) -> bool {
    let Some(key) = Key::create(HKEY_CURRENT_USER, subkey, KEY_SET_VALUE) else {
        return false;
    };
    let data = wide(&format!("\"{exe_path}\""));
    // SAFETY: byte length includes the terminating NUL, as REG_SZ requires.
    let rc = unsafe {
        RegSetValueExW(key.0, wide(value).as_ptr(), 0, REG_SZ, data.as_ptr().cast(), (data.len() * 2) as u32)
    };
    rc == ERROR_SUCCESS
}

pub fn delete_run_entry_in(subkey: &str, value: &str) -> bool {
    let Some(key) = Key::open(HKEY_CURRENT_USER, subkey, KEY_SET_VALUE) else {
        return false;
    };
    // SAFETY: valid key and string.
    unsafe { RegDeleteValueW(key.0, wide(value).as_ptr()) == ERROR_SUCCESS }
}

pub fn get_run_entry() -> Option<String> {
    get_run_entry_in(RUN_KEY, RUN_VALUE)
}

pub fn set_run_entry(exe_path: &str) -> bool {
    set_run_entry_in(RUN_KEY, RUN_VALUE, exe_path)
}

pub fn delete_run_entry() -> bool {
    delete_run_entry_in(RUN_KEY, RUN_VALUE)
}

/// True when HKLM `GdiIcmGammaRange` is a DWORD of at least 256.
pub fn gamma_range_enabled() -> bool {
    let Some(key) = Key::open(HKEY_LOCAL_MACHINE, ICM_KEY, KEY_READ | KEY_WOW64_64KEY) else {
        return false;
    };
    let mut ty = 0u32;
    let mut data = 0u32;
    let mut len = 4u32;
    // SAFETY: data is a 4-byte buffer matching len.
    let rc = unsafe {
        RegQueryValueExW(key.0, wide(GAMMA_RANGE_VALUE).as_ptr(), null(), &mut ty, (&mut data as *mut u32).cast(), &mut len)
    };
    rc == ERROR_SUCCESS && ty == REG_DWORD && len == 4 && data >= GAMMA_RANGE_FULL
}

/// Writes HKLM `GdiIcmGammaRange` = 256. Needs administrator rights.
pub fn write_gamma_range() -> bool {
    let Some(key) = Key::create(HKEY_LOCAL_MACHINE, ICM_KEY, KEY_SET_VALUE | KEY_WOW64_64KEY) else {
        return false;
    };
    let data = GAMMA_RANGE_FULL;
    // SAFETY: 4-byte DWORD.
    let rc = unsafe {
        RegSetValueExW(key.0, wide(GAMMA_RANGE_VALUE).as_ptr(), 0, REG_DWORD, (&data as *const u32).cast(), 4)
    };
    rc == ERROR_SUCCESS
}

/// Relaunches this exe elevated with `--set-gamma-range` and waits. False if UAC is cancelled or it fails.
pub fn request_gamma_range_elevated() -> bool {
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    let file = wide(&exe.to_string_lossy());
    let verb = wide("runas");
    let params = wide("--set-gamma-range");
    // SAFETY: zeroed struct is valid; cbSize and used fields set below, strings outlive the call.
    unsafe {
        let mut info: SHELLEXECUTEINFOW = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
        info.fMask = SEE_MASK_NOCLOSEPROCESS;
        info.lpVerb = verb.as_ptr();
        info.lpFile = file.as_ptr();
        info.lpParameters = params.as_ptr();
        info.nShow = SW_HIDE;
        if ShellExecuteExW(&mut info) == 0 || info.hProcess.is_null() {
            return false;
        }
        let mut code = 1u32;
        let ok = WaitForSingleObject(info.hProcess, INFINITE) == WAIT_OBJECT_0
            && GetExitCodeProcess(info.hProcess, &mut code) != 0;
        CloseHandle(info.hProcess);
        ok && code == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn run_entry_roundtrip_in_test_key() {
        let (k, v) = (r"Software\BrightMarmotTest", "BrightMarmot");
        assert!(set_run_entry_in(k, v, r"C:\Apps\brightmarmot.exe"));
        assert_eq!(get_run_entry_in(k, v).as_deref(), Some(r#""C:\Apps\brightmarmot.exe""#));
        assert!(delete_run_entry_in(k, v));
        assert_eq!(get_run_entry_in(k, v), None);
    }
}
