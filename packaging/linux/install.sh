#!/usr/bin/env bash
# Builds Warp Notes (release) and installs the runnable file + desktop entry
# + hicolor icons so the Linux taskbar/launcher shows the app icon (matched
# via StartupWMClass). The staged binary in out/ is the deliverable and what
# the desktop entry launches - no zip, no installer.
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd "$here/../.." && pwd)"   # script lives in packaging/linux
out_dir="$here/out"

(cd "$repo" && cargo build --release)
# cargo may place artifacts off-repo (e.g. CARGO_TARGET_DIR on a fast
# local drive while the repo lives elsewhere)
target_dir="$(cd "$repo" && cargo metadata --no-deps --format-version 1 | jq -r '.target_directory')"

bin="$target_dir/release/warp-notes"
if [ ! -x "$bin" ]; then
	echo "release binary not found: $bin" >&2
	exit 1
fi

mkdir -p "$out_dir"
rm -rf "${out_dir:?}"/*
cp "$bin" "$out_dir/warp-notes"

icon_root="${XDG_DATA_HOME:-$HOME/.local/share}/icons/hicolor"
for f in "$here"/icons/warpnotes-*.png; do
	size="$(basename "$f" .png)"; size="${size#warpnotes-}"
	dir="$icon_root/${size}x${size}/apps"
	mkdir -p "$dir"
	cp "$f" "$dir/warpnotes.png"
done
mkdir -p "$icon_root/scalable/apps"
cp "$here/icons/warpnotes.svg" "$icon_root/scalable/apps/"

app_dir="${XDG_DATA_HOME:-$HOME/.local/share}/applications"
mkdir -p "$app_dir"
cp "$here/warp-notes.desktop" "$app_dir/"

command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$app_dir" || true
command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -tf "$icon_root" || true
echo "built + installed: $out_dir/warp-notes, $app_dir/warp-notes.desktop + hicolor icons"
