<h1 align="center">VAIS apps</h1>

<p align="center">
  <b>Keyboard-first desktop apps in Rust, built on <a href="https://gpui-kit.com">gpui-kit</a> (GPUI from Zed).</b>
</p>

<p align="center">
  <img alt="Written in Rust" src="https://img.shields.io/badge/written%20in-Rust-30a46c?style=flat-square&logo=rust&logoColor=white">
  <img alt="Runs on Linux, macOS and Windows" src="https://img.shields.io/badge/runs%20on-Linux%20%C2%B7%20macOS%20%C2%B7%20Windows-30a46c?style=flat-square">
  <img alt="License: MIT" src="https://img.shields.io/badge/license-MIT-30a46c?style=flat-square">
</p>

VAIS stands for *vibecoded AI slop*: these apps are written with AI coding agents, one feature
at a time. They share a look (the Cechi light and dark themes), the habit of doing everything
from the keyboard and, as more apps join, their building blocks. Linux is where they are
developed and tested; CI builds and tests them on macOS and Windows as well.

## The apps

### <img src="apps/vais-commander/assets/vais-commander.svg" width="28" height="28" alt=""> VAIS Commander

A keyboard-driven file manager in the spirit of Total and Midnight Commander: a configurable grid
of panels in tabs, SFTP and FTP, archives, thumbnails, a command palette and coding agents that
work with the panels.

[README](apps/vais-commander/README.md) · [Download](https://github.com/cechi/vais-apps/releases/latest) · [What's new](apps/vais-commander/CHANGELOG.md)

<!-- scripts/demo/screenshots.sh in apps/vais-commander takes these again. -->
<table>
<tr>
<td width="50%" valign="top"><a href="apps/vais-commander/docs/screenshots/panels.png"><img src="apps/vais-commander/docs/screenshots/panels.png" alt="VAIS Commander with the places sidebar, a demo project with two marked files on the left and its photos as thumbnails on the right" width="100%"></a><p align="center"><sub><b>Panels.</b> Marks, sorting, columns and thumbnails; the places sidebar with favorites, volumes, trash and recent folders.</sub></p></td>
<td width="50%" valign="top"><a href="apps/vais-commander/docs/screenshots/layout-2x2.png"><img src="apps/vais-commander/docs/screenshots/layout-2x2.png" alt="A 2x2 grid of panels: source files, downloads sorted by size, music in the brief view and photos with a preview of the one under the cursor" width="100%"></a><p align="center"><sub><b>Layouts.</b> 1x2, 2x1, 2x2 up to 3x3 (Alt+1…9), each tab with its own grid; previews beside or under the list.</sub></p></td>
</tr>
<tr>
<td width="50%" valign="top"><a href="apps/vais-commander/docs/screenshots/operations.png"><img src="apps/vais-commander/docs/screenshots/operations.png" alt="Two panels stacked in a 2x1 layout and the operations panel at the bottom copying a 3.9 GiB DVD image" width="100%"></a><p align="center"><sub><b>Operations.</b> Copies, moves and deletes run in the background, with speed and progress in the operations panel (Ctrl+J).</sub></p></td>
<td width="50%" valign="top"><a href="apps/vais-commander/docs/screenshots/agents.png"><img src="apps/vais-commander/docs/screenshots/agents.png" alt="Claude Code in the Agents panel at the right, which opened the photos in the right panel and added up their sizes" width="100%"></a><p align="center"><sub><b>Agents.</b> Claude Code, OpenCode or a shell in the Agents panel (Ctrl+Shift+B); the agents drive the panels over MCP.</sub></p></td>
</tr>
<tr>
<td width="50%" valign="top"><a href="apps/vais-commander/docs/screenshots/viewer.png"><img src="apps/vais-commander/docs/screenshots/viewer.png" alt="The viewer on a Rust source file with syntax highlighting and line numbers" width="100%"></a><p align="center"><sub><b>Viewer and editor.</b> F3 and F4 open files in a tab, with syntax highlighting.</sub></p></td>
<td width="50%" valign="top"><a href="apps/vais-commander/docs/screenshots/palette.png"><img src="apps/vais-commander/docs/screenshots/palette.png" alt="The command palette filtered to the layout commands, with their shortcuts" width="100%"></a><p align="center"><sub><b>Command palette.</b> Every command with its shortcut (Ctrl+Shift+P).</sub></p></td>
</tr>
</table>

## Layout

```
apps/<app>/      one crate per application, with its assets, scripts and CHANGELOG.md
Cargo.toml       the workspace: members and build profiles
```

## Build

```sh
cargo build                      # every app
cargo run -p vais-commander      # one app
cargo test                       # all tests
```

## Releases

Each app is released on its own. The tag names the app and its version, e.g.
`vais-commander-v0.5.0`: the `Release` workflow builds that app, checks the tag against its
`Cargo.toml` and publishes a release named after its README heading ("VAIS Commander
0.5.0") with its `CHANGELOG.md` section as the notes. A suffix after the version
(`-beta.1`) makes a pre-release.

All apps share the repository's release list, but only VAIS Commander's releases become
**Latest** (`LATEST_APP` in `.github/workflows/release.yml`), so
`/releases/latest/download/...` keeps pointing at it. Links to another app's downloads name
its tag.

## License

[MIT](LICENSE)
