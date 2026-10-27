//! The long-lived tray process: single instance, hidden top-level window, tray icon and menu.

use std::cell::RefCell;
use std::mem::size_of;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Shell::{NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW, Shell_NotifyIconW};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, AppendMenuW, ASFW_ANY, CreateIcon, CreatePopupMenu, CreateWindowExW, DefWindowProcW,
    DestroyIcon, DestroyMenu, DispatchMessageW, GetCursorPos, GetMessageW, HICON, MF_STRING, MSG,
    PostMessageW, PostQuitMessage, RegisterClassW, SetForegroundWindow, TPM_BOTTOMALIGN, TPM_RETURNCMD, TPM_RIGHTBUTTON,
    TrackPopupMenu, TranslateMessage, WM_APP, WM_DESTROY, WM_LBUTTONDBLCLK, WM_NULL, WM_RBUTTONUP, WNDCLASSW,
    WS_EX_TOOLWINDOW, WS_OVERLAPPED,
};

use super::ipc::{MSG_OPEN_SETTINGS, MUTEX_TRAY, TRAY_CLASS, notify_tray, registered_message, single_instance, spawn_self};
use super::wide;
use crate::icon::app_icon_rgba;

const WM_TRAY: u32 = WM_APP + 1;
/// Posted by the menu's Exit, so shutdown runs from the top-level message loop rather than
/// inside the menu or the shell's icon callback.
const WM_APP_EXIT: u32 = WM_APP + 2;
const ID_SETTINGS: usize = 1;
const ID_EXIT: usize = 2;
const ICON_ID: u32 = 1;
const ICON_SIZE: u32 = 32;
/// Window title; must never equal the Settings window title (see `ipc::SETTINGS_TITLE`).
const TRAY_TITLE: &str = "BrightMarmot.TrayWindow";

/// Per-process tray state. Never keep the borrow across a message-pumping call.
#[derive(Default)]
struct State {
    icon: HICON,
    msg_open_settings: u32,
    msg_taskbar_created: u32,
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
}

pub fn run() -> i32 {
    let Some(_guard) = single_instance(MUTEX_TRAY) else {
        notify_tray(MSG_OPEN_SETTINGS);
        return 0;
    };
    let icon = create_icon();
    if icon.is_null() {
        return 1;
    }
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.icon = icon;
        s.msg_open_settings = registered_message(MSG_OPEN_SETTINGS);
        s.msg_taskbar_created = registered_message("TaskbarCreated");
    });
    let hwnd = create_window();
    if hwnd.is_null() {
        // SAFETY: icon from CreateIcon, destroyed once.
        unsafe { DestroyIcon(icon) };
        return 1;
    }
    add_icon(hwnd);

    let mut msg: MSG = unsafe { std::mem::zeroed() };
    // SAFETY: standard message loop on the thread that owns the window.
    unsafe {
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        DestroyIcon(icon);
    }
    0
}

/// 32-bpp icon from the generated RGBA (converted to BGRA); alpha carries the shape.
fn create_icon() -> HICON {
    let bgra: Vec<u8> = app_icon_rgba(ICON_SIZE).chunks_exact(4).flat_map(|p| [p[2], p[1], p[0], p[3]]).collect();
    // AND mask: 1 bit per pixel, rows padded to 16 bits; all zero = opaque (alpha is used for 32 bpp).
    let mask = vec![0u8; (ICON_SIZE as usize).div_ceil(16) * 2 * ICON_SIZE as usize];
    // SAFETY: buffers match the declared dimensions and are copied by CreateIcon.
    unsafe {
        CreateIcon(GetModuleHandleW(null()), ICON_SIZE as i32, ICON_SIZE as i32, 1, 32, mask.as_ptr(), bgra.as_ptr())
    }
}

