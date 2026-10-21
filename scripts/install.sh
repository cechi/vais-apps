#!/usr/bin/env bash
# Install VAIS Commander from an extracted release archive for the current user:
# binary to ~/.local/bin, launcher entry and icon to ~/.local/share.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
bin_dir="$HOME/.local/bin"
app_dir="$HOME/.local/share/applications"
icon_dir="$HOME/.local/share/icons/hicolor/scalable/apps"
mkdir -p "$bin_dir" "$app_dir" "$icon_dir"

install -m 755 "$here/vais-commander" "$bin_dir/vais-commander"
install -m 644 "$here/vais-commander.desktop" "$app_dir/vais-commander.desktop"
install -m 644 "$here/vais-commander.svg" "$icon_dir/vais-commander.svg"

command -v update-desktop-database >/dev/null && update-desktop-database "$app_dir" || true
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q -t "$HOME/.local/share/icons/hicolor" 2>/dev/null || true

echo "installed to $bin_dir/vais-commander (make sure ~/.local/bin is in your PATH)"
