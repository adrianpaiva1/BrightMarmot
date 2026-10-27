# BrightMarmot Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A tiny Windows tray app that applies per-game gamma / contrast / shadow boost / vibrance / hue while a game is in front, and restores the user's normal colours otherwise.

**Architecture:** One Rust binary, three modes: tray (default), `--settings` (egui window, own short-lived process), `--set-gamma-range` (elevated one-shot registry write). All decision logic (config, ramp maths, vibrance mapping, restore file, the apply/restore engine) is platform-independent and unit-tested on Linux; thin `win/` modules wrap Win32 and NvAPI behind a `DisplayBackend` trait.

**Tech Stack:** Rust stable (edition 2024), target `x86_64-pc-windows-gnu` cross-compiled from WSL Ubuntu 24.04 with `mingw-w64`; `serde` + `serde_json`; `windows-sys`; `eframe`/`egui` (glow renderer); `rfd` (file dialog); `tempfile` (dev).

**Spec:** `docs/superpowers/specs/2026-10-26-brightmarmot-design.md` — read it before starting any task.

## Global Constraints

- Build target `x86_64-pc-windows-gnu`, linker `x86_64-w64-mingw32-gcc`. No MSVC, no Visual Studio.
- Single executable `brightmarmot.exe`. Args: none → tray; `--settings` → settings window; `--set-gamma-range` → write registry value and exit; `--probe` → hardware probe (debug builds only).
- `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]` — debug builds are console apps so output is visible from WSL.
- Pure modules (`config`, `ramp`, `vibrance`, `restore`, `engine`, `apps`, `icon`) must not use `windows-sys` or `#[cfg(windows)]`; plain `cargo test` on Linux runs them. Windows-only code lives in `src/win/` and `src/settings/` behind `#[cfg(windows)]`; `eframe`, `rfd`, `windows-sys` are `[target.'cfg(windows)'.dependencies]`.
- Anti-cheat rules (spec §4), verbatim intent: no `SetWindowsHookEx`; `SetWinEventHook` only with `WINEVENT_OUTOFCONTEXT`; `OpenProcess` only with `PROCESS_QUERY_LIMITED_INFORMATION`; never read/write other processes' memory, inject, enumerate a game's modules, or send input/messages to other apps; no overlays, drivers, or DRS writes. Display changes only via `SetDeviceGammaRamp` and NvAPI DVC/hue.
- No network access anywhere.
- Tray process: no eframe/winit/GPU context, no polling; the only timer is the one-shot re-apply (750 ms) after a foreground change or display change.
- Config: `%APPDATA%\BrightMarmot\config.json`. Restore file: `%LOCALAPPDATA%\BrightMarmot\restore.json`. Both written atomically (temp file + rename).
- Ranges (UI order Gamma, Contrast, Vibrance, Hue, Shadow Boost): Gamma 0.50–3.00 neutral 1.00 (2 dp); Contrast 0.50–2.00 neutral 1.00 (2 dp); Vibrance 0–100 neutral 50; Hue 0–359 neutral 0; Shadow Boost 0–100 neutral 0.
- Registry: Run key `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, value `BrightMarmot` = `"<exe path>"`. Gamma range `HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\ICM`, DWORD `GdiIcmGammaRange` = 256.
- Theme: near-black background `#1B1C1F`, panels `#26272B` with border `#36373C`, text `#F0F0F0`, secondary text `#A0A0A8`, entry boxes `#111214`, accent `#F5A623`.
- Every commit message ends with the trailer `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- Steps marked **(user)** need the user at their Windows desktop; ask them and wait for their answer.

## Review Focus

1. **Hand-edited or partly broken config** (missing fields, out-of-range values, case-only duplicate exe names) → loads with neutral defaults, clamped values, first duplicate kept; never crashes. *(Task 1 tests `missing_fields_get_neutral_defaults`, `out_of_range_values_are_clamped`, `case_duplicates_and_empty_exe_dropped`.)*
2. **Malformed restore file** (truncated JSON, ramp not 3×256) → treated as empty / bad entry dropped; app starts normally. *(Task 3 tests `truncated_json_is_empty`, `bad_ramp_shape_entry_dropped`.)*
3. **Windows rejects the gamma ramp** (full gamma range off) → vibrance and hue still apply, ramp not marked changed, nothing retried in a loop. *(Task 4 test `rejected_ramp_still_applies_vibrance`.)*
4. **Foreground jumps straight from one game to another** (other monitor, or same monitor with different switches) → first game's changes fully undone where the second doesn't override them; normals never re-captured from game colours. *(Task 4 tests `switch_game_same_monitor_resets_unused_parts`, `switch_to_game_on_other_monitor_restores_first`.)*
5. **A restore call fails** (monitor unplugged, driver reset) → monitor stays tracked and its saved normals stay on disk; retried on the next foreground change, no loop. *(Task 4 test `failed_restore_is_retried_next_update`.)*

---

## File Structure

```
Cargo.toml, .cargo/config.toml, .gitignore, README.md, scripts/install.sh
src/main.rs            mode dispatch
src/config.rs          Config/Game/Control model, sanitize, load/save, matching      (pure)
src/ramp.rs            GammaRamp, curve, layered ramp                                 (pure)
src/vibrance.rs        VibranceInfo, UI ↔ raw mapping                                 (pure)
src/restore.rs         RestoreFile: saved normals per monitor                         (pure)
src/engine.rs          DisplayBackend trait, Engine apply/restore, recover()         (pure)
src/apps.rs            filter window list → running apps for Add Game                 (pure)
src/icon.rs            generated app icon RGBA                                        (pure)
src/win/mod.rs
src/win/gamma.rs       monitors, monitor of window, get/set gamma ramp
src/win/nvapi.rs       NvAPI loader, vibrance, hue
src/win/display.rs     WinDisplay: DisplayBackend; nvidia_colour_available()
src/win/probe.rs       debug-only hardware probe
src/win/registry.rs    Run key, gamma range, elevation
src/win/ipc.rs         mutexes, registered messages, find/notify windows, spawn self
src/win/foreground.rs  foreground hook + Foreground of a window
src/win/windows_list.rs  enumerate top-level windows → WindowInfo
src/win/exe_icon.rs    exe icon → RGBA
src/win/tray.rs        hidden window, tray icon, menu, message loop, engine wiring
src/settings/mod.rs    run(), SettingsApp, save + notify
src/settings/theme.rs  visuals, colour consts, toggle switch widget
src/settings/game_view.rs  right panel for one game
src/settings/add_game.rs   Add Game modal
```

---

### Task 1: Toolchain, crate skeleton, config model

**Files:**
- Create: `Cargo.toml`, `.cargo/config.toml`, `.gitignore`, `src/main.rs`, `src/config.rs`

**Interfaces:**
- Produces (`src/config.rs`):
  - `#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)] pub struct Control { pub enabled: bool, pub value: f64 }`
  - `#[derive(Clone, Copy, Debug, PartialEq, Eq)] pub enum ControlKind { Gamma, Contrast, Vibrance, Hue, ShadowBoost }` with `pub const ALL: [ControlKind; 5]` (that UI order), `pub fn range(self) -> (f64, f64)`, `pub fn neutral(self) -> f64`, `pub fn decimals(self) -> usize`, `pub fn label(self) -> &'static str` ("Gamma", "Contrast", "Vibrance", "Hue", "Shadow Boost")
  - `#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)] pub struct Game { pub exe_name: String, pub path: String, pub name: String, pub gamma: Control, pub contrast: Control, pub shadow_boost: Control, pub vibrance: Control, pub hue: Control }` (field names exactly as spec §6)
  - `impl Game { pub fn new(path: &str) -> Game; pub fn control(&self, k: ControlKind) -> Control; pub fn control_mut(&mut self, k: ControlKind) -> &mut Control; pub fn uses_ramp(&self) -> bool }`
  - `#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)] pub struct Config { pub start_with_windows: bool, pub gamma_range_prompt_answered: bool, pub games: Vec<Game> }`
  - `#[derive(Debug, PartialEq, Eq)] pub enum AddOutcome { Added(usize), AlreadyExists(usize) }`
  - `impl Config { pub fn find_game(&self, exe_name: &str) -> Option<&Game>; pub fn add_game(&mut self, path: &str) -> AddOutcome; pub fn remove_game(&mut self, index: usize); pub fn from_json(s: &str) -> Result<Config, serde_json::Error>; pub fn to_json(&self) -> String }`
  - `pub fn exe_name_from_path(path: &str) -> String`
  - `pub enum LoadOutcome { Loaded, Missing, Corrupt }`
  - `pub fn load(path: &Path) -> (Config, LoadOutcome)`; `pub fn save(path: &Path, config: &Config) -> std::io::Result<()>`; `pub fn default_path() -> Option<PathBuf>`
  - `pub(crate) fn write_atomic(path: &Path, contents: &str) -> std::io::Result<()>` (creates parent dirs, writes `<name>.tmp`, renames) — reused by Task 3.

