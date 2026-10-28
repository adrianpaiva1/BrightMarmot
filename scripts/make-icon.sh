#!/usr/bin/env bash
# Regenerates assets/icon.ico (embedded in the exe, used by the tray) and
# assets/icon-256.png (Settings window icon) from assets/icon.svg.
# Needs the resvg CLI: cargo install resvg
set -euo pipefail
cd "$(dirname "$0")/.."

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

sizes=(16 20 24 32 40 48 64 256)
for s in "${sizes[@]}"; do
  resvg -w "$s" -h "$s" assets/icon.svg "$tmp/$s.png"
done
cp "$tmp/256.png" assets/icon-256.png

# Pack the PNGs into a .ico (PNG-compressed entries, supported since Windows Vista).
python3 -I - "$tmp" assets/icon.ico "${sizes[@]}" <<'PY'
import struct, sys
src, out, *sizes = sys.argv[1:]
pngs = [open(f"{src}/{s}.png", "rb").read() for s in sizes]
offset = 6 + 16 * len(pngs)
entries = b""
for s, data in zip(sizes, pngs):
    n = int(s) % 256  # 0 means 256
    entries += struct.pack("<BBBBHHII", n, n, 0, 0, 1, 32, len(data), offset)
    offset += len(data)
with open(out, "wb") as f:
    f.write(struct.pack("<HHH", 0, 1, len(pngs)) + entries + b"".join(pngs))
PY

echo "Wrote assets/icon.ico and assets/icon-256.png"
