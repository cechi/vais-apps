# VAIS apps

A Cargo workspace of desktop apps in Rust on gpui-kit. Each app is a crate in `apps/<app>/`
with its own assets, scripts, `README.md` and `CHANGELOG.md`; the workspace root holds the
members, the build profiles, one `Cargo.lock` and one `target/`. See `README.md` for builds
and the release scheme (tags `<app>-vX.Y.Z`).

## Commits

- Commit with the repository's local identity (already in `.git/config`, shared by every
  worktree), never a global one.
- `cargo fmt --all` before every commit: CI's first step is `cargo fmt --all -- --check`.
- One feature per commit, with its README update and, when users see it, a line under
  `## Unreleased` in the app's `CHANGELOG.md`.
- Push only when the user asks.

## Parallel work: one worktree per task

The main checkout stays on `master`. That is where work is merged and released from, and
the user's running VAIS Commander is a symlink into its `target/release`. Work that runs in
parallel with other work goes into a worktree per task, not per app:

```sh
git worktree add -b feature/<app>-<task> .claude/worktrees/<task>
cp -a --reflink=always target .claude/worktrees/<task>/target   # btrfs: seconds, no extra space
```

- Without the reflinked `target/` a worktree builds gpui from scratch (tens of GB, many
  minutes). Do not share one `CARGO_TARGET_DIR` between worktrees: cargo locks it, so builds
  queue up, and the workspace crates of different branches keep rebuilding each other.
- An agent in a worktree commits only to its feature branch: no commits to `master`, no
  push, no `scripts/install-desktop.sh` (it relinks the user's live binary).
- The main session merges a finished branch into `master` (`cargo fmt`, tests, a check in a
  nested niri), then removes the worktree: `git worktree remove .claude/worktrees/<task>`.
- When only one task is running, work in the main checkout; no worktree is needed.
- `/task-start` (`.claude/skills/task-start/`) does all of this: worktree, reflinked
  `target/`, a herdr tab with its own Claude session and the brief; `--close` cleans up.

## Shared code

Code used by more than one app moves into a shared crate (`crates/vais-core`) only when a
second app needs it, not ahead of time. Changes to shared code go in small commits that are
merged into `master` right away, not with the rest of a feature, and other branches rebase
onto them. Two branches editing the same shared files at once makes slow, painful merges.

## Testing the UI

UI changes are verified in a nested niri compositor, never by sending keys to the user's
live session.

- Every agent that runs UI tests at the same time needs its own nested niri and its own
  `XDG_CONFIG_HOME` / `XDG_CACHE_HOME` under its scratchpad. Two agents sending keys into
  one nested niri collide. Each new nested niri also opens a window on the user's desktop,
  so keep one per agent running between tests rather than starting one per test.
- A test instance of VAIS Commander takes over
  `/run/user/1000/vais-commander/bridge.sock`. After killing the test instance, remove its
  socket and point `bridge.sock` back at the live instance's `bridge-<pid>.sock`.
- Run test copies of a binary from the scratchpad or a worktree's `target/`, never through
  `~/.local/bin`.