- [ ] **Step 1: Install the toolchain**

Run: `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal && . "$HOME/.cargo/env" && rustup target add x86_64-pc-windows-gnu`
**(user)** Ask the user to run `! sudo apt-get install -y mingw-w64` (needs their sudo password).
Expected: `cargo --version` prints a version; `x86_64-w64-mingw32-gcc --version` prints a version.

- [ ] **Step 2: Create the crate**

`git init`; `cargo init --name brightmarmot` (edition 2024); `cargo add serde --features derive`; `cargo add serde_json`; `cargo add --dev tempfile`. `.gitignore`: `/target`. `.cargo/config.toml`:

```toml
[target.x86_64-pc-windows-gnu]
linker = "x86_64-w64-mingw32-gcc"
```

`Cargo.toml` release profile: `opt-level = "s"`, `lto = true`, `codegen-units = 1`, `strip = true`, `panic = "abort"`.
`src/main.rs`: the `windows_subsystem` attribute from Global Constraints, `#![cfg_attr(not(windows), allow(dead_code))]`, `mod config;`, a `#[cfg(windows)] fn main()` that exits 0 for now, and `#[cfg(not(windows))] fn main()` that prints `BrightMarmot runs on Windows only.` and exits 1.

- [ ] **Step 3: Write the failing tests** (in `src/config.rs`, `#[cfg(test)] mod tests`)

```rust
#[test] fn exe_name_from_windows_and_unix_paths() {
    assert_eq!(exe_name_from_path(r"C:\XboxGames\Call of Duty\Content\cod.exe"), "cod.exe");
    assert_eq!(exe_name_from_path("D:/Games/bf6.exe"), "bf6.exe");
    assert_eq!(exe_name_from_path("bf6.exe"), "bf6.exe");
}
#[test] fn new_game_defaults() {
    let g = Game::new(r"C:\Games\COD.EXE");
    assert_eq!((g.exe_name.as_str(), g.name.as_str(), g.path.as_str()), ("COD.EXE", "COD", r"C:\Games\COD.EXE"));
    for k in ControlKind::ALL { assert!(!g.control(k).enabled); assert_eq!(g.control(k).value, k.neutral()); }
    assert_eq!((g.gamma.value, g.contrast.value, g.vibrance.value, g.hue.value, g.shadow_boost.value), (1.0, 1.0, 50.0, 0.0, 0.0));
}
#[test] fn uses_ramp_only_for_gamma_contrast_shadow() {
    let mut g = Game::new("a.exe"); g.vibrance.enabled = true; g.hue.enabled = true;
    assert!(!g.uses_ramp());
    g.shadow_boost.enabled = true; assert!(g.uses_ramp());
}
#[test] fn find_game_is_case_insensitive() {
    let mut c = Config::default(); c.add_game(r"C:\x\cod.exe");
    assert!(c.find_game("COD.EXE").is_some());
    assert!(c.find_game("bf6.exe").is_none());
}
#[test] fn add_duplicate_returns_existing() {
    let mut c = Config::default();
    assert_eq!(c.add_game(r"C:\a\cod.exe"), AddOutcome::Added(0));
    assert_eq!(c.add_game(r"D:\b\COD.exe"), AddOutcome::AlreadyExists(0));
    assert_eq!(c.games.len(), 1);
}
#[test] fn json_roundtrip() {
    let mut c = Config::default(); c.start_with_windows = true; c.add_game(r"C:\a\cod.exe");
    c.games[0].gamma = Control { enabled: true, value: 1.3 };
    assert_eq!(Config::from_json(&c.to_json()).unwrap(), c);
}
#[test] fn missing_fields_get_neutral_defaults() {
    let c = Config::from_json(r#"{"games":[{"exe_name":"cod.exe"}]}"#).unwrap();
    let g = &c.games[0];
    assert!(!c.start_with_windows && !c.gamma_range_prompt_answered);
    assert_eq!((g.name.as_str(), g.path.as_str()), ("cod", ""));
    assert_eq!((g.gamma.value, g.vibrance.value, g.gamma.enabled), (1.0, 50.0, false));
}
#[test] fn out_of_range_values_are_clamped() {
    let c = Config::from_json(r#"{"games":[{"exe_name":"a.exe",
        "gamma":{"enabled":true,"value":99},"contrast":{"enabled":true,"value":0.1},
        "hue":{"enabled":true,"value":400},"vibrance":{"enabled":true,"value":-5}}]}"#).unwrap();
    let g = &c.games[0];
    assert_eq!((g.gamma.value, g.contrast.value, g.hue.value, g.vibrance.value), (3.0, 0.5, 359.0, 0.0));
}
#[test] fn case_duplicates_and_empty_exe_dropped() {
    let c = Config::from_json(r#"{"games":[{"exe_name":"cod.exe","path":"first"},
        {"exe_name":"COD.EXE","path":"second"},{"exe_name":""}]}"#).unwrap();
    assert_eq!(c.games.len(), 1);
    assert_eq!(c.games[0].path, "first");
}
#[test] fn load_missing_file() {
    let d = tempfile::tempdir().unwrap();
    let (c, o) = load(&d.path().join("config.json"));
    assert_eq!(c, Config::default()); assert!(matches!(o, LoadOutcome::Missing));
}
#[test] fn load_corrupt_renames_aside() {
    let d = tempfile::tempdir().unwrap(); let p = d.path().join("config.json");
    std::fs::write(&p, "not json").unwrap();
    let (c, o) = load(&p);
    assert_eq!(c, Config::default()); assert!(matches!(o, LoadOutcome::Corrupt));
    assert!(!p.exists());
    assert_eq!(std::fs::read_to_string(d.path().join("config.json.bad")).unwrap(), "not json");
}
#[test] fn save_creates_dirs_and_leaves_no_temp() {
    let d = tempfile::tempdir().unwrap(); let p = d.path().join("sub").join("config.json");
    let mut c = Config::default(); c.add_game("bf6.exe");
    save(&p, &c).unwrap();
    assert_eq!(load(&p).0, c);
    assert!(!d.path().join("sub").join("config.json.tmp").exists());
}
```

- [ ] **Step 4: Run tests to verify they fail**

Run: `cargo test`
Expected: compile errors for the undefined items.

- [ ] **Step 5: Implement `src/config.rs`**

