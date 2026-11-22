#!/bin/sh
# Run from the repository root. Output is an Apple icon family (.icns).
set -eu
output=$1
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT
rustc --edition=2024 -O scripts/macos-icon.rs -o "$scratch/render-icon"
"$scratch/render-icon" "$scratch/icon.tga"
sips -s format png "$scratch/icon.tga" --out "$scratch/icon.png" >/dev/null
mkdir "$scratch/Rectangle.iconset"
for size in 16 32 128 256 512; do
    sips -z "$size" "$size" "$scratch/icon.png" --out "$scratch/Rectangle.iconset/icon_${size}x${size}.png" >/dev/null
    retina=$((size * 2))
    sips -z "$retina" "$retina" "$scratch/icon.png" --out "$scratch/Rectangle.iconset/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$scratch/Rectangle.iconset" -o "$output"
