#!/bin/bash
# Records docs/demo.gif and docs/demo.mp4: a scripted tour of the app in a
# nested niri compositor. Needs niri, grim, wtype, ffmpeg, zip and
# ImageMagick; run it after `cargo build --release`.
# The nested compositor opens as a window of your session; do not touch the
# keyboard while it runs (about 40 s).
. "$(dirname "$0")/common.sh"
mkdir -p "$WORK/frames"

# ----- frames at ~8 fps while the scenario plays ------------------------------
( while true; do grim -g "$POS 1280x800" "$WORK/frames/f_$(date +%s%3N).png"; sleep 0.07; done ) &
GP=$!
sleep 1.5
wtype -d 120 're'; sleep 1.6; k Escape; sleep 0.8                       # quick search
k Down; sleep 0.25; k Down; sleep 0.25; k Down; sleep 0.4; k Return; sleep 0.9
wtype -M ctrl -P F3 -p F3 -m ctrl; sleep 2.8                            # thumbnails
wtype -M ctrl -P F2 -p F2 -m ctrl; sleep 0.7; k BackSpace; sleep 0.9
k Down; sleep 0.25; k Down; sleep 0.4; k Insert; sleep 0.4; k Insert; sleep 0.6
k F5; sleep 1.4; k Return; sleep 1.8                                    # copy
wtype -M ctrl j -m ctrl; sleep 0.6
k Up; sleep 0.4; k Return; sleep 1.0                                    # into the zip
k Down; sleep 0.25; k Down; sleep 0.4; k Return; sleep 0.9; k Down; sleep 0.25; k Down; sleep 0.4; k F3; sleep 2.2
wtype -M ctrl w -m ctrl; sleep 0.8; k BackSpace; sleep 0.6; k BackSpace; sleep 0.9
wtype -M ctrl -M shift p -m shift -m ctrl; sleep 0.9; wtype -d 120 'sort'; sleep 1.6; k Escape; sleep 0.7
wtype -M ctrl e -m ctrl; sleep 0.5; wtype -d 90 'ls -la src'; sleep 0.4; k Return; sleep 2.2
wtype -M ctrl w -m ctrl; sleep 1.4
kill "$GP"; wait "$GP" 2>/dev/null || true
wtype -M ctrl q -m ctrl; sleep 1
stop

# ----- assemble ----------------------------------------------------------------
cd "$WORK"
python3 - <<'PY'
import os
fs = sorted(f for f in os.listdir('frames') if f.endswith('.png'))
ts = [int(f[2:-4]) for f in fs]
with open('concat.txt', 'w') as out:
    for i, f in enumerate(fs):
        d = (ts[i + 1] - ts[i]) / 1000 if i + 1 < len(fs) else 1.0
        out.write(f"file 'frames/{f}'\nduration {d:.3f}\n")
    out.write(f"file 'frames/{fs[-1]}'\n")
print(len(fs), "frames,", (ts[-1] - ts[0]) / 1000, "s")
PY
ffmpeg -y -loglevel error -f concat -safe 0 -i concat.txt \
    -vf "fps=10,scale=1000:-1:flags=lanczos,split[s0][s1];[s0]palettegen=max_colors=128:stats_mode=diff[p];[s1][p]paletteuse=dither=bayer:bayer_scale=4:diff_mode=rectangle" \
    "$ROOT/docs/demo.gif"
ffmpeg -y -loglevel error -f concat -safe 0 -i concat.txt -vf "fps=15,scale=1280:-2" \
    -c:v libx264 -preset slow -crf 26 -pix_fmt yuv420p -movflags +faststart "$ROOT/docs/demo.mp4"
ls -la "$ROOT/docs/demo.gif" "$ROOT/docs/demo.mp4"