`exe_name_from_path` splits on both `\` and `/` (std `Path` on Linux does not treat `\` as a separator). `name` = exe file name minus a case-insensitive `.exe` suffix. Missing `Control` fields default to that control's neutral via per-field `#[serde(default = "...")]`. `from_json` then sanitizes: drop games with empty `exe_name`, drop later case-insensitive duplicates, fill empty `name` from `exe_name`, clamp each value into its range. `load`: missing → `Missing`; read error or parse error → rename to `config.json.bad` (overwriting) and return `Corrupt`. `default_path` = `%APPDATA%\BrightMarmot\config.json` from the `APPDATA` env var.

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test` — Expected: all config tests PASS.
Run: `cargo build --target x86_64-pc-windows-gnu && cargo test --target x86_64-pc-windows-gnu` — Expected: `target/x86_64-pc-windows-gnu/debug/brightmarmot.exe` exists, and the same tests PASS when run on Windows through WSL interop. (Task 7 relies on this path.)

- [ ] **Step 7: Commit**

`git add -A && git commit -m "feat: crate skeleton and config model"` (includes `docs/`).

---

### Task 2: Colour maths (gamma ramp curve, vibrance mapping)

**Files:**
- Create: `src/ramp.rs`, `src/vibrance.rs`; Modify: `src/main.rs` (add `mod`s)

**Interfaces:**
- Consumes: `Game`, `Control` (Task 1)
- Produces:
  - `pub type GammaRamp = [[u16; 256]; 3];` (R, G, B — same layout `SetDeviceGammaRamp` expects)
  - `pub fn identity() -> GammaRamp` (entry `i` = `i * 257`)
  - `#[derive(Clone, Copy, Debug, PartialEq)] pub struct CurveParams { pub gamma: f64, pub contrast: f64, pub shadow_boost: f64 }` with `pub const NEUTRAL: CurveParams` (1.0, 1.0, 0.0) and `pub fn from_game(g: &Game) -> CurveParams` (switched-off controls → neutral)
  - `pub fn curve(p: &CurveParams, x: f64) -> f64`
  - `pub fn layered(normal: &GammaRamp, p: &CurveParams) -> GammaRamp`
  - `#[derive(Clone, Copy, Debug, PartialEq)] pub struct VibranceInfo { pub current: i32, pub min: i32, pub max: i32, pub default: i32 }`
  - `pub fn ui_to_raw(ui: f64, info: &VibranceInfo) -> i32`

- [ ] **Step 1: Write the failing tests**

`src/ramp.rs`:
```rust
fn p(g: f64, c: f64, s: f64) -> CurveParams { CurveParams { gamma: g, contrast: c, shadow_boost: s } }
#[test] fn identity_endpoints() { let r = identity(); assert_eq!((r[0][0], r[1][128], r[2][255]), (0, 128 * 257, 65535)); }
#[test] fn neutral_curve_is_identity() { for i in 0..=10 { let x = i as f64 / 10.0; assert!((curve(&CurveParams::NEUTRAL, x) - x).abs() < 1e-12); } }
#[test] fn gamma_two_brightens_quarter_to_half() { assert!((curve(&p(2.0, 1.0, 0.0), 0.25) - 0.5).abs() < 1e-9); }
#[test] fn contrast_two_scales_around_mid() {
    let q = p(1.0, 2.0, 0.0);
    assert!((curve(&q, 0.75) - 1.0).abs() < 1e-9); assert!(curve(&q, 0.25).abs() < 1e-9); assert!((curve(&q, 0.6) - 0.7).abs() < 1e-9);
}
#[test] fn shadow_boost_lifts_darks_keeps_endpoints() {
    let q = p(1.0, 1.0, 100.0);
    assert!((curve(&q, 0.05) - 0.14025).abs() < 1e-9); assert_eq!(curve(&q, 0.0), 0.0); assert!((curve(&q, 1.0) - 1.0).abs() < 1e-12);
}
#[test] fn curve_monotonic_and_bounded_at_extremes() {
    for g in [0.5, 3.0] { for c in [0.5, 2.0] { for s in [0.0, 100.0] {
        let mut prev = -1.0;
        for i in 0..256 { let y = curve(&p(g, c, s), i as f64 / 255.0); assert!((0.0..=1.0).contains(&y) && y >= prev); prev = y; }
    }}}
}
fn odd_normal() -> GammaRamp {
    let mut r = [[0u16; 256]; 3];
    for i in 0..256 { r[0][i] = (i * 257) as u16; r[1][i] = (i * 200) as u16; r[2][i] = 30000; }
    r
}
#[test] fn layered_neutral_returns_normal_exactly() { assert_eq!(layered(&odd_normal(), &CurveParams::NEUTRAL), odd_normal()); }
#[test] fn layered_gamma_on_identity() { let r = layered(&identity(), &p(2.0, 1.0, 0.0)); assert!((r[0][64] as i32 - 32832).abs() <= 1); }
#[test] fn layered_follows_each_normal_channel() { let r = layered(&odd_normal(), &p(2.0, 1.0, 0.0)); assert!(r[2].iter().all(|&v| v == 30000)); }
#[test] fn from_game_uses_only_switched_on() {
    let mut g = crate::config::Game::new("a.exe");
    g.gamma = crate::config::Control { enabled: true, value: 2.0 };
    g.contrast = crate::config::Control { enabled: false, value: 1.8 };
    assert_eq!(CurveParams::from_game(&g), p(2.0, 1.0, 0.0));
}
```
`src/vibrance.rs`:
```rust
#[test] fn maps_positive_only_range() {
    let i = VibranceInfo { current: 0, min: 0, max: 63, default: 0 };
    assert_eq!([0.0, 50.0, 75.0, 100.0, 120.0, -5.0].map(|u| ui_to_raw(u, &i)), [0, 0, 32, 63, 63, 0]);
}
#[test] fn maps_symmetric_range() {
    let i = VibranceInfo { current: 0, min: -50, max: 50, default: 0 };
    assert_eq!([0.0, 25.0, 50.0, 100.0].map(|u| ui_to_raw(u, &i)), [-50, -25, 0, 50]);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test ramp vibrance` — Expected: compile errors.

- [ ] **Step 3: Implement `curve` (this formula is pinned by the tests), `layered`, `identity`, `from_game` in `src/ramp.rs`**

```rust
let mut y = x.powf(1.0 / p.gamma);
let s = 2.0 * p.shadow_boost / 100.0;          // ≤ 2 keeps the curve monotonic
y += s * y * (1.0 - y) * (1.0 - y);
y = (y - 0.5) * p.contrast + 0.5;
y.clamp(0.0, 1.0)
```
`layered`: for each `i`, `pos = curve(i/255) * 255`; snap `pos` to the nearest integer when within `1e-6`; linearly interpolate each channel of `normal` between `floor(pos)` and `min(floor+1, 255)`; round; clamp to `0..=65535`.

- [ ] **Step 4: Implement `ui_to_raw` in `src/vibrance.rs`**

Clamp `ui` to 0–100; 0–50 maps linearly to `min..=default`, 50–100 to `default..=max`; round to nearest.

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test` — Expected: all PASS.

- [ ] **Step 6: Commit** — `git add src/ramp.rs src/vibrance.rs src/main.rs && git commit -m "feat: gamma ramp curve and vibrance mapping"`

---

### Task 3: Restore file

**Files:**
- Create: `src/restore.rs`; Modify: `src/main.rs`

**Interfaces:**
- Consumes: `GammaRamp` (Task 2), `config::write_atomic` (Task 1)
- Produces:
  - `#[derive(Clone, Debug, PartialEq)] pub struct SavedNormal { pub ramp: Option<GammaRamp>, pub vibrance: Option<i32>, pub hue: Option<u32> }`
  - `pub struct RestoreFile` with `pub fn load(path: PathBuf) -> RestoreFile`, `pub fn in_memory() -> RestoreFile` (save is a no-op), `pub fn entries(&self) -> &BTreeMap<String, SavedNormal>`, `pub fn insert(&mut self, monitor: &str, saved: SavedNormal)`, `pub fn remove(&mut self, monitor: &str)`, `pub fn clear(&mut self)`, `pub fn save(&self) -> std::io::Result<()>`, `pub fn default_path() -> Option<PathBuf>` (`%LOCALAPPDATA%\BrightMarmot\restore.json`)
