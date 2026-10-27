# BrightMarmot — Design

Date: 2026-10-26
Status: draft, awaiting approval

## 1. What this is, and why

BrightMarmot (working name) is a small Windows tray app that gives each PC game
its own screen colours. You add a game, set its gamma / contrast / vibrance /
hue / shadow boost, and whenever that game is the window in front, its colours
are applied to the monitor it is on. Alt-Tab away or close the game, and the
monitor goes back to your normal colours.

It is a deliberately stripped-down clone of
[BrightRaider](https://github.com/BrightRaider/BrightRaider) (closed source; the
repo holds only docs and binaries, so this is a from-scratch build). BrightRaider
has grown a large feature set: hotkey presets, FPS limits, HDR, audio tools,
overlays, auto-brightness, game macros, licensing. The user wants only its
per-game colour profiles, done in a way that costs almost nothing in the
background and stays safe to run alongside online games with anti-cheat.

The user's reference for the look and behaviour is BrightRaider's Game Profiles
page and Add Game dialog (screenshots shared in the brainstorming session; the
relevant details are written out in §5).

## 2. Goals

1. Per-game colour profiles that apply automatically while the game is in front,
   and revert automatically when it is not.
2. Five colour controls per game, each with its own on/off switch: Gamma,
   Contrast, Vibrance, Hue, Shadow Boost.
3. Add a game either from a list of currently running applications or by picking
   an `.exe` from disk.
4. A "Start with Windows" option the user can turn on or off.
5. Very lightweight in the background: no polling, minimal memory, idle CPU at 0%.
6. Safe with anti-cheat: nothing that touches, reads, injects into, or sends input
   to any game.
7. NVIDIA GPUs only need to be supported (for vibrance and hue).

## 3. Non-goals (explicitly out)

- Hotkeys or hotkey presets of any kind (the user chose per-game only).
- FPS limiting, HDR switching, or any write to NVIDIA driver profiles (DRS).
- Audio features, crosshair/overlays, auto-brightness, macros/input automation,
  map scanners, process priority tweaks, break reminders.
- AMD or Intel vibrance/hue support.
- Licensing, update checks, telemetry — the app makes no network requests at all
  (my call).
- An installer or code signing. It is a single portable `.exe` for personal use
  (my call).
- Live preview of colours on the desktop while editing (my call — see §5.4).

## 4. Anti-goals (would be a failure even if it "works")

- **Anything that looks like a cheat to anti-cheat.** Hard rules for the code:
  - No `SetWindowsHookEx` of any kind (no keyboard/mouse/CBT hooks).
  - `SetWinEventHook` only with `WINEVENT_OUTOFCONTEXT` (no DLL loaded into other
    processes).
  - `OpenProcess` only with `PROCESS_QUERY_LIMITED_INFORMATION`. Never read or
    write another process's memory, never inject, never enumerate modules of a
    game process.
  - Never send input or window messages to other applications.
  - No overlay windows, no drivers, no kernel components.
  - Display changes only through: Windows GDI gamma ramp (`SetDeviceGammaRamp`)
    and NVIDIA NvAPI display-colour calls (digital vibrance, hue).
- **A heavy background process.** No timers or polling loops while idle, no GUI
  framework or GPU context alive in the background process.
- **Leaving the screen stuck in game colours** after an Alt-Tab, a game exit, an
  app exit, an app crash, or a logoff/shutdown.
- **Feature creep** toward BrightRaider's extras.

## 5. Behaviour

### 5.1 Processes

One executable, two modes (my call):

- **Tray mode** (`brightmarmot.exe`, no arguments): the long-lived background
  process. Owns the tray icon, watches the foreground window, applies and
  restores colours. Contains no GUI framework at runtime — only a hidden
  message-only window, the tray icon, and Win32 `MessageBox` for the one-time
  prompt in §5.7.
- **Settings mode** (`brightmarmot.exe --settings`): the settings window. Started
  by the tray process on demand; exits when the window is closed, so it uses no
  memory when not open.

