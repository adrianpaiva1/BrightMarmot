//! The long-lived tray process: single instance, hidden top-level window, tray icon and menu,
//! and the foreground hook that drives the colour engine.

use std::cell::{Cell, RefCell};
use std::mem::size_of;
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Accessibility::HWINEVENTHOOK;
use windows_sys::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext};
use windows_sys::Win32::UI::Shell::{NIF_ICON, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW, Shell_NotifyIconW};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, AppendMenuW, ASFW_ANY, CreatePopupMenu, CreateWindowExW, DefWindowProcW,
    DestroyIcon, DestroyMenu, DispatchMessageW, GetCursorPos, GetForegroundWindow, GetMessageW, GetSystemMetrics, HICON, IDI_APPLICATION, IDYES, IMAGE_ICON, LoadIconW, LoadImageW,
    KillTimer, MB_ICONINFORMATION, MB_ICONWARNING, MB_OK, MB_SETFOREGROUND, MB_YESNO, MF_STRING, MSG, MessageBoxW,
    PostMessageW, PostQuitMessage, RegisterClassW, SetForegroundWindow, SetTimer, SM_CXSMICON, SM_CYSMICON, TPM_BOTTOMALIGN, TPM_RETURNCMD,
    TPM_RIGHTBUTTON, TrackPopupMenu, TranslateMessage, WM_APP, WM_DESTROY, WM_DISPLAYCHANGE, WM_ENDSESSION,
    WM_LBUTTONDBLCLK, WM_NULL, WM_QUERYENDSESSION, WM_RBUTTONUP, WM_TIMER, WNDCLASSW, WS_EX_TOOLWINDOW,
    WS_OVERLAPPED,
};

use super::display::WinDisplay;
use super::foreground::{foreground_info, install_hook};
use super::ipc::{
    MSG_CONFIG_CHANGED, MSG_OPEN_SETTINGS, MUTEX_TRAY, TRAY_CLASS, notify_tray, registered_message, single_instance,
    spawn_self,
};
use super::{registry, wide};
use crate::config::{self, Config, LoadOutcome};
use crate::engine::{self, Engine};
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
/// Icon resource ID in the exe (assets/brightmarmot.rc).
const APP_ICON_RES: usize = 1;
/// One-shot re-apply after a foreground or display change (some games reset the ramp when they go fullscreen).
const REAPPLY_TIMER: usize = 1;
const REAPPLY_MS: u32 = 750;
/// Window title; must never equal the Settings window title (see `ipc::SETTINGS_TITLE`).
const TRAY_TITLE: &str = "BrightMarmot.TrayWindow";
const APP_TITLE: &str = "BrightMarmot";
const HOOK_FAILED: &str = "BrightMarmot couldn't watch for games, so colours won't switch. Try restarting it.";
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
    /// Set by `WM_QUERYENDSESSION`, cleared if the session end is cancelled: nothing is applied meanwhile.
    static SUSPENDED: Cell<bool> = const { Cell::new(false) };
}

