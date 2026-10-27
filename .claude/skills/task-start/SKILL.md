---
name: task-start
description: Starts an agent on a vais-apps task in its own worktree (feature/<task>, reflinked target/) and its own herdr tab with a fresh Claude session that the user can talk to. Also lists running tasks and closes finished ones. Use for "pusť na to agenta", "nový agent na …", "task-start", "zavři úkol …".
argument-hint: <task>[:model[:effort]] <what to do> | --list | --close <task>...
---

# /task-start

Arguments: `$ARGUMENTS`

Script: `node .claude/skills/task-start/task-start.mjs` (from the main checkout; timeout 300000).
It must run inside herdr (`HERDR_ENV=1`).

## Starting a task

1. **Name**: short, kebab-case, the app first: `lab-repl`, `commander-sftp-resume`. The branch is
   `feature/<name>`, the worktree `.claude/worktrees/<name>`, and the herdr tab and session get the
   same name.
2. **Model and effort**: if the user did not name them, choose:

   | task | model:effort |
   |---|---|
   | clear, local change or fix | `opus:medium` |
   | several layers, unclear, UI that needs a nested-niri check | `opus:high` |
   | design work, a new app or module | `opus:high`, or `fable:high` only when the user asks (Fable may be out of credits) |

3. **Brief**: write it to the scratchpad as Markdown. Cover the goal, what to read (files, other
   apps as reference), the scope (what not to do yet, decisions left to the user), and what it
   should produce. The script appends the fixed rules itself: worktree and branch, CLAUDE.md,
   memory path, commits only to the branch, no push, own nested niri, Czech, final summary. Do not
   repeat those.
4. Run:
   ```bash
   node .claude/skills/task-start/task-start.mjs <name>[:model[:effort]] --brief <file> [--base master]
   ```
   It creates the worktree from `master` (or reuses the existing `feature/<name>` branch), copies
   `target/` as a reflink, copies the brief into the worktree's git dir, opens the tab without
   focus, starts `claude --permission-mode auto -n <name>` and sends the prompt. If the session
   stops right away (out of usage credits, a bad model), the script exits with 1 and leaves the
   tab open. Tell the user; switching the model with `/model` in that tab also changes their
   default model.
5. Tell the user the tab, the branch, the model with one sentence on why, and the scope.

## Following up

- `--list`: task worktrees, their branches, commits ahead of master, and tab and agent state.
- Only when asked: `herdr agent get <name>`, `herdr agent read <name>`,
  `herdr agent wait <name> --until done`.
- Never answer questions or approvals in an agent's tab; pass them on to the user.

## Finishing

The main session merges per CLAUDE.md (`cargo fmt`, tests, a nested-niri check, a merge into
master in the main checkout). Then:

```bash
node .claude/skills/task-start/task-start.mjs --close <name>...
```

This closes the tab and removes the worktree when it has no uncommitted changes (its `target/`
goes with it). The branch is deleted only when it is merged into master; otherwise it stays.
