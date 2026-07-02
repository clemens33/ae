# Commands

```text
ae [name]              Start or reattach a session
ae [name] use <alias>  Start session with a specific agent as main
ae list [--all|--stopped|--needs-attn]
                       List sessions (running by default; --all adds stopped
                       history, --needs-attn only those needing attention)
ae status [name]       Show agent output without attaching
ae next [--attach]     Name the top running session needing attention (read-only;
                       alias: ae jump). --attach jumps to it. Non-zero when none.
ae doctor              Check local environment and ae config
ae doctor --refresh [name|all]
                       Regenerate helper scripts and workspace.md in existing sessions
ae rename [old] <new>  Rename a running session
ae watchdog <start|stop|status> [name]
                       Toggle the stale-agent watchdog (per-session, persists across resume)
ae telegram <setup|start|stop|status>
                       Machine-global Telegram bridge — see Telegram bridge reference
ae steward [--attach|--init]
                       Ensure the detached steward (meta-agent) is running; --attach
                       switches to it (--init scaffolds config + charter)
ae stop [name]         Pause session, keep ae + agent conversation state for resume
ae end|rm [-f] [--purge-history|--keep-history] [name]
                       End session: commit, push to ae/<name>, remove ae state. KEEPS the
                       per-session claude/codex conversation files by default (token history);
                       --purge-history deletes them.
ae version             Show version
ae help                Show short help
```

When run inside an ae session, `stop`, `end`, `status`, `watchdog`, and `doctor --refresh` detect the current session automatically.

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
| `attn:stale` | the watchdog gave up nudging an idle agent (max nudges) |
| `attn:waiting-user` | an agent declared it's waiting on you |
| `attn:blocked` | an agent declared it's blocked on an external dep |
| `attn:throttled` | an agent is being rate-limited upstream |

(`dead`/`stale`/`throttled` reuse the watchdog's own alert events;
`waiting-user`/`blocked` are self-declared. Pending unanswered `ask`/`review`
requests are a planned future reason.)

By default it shows **running sessions only** — stopped sessions are usually the
bulk of the list and just noise for monitoring. Flags:

| Flag | Shows |
|------|-------|
| *(none)* / `--running` | running sessions only |
| `--all` | running sessions, then stopped ones |
| `--stopped` | stopped sessions only |
| `--needs-attn` | only running sessions with an `attn:` reason; aliases: `--needs-me`, `--needs`, `--attn` |
| `--active` | only running sessions with recent activity (an ae event within ~5 min; `AE_LIST_ACTIVE_SECS` to tune); alias: `--busy` |
| `--json` | machine-readable digest (honours the filters above) |

For a live dashboard, wrap it with `watch`:

```bash
watch -n 10 'ae list'            # live view of running sessions
watch -n 10 'ae list --needs-attn' # only what needs your attention
```

### `--json` digest

`ae list --json` emits a single JSON object — a snapshot for a monitoring
script or agent. Pure bash output; no `jq` required to produce it. The filters
(`--running`/`--all`/`--stopped`/`--needs-attn`) decide which sessions appear.

```json
{
  "schema_version": 1,
  "generated_at": "2026-05-29T14:00:00Z",
  "sessions": [
    {
      "name": "my-feature", "status": "running",
      "mode": "local", "origin": "/…", "work_dir": "/…",
      "last_active_epoch": 1780000000,
      "needs_attention": true, "attention": "blocked", "attention_rank": 2,
      "agents": [
        {"ref": "claude:lead", "alias": "claude", "name": "lead",
         "session_id": "e795c9e9", "alive": true, "state": "blocked",
         "reason": "blocked"}
      ]
    }
  ]
}
```

`attention` is the session's single most-actionable reason (see the reason
table above); each agent's `reason` is its own contribution. `schema_version`
lets consumers gate on shape. Unanswered `ask`/`review` request edges and
richer per-agent timing fields are planned additions.

## `ae status [name]`

Prints the last ~80 lines from each agent's pane without attaching. Useful for a quick "what is everyone doing" snapshot. Marks each agent's binary name and pane id.

## `ae next` (alias `ae jump`)

The attention navigator — the action half of `ae list`. Names the single
**top-ranked running session needing attention**, using the *same* rollup and
severity ranking as `ae list` (`dead > stale > waiting-user > blocked >
throttled`):

```text
$ ae next
my-feature  attn:blocked  rank:2  codex:coworker
```

