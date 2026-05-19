# Commands

```text
ae [name]              Start or reattach a session
ae [name] use <alias>  Start session with a specific agent as main
ae list                List all sessions with agent health
ae status [name]       Show agent output without attaching
ae doctor              Check local environment and ae config
ae doctor --refresh [name|all]
                       Regenerate helper scripts and workspace.md in existing sessions
ae rename [old] <new>  Rename a running session
ae loop <start|stop|status> [name]
                       Toggle the stale-agent watchdog (per-session, persists across resume)
ae stop [name]         Pause session, keep state for later
ae end|rm [name]       Commit, push to ae/<name> branch, clean up
ae version             Show version
ae help                Show short help
```

When run inside an ae session, `stop`, `end`, `status`, `loop`, and `doctor --refresh` detect the current session automatically.

## Modes

ae creates sessions in one of three working-directory modes. Pick one with a flag at start time.

```bash
ae --local my-feature       # default — agents work in the current dir
ae --copy my-feature        # full cp -a; isolated copy
ae --worktree my-feature    # git worktree; lightweight branch isolation
```

See [Configuration → copy modes](../getting-started/config.md#copy-modes) for the trade-offs.

## `ae list`

Tabular view of every session known to ae, with per-agent health. Marks running sessions, stopped-but-persisted sessions, and shows the last-active timestamp.

## `ae status [name]`

Prints the last ~80 lines from each agent's pane without attaching. Useful for a quick "what is everyone doing" snapshot. Marks each agent's binary name and pane id.

## `ae doctor`

Pre-flight + post-upgrade self-test. Walks a fixed checklist of `OK / WARN / FAIL` items: bash/tmux/git presence, config file, agent executables, sessions directory, and so on. Returns non-zero if anything failed.

With `--refresh`, also regenerates every session helper from the currently-installed ae binary. Run after `git pull`:

```bash
ae doctor --refresh         # all sessions
ae doctor --refresh my-fix  # one session
```

## `ae loop`

```bash
ae loop start my-feature
ae loop stop my-feature
ae loop status my-feature
```

The [loop watchdog](../internals/loop.md) is on by default — only an explicit `false` / `no` / `off` / `0` in config or session meta keeps it off. `loop start` is idempotent; running it again just confirms the meta flag.

## `ae rename old-name new-name`

Rename a session. Renames the tmux session, moves the session directory, updates `session=` in meta, and regenerates `workspace.md` to reflect the new name. Running tmux server stays up.

## `ae stop`

Detach all agents, kill the tmux session, leave session state on disk. The next `ae <name>` resumes with the full conversation history (for agents that support session resume).

## `ae end` / `ae rm`

Wraps up:

1. Commits any pending changes in the working tree (or worktree).
2. Pushes to a branch named `ae/<session-name>` on the remote.
3. Kills the tmux session.
4. Removes session state from `~/.ae/sessions/`.

Pass `-f` to force without confirmation. `ae end all` ends every session.

## Hidden subcommands

The following are internal helpers ae invokes itself, prefixed with `_`. Don't call them directly:

- `_spawn`, `_retire` — pane lifecycle (called via `spawn` / `retire` session helpers).
- `_recover-pending` — re-attempt post-launch session ID capture (called by the loop watchdog).
- `_register-sid` — Codex first-task to self-register its session UUID (injected via `developer_instructions`).

They're listed only for transparency — your interface is the public commands above.