Single instance:
- Launching `brightmarmot.exe` while the tray process is already running opens
  Settings (via the running instance) instead of starting a second tray process.
- Opening Settings while a Settings window already exists focuses that window.

Reason for two processes: keeps the background process tiny, and egui/winit can
only create one event loop per process, so a closable-and-reopenable window
within the tray process is awkward.

### 5.2 Tray icon

- Double-click: open Settings.
- Right-click menu: **Settings**, **Exit**. Nothing else.
- Tooltip: "BrightMarmot".
- Re-add the icon if Explorer restarts (handle the `TaskbarCreated` message).

### 5.3 Applying colours (the core loop)

1. Tray process registers `SetWinEventHook(EVENT_SYSTEM_FOREGROUND, …,
   WINEVENT_OUTOFCONTEXT)` and otherwise sleeps in its message loop.
2. On a foreground change: get the window's process ID → `OpenProcess`
   (limited) → `QueryFullProcessImageNameW` → take the file name.
3. Match the file name, case-insensitively, against the configured games'
   `exe_name`. Matching is by file name only, not full path (my call — so it
   works regardless of which launcher or install location starts the game).
4. **If it matches a game:**
   - Find the monitor the game window is on (`MonitorFromWindow`, nearest).
   - If no game colours are currently active on that monitor, capture that
     monitor's current "normal" values first (§5.5).
   - Build the target values. Vibrance and hue: the game's value if its switch
     is on, otherwise the captured normal value. Gamma ramp: the game's curve
     layered on top of the captured normal ramp (§5.6); if all three ramp
     switches (gamma, contrast, shadow boost) are off, don't touch the ramp.
   - Apply the ramp on that monitor; apply vibrance and hue through NvAPI on that
     monitor. Skip any part whose switches are all off.
   - Re-apply once, shortly after (≈0.5–1 s, one-shot timer), because some games
     reset the gamma ramp when they go fullscreen. This is a single delayed
     re-apply, not polling.
5. **If it does not match** (or the foreground window is not a game in the list):
   restore normal values on any monitor that currently has game colours.
6. Re-apply the active game's colours on `WM_DISPLAYCHANGE` (resolution or mode
   changes reset gamma ramps).
7. Only the monitor the game is on is changed; other monitors are left alone
   (my call).

When a game exits, Windows moves the foreground to another window, which goes
through step 5 — no process watching needed.

If the game window moves to a different monitor while in front: builder's call
whether to handle it (e.g. `EVENT_SYSTEM_MOVESIZEEND` or
`EVENT_OBJECT_LOCATIONCHANGE` filtered to that window). Full-screen games rarely
move, so it is acceptable to only re-evaluate on the next foreground change.

### 5.4 Editing settings

- Every change in the Settings window is saved immediately (no Save button).
- The tray process picks up config changes without polling — builder's call
  between watching the config directory (`ReadDirectoryChangesW` /
  `FindFirstChangeNotificationW`) or the Settings process notifying the tray
  process's hidden window with a registered window message.
- No live preview on the desktop: while Settings is open, Settings is the window
  in front, so the game's colours are not applied. The user sees new values the
  next time they Alt-Tab into the game (my call, to keep it simple).

### 5.5 "Normal" colours and crash recovery

- "Normal" values for a monitor = whatever gamma ramp, vibrance and hue it has
  **right before** game colours are first applied to it (my call, refining "what
  the screen had when the app started" — this way changes the user makes in the
  NVIDIA Control Panel while the app is running are respected).
- Before applying any game colours, write the captured normal values for that
  monitor to a restore file (§6). Delete the entry once restored.
- On startup, if the restore file has entries, a previous run did not restore
  properly (crash, kill, power loss): apply those saved normal values first, then
  clear the file, then continue as usual.
- Restore on: non-matching foreground (§5.3), Exit, `WM_ENDSESSION`
  (logoff/shutdown), and `WM_QUERYENDSESSION`.
- Normal vibrance/hue are stored in the driver's raw units, not the UI scale, so
  restoring is exact.

### 5.6 Colour controls

Ranges below are taken from the user's BrightRaider screenshot (slider positions
match these ranges).