Read-only by default (it does not change tmux focus). Exits **non-zero** with a
message when nothing needs attention, so it composes in scripts and is a clean
primitive for a future monitoring agent. Tie-break across equally-severe
sessions: most-recent activity, then session name ascending (deterministic).

With **`--attach`** (alias `--switch`) it jumps straight to that session —
`tmux switch-client` when you're already inside tmux, `tmux attach-session`
otherwise. It re-checks the session still exists first, and no-ops with a
message if you're already in it. `-h`/`--help` prints usage; an unknown argument
exits non-zero.

```text
$ ae next --attach
# → switches your tmux client to my-feature (the blocked session)
```

## `ae doctor`

Pre-flight + post-upgrade self-test. Walks a fixed checklist of `OK / WARN / FAIL` items: bash/tmux/git presence, config file, agent executables, sessions directory, and so on. Returns non-zero if anything failed.

With `--refresh`, also regenerates every session helper from the currently-installed ae binary. Run after `git pull`:

```bash
ae doctor --refresh         # all sessions
ae doctor --refresh my-fix  # one session
```

## `ae watchdog`

```bash
ae watchdog start my-feature
ae watchdog stop my-feature
ae watchdog status my-feature
```

The [watchdog](../internals/watchdog.md) is on by default — only an explicit `false` / `no` / `off` / `0` in config or session meta keeps it off. `watchdog start` is idempotent; running it again just confirms the meta flag.

### Meta-agent (steward) sweep cadence

A session marked as the fleet steward with `[workspace] steward = true` (or its
legacy aliases `hub = true` / `meta = true`; persisted to
its meta as `meta_agent=true`) gets a different watchdog behaviour for its **main
agent**: instead of the stale-nudge watchdog, the watchdog sends a *"run your sweep
now"* nudge every `AE_WATCHDOG_SWEEP_SEC` seconds (default 300) and never escalates
the steward to a stale `attn:` alert (idle between sweeps is normal for a monitor).
Workers/spawned agents in the same session keep the normal watchdog.

Liveness is still guarded two ways: the dead/missing-pane checks catch a crashed
steward, and a **heartbeat** check catches a *live-but-not-sweeping* steward (model
stall, upstream throttle, wedge) — the steward's sweep helper rewrites
`~/.ae/sessions/<steward>/meta-agent-state.json` on each real sweep, and if that mtime
stops advancing past ~`2×AE_WATCHDOG_SWEEP_SEC` the watchdog raises one alert (cleared on
recovery). This is the file [`contrib/aemonitor`](../../contrib/aemonitor/) writes
by default; if you override its `--state` path for the steward, point it at this same
file or the watchdog heartbeat will false-alarm. The sweep nudges use `action=nudge`,
which is **not in the default telegram include set**, so routine sweeps don't
reach your phone (a custom `include` containing `nudge` would forward them).

## `ae steward`

The **steward** — your fleet's chief of staff: a single ae session that monitors
all your *other* ae sessions and is your one point of contact to them (it relays
your instructions to the other sessions and reports what needs you, via the
Telegram `say` channel). Once you set an objective (`objective: …` over Telegram) it also holds it, parks
your ideas, and answers `what next` — and may proactively nudge you when you drift,
but only through hard gates (concrete signal held two sweeps, a rate budget, quiet
hours, suggest-only; ignore a couple and it self-mutes for the day). See
[`contrib/aesteward`](../../contrib/aesteward/). It
is a monitor + relay + focus aide: per its charter it never ends/stops/edits
another session on its own, and only suggests — it dispatches nothing without
your say-so.

```text
ae steward          Ensure the detached `steward` session is running
ae steward --attach Switch/attach to the `steward` session
ae steward --init   Scaffold ~/.ae/steward/{steward.config,CHARTER.md} (never overwrites)
ae steward --help   Usage
```

`ae steward` launches the `steward` session with **full config isolation**: it
uses `~/.ae/steward/steward.config` as the config and neutralizes any
project-local `./.ae/config`, so the global config's `workers` never leak into
the single-agent steward regardless of the directory you run it from. The config
dir defaults to `${AE_HOME:-~/.ae}/steward` and is overridable with
`AE_STEWARD_DIR` (so an isolated `AE_HOME` run keeps its steward state out of
your live `~/.ae`).
Unlike normal `ae <name>` session starts, bare `ae steward` does **not** attach
or switch the current tmux client; use `ae steward --attach` when you want to
inspect the steward pane directly.

