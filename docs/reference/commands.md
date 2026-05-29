# Commands

```text
ae [name]              Start or reattach a session
ae [name] use <alias>  Start session with a specific agent as main
ae list [--all|--stopped|--needs-me]
                       List sessions (running by default; --all adds stopped
                       history, --needs-me only those waiting on you)
ae status [name]       Show agent output without attaching
ae doctor              Check local environment and ae config
ae doctor --refresh [name|all]
                       Regenerate helper scripts and workspace.md in existing sessions
ae rename [old] <new>  Rename a running session
ae loop <start|stop|status> [name]
                       Toggle the stale-agent watchdog (per-session, persists across resume)
ae telegram <setup|start|stop|status>
                       Machine-global Telegram bridge — see Telegram bridge reference
ae stop [name]         Pause session, keep ae + agent conversation state for resume
ae end|rm [name]       End session: commit, push to ae/<name>, REMOVE ae state AND
                       per-session claude/codex conversation files. Destructive.
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

Tabular view of ae sessions with per-agent health, declared state, and a
session-level `attn:<reason>` marker when a session needs attention.

The marker is a derived rollup — the single most-actionable reason across the
session's agents, by severity:

| Reason | Meaning |
|--------|---------|
| `attn:dead` | an agent's pane vanished (or the watchdog flagged it missing) |
| `attn:stale` | the loop watchdog gave up nudging an idle agent (max nudges) |
| `attn:waiting-user` | an agent declared it's waiting on you |
| `attn:blocked` | an agent declared it's blocked on an external dep |
| `attn:throttled` | an agent is being rate-limited upstream |

(`dead`/`stale`/`throttled` reuse the loop watchdog's own alert events;
`waiting-user`/`blocked` are self-declared. Pending unanswered `ask`/`review`
requests are a planned future reason.)

By default it shows **running sessions only** — stopped sessions are usually the
bulk of the list and just noise for monitoring. Flags:

| Flag | Shows |
|------|-------|
| *(none)* / `--running` | running sessions only |
| `--all` | running sessions, then stopped ones |
| `--stopped` | stopped sessions only |
| `--needs-me` | only running sessions with an `attn:` reason; aliases: `--needs`, `--attn` |

For a live dashboard, wrap it with `watch`:

```bash
watch -n 10 'ae list'            # live view of running sessions
watch -n 10 'ae list --needs-me' # only what needs your attention
```

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

## `ae telegram`

```bash
ae telegram setup       # interactive: writes [telegram] config + token file
ae telegram start       # spawn daemon now, persist enabled=true
ae telegram stop        # kill daemon, persist enabled=false
ae telegram status      # report intent + runtime + deps + token validation
```

Machine-global daemon that forwards filtered events from every ae session on this host to one Telegram chat. Single instance per machine (lock-guarded). Read-only in Stage 2 — chat → ae routing is Stage 3.

`jq` + `curl` are feature-only dependencies; ae's core commands work without them. See the [Telegram bridge](telegram.md) page for setup, config schema, and lifecycle.

## `ae rename old-name new-name`

Rename a session. Renames the tmux session, moves the session directory, updates `session=` in meta, and regenerates `workspace.md` to reflect the new name. Running tmux server stays up.

## `ae stop`

Pause a session for later resume. Detaches all agents and kills the tmux session, but leaves everything on disk: ae state at `~/.ae/sessions/<name>/` plus the per-agent conversation files at `~/.claude/projects/.../<uuid>.jsonl` and `~/.codex/sessions/.../<uuid>.jsonl`. The next `ae <name>` resumes with the full conversation history.

Use this when you're done for the day, switching contexts, or moving to another machine via `ae transfer`.

## `ae end` / `ae rm`

End a session for good. **Destructive** — removes the conversation history. If you want to resume later, use `ae stop` instead.

Wraps up:

1. Commits any pending changes in the working tree (or worktree).
2. Pushes to a branch named `ae/<session-name>` on the remote.
3. Kills the tmux session.
4. Removes ae state at `~/.ae/sessions/<name>/`.
5. **Removes the per-session Claude / Codex conversation files** (jsonl + rollout) for each agent slot in the session. Tool detection uses `agent_bin.<slot>` from meta. Gemini and OpenCode conversation files are left in place — their lookup helpers don't exist yet.

Pass `-f` to force without confirmation. `ae end all` ends every session.

## Hidden subcommands

The following are internal helpers ae invokes itself, prefixed with `_`. Don't call them directly:

- `_spawn`, `_retire` — pane lifecycle (called via `spawn` / `retire` session helpers).
- `_recover-pending` — re-attempt post-launch session ID capture (called by the loop watchdog).
- `_register-sid` — Codex first-task to self-register its session UUID (injected via `developer_instructions`).

They're listed only for transparency — your interface is the public commands above.