| Control | UI range | Neutral | Step / display | Implemented via |
|---|---|---|---|---|
| Gamma | 0.50 – 3.00 | 1.00 | 0.01, two decimals | gamma ramp |
| Contrast | 0.50 – 2.00 | 1.00 | 0.01, two decimals | gamma ramp |
| Shadow Boost | 0 – 100 | 0 | 1, integer | gamma ramp |
| Vibrance | 0 – 100 | 50 | 1, integer | NvAPI digital vibrance |
| Hue | 0 – 359 | 0 | 1, integer (degrees) | NvAPI hue angle |

- **Gamma** brightens/darkens midtones (values > 1 brighten). **Contrast** scales
  around mid-grey. **Shadow Boost** lifts the darkest tones more than the
  midtones, to make things in shadow visible without washing out the whole
  image.
- **Layered on the normal ramp** (my call): the captured normal ramp is often not
  the identity ramp (monitor calibration loaded at login, Night Light, f.lux) and
  may differ per channel. The game's three controls define one curve
  `g: [0,1] → [0,1]` (a switched-off control is neutral in `g`), and each
  channel's output is the captured normal ramp for that channel evaluated at
  `g(x)` (interpolated). So a calibrated or tinted screen keeps its calibration
  or tint while the game is in front, and switched-off controls really do keep
  their normal state. The exact formula for `g` is the builder's call, with
  these constraints: `g` is monotonic non-decreasing and clamped to [0,1],
  neutral values give `g(x) = x` (output = the captured ramp unchanged), and
  final values are clamped to 0–65535.
- **Vibrance** uses the NVIDIA Control Panel's percentage scale: 50 = driver
  default, 100 = maximum. The builder maps this to the driver's raw range as
  reported by the NvAPI DVC info call.
- When a control's switch is off, its slider and number field are greyed out and
  the normal value for that control is used.
- New games start with every switch off and every value at neutral (my call).

### 5.7 Windows' gamma limit

Windows refuses gamma ramps that deviate far from the identity ramp unless
`HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\ICM` has
`GdiIcmGammaRange` (DWORD) = `256`. Writing it needs admin, and it takes effect
after a restart.

- On first run, if the value is missing or not 256, the tray process shows a
  plain message box explaining what it is, why it is needed, that a one-time
  admin prompt follows, and that a restart is needed.
- On "Yes": relaunch the exe elevated (`ShellExecuteW` with `runas`) with a
  dedicated argument (e.g. `--set-gamma-range`); that process writes the value
  and exits. The main process never runs elevated.
