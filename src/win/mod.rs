pub mod display;
pub mod exe_icon;
pub mod gamma;
pub mod ipc;
pub mod nvapi;
pub mod registry;
pub mod windows_list;
#[cfg(debug_assertions)]
pub mod probe;

/// NUL-terminated UTF-16 for Win32 `W` functions.
pub(crate) fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}
