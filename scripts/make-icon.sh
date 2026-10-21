#!/bin/sh
# Renders apps/macos/Resources/AppIcon.svg into AppIcon.icns (needs Google Chrome for the SVG).
set -eu
cd "$(dirname "$0")/.."
SVG="$PWD/apps/macos/Resources/AppIcon.svg"
OUT="apps/macos/Resources/AppIcon.icns"
CHROME="${CHROME:-/Applications/Google Chrome.app/Contents/MacOS/Google Chrome}"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
"$CHROME" --headless=new --disable-gpu --hide-scrollbars --default-background-color=00000000 \
    --window-size=1024,1024 --screenshot="$WORK/master.png" "file://$SVG" >/dev/null 2>&1
SET="$WORK/AppIcon.iconset"
mkdir "$SET"
for size in 16 32 128 256 512; do
    sips -z "$size" "$size" "$WORK/master.png" --out "$SET/icon_${size}x${size}.png" >/dev/null
    double=$((size * 2))
    sips -z "$double" "$double" "$WORK/master.png" --out "$SET/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$SET" -o "$OUT"
echo "$OUT"