- On "No": remember that the user was asked (don't ask again on every start) and
  continue. Gamma changes beyond what Windows allows will fail.
- The Settings window shows the status ("Full gamma range: enabled / not enabled")
  with a button to run the same elevation step later (my call).
- If `SetDeviceGammaRamp` fails, do not crash or retry in a loop; vibrance/hue
  still apply.

### 5.8 Start with Windows

- A switch in Settings, off by default (my call).
- On: write `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` value
  `BrightMarmot` = `"<full path to exe>"`. Off: delete that value. No admin.
- On tray startup, if the setting is on and the stored path differs from the
  current exe path (the exe was moved), rewrite it.

### 5.9 No NVIDIA GPU / no NvAPI on that monitor

- If `nvapi64.dll` cannot be loaded, or the monitor has no associated NVIDIA
  display handle (e.g. a laptop where the integrated GPU drives the panel),
  vibrance and hue are skipped for that monitor; gamma/contrast/shadow boost
  still work.
- The vibrance and hue calls are undocumented (§8.1), so a driver update could
  remove or change them. If resolving a function ID returns nothing, or a get/set
  call returns an error, treat vibrance/hue as unavailable for that run: skip
  them, log nothing noisy, never crash or retry in a loop. Gamma still works.
- The Settings window greys out the Vibrance and Hue rows with a short note when
  NvAPI or these calls are unavailable on this machine.

## 6. Files and storage

- Config: `%APPDATA%\BrightMarmot\config.json` (my call: JSON). Contents:
  - `start_with_windows` (bool)
  - `gamma_range_prompt_answered` (bool)
  - `games`: list of
    - `exe_name` (e.g. `cod.exe`; unique, case-insensitive — adding a duplicate is
      refused)
    - `path` (full path when known; shown in the UI and used for the icon)
    - `name` (display name = exe file name without `.exe`; not editable, my call)
    - `gamma`, `contrast`, `shadow_boost`, `vibrance`, `hue`: each
      `{ "enabled": bool, "value": number }`
- Restore file: `%LOCALAPPDATA%\BrightMarmot\restore.json` — per monitor device
  name: saved gamma ramp, raw vibrance level, raw hue angle.
- A missing or unreadable config file is treated as empty, never as a crash; a
  corrupt one is renamed aside (e.g. `config.json.bad`) and replaced with an
  empty config.
- Writes are atomic (write temp file, then rename).

## 7. Settings window

Dark theme matching the BrightRaider screenshots: near-black background, dark
grey panels with rounded corners, white text, grey secondary text, orange accent
(≈ `#F5A623`) for active sliders and switches.

### 7.1 Main window layout

- **Left panel:** list of added games, each row = exe icon + name; selected row
  highlighted. Below the list: **+ Add game** and **Remove** buttons. Below
  those, an app section: **Start with Windows** switch and the full gamma range
  status/button from §5.7.
- **Right panel (selected game):**
  - Header: exe icon, name (bold), full path (secondary text).
  - Section title "Own colours for this game", subtitle "Each value you switch on
    replaces your normal value while this game is in front."
  - Five rows — Gamma, Contrast, Vibrance, Hue, Shadow Boost — each with: label,
    slider, numeric entry box (typing a value updates the slider, clamped to the
    range), on/off switch.
- **Empty state** (no games): right panel says to add a game, with an Add game
  button.
- Remove: builder's call whether to confirm.
- Icons: extracted from the exe (e.g. `SHGetFileInfoW` / `ExtractIconExW`); fall
  back to a generic icon if extraction fails (e.g. permission-restricted install
  folders).

### 7.2 Add Game dialog

A separate window or modal, matching the second screenshot:

- Title "Add Game", heading "Running applications".
- A list of running applications that have a visible top-level window. Each row:
  exe icon, window title (bold), process name (secondary text, no `.exe`).
  - Include: visible, unowned, non-tool, non-cloaked top-level windows with a
    non-empty title.
  - One row per exe (dedupe).
  - Exclude BrightMarmot itself and games already added.
- **Refresh** button re-enumerates the list.
- **Pick .exe from disk…** opens a native file-open dialog filtered to `*.exe`.
  Picking a file adds that game.
- **Cancel** and **Add** buttons; Add is disabled until a row is selected.
  Double-clicking a row also adds it (my call).
- After adding, the dialog closes and the new game is selected in the main
  window.
- If an exe is already in the list, it is not added again; the existing entry is
  selected instead.

## 8. How it gets made

### 8.1 Decided

- **Language:** Rust (stable).
- **Target:** `x86_64-pc-windows-gnu`, cross-compiled from WSL (Ubuntu 24.04)
  using `mingw-w64`. No Visual Studio or MSVC Build Tools. The user installs
  `rustup` and the `mingw-w64` apt package; nothing on the Windows side.
- **Win32 access:** the `windows` (or `windows-sys`) crate.
- **NvAPI:** load `nvapi64.dll` at runtime with `LoadLibraryW`, resolve the
  exported `nvapi_QueryInterface`, and get function pointers by ID. No NVIDIA
  SDK needed at build time.
  - Initialize and display-handle functions (`NvAPI_Initialize`,
    `NvAPI_EnumNvidiaDisplayHandle`, `NvAPI_GetAssociatedNvidiaDisplayHandle`):
    take IDs and layouts from NVIDIA's public NVAPI SDK headers (`nvapi.h`,
    `nvapi_interface.h` in the `NVIDIA/nvapi` GitHub repo).
  - Digital vibrance (`NvAPI_GetDVCInfo(Ex)`, `NvAPI_SetDVCLevel(Ex)`) and hue
    (`NvAPI_GetHUEInfo`, `NvAPI_SetHUEAngle`) are **not** in NVIDIA's public
    headers; they are undocumented calls the driver exports. These are the same
    calls VibranceGUI and similar tools use. Take their IDs and struct layouts
    from a well-known open-source project (e.g. NvAPIWrapper, cross-checked
    against vibranceGUI) as a reference only, without copying code, and verify
    them on the user's machine: get the current value, set it, read it back,
    restore it.
  - Only these families are allowed: initialize, enumerate/associate display
    handles, get/set DVC (digital vibrance), get/set hue.
