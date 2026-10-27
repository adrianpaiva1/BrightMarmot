//! The long-lived tray process: single instance, hidden top-level window, tray icon and menu,
//! and the foreground hook that drives the colour engine.

use std::cell::{Cell, RefCell};
use std::mem::size_of;
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Accessibility::HWINEVENTHOOK;
use windows_sys::Win32::UI::Shell::{NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW, Shell_NotifyIconW};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, AppendMenuW, ASFW_ANY, CreateIcon, CreatePopupMenu, CreateWindowExW, DefWindowProcW,
    DestroyIcon, DestroyMenu, DispatchMessageW, GetCursorPos, GetForegroundWindow, GetMessageW, HICON, IDYES,
    KillTimer, MB_ICONINFORMATION, MB_SETFOREGROUND, MB_YESNO, MF_STRING, MSG, MessageBoxW, PostMessageW,
    PostQuitMessage, RegisterClassW, SetForegroundWindow, SetTimer, TPM_BOTTOMALIGN, TPM_RETURNCMD, TPM_RIGHTBUTTON,
    TrackPopupMenu, TranslateMessage, WM_APP, WM_DESTROY, WM_DISPLAYCHANGE, WM_ENDSESSION, WM_LBUTTONDBLCLK, WM_NULL,
    WM_QUERYENDSESSION, WM_RBUTTONUP, WM_TIMER, WNDCLASSW, WS_EX_TOOLWINDOW, WS_OVERLAPPED,
};

use super::display::WinDisplay;
use super::foreground::{foreground_info, install_hook};
use super::ipc::{
    MSG_CONFIG_CHANGED, MSG_OPEN_SETTINGS, MUTEX_TRAY, TRAY_CLASS, notify_tray, registered_message, single_instance,
    spawn_self,
};
use super::{registry, wide};
use crate::config::{self, Config};
use crate::engine::{self, Engine};
use crate::icon::app_icon_rgba;
use crate::restore::{self, RestoreFile};

const WM_TRAY: u32 = WM_APP + 1;
/// Posted by the menu's Exit, so shutdown runs from the top-level message loop rather than
/// inside the menu or the shell's icon callback.
const WM_APP_EXIT: u32 = WM_APP + 2;
/// Posted when an evaluation found the colour state in use (re-entrant call); wParam = arm_timer.
const WM_APP_EVALUATE: u32 = WM_APP + 3;
const ID_SETTINGS: usize = 1;
const ID_EXIT: usize = 2;
const ICON_ID: u32 = 1;
const ICON_SIZE: u32 = 32;
/// One-shot re-apply after a foreground or display change (some games reset the ramp when they go fullscreen).
const REAPPLY_TIMER: usize = 1;
const REAPPLY_MS: u32 = 750;
/// Window title; must never equal the Settings window title (see `ipc::SETTINGS_TITLE`).
const TRAY_TITLE: &str = "BrightMarmot.TrayWindow";
const APP_TITLE: &str = "BrightMarmot";
const GAMMA_PROMPT: &str = "Windows limits how much an app can change your screen's gamma, so stronger Gamma, Contrast and Shadow Boost settings may not work.\n\nBrightMarmot can lift that limit by changing one Windows setting (GdiIcmGammaRange). This needs admin rights once and takes effect after you restart Windows.\n\nChange it now?";

/// Handles and message ids fixed at startup; readable at any time without a borrow.
#[derive(Clone, Copy)]
struct Fixed {
    hwnd: HWND,
    icon: HICON,
    msg_open_settings: u32,
    msg_taskbar_created: u32,
    msg_config_changed: u32,
}

/// Colour state. Never hold the borrow across a call that pumps messages (menu, message box,
/// elevation wait): the foreground hook callback runs from message pumping and borrows it.
struct Colours {
    config_path: Option<PathBuf>,
    config: Config,
    engine: Engine,
    /// The only `WinDisplay` of the process: its "NvAPI failed" flag is per instance.
    display: WinDisplay,
    store: RestoreFile,
}

thread_local! {
    static FIXED: Cell<Option<Fixed>> = const { Cell::new(None) };
    static COLOURS: RefCell<Option<Colours>> = const { RefCell::new(None) };
    /// Set by `shutdown`: from then on nothing is applied and the icon is not re-added.
    static EXITING: Cell<bool> = const { Cell::new(false) };
}