pub fn run() -> i32 {
    // Before any window exists, so the tray icon is loaded at the right size for the screen's DPI. Failure is fine.
    // SAFETY: plain call.
    unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    let Some(_guard) = single_instance(MUTEX_TRAY) else {
        notify_tray(MSG_OPEN_SETTINGS);
        return 0;
    };

    // 1. Config.
    let config_path = config::default_path();
    let (mut config, outcome) = config_path.as_deref().map_or((Config::default(), LoadOutcome::Missing), config::load);

    // 2. Put back colours a previous run left changed (crash, kill, power loss).
    let mut display = WinDisplay::new();
    let mut store = restore::default_path().map(RestoreFile::load).unwrap_or_else(RestoreFile::in_memory);
    engine::recover(&mut store, &mut display);

    // 3. Start with Windows: follow the exe if it moved.
    if config.start_with_windows {
        update_run_entry();
    }

    // 4. Say so if a corrupt config was moved aside.
    if matches!(outcome, LoadOutcome::Corrupt) {
        tell_user(config::CORRUPT_NOTICE);
    }

    // 5. One-time offer to lift Windows' gamma limit.
    if !registry::gamma_range_enabled() && !config.gamma_range_prompt_answered {
        ask_gamma_range(&mut config, config_path.as_deref());
    }

    // 6. Window, icon, hook, then apply in case a game is already in front.
    let icon = load_icon();
    let hwnd = create_window();
    if hwnd.is_null() {
        // SAFETY: icon from LoadImageW/LoadIconW, destroyed once (harmless no-op for null or the shared stock icon).
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
        // No COLOURS borrow is held here; window messages dispatched by the box may borrow it.
        tell_user(HOOK_FAILED);
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
    // SAFETY: icon from LoadImageW/LoadIconW, no longer used by the (removed) tray icon.
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
    // Re-read first: Settings may have saved while the box was open. Only a file that was read
    // or is absent is adopted and written back; one that couldn't be read is never written over.
    let save_to = match path.map(config::load) {
        Some((c, LoadOutcome::Loaded | LoadOutcome::Missing)) => {
            *config = c;
            path
        }
        _ => None,
    };
    config.gamma_range_prompt_answered = true;
    if let Some(p) = save_to {
        let _ = config::save(p, config);
    }
    if answer == IDYES {
        registry::request_gamma_range_elevated();
    }
}

/// OK box with a warning icon and no owner. It pumps messages: never call it with a `COLOURS` borrow held.
fn tell_user(text: &str) {
    let (text, title) = (wide(text), wide(APP_TITLE));
    // SAFETY: no owner window; strings outlive the call.
    unsafe { MessageBoxW(null_mut(), text.as_ptr(), title.as_ptr(), MB_OK | MB_ICONWARNING | MB_SETFOREGROUND) };
}

/// The exe's embedded icon at the small-icon size for the current DPI; the stock application icon
/// if that fails, and null (no icon) if even that does.
fn load_icon() -> HICON {
    // SAFETY: resource id passed as MAKEINTRESOURCE; the returned icon is destroyed once at exit.
    unsafe {
        let icon = LoadImageW(
            GetModuleHandleW(null()),
            APP_ICON_RES as *const u16,
            IMAGE_ICON,
            GetSystemMetrics(SM_CXSMICON),
            GetSystemMetrics(SM_CYSMICON),
            0,
        );
        if icon.is_null() { LoadIconW(null_mut(), IDI_APPLICATION) } else { icon as HICON }
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

fn suspended() -> bool {
    SUSPENDED.try_with(Cell::get).unwrap_or(true)
}

fn set_suspended(on: bool) {
    let _ = SUSPENDED.try_with(|s| s.set(on));
}

/// Runs `f` on the colour state; `None` if it is missing or already borrowed.
///
/// Invariant: no `COLOURS` borrow spans a message-pumping call, so a borrow can only fail if a
/// callback re-enters from inside a borrow, which the code never allows. `evaluate` still
/// defers itself on a failed borrow; `restore_all` and `reload_config` are no-ops then.
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
    // Some(applied), or None when the state is already borrowed (re-entered; see `with_colours`).
    let outcome = COLOURS.try_with(|cell| match cell.try_borrow_mut() {
        Ok(mut slot) => Some(slot.as_mut().is_some_and(|c| {
            !exiting() && !suspended() && c.engine.update(fg.as_ref(), &c.config, &mut c.display, &mut c.store)
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

fn kill_reapply_timer() {
    if let Some(f) = fixed() {
        // SAFETY: our own window's timer; harmless if it is not set.
        unsafe { KillTimer(f.hwnd, REAPPLY_TIMER) };
    }
}

/// Normal colours back, re-apply timer off, icon removed. Idempotent; nothing is applied afterwards.
/// The WinEvent hook is not unhooked here: the system frees it when this thread exits.
fn shutdown() {
    if exiting() {
        return;
    }
    set_exiting();
    kill_reapply_timer();
    restore_all();
    remove_icon();
}

/// Adopts the file only if it was read or is absent. If it couldn't be read, or was corrupt
/// (and has been moved aside), the last good config stays in use.
fn reload_config() {
    with_colours(|c| {
        if let Some(p) = &c.config_path {
            match config::load(p) {
                (loaded, LoadOutcome::Loaded | LoadOutcome::Missing) => c.config = loaded,
                (_, LoadOutcome::Unreadable | LoadOutcome::Corrupt) => {}
            }
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
            // Hold off the re-apply timer and hook until WM_ENDSESSION says whether the session ends.
            set_suspended(true);
            kill_reapply_timer();
            restore_all();
            1
        }
        WM_ENDSESSION => {
            if wparam != 0 {
                // Logoff/shutdown, or Restart Manager closing us (ENDSESSION_CLOSEAPP): exit like Exit does.
                shutdown();
                // SAFETY: plain call.
                unsafe { PostQuitMessage(0) };
            } else {
                // Cancelled after WM_QUERYENDSESSION restored: resume and re-apply for the window in front.
                set_suspended(false);
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
