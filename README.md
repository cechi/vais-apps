# VAIS apps

VAIS stands for *vibecoded AI slop*. Desktop applications in Rust on
[gpui-kit](https://gpui-kit.com) (GPUI from Zed), kept in one Cargo workspace so they share
one dependency tree, one `Cargo.lock` and one `target/`.

| App | What it is |
| --- | --- |
| [VAIS Commander](apps/vais-commander/README.md) | Keyboard-driven file manager with a configurable grid of panels |

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
