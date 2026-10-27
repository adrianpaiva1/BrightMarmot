# BrightMarmot

A small Windows tray app that gives each PC game its own screen colours. Add a
game, set its Gamma, Contrast, Vibrance, Hue and Shadow Boost, and whenever that
game is the window in front, those colours are applied to the monitor it is on.
Alt-Tab away or close the game and the monitor goes back to your normal colours.

It has no polling, no hooks, and never touches a game process. NVIDIA GPUs only
(vibrance and hue use NvAPI; without it those two are skipped).

## Build and install

Built from WSL, cross-compiled to Windows:

```sh
rustup target add x86_64-pc-windows-gnu
sudo apt install mingw-w64
scripts/install.sh
```

The script builds a release exe and copies it to
`%LOCALAPPDATA%\Programs\BrightMarmot\brightmarmot.exe`. If BrightMarmot is
running it refuses; exit it from the tray first. Start the exe from Explorer.

## Use

- Double-click the tray icon to open Settings; right-click for the menu (Exit is there).
- **Add game** lists running applications, or browse to an `.exe`.
- Each colour control has its own on/off switch; only enabled ones are applied.
  Ranges: Gamma 0.50-3.00, Contrast 0.50-2.00, Vibrance 0-100, Hue 0-359,
  Shadow Boost 0-100.
- **Start with Windows** (off by default) launches the tray app at sign-in.

### Windows gamma range

Windows rejects gamma ramps far from normal unless `GdiIcmGammaRange` = 256 is
set under `HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion\ICM`. On first run
the app asks, and on "Yes" shows a one-time admin prompt to write it. Restart
Windows afterwards. Settings shows the current status and a button to do it later.

## Limitations

- HDR: while Windows HDR is on for a monitor, gamma/contrast/shadow boost
  probably have no effect there; vibrance should.
- Some exclusive-fullscreen games set their own gamma; the app re-applies once
  shortly after a window change, but a game that keeps resetting it may win.
- Vibrance and hue use undocumented NvAPI calls; a driver update could break
  them (the app then greys those controls out).
- The full gamma range needs a restart after the registry change.
- Other gamma tools (f.lux, Night Light) may interact. An unsigned exe may be
  flagged by antivirus.

## Uninstall

1. Turn off Start with Windows in Settings, then Exit from the tray.
2. Delete `%LOCALAPPDATA%\Programs\BrightMarmot`, `%APPDATA%\BrightMarmot` and
   `%LOCALAPPDATA%\BrightMarmot`.
3. Optionally delete the `GdiIcmGammaRange` value from the ICM key above.