- **Settings UI:** egui via eframe, only in settings mode.
- **Output:** one `brightmarmot.exe`.

### 8.2 Left to the builder

- Tray icon via raw Win32 (`Shell_NotifyIconW`) or a crate — but the tray process
  must not pull up winit/eframe or a GPU context.
- eframe renderer (glow vs wgpu); prefer whichever gives the smaller, more
  reliable result.
- File dialog implementation (`IFileOpenDialog` directly or a crate such as
  `rfd`).
- Config-change notification mechanism (§5.4).
- Gamma ramp formula (within §5.6 constraints).
- Crate layout and module structure.
- An app icon (simple; anything reasonable).

### 8.3 Development and testing

- Keep pure logic (ramp math, vibrance scale mapping, config parse/serialize,
  exe-name matching, window filtering rules) free of Win32 calls so it unit-tests
  with plain `cargo test` in WSL.
- Windows-specific code can be exercised from WSL by running the built `.exe`
  (WSL can launch Windows executables directly).
- The exe must be copied to a Windows path before turning on Start with Windows —
  a `\\wsl.localhost\…` path won't work at login. Provide a small script (e.g.
  `scripts/install.sh`) that builds release and copies the exe to a Windows
  folder such as `%LOCALAPPDATA%\Programs\BrightMarmot\`.
- Manual test checklist on the user's real Windows + NVIDIA machine:
  - Add a game from the running list and from disk; duplicates refused.
  - Alt-Tab into the game → colours change on its monitor only; Alt-Tab out →
    normal colours return; close the game → normal colours return.
  - Only switched-on controls change; switched-off controls keep normal values.
  - Kill the tray process from Task Manager while a game is in front → screen
    stays changed; start the app again → normal colours restored.
  - Exit from tray while a game is in front → normal colours restored.
  - Start with Windows on/off → registry value written/removed; app starts at
    login.
  - Full gamma range prompt: Yes path (UAC, value written) and No path (not asked
    again).
  - Idle resource check (below).

## 9. Resource targets

Measured in Task Manager / Process Explorer with the tray process idle and
Settings closed:

- CPU: 0% when nothing is changing; no periodic wake-ups (only the one-shot
  re-apply timer after a foreground change).
- Memory: aim for under ~10 MB private working set for the tray process.
- Settings process: exists only while the window is open.

## 10. Known limitations and risks

- **HDR displays:** while Windows HDR is on for a monitor, Windows ignores the
  GDI gamma ramp as far as we know, so gamma/contrast/shadow boost won't take
  effect there; vibrance should. Document it; don't work around it.
- **Games that reset gamma:** some exclusive-fullscreen games set their own
  gamma ramp; the one-shot re-apply (§5.3) is the mitigation. If a specific game
  still resets it, that is a follow-up, not a reason to add polling.
- **Other gamma tools** (f.lux, DisplayCAL loader, Night Light) may interact;
  normal-value capture means their state is what gets restored.
- **Undocumented NVIDIA calls:** vibrance and hue rely on undocumented NvAPI
  functions (§8.1). They have been stable for years and other tools depend on
  them, but a driver update could break them; the app then greys those two
  controls out (§5.9) rather than failing.
- **Full gamma range needs a restart** after the registry change.
- **Antivirus:** with no keyboard hook, false positives are less likely than with
  BrightRaider, but an unsigned exe may still be flagged occasionally.
