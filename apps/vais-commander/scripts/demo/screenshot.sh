#!/bin/bash
# Takes docs/screenshot.png, the picture on the repository's front page: the
# demo project on the left with two files marked, its photos as thumbnails
# on the right. Same setup as record.sh (see common.sh); run it after
# `cargo build --release` and do not touch the keyboard while it runs
# (about 15 s).

# The right panel opens on the photos as thumbnails.
tweak_config() {
    python3 - "$1" <<'PY'
import sys
path = sys.argv[1]
s = open(path).read()
old = 'path = "/tmp/vais-demo/out"\nshow_hidden = false\nsort = "name"\nsort_desc = false\nview = "full"'
assert s.count(old) == 1, "demo config changed: update screenshot.sh"
open(path, "w").write(s.replace(old, old.replace("/out", "/photos").replace('"full"', '"thumbnails"')))
PY
}
. "$(dirname "$0")/common.sh"

sleep 2.5                                                               # thumbnails render
for _ in 1 2 3 4 5; do k Down; sleep 0.2; done
k Insert; sleep 0.3; k Insert; sleep 0.8                                # mark two files
grim -g "$POS 1280x800" "$WORK/shot.png"
wtype -M ctrl q -m ctrl; sleep 1
stop

magick "$WORK/shot.png" -strip -define png:compression-level=9 "$ROOT/docs/screenshot.png"
ls -la "$ROOT/docs/screenshot.png"
