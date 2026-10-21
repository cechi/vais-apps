#!/usr/bin/env bash
# Install VAIS Commander for the current user: binary link, .desktop entry and icon.
# Usage: scripts/install-desktop.sh [debug]   (default links the release build)
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
profile="${1:-release}"
bin="$root/target/$profile/vais-commander"

if [[ ! -x "$bin" ]]; then
    echo "building $profile binary…"
    if [[ "$profile" == release ]]; then
        (cd "$root" && cargo build --release)
    else
        (cd "$root" && cargo build)
    fi
fi

bin_dir="$HOME/.local/bin"
app_dir="$HOME/.local/share/applications"
icon_dir="$HOME/.local/share/icons/hicolor/scalable/apps"
mkdir -p "$bin_dir" "$app_dir" "$icon_dir"

# A symlink, so a rebuild updates the launcher entry too.
ln -sfn "$bin" "$bin_dir/vais-commander"
install -m 644 "$root/assets/vais-commander.desktop" "$app_dir/vais-commander.desktop"
install -m 644 "$root/assets/vais-commander.svg" "$icon_dir/vais-commander.svg"

command -v update-desktop-database >/dev/null && update-desktop-database "$app_dir" || true
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q -t "$HOME/.local/share/icons/hicolor" 2>/dev/null || true

echo "installed:"
echo "  $bin_dir/vais-commander -> $bin"
echo "  $app_dir/vais-commander.desktop"
echo "  $icon_dir/vais-commander.svg"