First time: run `ae steward --init` to scaffold the config + charter from
[`contrib/aesteward`](../../contrib/aesteward/) (placeholders for the charter and
[`aemonitor`](../../contrib/aemonitor/) paths are substituted), edit them to
taste, then `ae steward`. The charter wires the deterministic sweep to
`aemonitor`, defines the objective-armed focus aide, and tells the agent its only channel to you is
`say`.

To talk to the steward from your phone, run the [Telegram bridge](telegram.md):
plain messages route to the running steward automatically (no `/use` setup), and
`/use <session> <agent>` redirects to another session when you want (`/use clear`
returns to the steward) — see
[Steward-centric routing](telegram.md#steward-centric-routing-talk-to-the-meta-agent-not-ten-sessions).

**Deprecated alias + legacy scaffolds:** `ae hub` still works and maps to the
same launcher. A pre-rename `~/.ae/meta-hub/hub.config` scaffold (from
`ae hub --init`) is still honoured — it keeps its `hub` session name so its baked
charter paths and resume state stay consistent (`AE_HUB_DIR` is honoured too).
Migrate with `ae end hub && ae steward --init && ae steward`.

`steward` is a reserved subcommand (as is `hub`). If you ever need a normal
session literally named `steward`, `ae --local steward` reaches the generic start
path (the first argument is then no longer `steward`).

## `ae telegram`

```bash
ae telegram setup       # interactive: writes [telegram] config + token file
ae telegram start       # spawn daemon now, persist enabled=true
ae telegram stop        # kill daemon, persist enabled=false
ae telegram status      # report intent + runtime + deps + token validation
```

Machine-global daemon that bridges every ae session on this host to one Telegram chat. Single instance per machine (lock-guarded). Outbound forwards filtered events to chat. Inbound (when `allowed_user_ids` is set) offers three ways to reach an agent: **reply** to a forwarded event (routes to that agent), the compact **`@session:agent <msg>`** prefix, and a sticky **`/use <session> <agent>`** default for plain messages — plus the explicit `/list` and `/session <name|id-prefix> send|ask <agent> <msg>`. All paths share the same session/agent revalidation. Inbound is from the configured private chat only — auth requires matching `from.id` + `chat.id` + a private chat.

`jq` + `curl` are feature-only dependencies; ae's core commands work without them. See the [Telegram bridge](telegram.md) page for setup, config schema, inbound trust boundary, and lifecycle.

## `ae rename old-name new-name`

Rename a session. Renames the tmux session, moves the session directory, updates `session=` in meta, and regenerates `workspace.md` to reflect the new name. Running tmux server stays up.

## `ae stop`

Pause a session for later resume. Detaches all agents and kills the tmux session, but leaves everything on disk: ae state at `~/.ae/sessions/<name>/` plus the per-agent conversation files at `~/.claude/projects/.../<uuid>.jsonl` and `~/.codex/sessions/.../<uuid>.jsonl`. The next `ae <name>` resumes with the full conversation history.

Use this when you're done for the day, switching contexts, or moving to another machine via `ae transfer`.

## `ae end` / `ae rm`

End a session for good. Removes ae's own state; **keeps the agent conversation
history by default**. If you want to resume later, use `ae stop` instead.

Wraps up:

1. Commits any pending changes in the working tree (or worktree).
2. Pushes to a branch named `ae/<session-name>` on the remote.
3. Kills the tmux session.
4. Removes ae state at `~/.ae/sessions/<name>/`.
5. **Keeps the per-session Claude / Codex conversation files** (jsonl + rollout) by
   default — they are the only local record of that session's token usage, retained
   for later usage/cost reporting. Purge them with `ae end --purge-history` (or set
   `[workspace] purge_agent_history = true` as the default). Tool detection uses
   `agent_bin.<slot>` from meta; Gemini and OpenCode files are always left in place.

### Controlling conversation-file cleanup

| Precedence | Source | Effect |
|---|---|---|
| 1 (highest) | `ae end --purge-history` / `--keep-history` | Force purge / keep for this run |
| 2 | `[workspace] purge_agent_history = true\|false` | Default policy |
| 3 (default) | *(unset)* | **Keep** |

Pass `-f` to force without confirmation. `ae end all` ends every session.

## Hidden subcommands

The following are internal helpers ae invokes itself, prefixed with `_`. Don't call them directly:

- `_spawn`, `_retire` — pane lifecycle (called via `spawn` / `retire` session helpers).
- `_recover-pending` — re-attempt post-launch session ID capture (called by the watchdog).
- `_register-sid` — Codex first-task to self-register its session UUID (injected via `developer_instructions`).

They're listed only for transparency — your interface is the public commands above.
