#!/usr/bin/env bash
# Build BrightMarmot for Windows from WSL and copy it to %LOCALAPPDATA%\Programs\BrightMarmot.
set -euo pipefail

cd "$(dirname "$0")/.."
[ -f "$HOME/.cargo/env" ] && . "$HOME/.cargo/env"

cargo build --release --target x86_64-pc-windows-gnu

# Capture first: grep -q exiting early would SIGPIPE the pipeline under pipefail.
# A failing tasklist.exe (interop off) reads as "not running".
running=$(tasklist.exe 2>/dev/null | tr -d '\r' || true)
if grep -qi '^brightmarmot\.exe' <<<"$running"; then
    echo "Exit BrightMarmot from the tray first."
    exit 1
fi

local_appdata_win=$(cd /mnt/c && cmd.exe /c 'echo %LOCALAPPDATA%' 2>/dev/null | tr -d '\r')
dest="$(wslpath "$local_appdata_win")/Programs/BrightMarmot"
mkdir -p "$dest"
cp target/x86_64-pc-windows-gnu/release/brightmarmot.exe "$dest/brightmarmot.exe"
echo "Installed to $(wslpath -w "$dest")\\brightmarmot.exe"
