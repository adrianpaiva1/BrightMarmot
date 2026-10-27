//! The shell's large icon for an exe, as RGBA pixels.

use std::ffi::c_void;
use std::mem::{size_of, zeroed};
use std::ptr::null_mut;
use windows_sys::Win32::Graphics::Gdi::{
    BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, DIB_RGB_COLORS, DeleteObject, GetDC, GetDIBits, GetObjectW, HBITMAP,
    HDC, ReleaseDC,
};
use windows_sys::Win32::UI::Shell::{SHFILEINFOW, SHGFI_ICON, SHGFI_LARGEICON, SHGetFileInfoW};
use windows_sys::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, HICON, ICONINFO};

use super::wide;

struct Icon(HICON);
impl Drop for Icon {
    fn drop(&mut self) {
        // SAFETY: icon handle owned by this guard, destroyed once.
        unsafe { DestroyIcon(self.0) };
    }
}

struct Bitmap(HBITMAP);
impl Drop for Bitmap {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: bitmap from GetIconInfo, owned by the caller, deleted once.
            unsafe { DeleteObject(self.0) };
        }
    }
}

struct ScreenDc(HDC);
impl Drop for ScreenDc {
    fn drop(&mut self) {
        // SAFETY: DC from GetDC(null), released once.
        unsafe { ReleaseDC(null_mut(), self.0) };
    }
}

/// `(width, height, straight RGBA)` of the exe's large shell icon (32×32 at 100 % scaling),
/// or `None` if the file has no usable icon. Every handle is released on every path.
pub fn exe_icon_rgba(path: &str) -> Option<(usize, usize, Vec<u8>)> {
    if path.is_empty() {
        return None;
    }
    let path = wide(path);
    // SAFETY: zeroed SHFILEINFOW is valid; size passed matches.
    let icon = unsafe {
        let mut info: SHFILEINFOW = zeroed();
        let ok = SHGetFileInfoW(path.as_ptr(), 0, &mut info, size_of::<SHFILEINFOW>() as u32, SHGFI_ICON | SHGFI_LARGEICON);
        if ok == 0 || info.hIcon.is_null() {
            return None;
        }
        Icon(info.hIcon)
    };

    // SAFETY: zeroed ICONINFO is valid; both bitmaps are owned by us after the call.
    let (color, _mask) = unsafe {
        let mut ii: ICONINFO = zeroed();
        if GetIconInfo(icon.0, &mut ii) == 0 {
            return None;
        }
        (Bitmap(ii.hbmColor), Bitmap(ii.hbmMask))
    };
    if color.0.is_null() {
        return None; // monochrome icon
    }

    // SAFETY: BITMAP buffer of the size passed.
    let (w, h) = unsafe {
        let mut bm: BITMAP = zeroed();
        if GetObjectW(color.0, size_of::<BITMAP>() as i32, (&mut bm as *mut BITMAP).cast::<c_void>()) == 0 {
            return None;
        }
        (bm.bmWidth, bm.bmHeight)
    };
    if !(1..=256).contains(&w) || !(1..=256).contains(&h) {
        return None;
    }

    // SAFETY: screen DC released by the guard.
    let dc = ScreenDc(unsafe { GetDC(null_mut()) });
    if dc.0.is_null() {
        return None;
    }
    let mut px = vec![0u8; (w * h * 4) as usize];
    // SAFETY: px holds w*h 32-bit pixels, matching the top-down 32 bpp header.
    let lines = unsafe {
        let mut bmi: BITMAPINFO = zeroed();
        bmi.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
        bmi.bmiHeader.biWidth = w;
        bmi.bmiHeader.biHeight = -h;
        bmi.bmiHeader.biPlanes = 1;
        bmi.bmiHeader.biBitCount = 32;
        bmi.bmiHeader.biCompression = BI_RGB;
        GetDIBits(dc.0, color.0, 0, h as u32, px.as_mut_ptr().cast(), &mut bmi, DIB_RGB_COLORS)
    };
    if lines != h {
        return None;
    }

    for p in px.chunks_exact_mut(4) {
        p.swap(0, 2); // BGRA -> RGBA
    }
    if px.chunks_exact(4).all(|p| p[3] == 0) {
        for p in px.chunks_exact_mut(4) {
            p[3] = 255; // icon without an alpha channel
        }
    }
    Some((w as usize, h as usize, px))
}