fn create_window() -> HWND {
    let class = wide(TRAY_CLASS);
    let title = wide(TRAY_TITLE);
    // SAFETY: valid class description; strings outlive the calls. Top-level (not HWND_MESSAGE) and never shown.
    unsafe {
        let hinst = GetModuleHandleW(null());
        let wc = WNDCLASSW {
            style: 0,
            lpfnWndProc: Some(wndproc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinst,
            hIcon: null_mut(),
            hCursor: null_mut(),
            hbrBackground: null_mut(),
            lpszMenuName: null(),
            lpszClassName: class.as_ptr(),
        };
        if RegisterClassW(&wc) == 0 {
            return null_mut();
        }
        CreateWindowExW(WS_EX_TOOLWINDOW, class.as_ptr(), title.as_ptr(), WS_OVERLAPPED, 0, 0, 0, 0, null_mut(), null_mut(), hinst, null())
    }
}

fn icon_data(hwnd: HWND, icon: HICON) -> NOTIFYICONDATAW {
    // SAFETY: NOTIFYICONDATAW is plain data; all-zero is valid.
    let mut nid: NOTIFYICONDATAW = unsafe { std::mem::zeroed() };
    nid.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
    nid.hWnd = hwnd;
    nid.uID = ICON_ID;
    nid.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
    nid.uCallbackMessage = WM_TRAY;
    nid.hIcon = icon;
    for (dst, src) in nid.szTip.iter_mut().zip("BrightMarmot".encode_utf16()) {
        *dst = src;
    }
    nid
}

fn add_icon(hwnd: HWND) {
    let icon = STATE.with(|s| s.borrow().icon);
    let nid = icon_data(hwnd, icon);
    // SAFETY: valid NOTIFYICONDATAW.
    unsafe { Shell_NotifyIconW(NIM_ADD, &nid) };
}

fn remove_icon(hwnd: HWND) {
    let nid = icon_data(hwnd, null_mut());
    // SAFETY: valid NOTIFYICONDATAW.
    unsafe { Shell_NotifyIconW(NIM_DELETE, &nid) };
}

fn open_settings() {
    // SAFETY: plain call.
    unsafe { AllowSetForegroundWindow(ASFW_ANY) };
    spawn_self("--settings");
}

fn show_menu(hwnd: HWND) {
    // SAFETY: menu created, tracked and destroyed here; no STATE borrow is held while the menu pumps messages.
    let cmd = unsafe {
        let menu = CreatePopupMenu();
        if menu.is_null() {
            return;
        }
        AppendMenuW(menu, MF_STRING, ID_SETTINGS, wide("Settings").as_ptr());
        AppendMenuW(menu, MF_STRING, ID_EXIT, wide("Exit").as_ptr());
        let mut pt = POINT { x: 0, y: 0 };
        GetCursorPos(&mut pt);
        SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenu(menu, TPM_RIGHTBUTTON | TPM_BOTTOMALIGN | TPM_RETURNCMD, pt.x, pt.y, 0, hwnd, null());
        PostMessageW(hwnd, WM_NULL, 0, 0);
        DestroyMenu(menu);
        cmd
    };
    match cmd as usize {
        ID_SETTINGS => open_settings(),
        // SAFETY: posting to our own window.
        ID_EXIT => unsafe {
            PostMessageW(hwnd, WM_APP_EXIT, 0, 0);
        },
        _ => {}
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // try_with/try_borrow: never panic (e.g. messages during creation or teardown).
    let (open_msg, created_msg) = STATE
        .try_with(|s| s.try_borrow().map(|s| (s.msg_open_settings, s.msg_taskbar_created)).unwrap_or((0, 0)))
        .unwrap_or((0, 0));
    match msg {
        WM_TRAY => {
            match (lparam as u32) & 0xFFFF {
                WM_LBUTTONDBLCLK => open_settings(),
                WM_RBUTTONUP => show_menu(hwnd),
                _ => {}
            }
            0
        }
        WM_APP_EXIT => {
            remove_icon(hwnd);
            // Leave the loop without DestroyWindow; the window goes with the thread at process exit.
            // SAFETY: plain call.
            unsafe { PostQuitMessage(0) };
            0
        }
        WM_DESTROY => {
            // Only reached if something else destroys the window (e.g. WM_CLOSE).
            remove_icon(hwnd);
            // SAFETY: plain call.
            unsafe { PostQuitMessage(0) };
            0
        }
        m if m != 0 && m == open_msg => {
            open_settings();
            0
        }
        m if m != 0 && m == created_msg => {
            add_icon(hwnd);
            0
        }
        // SAFETY: forwarding unhandled messages.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}
