# The setup shared by record.sh and screenshot.sh: demo data, a nested niri
# compositor and the app in it on the demo config, with guards so keys only
# ever reach that window. Sourced, after an optional tweak_config() that
# edits the copied config; leaves ROOT, WORK, POS (the window's position for
# grim), the exported WAYLAND_DISPLAY, k() and stop() behind.
set -eu
ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
WORKSPACE=$(cd "$ROOT/../.." && pwd)
WORK=${DEMO_WORK:-$(mktemp -d)}
DATA=/tmp/vais-demo
mkdir -p "$WORK/xdg/vais-commander" "$WORK/xdg-cache"

# ----- demo data: a small project with photos and an archive -----------------
rm -rf "$DATA"; mkdir -p "$DATA/docs" "$DATA/src/ui" "$DATA/photos" "$DATA/out"
printf 'fn main() {\n    println!("hello from VAIS");\n}\n' > "$DATA/src/main.rs"
printf 'pub fn render() {}\n' > "$DATA/src/ui/panel.rs"
printf '# Project\n\nA small demo project.\n' > "$DATA/README.md"
printf 'notes\n' > "$DATA/notes.txt"
printf 'release checklist\n' > "$DATA/docs/release.md"
printf 'design notes\n' > "$DATA/docs/design.md"
for i in 1 2 3 4 5 6; do
    magick -size 640x480 gradient:"hsl($((i*55)),80%,45%)-hsl($((i*55+40)),90%,70%)" \
        -swirl $((i*30)) "$DATA/photos/photo-$i.jpg"
done
(cd "$DATA" && zip -qr project.zip src docs README.md)

# last_version = this build's, so the What's new tab stays closed.
VERSION=$(grep -m1 '^version' "$ROOT/Cargo.toml" | sed -E 's/.*"([^"]+)".*/\1/')
sed "s/^version = 2$/version = 2\nlast_version = \"$VERSION\"/" "$ROOT/scripts/demo/config.toml" \
    > "$WORK/xdg/vais-commander/config.toml"
# A script can adjust the copy before the app reads it.
if declare -F tweak_config > /dev/null; then tweak_config "$WORK/xdg/vais-commander/config.toml"; fi

# ----- nested compositor and the app ------------------------------------------
niri -c "$ROOT/scripts/demo/niri.kdl" > "$WORK/niri.log" 2>&1 &
NP=$!
sleep 2
WD=$(grep -o 'wayland-[0-9]*' "$WORK/niri.log" | head -1)
SOCK=$(grep -o '/run/user/[0-9]*/niri\.wayland-[0-9]*\.[0-9]*\.sock' "$WORK/niri.log" | head -1)
WAYLAND_DISPLAY=$WD XDG_CONFIG_HOME=$WORK/xdg XDG_CACHE_HOME=$WORK/xdg-cache \
    "$WORKSPACE/target/release/vais-commander" > "$WORK/app.log" 2>&1 &
AP=$!
sleep 5
stop() { kill "$AP" 2>/dev/null || true; kill "$NP" 2>/dev/null || true; }
# Guards: keys go only to our window, running on the demo config.
NIRI_SOCKET=$SOCK niri msg --json focused-window | grep -q '"title":"VAIS Commander' || { echo "focus guard failed"; stop; exit 1; }
grep -q "ignoring invalid config" "$WORK/app.log" && { echo "config guard failed"; stop; exit 1; }
WIN=$(NIRI_SOCKET=$SOCK niri msg --json windows)
echo "$WIN" | grep -q '"window_size":\[1280,800\]' || { echo "window guard failed: $WIN"; stop; exit 1; }
POS=$(echo "$WIN" | python3 -c 'import json,sys; w=[w for w in json.load(sys.stdin) if w["title"].startswith("VAIS")][0]; x,y=w["layout"]["tile_pos_in_workspace_view"]; print(f"{int(x)},{int(y)}")')
export WAYLAND_DISPLAY=$WD
k() { wtype -P "$1" -p "$1"; }