pub fn run() -> i32 {
    let Some(_guard) = single_instance(MUTEX_TRAY) else {
        notify_tray(MSG_OPEN_SETTINGS);
        return 0;
    };

    // 1. Config.
    let config_path = config::default_path();
    let mut config = config_path.as_deref().map(|p| config::load(p).0).unwrap_or_default();

    // 2. Put back colours a previous run left changed (crash, kill, power loss).
    let mut display = WinDisplay::new();
    let mut store = restore::default_path().map(RestoreFile::load).unwrap_or_else(RestoreFile::in_memory);
    engine::recover(&mut store, &mut display);

    // 3. Start with Windows: follow the exe if it moved.
    if config.start_with_windows {
        update_run_entry();
    }

    // 4. One-time offer to lift Windows' gamma limit.
    if !registry::gamma_range_enabled() && !config.gamma_range_prompt_answered {
        ask_gamma_range(&mut config, config_path.as_deref());
    }

    // 5. Window, icon, hook, then apply in case a game is already in front.
    let icon = create_icon();
    if icon.is_null() {
        return 1;
    }
    let hwnd = create_window();
    if hwnd.is_null() {
        // SAFETY: icon from CreateIcon, destroyed once.
        unsafe { DestroyIcon(icon) };
        return 1;
    }
    FIXED.set(Some(Fixed {
        hwnd,
        icon,
        msg_open_settings: registered_message(MSG_OPEN_SETTINGS),
        msg_taskbar_created: registered_message("TaskbarCreated"),
        msg_config_changed: registered_message(MSG_CONFIG_CHANGED),
    }));
    COLOURS.set(Some(Colours { config_path, config, engine: Engine::new(), display, store }));
    add_icon();
    if !install_hook(Some(on_foreground)) {
        eprintln!("BrightMarmot: could not watch the foreground window");
    }
    evaluate(true);

    let mut msg: MSG = unsafe { std::mem::zeroed() };
    // SAFETY: standard message loop on the thread that owns the window.
    unsafe {
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
    // Normally already done by Exit; covers a loop that ended any other way. The window is
    // destroyed with this thread when the process exits.
    shutdown();
    COLOURS.set(None);
    // SAFETY: icon from CreateIcon, no longer used by the (removed) tray icon.
    unsafe { DestroyIcon(icon) };
    0
}

fn update_run_entry() {
    let Ok(exe) = std::env::current_exe() else { return };
    let exe = exe.to_string_lossy();
    if registry::get_run_entry().as_deref() != Some(format!("\"{exe}\"").as_str()) {
        registry::set_run_entry(&exe);
    }
}

/// Asks once; the answer is remembered either way. Runs before the window and hook exist.
fn ask_gamma_range(config: &mut Config, path: Option<&Path>) {
    let (text, title) = (wide(GAMMA_PROMPT), wide(APP_TITLE));
    // SAFETY: no owner window; strings outlive the call. No state borrow is held while it pumps.
    let answer = unsafe {
        MessageBoxW(null_mut(), text.as_ptr(), title.as_ptr(), MB_YESNO | MB_ICONINFORMATION | MB_SETFOREGROUND)
    };
    // Re-read first: Settings may have saved while the box was open.
    if let Some(p) = path {
        *config = config::load(p).0;
    }
    config.gamma_range_prompt_answered = true;
    if let Some(p) = path {
        let _ = config::save(p, config);
    }
    if answer == IDYES {
        registry::request_gamma_range_elevated();
    }
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

fn fixed() -> Option<Fixed> {
    FIXED.try_with(Cell::get).ok().flatten()
}

fn exiting() -> bool {
    EXITING.try_with(Cell::get).unwrap_or(true)
}

fn set_exiting() {
    let _ = EXITING.try_with(|e| e.set(true));
}

/// Runs `f` on the colour state; `None` if it is missing or already borrowed.
fn with_colours<R>(f: impl FnOnce(&mut Colours) -> R) -> Option<R> {
    COLOURS.try_with(|cell| cell.try_borrow_mut().ok().and_then(|mut slot| slot.as_mut().map(f))).ok().flatten()
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
    for (dst, src) in nid.szTip.iter_mut().zip(APP_TITLE.encode_utf16()) {
        *dst = src;
    }
    nid
}

fn add_icon() {
    let Some(f) = fixed() else { return };
    let nid = icon_data(f.hwnd, f.icon);
    // SAFETY: valid NOTIFYICONDATAW.
    unsafe { Shell_NotifyIconW(NIM_ADD, &nid) };
}

fn remove_icon() {
    let Some(f) = fixed() else { return };
    let nid = icon_data(f.hwnd, null_mut());
    // SAFETY: valid NOTIFYICONDATAW.
    unsafe { Shell_NotifyIconW(NIM_DELETE, &nid) };
}

/// Applies or restores for the window in front. When a game's colours were applied and
/// `arm_timer` is set, arms the one-shot re-apply; the timer's own evaluation never re-arms it.
fn evaluate(arm_timer: bool) {
    let Some(f) = fixed() else { return };
    // SAFETY: plain call.
    let fg = foreground_info(unsafe { GetForegroundWindow() });
    // Some(applied), or None when the state is already borrowed (re-entered).
    let outcome = COLOURS.try_with(|cell| match cell.try_borrow_mut() {
        Ok(mut slot) => Some(slot.as_mut().is_some_and(|c| {
            !exiting() && c.engine.update(fg.as_ref(), &c.config, &mut c.display, &mut c.store)
        })),
        Err(_) => None,
    });
    match outcome {
        // SAFETY: timer on our own window; SetTimer with the same id restarts it.
        Ok(Some(true)) if arm_timer => unsafe {
            SetTimer(f.hwnd, REAPPLY_TIMER, REAPPLY_MS, None);
        },
        // Re-entered while the state was in use: evaluate again from the message loop.
        // SAFETY: posting to our own window.
        Ok(None) => unsafe {
            PostMessageW(f.hwnd, WM_APP_EVALUATE, arm_timer as usize, 0);
        },
        _ => {}
    }
}

fn restore_all() {
    with_colours(|c| c.engine.restore_all(&mut c.display, &mut c.store));
}

/// Normal colours back, re-apply timer off, icon removed. Idempotent; nothing is applied afterwards.
fn shutdown() {
    if exiting() {
        return;
    }
    set_exiting();
    if let Some(f) = fixed() {
        // SAFETY: our own window's timer; harmless if it is not set.
        unsafe { KillTimer(f.hwnd, REAPPLY_TIMER) };
    }
    restore_all();
    remove_icon();
}

fn reload_config() {
    with_colours(|c| {
        if let Some(p) = &c.config_path {
            c.config = config::load(p).0;
        }
    });
}

unsafe extern "system" fn on_foreground(_: HWINEVENTHOOK, _: u32, _: HWND, _: i32, _: i32, _: u32, _: u32) {
    evaluate(true);
}

fn open_settings() {
    // SAFETY: plain call.
    unsafe { AllowSetForegroundWindow(ASFW_ANY) };
    spawn_self("--settings");
}

fn show_menu(hwnd: HWND) {
    // SAFETY: menu created, tracked and destroyed here; no state borrow is held while the menu pumps messages.
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
    let (open_msg, created_msg, config_msg) =
        fixed().map_or((0, 0, 0), |f| (f.msg_open_settings, f.msg_taskbar_created, f.msg_config_changed));
    match msg {
        WM_TRAY => {
            if !exiting() {
                match (lparam as u32) & 0xFFFF {
                    WM_LBUTTONDBLCLK => open_settings(),
                    WM_RBUTTONUP => show_menu(hwnd),
                    _ => {}
                }
            }
            0
        }
        WM_APP_EXIT => {
            shutdown();
            // Leave the loop without DestroyWindow; the window goes with the thread at process exit.
            // SAFETY: plain call.
            unsafe { PostQuitMessage(0) };
            0
        }
        WM_APP_EVALUATE => {
            evaluate(wparam != 0);
            0
        }
        WM_TIMER if wparam == REAPPLY_TIMER => {
            // SAFETY: our own window's timer.
            unsafe { KillTimer(hwnd, REAPPLY_TIMER) };
            evaluate(false);
            0
        }
        WM_DISPLAYCHANGE => {
            evaluate(true);
            0
        }
        WM_QUERYENDSESSION => {
            restore_all();
            1
        }
        WM_ENDSESSION => {
            if wparam != 0 {
                set_exiting();
                restore_all();
            } else {
                // Shutdown was cancelled after WM_QUERYENDSESSION restored: re-apply for the window in front.
                evaluate(false);
            }
            0
        }
        WM_DESTROY => {
            // Only reached if something else destroys the window (e.g. WM_CLOSE).
            shutdown();
            // SAFETY: plain call.
            unsafe { PostQuitMessage(0) };
            0
        }
        m if m != 0 && m == open_msg => {
            if !exiting() {
                open_settings();
            }
            0
        }
        m if m != 0 && m == created_msg => {
            if !exiting() {
                add_icon();
            }
            0
        }
        m if m != 0 && m == config_msg => {
            reload_config();
            evaluate(false);
            0
        }
        // SAFETY: forwarding unhandled messages.
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}
