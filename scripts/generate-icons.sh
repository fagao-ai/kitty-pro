#!/usr/bin/env bash

set -euo pipefail

project_dir="$(cd "$(dirname "$0")/.." && pwd)"
icon_dir="$project_dir/assets/icon"
temporary_dir="$(mktemp -d /tmp/kitty-pro-icons.XXXXXX)"
iconset_dir="$temporary_dir/kitty-pro.iconset"
mkdir -p "$iconset_dir"
trap 'rm -rf "$temporary_dir"' EXIT

for command_name in rsvg-convert magick iconutil; do
  if ! command -v "$command_name" >/dev/null 2>&1; then
    echo "Missing required command: $command_name" >&2
    exit 1
  fi
done

render_png() {
  local source="$1"
  local size="$2"
  local output="$3"
  rsvg-convert --format png --width "$size" --height "$size" \
    --output "$output" "$source"
}

for size in 16 32 48 64 128 256 512 1024; do
  render_png "$icon_dir/kitty-pro.svg" "$size" \
    "$icon_dir/kitty-pro-$size.png"
done

render_png "$icon_dir/kitty-pro-tray.svg" 64 "$icon_dir/kitty-pro-tray.png"

cp "$icon_dir/kitty-pro-16.png" "$iconset_dir/icon_16x16.png"
cp "$icon_dir/kitty-pro-32.png" "$iconset_dir/icon_16x16@2x.png"
cp "$icon_dir/kitty-pro-32.png" "$iconset_dir/icon_32x32.png"
cp "$icon_dir/kitty-pro-64.png" "$iconset_dir/icon_32x32@2x.png"
cp "$icon_dir/kitty-pro-128.png" "$iconset_dir/icon_128x128.png"
cp "$icon_dir/kitty-pro-256.png" "$iconset_dir/icon_128x128@2x.png"
cp "$icon_dir/kitty-pro-256.png" "$iconset_dir/icon_256x256.png"
cp "$icon_dir/kitty-pro-512.png" "$iconset_dir/icon_256x256@2x.png"
cp "$icon_dir/kitty-pro-512.png" "$iconset_dir/icon_512x512.png"
cp "$icon_dir/kitty-pro-1024.png" "$iconset_dir/icon_512x512@2x.png"
iconutil --convert icns --output "$icon_dir/kitty-pro.icns" "$iconset_dir"

magick \
  "$icon_dir/kitty-pro-16.png" \
  "$icon_dir/kitty-pro-32.png" \
  "$icon_dir/kitty-pro-48.png" \
  "$icon_dir/kitty-pro-64.png" \
  "$icon_dir/kitty-pro-128.png" \
  "$icon_dir/kitty-pro-256.png" \
  "$icon_dir/kitty-pro.ico"

# The NSH app bundles the icon out of its own crate, so the 1024px render is copied
# in rather than referenced across packages.
cp "$icon_dir/kitty-pro-1024.png" \
  "$project_dir/packages/nsh-app/assets/icon-1024.png"

echo "Generated Kitty Pro icons from $icon_dir/kitty-pro.svg"
