#!/bin/bash
# Takes docs/screenshots/*.png, the gallery on the repository's front page:
# panels and thumbnails, the command palette, a 2x2 grid with a preview, the
# operations panel during a copy, the viewer, and Claude Code in the Agents
# panel driving the panels. Same setup as record.sh (see common.sh), on
# screenshots.toml and more demo data. Run it after `cargo build --release`
# and do not touch the keyboard while it runs (about 70 s). The agent shot
# needs Claude Code installed and logged in; its run is a real (short)
# session.

# A project, downloads and music next to the demo data of record.sh.
extra_data() {
    local p="$DATA/projects/aurora"
    mkdir -p "$p/src/render" "$p/assets/shaders" "$p/tests" "$p/benches" \
        "$DATA/Downloads" "$DATA/music/Nils Frahm - Spaces" "$DATA/music/Ólafur Arnalds - re:member"
    cat > "$p/src/main.rs" <<'RS'
//! Aurora: a small path tracer that renders a scene description to PNG.

mod camera;
mod render;
mod scene;

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;

/// Render a scene to an image.
#[derive(Parser)]
struct Args {
    /// The scene, in RON.
    scene: PathBuf,
    /// Samples per pixel.
    #[arg(short, long, default_value_t = 256)]
    samples: u32,
    /// Where the image goes.
    #[arg(short, long, default_value = "out.png")]
    output: PathBuf,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let scene = scene::load(&args.scene)
        .with_context(|| format!("reading {}", args.scene.display()))?;
    let camera = camera::Camera::looking_at(scene.eye, scene.target, 40.0);
    let image = render::trace(&scene, &camera, args.samples);
    image.save(&args.output)?;
    println!("{} written, {} samples", args.output.display(), args.samples);
    Ok(())
}
RS
    printf '//! The pinhole camera.\n' > "$p/src/camera.rs"
    printf '//! Scenes in RON.\n' > "$p/src/scene.rs"
    printf '//! Rays and bounces.\n' > "$p/src/render/mod.rs"
    printf '//! Materials.\n' > "$p/src/render/material.rs"
    printf '[package]\nname = "aurora"\nversion = "0.3.0"\nedition = "2024"\n' > "$p/Cargo.toml"
    printf '# Aurora\n\nA small path tracer.\n' > "$p/README.md"
    printf 'MIT License\n' > "$p/LICENSE"
    printf '#version 450\n' > "$p/assets/shaders/tonemap.frag"
    printf '#[test]\nfn renders() {}\n' > "$p/tests/render.rs"
    printf '// criterion\n' > "$p/benches/trace.rs"
    # Big downloads as sparse files: their size without the space.
    truncate -s 3900M "$DATA/Downloads/debian-13.1.0-amd64-DVD-1.iso"
    truncate -s 312M "$DATA/Downloads/blender-4.5.3-linux-x64.tar.xz"
    truncate -s 96M "$DATA/Downloads/rust-1.91.0-x86_64-unknown-linux-gnu.tar.xz"
    truncate -s 2400K "$DATA/Downloads/annual-report-2025.pdf"
    truncate -s 18M "$DATA/Downloads/MesloLGS-NF.zip"
    cp "$DATA/project.zip" "$DATA/Downloads/aurora-0.3.0.zip"
    local i=1
    for t in "An Aborted Beginning" "Says" "Hammers" "Familiar" "Over There, It's Raining"; do
        truncate -s "$((20 + i * 7))M" "$DATA/music/Nils Frahm - Spaces/0$i $t.flac"
        i=$((i + 1))
    done
    i=1
    for t in "re:member" "brotsjór" "saman" "unfold" "inconsist"; do
        truncate -s "$((15 + i * 5))M" "$DATA/music/Ólafur Arnalds - re:member/0$i $t.flac"
        i=$((i + 1))
    done
    printf 'notes\n' > "$DATA/music/playlist.m3u"
    # Dates spread over the last months, not all "now".
    local d=1
    for f in "$DATA"/Downloads/* "$p"/src/*.rs "$p"/*.* "$DATA"/music/*/*; do
        touch -d "-$((d * 37)) hours" "$f"
        d=$((d + 1))
    done
}
DEMO_CONFIG="$(dirname "$0")/screenshots.toml"
. "$(dirname "$0")/common.sh"
OUT="$ROOT/docs/screenshots"
mkdir -p "$OUT"
shot() {
    grim -g "$POS 1280x800" "$WORK/$1.png"
    magick "$WORK/$1.png" -strip -define png:compression-level=9 "$OUT/$1.png"
}
ctrl() { wtype -M ctrl "$@" -m ctrl; }
# The cursor moves by counting rows: the listings below are fixed by the demo
# data (directories first, then files, by name).
down() { for _ in $(seq "$1"); do k Down; sleep 0.12; done; }

sleep 2.5

# 1x2: two files marked on the left, the photos as thumbnails on the right.
down 8                                                                # .. docs Downloads music out photos projects src [notes.txt]
k Insert; sleep 0.3; k Insert; sleep 0.8
shot panels

# The command palette.
wtype -M ctrl -M shift p -m shift -m ctrl; sleep 0.8
wtype -d 80 'layout'; sleep 1
shot palette
k Escape; sleep 0.5

# 2x2, the last panel previewing a photo.
ctrl 2; sleep 1.2
k Tab; sleep 0.2; k Tab; sleep 0.2; k Tab; sleep 0.3
k Down; sleep 0.3; k Down; sleep 1.2
shot layout-2x2

# 2x1 with the operations panel: copying the DVD image down.
ctrl 3; sleep 1
ctrl j; sleep 0.6
down 4                                                                # .. annual aurora blender [debian]
k Insert; sleep 0.2; k Down; sleep 0.2; k Insert; sleep 0.3           # and rust, after MesloLGS
k F5; sleep 1; k Return; sleep 0.7
shot operations
sleep 6; ctrl j; sleep 0.5                                             # the copy is done; panel away

# The viewer on the project's main.rs.
ctrl 4; sleep 1
down 3; k Return; sleep 0.8                                         # .. assets benches [src]
down 3; k F3; sleep 1.5                                               # .. render camera.rs [main.rs]
shot viewer
ctrl w; sleep 0.8

# Claude Code in the Agents panel opens the photos in the right panel.
ctrl b; sleep 0.5                                                     # no sidebar: room for the agent
wtype -M ctrl -M shift n -m shift -m ctrl; sleep 1
k Return; sleep 8                                                     # Claude Code, the first choice
wtype -d 25 'Open the photos folder in the right panel and tell me how big the photos are together.'
sleep 0.3; k Return
# Approvals (folder trust, the vais tools) take Enter, their first answer:
# yes. An Enter with nothing to approve sends an empty line, which is ignored.
for _ in 1 2 3 4 5 6; do sleep 5; k Return; done
sleep 4
shot agents

ctrl q; sleep 1
stop
rm -f "$DATA/out/"*.iso "$DATA/out/"*.tar.xz                          # tmpfs: the copies are real
ls -la "$OUT"
