#!/usr/bin/env bash
# Make VAIS Commander the file chooser of xdg-desktop-portal for the current
# user: the dialog Firefox, Chromium, GTK, Qt and Flatpak applications open.
#
# Usage: install-portal.sh [--system] [--force] [--uninstall] [path/to/vais-commander]
#   --system     put the .portal file into /usr/share (sudo); needed for
#                xdg-desktop-portal older than 1.17, which ignores ~/.local
#   --force      make vais the FileChooser backend in portals.conf even when
#                another backend is configured there now (a backup is kept)
#   --uninstall  remove everything this script installed
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
name=org.freedesktop.impl.portal.desktop.vais
iface=org.freedesktop.impl.portal.FileChooser
data="${XDG_DATA_HOME:-$HOME/.local/share}"
conf_dir="${XDG_CONFIG_HOME:-$HOME/.config}/xdg-desktop-portal"
conf="$conf_dir/portals.conf"
service="$data/dbus-1/services/$name.service"
user_portal="$data/xdg-desktop-portal/portals/vais.portal"
system_portal=/usr/share/xdg-desktop-portal/portals/vais.portal

system=0 force=0 uninstall=0 bin=""
for arg in "$@"; do
    case "$arg" in
        --system) system=1 ;;
        --force) force=1 ;;
        --uninstall) uninstall=1 ;;
        -h|--help) sed -n '2,12p' "$0"; exit 0 ;;
        *) bin="$arg" ;;
    esac
done

restart_portal() {
    if command -v systemctl >/dev/null && systemctl --user is-active -q xdg-desktop-portal.service; then
        systemctl --user restart xdg-desktop-portal.service && echo "restarted xdg-desktop-portal"
    else
        echo "restart xdg-desktop-portal (or log in again) for the change to take effect"
    fi
}

if (( uninstall )); then
    rm -f "$service" "$user_portal"
    [[ -f $system_portal ]] && sudo rm -f "$system_portal"
    if [[ -f $conf ]]; then
        # Our line goes; a list we were put in front of keeps the rest.
        sed -i -E "/^$iface=vais;?(gtk;?)?$/d; s/^($iface=)vais;/\1/" "$conf"
    fi
    restart_portal
    echo "removed the VAIS Commander file chooser backend"
    exit 0
fi

# The binary: given, in PATH, the per-user install, or the release build.
if [[ -z $bin ]]; then
    for candidate in "$(command -v vais-commander 2>/dev/null || true)" "$HOME/.local/bin/vais-commander" "$here/../target/release/vais-commander"; do
        [[ -n $candidate && -x $candidate ]] && { bin="$candidate"; break; }
    done
fi
[[ -n $bin && -x $bin ]] || { echo "vais-commander binary not found; pass its path" >&2; exit 1; }
bin="$(realpath "$bin")"

portal_src="$here/vais.portal"
[[ -f $portal_src ]] || portal_src="$here/../assets/vais.portal"
[[ -f $portal_src ]] || { echo "vais.portal not found next to this script" >&2; exit 1; }

# D-Bus starts the backend on demand.
mkdir -p "$(dirname "$service")"
cat > "$service" <<EOF
[D-BUS Service]
Name=$name
Exec=$bin --portal
EOF
echo "wrote $service"

# What the backend implements, where xdg-desktop-portal looks for it.
if (( system )); then
    sudo install -Dm644 "$portal_src" "$system_portal"
    echo "installed $system_portal"
else
    install -Dm644 "$portal_src" "$user_portal"
    echo "installed $user_portal"
fi

# Which backend the FileChooser interface uses.
mkdir -p "$conf_dir"
if [[ ! -f $conf ]]; then
    printf '[preferred]\n%s=vais;gtk;\n' "$iface" > "$conf"
    echo "created $conf"
elif current="$(grep -E "^$iface=" "$conf" || true)" && [[ -n $current ]]; then
    value="${current#*=}"
    if [[ $value == vais* ]]; then
        echo "$conf already prefers vais"
    elif (( force )); then
        cp "$conf" "$conf.bak"
        sed -i -E "s|^$iface=.*|$iface=vais;gtk;|" "$conf"
        echo "changed $conf ($current -> $iface=vais;gtk;; backup in portals.conf.bak)"
    else
        echo "NOT changed: $conf has '$current'; run again with --force to prefer vais"
    fi
elif grep -q '^\[preferred\]' "$conf"; then
    sed -i "/^\[preferred\]/a $iface=vais;gtk;" "$conf"
    echo "added the FileChooser line to $conf"
else
    printf '\n[preferred]\n%s=vais;gtk;\n' "$iface" >> "$conf"
    echo "added [preferred] with the FileChooser line to $conf"
fi

# The activated process needs the display.
if command -v systemctl >/dev/null; then
    env="$(systemctl --user show-environment 2>/dev/null || true)"
    for var in WAYLAND_DISPLAY XDG_CURRENT_DESKTOP; do
        grep -q "^$var=" <<<"$env" || echo "warning: $var is not in the systemd user environment; run: dbus-update-activation-environment --systemd WAYLAND_DISPLAY DISPLAY XDG_CURRENT_DESKTOP"
    done
fi

restart_portal
cat <<EOF
done. Try it:
  GTK_USE_PORTAL=1 zenity --file-selection
Applications pick the portal dialog as described in README.md ("System file picker").
EOF