- On disk: JSON object keyed by monitor device name (e.g. `\\.\DISPLAY1`), each `{ "ramp": [[256 u16] × 3] | null, "vibrance": i32 | null, "hue": u32 | null }` (serde can't derive for `[u16; 256]`; use a private serde struct with `Vec<Vec<u16>>` and convert).

- [ ] **Step 1: Write the failing tests**

```rust
fn sample() -> SavedNormal { SavedNormal { ramp: Some(crate::ramp::identity()), vibrance: Some(-3), hue: None } }
#[test] fn roundtrip_save_load() {
    let d = tempfile::tempdir().unwrap(); let p = d.path().join("restore.json");
    let mut f = RestoreFile::load(p.clone()); f.insert(r"\\.\DISPLAY1", sample()); f.save().unwrap();
    assert_eq!(RestoreFile::load(p).entries().get(r"\\.\DISPLAY1"), Some(&sample()));
}
#[test] fn save_when_empty_deletes_file() {
    let d = tempfile::tempdir().unwrap(); let p = d.path().join("restore.json");
    let mut f = RestoreFile::load(p.clone()); f.insert("m", sample()); f.save().unwrap();
    f.clear(); f.save().unwrap(); assert!(!p.exists());
}
#[test] fn missing_file_is_empty() { let d = tempfile::tempdir().unwrap(); assert!(RestoreFile::load(d.path().join("x.json")).entries().is_empty()); }
#[test] fn truncated_json_is_empty() {
    let d = tempfile::tempdir().unwrap(); let p = d.path().join("restore.json");
    std::fs::write(&p, r#"{"\\\\.\\DISPLAY1": {"ramp": [[1,2"#).unwrap();
    assert!(RestoreFile::load(p).entries().is_empty());
}
#[test] fn bad_ramp_shape_entry_dropped() {
    let d = tempfile::tempdir().unwrap(); let p = d.path().join("restore.json");
    std::fs::write(&p, r#"{"bad": {"ramp": [[1,2,3],[1],[1]], "vibrance": 1, "hue": null},
                          "good": {"ramp": null, "vibrance": 7, "hue": 10}}"#).unwrap();
    let f = RestoreFile::load(p);
    assert_eq!(f.entries().len(), 1);
    assert_eq!(f.entries()["good"], SavedNormal { ramp: None, vibrance: Some(7), hue: Some(10) });
}
```

- [ ] **Step 2: Run tests to verify they fail** — `cargo test restore` → compile errors.
- [ ] **Step 3: Implement `src/restore.rs`** — `save` with entries → `write_atomic`; with no entries → remove the file (ignore NotFound).
- [ ] **Step 4: Run tests to verify they pass** — `cargo test` → all PASS.
- [ ] **Step 5: Commit** — `git add src/restore.rs src/main.rs && git commit -m "feat: restore file for saved normal colours"`

---

### Task 4: Engine (apply / restore state machine)

**Files:**
- Create: `src/engine.rs`; Modify: `src/main.rs`

**Interfaces:**
- Consumes: `Config::find_game`, `Game::uses_ramp` (Task 1); `GammaRamp`, `CurveParams`, `layered`, `VibranceInfo`, `ui_to_raw` (Task 2); `RestoreFile`, `SavedNormal` (Task 3)
- Produces:
  - `#[derive(Clone, Debug, PartialEq)] pub struct Foreground { pub exe_name: String, pub monitor: String }` (`monitor` = GDI device name, e.g. `\\.\DISPLAY1`)
  - `pub trait DisplayBackend { fn get_ramp(&mut self, monitor: &str) -> Option<GammaRamp>; fn set_ramp(&mut self, monitor: &str, ramp: &GammaRamp) -> bool; fn get_vibrance(&mut self, monitor: &str) -> Option<VibranceInfo>; fn set_vibrance(&mut self, monitor: &str, raw: i32) -> bool; fn get_hue(&mut self, monitor: &str) -> Option<u32>; fn set_hue(&mut self, monitor: &str, degrees: u32) -> bool; }`
  - `pub struct Engine` with `pub fn new() -> Engine`, `pub fn update(&mut self, fg: Option<&Foreground>, config: &Config, backend: &mut dyn DisplayBackend, store: &mut RestoreFile) -> bool` (returns true when a game matched), `pub fn restore_all(&mut self, backend: &mut dyn DisplayBackend, store: &mut RestoreFile)`, `pub fn active_monitors(&self) -> Vec<String>`
  - `pub fn recover(store: &mut RestoreFile, backend: &mut dyn DisplayBackend)`

**Rules `update` implements** (spec §5.3, §5.5, §5.6):
1. Target = `fg` whose `exe_name` matches a game → `(fg.monitor, game)`; otherwise none.
2. Restore every active monitor other than the target's monitor.
3. For the target monitor, if it is not active: capture `SavedNormal { ramp: get_ramp, vibrance: get_vibrance(..).map(|i| i.current), hue: get_hue }`, keep the `VibranceInfo` in memory, `store.insert` + `store.save()` (ignore the error), and mark the monitor active with nothing modified. If it is already active, never re-capture.
4. Ramp: if `game.uses_ramp()` and the normal ramp is known → `set_ramp(layered(normal, from_game(game)))`, success ⇒ ramp modified. Else, if the ramp is modified → `set_ramp(normal)`, success ⇒ not modified. Unknown normal ramp ⇒ never touch the ramp.
5. Vibrance: same pattern with `ui_to_raw(game.vibrance.value, info)` when `game.vibrance.enabled` and the info is known. Hue: same pattern with `game.hue.value.round()` clamped to 0–359.
6. Restoring a monitor sets every modified part back to normal. Only if every set succeeded: remove it from active and `store.remove` + `store.save()`. Otherwise it stays active (succeeded parts unmarked) and is retried on the next `update`.
7. `recover`: for every store entry set each `Some` part, then `store.clear()` + `store.save()` regardless of failures.

- [ ] **Step 1: Write the failing tests** (`#[cfg(test)] mod tests` with a fake backend)

```rust
const D1: &str = r"\\.\DISPLAY1"; const D2: &str = r"\\.\DISPLAY2";
#[derive(Default)] struct Mon { ramp: Option<GammaRamp>, vib: Option<VibranceInfo>, hue: Option<u32>,
    reject_ramp: bool, reject_all: bool, ramp_gets: usize, ramp_sets: usize, hue_sets: usize }
#[derive(Default)] struct Fake { m: BTreeMap<String, Mon> }
// impl DisplayBackend for Fake: get_* return the Mon's field (counting ramp_gets);
// set_* first count the call (ramp_sets / hue_sets count every attempt, rejected or not),
// then return false if reject_all (or reject_ramp for set_ramp), else store the value
// (set_vibrance writes vib.current) and return true.
fn normal() -> GammaRamp { let mut r = crate::ramp::identity(); r[1] = [20000; 256]; r }
fn info() -> VibranceInfo { VibranceInfo { current: 5, min: 0, max: 63, default: 0 } }
fn fake(mons: &[&str]) -> Fake { /* each monitor: ramp Some(normal()), vib Some(info()), hue Some(0) */ }
fn game(exe: &str, gamma: Option<f64>, vib: Option<f64>, hue: Option<f64>) -> Game { /* Game::new + enabled controls */ }
fn fg(exe: &str, mon: &str) -> Foreground { Foreground { exe_name: exe.into(), monitor: mon.into() } }
fn cfg(games: Vec<Game>) -> Config { Config { games, ..Config::default() } }

#[test] fn game_in_front_applies_switched_on_controls() {
    let (mut f, mut s, mut e) = (fake(&[D1]), RestoreFile::in_memory(), Engine::new());
    let c = cfg(vec![game("cod.exe", Some(2.0), Some(70.0), None)]);
    assert!(e.update(Some(&fg("COD.EXE", D1)), &c, &mut f, &mut s));
    let m = &f.m[D1];
    assert_eq!(m.ramp, Some(layered(&normal(), &CurveParams { gamma: 2.0, contrast: 1.0, shadow_boost: 0.0 })));
    assert_eq!(m.vib.unwrap().current, 25);
    assert_eq!(m.hue_sets, 0);
}
#[test] fn normal_saved_to_store_on_first_apply() {
    /* after update(cod on D1): s.entries()[D1] == SavedNormal { ramp: Some(normal()), vibrance: Some(5), hue: Some(0) } */
}
#[test] fn non_game_in_front_restores_normal() {
    /* update(cod on D1) then update(None): ramp == Some(normal()), vib.current == 5,
       e.active_monitors().is_empty(), s.entries().is_empty() */
}
#[test] fn ramp_untouched_when_all_ramp_switches_off() {
    /* game("cod.exe", None, Some(70.0), None): update, then update(None) → f.m[D1].ramp_sets == 0 */
}
#[test] fn repeat_update_reapplies_without_recapture() {
    /* update(cod) twice → ramp_sets == 2, ramp_gets == 1 */
}
#[test] fn switch_game_same_monitor_resets_unused_parts() {
    /* games A=cod (gamma 2.0) and B=bf6 (vibrance 70 only); update(cod,D1), update(bf6,D1):
       ramp == Some(normal()), vib.current == 25, ramp_gets == 1, s.entries()[D1].ramp == Some(normal()) */
}
#[test] fn switch_to_game_on_other_monitor_restores_first() {
    /* fake(&[D1, D2]); update(cod,D1), update(bf6 with gamma 2.0, D2):
       f.m[D1].ramp == Some(normal()), f.m[D2].ramp != Some(normal()), e.active_monitors() == vec![D2] */
}
#[test] fn rejected_ramp_still_applies_vibrance() {
    /* D1.reject_ramp = true; game cod gamma 2.0 + vibrance 70; update → vib.current == 25, ramp == Some(normal());
       update(None) → ramp_sets == 1 (only the rejected attempt; no restore write) */
}
#[test] fn failed_restore_is_retried_next_update() {
    /* update(cod gamma 2.0, D1); set D1.reject_all = true; update(None) → active_monitors == [D1], s has D1;
       reject_all = false; update(None) → ramp == Some(normal()), active empty, store empty */
}
#[test] fn missing_normal_ramp_leaves_ramp_alone() {
    /* D1.ramp = None; game gamma 2.0 → ramp_sets == 0 */
}
#[test] fn restore_all_restores_every_active_monitor() { /* apply on D1, then restore_all → normal, active empty */ }
#[test] fn recover_applies_saved_normals_and_clears_store() {
    /* store has D1 → SavedNormal { ramp: Some(identity()), vibrance: Some(9), hue: Some(3) }; f D1 has normal();
       recover → f D1 ramp == Some(identity()), vib.current == 9, hue == Some(3), store empty */
}
```
Write each `/* ... */` body as real code with exactly those assertions.

- [ ] **Step 2: Run tests to verify they fail** — `cargo test engine` → compile errors.
- [ ] **Step 3: Implement `src/engine.rs`** per the rules above. Per-monitor state: `SavedNormal`, `Option<VibranceInfo>`, and three `modified` flags.
- [ ] **Step 4: Run tests to verify they pass** — `cargo test` → all PASS.
- [ ] **Step 5: Commit** — `git add src/engine.rs src/main.rs && git commit -m "feat: apply/restore engine"`

---

### Task 5: Windows gamma ramp + probe

**Files:**
- Create: `src/win/mod.rs`, `src/win/gamma.rs`, `src/win/probe.rs`; Modify: `src/main.rs`, `Cargo.toml`

**Interfaces:**
- Consumes: `GammaRamp`, `layered`, `CurveParams` (Task 2)
- Produces (`src/win/gamma.rs`): `pub fn monitors() -> Vec<String>` (device names via `EnumDisplayMonitors` + `GetMonitorInfoW`), `pub fn monitor_of_window(hwnd: HWND) -> Option<String>` (`MonitorFromWindow(.., MONITOR_DEFAULTTONEAREST)`), `pub fn get_ramp(device: &str) -> Option<GammaRamp>`, `pub fn set_ramp(device: &str, ramp: &GammaRamp) -> bool` (`CreateDCW(None, device, None, None)` → `Get/SetDeviceGammaRamp` → `DeleteDC`)
- Produces (`src/win/probe.rs`, `#[cfg(debug_assertions)]`): `pub fn run() -> i32`

- [ ] **Step 1: Add the dependency** — `cargo add windows-sys --target 'cfg(windows)' --features Win32_Foundation,Win32_Graphics_Gdi,Win32_UI_ColorSystem` (add further `Win32_*` features in later tasks as the compiler asks).
- [ ] **Step 2: Implement `gamma.rs`**, and in `main.rs` dispatch `--probe` (debug only) to `win::probe::run()`.
- [ ] **Step 3: Implement the probe's gamma part** — for each monitor print `<device>: read ok|read failed`, then set `layered(normal, gamma 1.05)` and print `set ok|set rejected`, sleep 1 s, set `normal` back and print `restored ok|restore failed`.
- [ ] **Step 4: Verify on the real machine** — tell the user the screen will brighten slightly for a second, then run:
`cargo build --target x86_64-pc-windows-gnu && ./target/x86_64-pc-windows-gnu/debug/brightmarmot.exe --probe`
Expected: one line per monitor with `read ok`, `set ok` and `restored ok`. **(user)** confirm the screen brightened briefly and came back.
- [ ] **Step 5: Commit** — `git add -A && git commit -m "feat: Windows gamma ramp access and probe"`

---

### Task 6: NvAPI vibrance/hue + WinDisplay backend

**Files:**
- Create: `src/win/nvapi.rs`, `src/win/display.rs`; Modify: `src/win/probe.rs`, `src/win/mod.rs`

**Interfaces:**
- Consumes: `VibranceInfo` (Task 2), `DisplayBackend` (Task 4), `gamma::*` (Task 5)
- Produces:
  - `pub struct NvApi` with `pub fn load() -> Option<NvApi>`, `pub fn vibrance_supported(&self) -> bool`, `pub fn get_vibrance(&self, device: &str) -> Option<VibranceInfo>`, `pub fn set_vibrance(&self, device: &str, raw: i32) -> bool`, `pub fn get_hue(&self, device: &str) -> Option<u32>`, `pub fn set_hue(&self, device: &str, degrees: u32) -> bool`
  - `pub struct WinDisplay` with `pub fn new() -> WinDisplay` (holds `Option<NvApi>`), `impl DisplayBackend for WinDisplay`
  - `pub fn nvidia_colour_available() -> bool` (NvAPI loaded, vibrance supported, and `get_vibrance` is `Some` for at least one monitor)

**NvAPI details.** `LoadLibraryW("nvapi64.dll")` → `GetProcAddress("nvapi_QueryInterface")` → `extern "C" fn(u32) -> *const c_void`. Status `0` = OK. Display handle per call: `NvAPI_GetAssociatedNvidiaDisplayHandle(device_name_ansi, *mut handle)`; output id `0`. Struct version = `size_of::<T>() as u32 | (1 << 16)`.

| Function | Expected ID | Source |
|---|---|---|
| `NvAPI_Initialize` | `0x0150E828` | NVIDIA `nvapi_interface.h` |
| `NvAPI_GetAssociatedNvidiaDisplayHandle` | `0x35C29134` | NVIDIA `nvapi_interface.h` |
| `NvAPI_GetDVCInfoEx` | `0x0E45002D` | NvAPIWrapper `FunctionId.cs` |
| `NvAPI_SetDVCLevelEx` | `0x4A82C2B1` | NvAPIWrapper `FunctionId.cs` |
| `NvAPI_GetHUEInfo` | `0x95B64341` | NvAPIWrapper `FunctionId.cs` |
| `NvAPI_SetHUEAngle` | `0xF5A0F22C` | NvAPIWrapper `FunctionId.cs` |

The IDs above are from memory: confirm each against the named source (`github.com/NVIDIA/nvapi`; `github.com/falahati/NvAPIWrapper`, cross-check with vibranceGUI). If they differ, the source wins. Take the DVC/hue function signatures and struct layouts from the same NvAPIWrapper source; expected: `DVC_INFO_EX { version: u32, current: i32, min: i32, max: i32, default: i32 }`, `HUE_INFO { version: u32, current_angle: u32, default_angle: u32 }`. Cite the source in a comment above the ID constants. Copy facts only, not code. Only the functions in this table may be resolved (spec §8.1). Missing or failing DVC/hue functions ⇒ `vibrance_supported() == false` and the getters return `None`; never panic.

- [ ] **Step 1: Implement `nvapi.rs`** with the IDs confirmed, and `display.rs`.
- [ ] **Step 2: Extend the probe** — print `nvapi: loaded|not loaded`, `vibrance supported: yes|no`, and per monitor `vibrance current/min/max/default` and `hue current`. Then set vibrance to `max`, read back, print `vibrance set ok (readback N)`, sleep 1 s, restore the original value and print `vibrance restored`. Do the same for hue with `current + 30` mod 360.
- [ ] **Step 3: Verify on the real machine** — tell the user colours will flash for about 2 s, then run the probe command from Task 5.
Expected: `nvapi: loaded`, `vibrance supported: yes`, sensible ranges (e.g. `min ≤ default ≤ max`), and readbacks equal to the values set. **(user)** confirm saturation and hue visibly changed and came back.
- [ ] **Step 4: Commit** — `git add -A && git commit -m "feat: NvAPI vibrance/hue and Windows display backend"`

---

### Task 7: Registry — Start with Windows and full gamma range

**Files:**
- Create: `src/win/registry.rs`; Modify: `src/main.rs`, `src/win/mod.rs`

**Interfaces:**
- Produces:
  - `pub fn get_run_entry_in(subkey: &str, value: &str) -> Option<String>`, `pub fn set_run_entry_in(subkey: &str, value: &str, exe_path: &str) -> bool` (stores `"<exe_path>"` with quotes), `pub fn delete_run_entry_in(subkey: &str, value: &str) -> bool` (all under HKCU)
  - `pub fn get_run_entry() -> Option<String>`, `pub fn set_run_entry(exe_path: &str) -> bool`, `pub fn delete_run_entry() -> bool` (using the Run key/value from Global Constraints)
  - `pub fn gamma_range_enabled() -> bool`; `pub fn write_gamma_range() -> bool` (needs admin)
  - `pub fn request_gamma_range_elevated() -> bool` — `ShellExecuteExW` with verb `runas` on `current_exe()` and `--set-gamma-range`, `SEE_MASK_NOCLOSEPROCESS`; wait for it; true if exit code 0. UAC cancelled ⇒ false.
- `main.rs`: `--set-gamma-range` ⇒ exit `0` if `write_gamma_range()` succeeded, else `1`.

- [ ] **Step 1: Write the failing test** (`#[cfg(all(test, windows))]` in `registry.rs`)

```rust
#[test] fn run_entry_roundtrip_in_test_key() {
    let (k, v) = (r"Software\BrightMarmotTest", "BrightMarmot");
    assert!(set_run_entry_in(k, v, r"C:\Apps\brightmarmot.exe"));
    assert_eq!(get_run_entry_in(k, v).as_deref(), Some(r#""C:\Apps\brightmarmot.exe""#));
    assert!(delete_run_entry_in(k, v));
    assert_eq!(get_run_entry_in(k, v), None);
}
```
- [ ] **Step 2: Run it to verify it fails** — `cargo test --target x86_64-pc-windows-gnu registry` → compile error.
- [ ] **Step 3: Implement `registry.rs`** (`Win32_System_Registry`, `Win32_UI_Shell`, `Win32_System_Threading` features). The test helpers create the subkey if it's missing.
- [ ] **Step 4: Run it to verify it passes** — `cargo test --target x86_64-pc-windows-gnu registry` → PASS. Then `reg.exe delete 'HKCU\Software\BrightMarmotTest' /f` to tidy up.
- [ ] **Step 5: Verify the read side** — add a probe line `gamma range: enabled|not enabled`, run the probe, and compare it with `reg.exe query 'HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\ICM' /v GdiIcmGammaRange` (enabled ⇔ `0x100`). They must agree. The elevated write itself is exercised by Task 8's `Enable…` button.
- [ ] **Step 6: Commit** — `git add -A && git commit -m "feat: Run key and gamma range registry helpers"`

---

### Task 8: Settings window — main view

**Files:**
- Create: `src/icon.rs`, `src/win/ipc.rs`, `src/win/exe_icon.rs`, `src/settings/mod.rs`, `src/settings/theme.rs`, `src/settings/game_view.rs`; Modify: `src/main.rs`, `Cargo.toml`

**Interfaces:**
- Consumes: `config::*` (Task 1), `registry::*` (Task 7), `display::nvidia_colour_available` (Task 6)
- Produces:
  - `src/icon.rs`: `pub fn app_icon_rgba(size: u32) -> Vec<u8>` — a filled `#F5A623` disc on a transparent background.
  - `src/win/ipc.rs`: consts `TRAY_CLASS = "BrightMarmot.TrayWindow"`, `SETTINGS_TITLE = "BrightMarmot"`, `MSG_CONFIG_CHANGED = "BrightMarmot.ConfigChanged"`, `MSG_OPEN_SETTINGS = "BrightMarmot.OpenSettings"`, `MUTEX_TRAY = "Local\\BrightMarmot.Tray"`, `MUTEX_SETTINGS = "Local\\BrightMarmot.Settings"`; `pub struct InstanceGuard` (closes the mutex on drop); `pub fn single_instance(name: &str) -> Option<InstanceGuard>` (`None` if already held); `pub fn registered_message(name: &str) -> u32`; `pub fn notify_tray(msg_name: &str) -> bool` (`FindWindowW(TRAY_CLASS)` + `PostMessageW`); `pub fn focus_window_titled(title: &str) -> bool`; `pub fn spawn_self(arg: &str) -> bool` (`std::process::Command` on `current_exe()`).
  - `src/win/exe_icon.rs`: `pub fn exe_icon_rgba(path: &str) -> Option<(usize, usize, Vec<u8>)>` — 32×32 via `SHGetFileInfoW(SHGFI_ICON | SHGFI_LARGEICON)` → `GetIconInfo` → `GetDIBits` (BGRA → RGBA; if every alpha is 0, set alpha 255) → `DestroyIcon`.
  - `src/settings/theme.rs`: colour consts from Global Constraints, `pub fn apply(ctx: &egui::Context)`, `pub fn toggle(ui: &mut egui::Ui, on: &mut bool) -> egui::Response` (pill switch about 44×24; accent when on, outlined when off).
  - `src/settings/game_view.rs`: `pub fn show(ui: &mut egui::Ui, game: &mut Game, icon: Option<&egui::TextureHandle>, nv_available: bool) -> bool` (true if anything changed).
  - `src/settings/mod.rs`: `pub fn run() -> i32`; `#[derive(Default)] pub struct IconCache` (map exe path → `Option<egui::TextureHandle>`) with `pub fn get(&mut self, ctx: &egui::Context, exe_path: &str) -> Option<egui::TextureHandle>` (loads via `exe_icon_rgba` on first request; empty path or failure is cached as `None`).

- [ ] **Step 1: Write the failing icon test** (`src/icon.rs`)

```rust
#[test] fn icon_is_orange_disc() {
    let s = 32; let px = app_icon_rgba(s);
    assert_eq!(px.len(), (s * s * 4) as usize);
    let at = |x: u32, y: u32| &px[((y * s + x) * 4) as usize..][..4];
    assert_eq!(at(16, 16), &[0xF5, 0xA6, 0x23, 0xFF]);
    assert_eq!(at(0, 0)[3], 0);
}
```
- [ ] **Step 2: Run it to verify it fails, implement `app_icon_rgba`, run it to verify it passes** — `cargo test icon`.
- [ ] **Step 3: Add the dependency** — `cargo add eframe --target 'cfg(windows)' --no-default-features --features glow,default_fonts`. egui APIs move between versions: follow the docs for the version that was resolved.
- [ ] **Step 4: Implement `ipc.rs`, `exe_icon.rs`, `theme.rs`, `game_view.rs`, `settings/mod.rs`**; `main.rs` dispatches `--settings` → `settings::run()`.

`run()`: if `single_instance(MUTEX_SETTINGS)` is `None` → `focus_window_titled(SETTINGS_TITLE)`, return 0. Load config from `config::default_path()`; compute `nv_available` and `gamma_range_enabled()` once. eframe window: title `SETTINGS_TITLE`, inner size 1000×640, min 760×480, icon from `app_icon_rgba(64)`.

Layout (spec §7.1; copy is exact):
- Left panel (about 300 px): game rows (icon + name, selected row highlighted), then buttons `+ Add game` (opens Task 9's dialog; until then, disabled) and `Remove` (removes the selected game without confirming — my call; disabled when nothing is selected). Below them, a `Start with Windows` toggle, then either `Full gamma range: enabled` or `Full gamma range: not enabled` with an `Enable…` button and a hint `Needs admin once and a Windows restart.`
- Right panel: header (32 px icon, bold name, `path` in secondary text). Section title `Own colours for this game`, subtitle `Each value you switch on replaces your normal value while this game is in front.` Then one row per `ControlKind::ALL`: label, slider (`show_value(false)`), a number box (`DragValue` clamped to the range, with the kind's decimals), and the toggle. Slider and box are greyed out (disabled) when the switch is off. If `!nv_available`, the Vibrance and Hue rows are disabled and a note follows them: `Vibrance and hue need an NVIDIA GPU driving this screen.`
- Empty state (no games): right panel shows `Add a game to get started.` and an `Add game` button.

Behaviour:
- Changes set `dirty`. When `dirty` and no pointer button is down: `config::save`, `ipc::notify_tray(MSG_CONFIG_CHANGED)`, clear `dirty`. Also save on exit if dirty.
- The Start with Windows toggle writes `config.start_with_windows` and immediately calls `registry::set_run_entry(current_exe)` or `delete_run_entry()`.
- `Enable…` calls `registry::request_gamma_range_elevated()`, then re-reads `gamma_range_enabled()`.
- Icons come from `IconCache::get`; on `None`, draw a plain `#36373C` rounded square.

- [ ] **Step 5: Verify (user)** — from WSL, write a two-game test config:
`mkdir -p /mnt/c/Users/adrian/AppData/Roaming/BrightMarmot && printf '{"games":[{"exe_name":"notepad.exe","path":"C:\\\\Windows\\\\notepad.exe"},{"exe_name":"bf6.exe"}]}' > /mnt/c/Users/adrian/AppData/Roaming/BrightMarmot/config.json`
Then run `cargo build --target x86_64-pc-windows-gnu && ./target/x86_64-pc-windows-gnu/debug/brightmarmot.exe --settings`. Check:
  - It looks like the user's first screenshot, minus FPS/HDR (user confirms).
  - Switch Gamma on and drag it to 1.30, then release. `cat` the config file and confirm it shows `"gamma":{"enabled":true,"value":1.3}`.
  - Type `9` into the Gamma box and confirm it shows `3.00`.
  - Running a second `--settings` focuses the existing window instead of opening a new one.
  - Turn Start with Windows on, then run `reg.exe query 'HKCU\Software\Microsoft\Windows\CurrentVersion\Run' /v BrightMarmot` and confirm it shows the quoted exe path. Turn it off and confirm the value is gone.
  - Remove bf6 and confirm it's gone from the file.
  - Click `Enable…`: a UAC prompt appears, and after approving it the status reads `enabled`.
- [ ] **Step 6: Commit** — `git add -A && git commit -m "feat: settings window main view"`

---

### Task 9: Add Game dialog

**Files:**
- Create: `src/apps.rs`, `src/win/windows_list.rs`, `src/settings/add_game.rs`; Modify: `src/settings/mod.rs`, `src/main.rs`, `Cargo.toml`

**Interfaces:**
- Consumes: `Config::find_game`, `Config::add_game`, `exe_name_from_path` (Task 1); `exe_icon_rgba` (Task 8)
- Produces:
  - `src/apps.rs`: `#[derive(Clone, Debug)] pub struct WindowInfo { pub exe_path: String, pub title: String, pub visible: bool, pub has_owner: bool, pub tool_window: bool, pub cloaked: bool }`; `#[derive(Clone, Debug, PartialEq)] pub struct RunningApp { pub exe_path: String, pub exe_name: String, pub process_name: String, pub title: String }`; `pub fn running_apps(windows: &[WindowInfo], self_exe_name: &str, config: &Config) -> Vec<RunningApp>`
  - `src/win/windows_list.rs`: `pub fn top_level_windows() -> Vec<WindowInfo>` (`EnumWindows`; `IsWindowVisible`; `GetWindow(GW_OWNER)`; `WS_EX_TOOLWINDOW`; `DwmGetWindowAttribute(DWMWA_CLOAKED)`; `GetWindowTextW`; exe via `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)` + `QueryFullProcessImageNameW`, empty string on failure)
  - `src/settings/add_game.rs`: `pub struct AddGameDialog`; `pub enum DialogResult { Open, Cancelled, Add(String /* exe path */) }`; `pub fn show(&mut self, ctx: &egui::Context, icons: &mut IconCache) -> DialogResult`

- [ ] **Step 1: Write the failing tests** (`src/apps.rs`)

```rust
fn w(path: &str, title: &str) -> WindowInfo { WindowInfo { exe_path: path.into(), title: title.into(), visible: true, has_owner: false, tool_window: false, cloaked: false } }
#[test] fn keeps_plain_app_windows_with_names() {
    let r = running_apps(&[w(r"C:\B\brave.exe", "South Park - Brave")], "brightmarmot.exe", &Config::default());
    assert_eq!(r, vec![RunningApp { exe_path: r"C:\B\brave.exe".into(), exe_name: "brave.exe".into(), process_name: "brave".into(), title: "South Park - Brave".into() }]);
}
#[test] fn filters_hidden_owned_tool_cloaked_untitled_unknown() {
    let mut v = vec![];
    for f in 0..6 { let mut x = w(&format!(r"C:\a{f}.exe"), "T"); match f {
        0 => x.visible = false, 1 => x.has_owner = true, 2 => x.tool_window = true,
        3 => x.cloaked = true, 4 => x.title = "  ".into(), _ => x.exe_path = String::new() }; v.push(x); }
    assert!(running_apps(&v, "brightmarmot.exe", &Config::default()).is_empty());
}
#[test] fn excludes_self_framehost_and_added_games_and_dedupes() {
    let mut c = Config::default(); c.add_game(r"C:\g\cod.exe");
    let v = [w(r"C:\x\BrightMarmot.exe", "BrightMarmot"), w(r"C:\W\ApplicationFrameHost.exe", "Calc"),
             w(r"C:\g\COD.exe", "Call of Duty"), w(r"C:\b\brave.exe", "One"), w(r"C:\b\brave.exe", "Two")];
    let r = running_apps(&v, "brightmarmot.exe", &c);
    assert_eq!(r.len(), 1); assert_eq!(r[0].title, "One");
}
```
- [ ] **Step 2: Run tests to verify they fail** — `cargo test apps` → compile errors.
- [ ] **Step 3: Implement `running_apps`** — keep windows that are visible, unowned, not tool windows, not cloaked, with a non-blank title and a non-empty exe path. Exclude `self_exe_name`, `applicationframehost.exe`, and already-added games (all case-insensitive). Keep the first window per exe and preserve input order.
- [ ] **Step 4: Run tests to verify they pass** — `cargo test` → all PASS.
- [ ] **Step 5: Implement `windows_list.rs` and `add_game.rs`, and wire them up** — `cargo add rfd --target 'cfg(windows)'`.

Dialog (spec §7.2; copy is exact): an `egui::Modal` (or a centred `egui::Window` over a dimmed backdrop if the resolved egui has no `Modal`) titled `Add Game` with the heading `Running applications`. It shows a scrollable list of rows: 32 px icon, bold `title`, and `process_name` in secondary text. Clicking a row selects it; double-clicking adds it. Buttons: `Refresh` (re-enumerate), `Pick .exe from disk…` (`rfd::FileDialog` filtered to `exe`; a picked file returns `Add(path)` immediately), `Cancel`, and `Add` (disabled until a row is selected). The list is enumerated when the dialog opens.

In `settings/mod.rs`: `+ Add game` and the empty-state button open the dialog. `Add(path)` → `config.add_game(path)`; `Added(i)` or `AlreadyExists(i)` selects `i`; mark dirty.
- [ ] **Step 6: Verify (user)**
  - Open the dialog. Running apps (e.g. Windows Terminal, a browser) are listed with icons, titles and process names; BrightMarmot itself and already-added games are not.
  - Open Notepad, click `Refresh`, and Notepad appears. `Add` puts `notepad` in the list, selected, and in `config.json`.
  - Use `Pick .exe from disk…` to choose `C:\Windows\System32\mspaint.exe` (or any exe); it's added.
  - Pick Notepad's exe again: no duplicate, and the existing entry is selected.
- [ ] **Step 7: Commit** — `git add -A && git commit -m "feat: add game from running apps or disk"`

---

### Task 10: Tray process shell

**Files:**
- Create: `src/win/tray.rs`; Modify: `src/main.rs`, `src/win/mod.rs`

**Interfaces:**
- Consumes: `ipc::*` (Task 8), `icon::app_icon_rgba` (Task 8)
- Produces: `pub fn run() -> i32` (no arguments → tray mode). Task 11 adds the engine to this module's state.

Behaviour (spec §5.1–§5.2):
- `single_instance(MUTEX_TRAY)` is `None` → `notify_tray(MSG_OPEN_SETTINGS)`, return 0.
- Create a **hidden top-level window** (class `TRAY_CLASS`, never shown, `WS_EX_TOOLWINDOW`). Not `HWND_MESSAGE`: message-only windows don't receive the `TaskbarCreated`, `WM_DISPLAYCHANGE` or `WM_ENDSESSION` broadcasts. This deliberately deviates from the spec's "message-only window"; the intent (an invisible window) is unchanged.
- Tray icon: `Shell_NotifyIconW` with an `HICON` built from `app_icon_rgba(32)` (`CreateIcon` or `CreateIconIndirect`), tooltip `BrightMarmot`, and a callback message.
- Double-click → open settings. Right-click → popup menu `Settings`, `Exit` (`SetForegroundWindow` before `TrackPopupMenu`, post `WM_NULL` after).
- Open settings = `AllowSetForegroundWindow(ASFW_ANY)` then `spawn_self("--settings")`. Same for the registered `MSG_OPEN_SETTINGS`.
- Registered `TaskbarCreated` → re-add the icon.
- Exit → delete the icon, `PostQuitMessage(0)`.
- Per-process state lives in a `thread_local!` `RefCell`. Never hold the borrow across calls that pump messages (`TrackPopupMenu`, `MessageBoxW`, waiting on elevation): callbacks re-enter.

- [ ] **Step 1: Implement `tray.rs`**; `main.rs` with no arguments → `win::tray::run()`.
- [ ] **Step 2: Verify (user)** — `cargo build --target x86_64-pc-windows-gnu && (./target/x86_64-pc-windows-gnu/debug/brightmarmot.exe &)`. Check:
  - The orange tray icon appears with the tooltip `BrightMarmot`.
  - Double-click opens Settings.
  - The right-click menu shows `Settings` and `Exit`.
  - Running the exe a second time opens Settings, and `tasklist.exe /FI "IMAGENAME eq brightmarmot.exe"` shows only one tray process (plus Settings if it's open).
  - After `taskkill.exe /F /IM explorer.exe; explorer.exe`, the icon comes back.
  - `Exit` removes the icon and ends the process.
- [ ] **Step 3: Commit** — `git add -A && git commit -m "feat: tray icon, menu and single instance"`

---

### Task 11: Tray wiring — foreground hook, engine, startup duties

**Files:**
- Create: `src/win/foreground.rs`; Modify: `src/win/tray.rs`

**Interfaces:**
- Consumes: `Engine`, `recover`, `Foreground` (Task 4); `WinDisplay` (Task 6); `RestoreFile` (Task 3); `config::load`/`default_path`/`exe_name_from_path` (Task 1); `gamma::monitor_of_window` (Task 5); `registry::*` (Task 7); `ipc::registered_message` (Task 8)
- Produces (`src/win/foreground.rs`): `pub fn install_hook(callback: WINEVENTPROC) -> bool` (`SetWinEventHook(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND, 0, callback, 0, 0, WINEVENT_OUTOFCONTEXT)`); `pub fn foreground_info(hwnd: HWND) -> Option<Foreground>` (null hwnd, `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)` failure, or empty path ⇒ `None`)

Startup order in `tray::run()`, after the single-instance check:
1. `config::load(default_path)`.
2. `RestoreFile::load(default_path)` then `engine::recover(&mut store, &mut display)`.
3. If `config.start_with_windows` and `registry::get_run_entry()` ≠ `"<current_exe>"`, call `set_run_entry(current_exe)`.
4. If `!gamma_range_enabled() && !config.gamma_range_prompt_answered`, show `MessageBoxW` (Yes/No, title `BrightMarmot`) with exactly this text:
   `Windows limits how much an app can change your screen's gamma, so stronger Gamma, Contrast and Shadow Boost settings may not work.\n\nBrightMarmot can lift that limit by changing one Windows setting (GdiIcmGammaRange). This needs admin rights once and takes effect after you restart Windows.\n\nChange it now?`
   Set `gamma_range_prompt_answered = true` and save. On Yes, call `request_gamma_range_elevated()`.
5. Create the window and tray icon (Task 10), `install_hook`, then `evaluate(arm_timer = true)` once, in case a game is already in front.

`evaluate(arm_timer)`: `foreground_info(GetForegroundWindow())` → `engine.update(...)`; if it returns true and `arm_timer`, `SetTimer(hwnd, REAPPLY_TIMER, 750, None)`.

Message handling:
- Hook callback → `evaluate(true)`.
- `WM_TIMER(REAPPLY_TIMER)` → `KillTimer`, then `evaluate(false)`. Never re-arm here: that would turn the timer into polling.
- `WM_DISPLAYCHANGE` → `evaluate(true)`.
- Registered `MSG_CONFIG_CHANGED` → reload the config, then `evaluate(false)`.
- `WM_QUERYENDSESSION` → `restore_all`, return TRUE. `WM_ENDSESSION` with `wParam != 0` → `restore_all`.
- `Exit` → `restore_all` before removing the icon.

- [ ] **Step 1: Implement `foreground.rs` and the wiring.**
- [ ] **Step 2: Verify (user)** — in Settings, add Notepad with Gamma 1.5 and Vibrance 80 switched on. Start the tray (debug build) and check:
  - Click into Notepad: its monitor changes (brighter and more saturated) and other monitors don't. `%LOCALAPPDATA%\BrightMarmot\restore.json` exists.
  - Alt-Tab away: normal colours return and `restore.json` is gone.
  - Close Notepad while it's in front: normal colours return.
  - With Notepad in front, `taskkill.exe /F /IM brightmarmot.exe`: the screen stays changed. Start the tray again: normal colours come back and `restore.json` is gone.
  - With Notepad in front, `Exit` from the tray: normal colours return.
  - Set `gamma_range_prompt_answered` to false in the config: the prompt appears only if the range isn't enabled, and only once.
  - Turn Start with Windows on, move the exe, and start it from the new place: the Run value now points to the new path.
- [ ] **Step 3: Commit** — `git add -A && git commit -m "feat: per-game colours on foreground change"`

---

### Task 12: Install script, README, acceptance

**Files:**
- Create: `scripts/install.sh`, `README.md`

- [ ] **Step 1: Write `scripts/install.sh`** — `set -euo pipefail`; `cargo build --release --target x86_64-pc-windows-gnu`. If `tasklist.exe` shows `brightmarmot.exe`, print `Exit BrightMarmot from the tray first.` and exit 1. Destination: `$(wslpath "$(cd /mnt/c && cmd.exe /c 'echo %LOCALAPPDATA%' | tr -d '\r')")/Programs/BrightMarmot`; `mkdir -p` it, copy the exe, and print `Installed to <Windows path>`.
- [ ] **Step 2: Write `README.md`** (short) — what it does; build and install (`rustup`, `mingw-w64`, `scripts/install.sh`); how to use it (add a game, switches); the Windows gamma-range prompt; limitations from spec §10 (HDR, games that reset gamma, undocumented NVIDIA calls, restart after the registry change); how to uninstall (turn off Start with Windows, Exit, delete the exe and the two `BrightMarmot` folders, optionally remove `GdiIcmGammaRange`).
- [ ] **Step 3: Install and measure** — run `scripts/install.sh`. **(user)** starts `%LOCALAPPDATA%\Programs\BrightMarmot\brightmarmot.exe` from Explorer (no console window should appear). Leave it idle with Settings closed for 60 s, then run `powershell.exe -NoProfile -Command "Get-Process brightmarmot | Select-Object Id,@{n='PrivateMB';e={[math]::Round(\$_.PrivateMemorySize64/1MB,1)}},CPU"` twice, 30 s apart.
Expected: one process, `PrivateMB` under about 10, and `CPU` unchanged between the two readings. If memory is over target, report the number to the user; don't hide it.
- [ ] **Step 4: Acceptance (user)** — run the spec §8.3 manual checklist against the installed exe with a real game (e.g. `cod.exe`), including a sign-out/sign-in with Start with Windows on.
- [ ] **Step 5: Commit** — `git add -A && git commit -m "docs: README and install script"`
