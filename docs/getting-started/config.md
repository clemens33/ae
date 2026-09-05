# Configuration

`~/.ae/config` is auto-created on first run. Per-project overrides go in `.ae/config` inside your project directory (the project file shadows the global file key by key).

## Example

```toml
[profiles]
claude = "claude --permission-mode bypassPermissions --model claude-opus-4-8"
codex = "codex --yolo -m gpt-5.5 -c model_reasoning_effort=high"
gemini = "gemini --yolo -m gemini-2.5-pro"
grokbuild = "grok --always-approve -m grok-4.6 --effort high"
opencode = "opencode -m google/gemini-3-pro-preview"

[roster]
lead = claude
colead = codex

[workspace]
main = lead
workers = colead
layout = lead-pair
watchdog = true

[prompt]
instructions = "Always write tests. Prefer TypeScript."
```

## `[profiles]`

Register any CLI tool as a profile — a reusable launch recipe. The value is the shell command to launch it. ae extracts the executable name from the command and verifies it's on `PATH` during `ae doctor`.

### Multiple identities of one CLI

One binary can serve several logins/subscriptions — Claude Code selects its
identity via `CLAUDE_CONFIG_DIR`, so an env prefix in the profile command is all
it takes:

```toml
[profiles]
fable5   = "claude --permission-mode bypassPermissions --model fable --effort xhigh"
fablemic = "CLAUDE_CONFIG_DIR=$HOME/.claude-mic claude --permission-mode bypassPermissions --model fable --effort xhigh"
claude2  = "CLAUDE_CONFIG_DIR=$HOME/.claude2 claude --permission-mode bypassPermissions --model opus --effort xhigh"
```

Each profile gets its own login (macOS keychain entries are keyed by the config
dir), its own usage pool, and its own history — so one workspace can mix a work
subscription and a personal one seat by seat (bind each to its own `[roster]`
name). `/login` once per identity.

Two traps:

- **Inline the env var — don't rely on a shell function.** A fish/zsh wrapper
  like `claude-mic` doesn't exist in the bash that launches agent panes.
- **Don't create wrapper binaries** (`claude2`, `claude-mic` on `PATH`): ae's
  session machinery keys on the exact executable name, and a renamed binary is
  deliberately treated as an unknown tool — no session IDs, no exact resume.

Current limitation: env-prefixed commands are classified by a raw prefix match
today, so the identity profiles launch fine but degrade to generic-tool handling
(no `--session-id`, heuristic resume) until
[#32](https://github.com/clemens33/ae/issues/32) lands. Track that issue if you
adopt this pattern.

## `[roster]`

Bind names to profiles: `name = profile`. The NAME is the agent's identity — it's
what you address with `send`/`ask`/`spawn`, and what pane titles, borders, and
`ae list` show; the profile is metadata (`ae list` shows it alongside the name),
and the same profile can back more than one name. Every seat in
`[workspace] main`/`workers` must be bound here — ae refuses the launch
otherwise and lists every violation. A name bound here but not seated is legal:
`ae <session> use <name>` starts it as main instead of the configured one. Spawn
on demand with `spawn <name> --using <profile>`.

## `[workspace]`

| Key       | Description                                          | Default       |
|-----------|------------------------------------------------------|---------------|
| `main`    | `[roster]` name for the standing main seat. Under `lead-pair` this is a *technical* lifecycle anchor (compact handover, non-retirable), not a rank | `lead` |
| `workers` | Comma-separated `[roster]` names launched at startup. Under `lead-pair` the FIRST worker is the colead seat — an equal leadership peer of the lead (interchangeable, same level). Recommended default: the colead ONLY — builders/reviewers are spawned on demand per slice and retired when done | `colead` |
| `layout`  | `lead-pair` (lead + colead share window 0, other workers in window 1), `lead-solo` (lead alone in window 0, workers in window 1), `vertical` (side-by-side splits), `horizontal` (stacked splits) | `lead-pair`   |
| `copy`    | Working directory mode (see below)                   | `local`       |
| `watchdog`    | Auto-start the watchdog (`true` / `false`)            | `true`        |
| `palette` | `darcula` (the JetBrains dark), `a` (neutral dark), `b` (warmer neutrals) | `darcula` |
| `icons`   | `off` draws the ASCII fallback instead of the glyph set | `on`          |
| `theme`   | `off` leaves your own status line, pane borders and menu styles alone | `on`  |
| `motion`  | `off` freezes the spinner on its mark                 | `on`          |

Names show in pane borders and are how agents address each other. Under `lead-pair`
the windows carry role names (`0:leads`, `1:workers`); under `lead-solo` window 0 keeps
the session name and only window 1 is role-named (`workers`).

The status bar has two lines. The first is this session: its attention mark and name,
then the windows, then the branch, the goal, the shortened path and the watch segment.
The second is the **fleet strip** — every ae session on this tmux server, most actionable
first, each one clickable to switch to it — and on its right the agents of this session.
Every pane also carries a border title: `<name> · <profile> · <mark> <reason>`.

The marks are the **watchdog's verdict**, never a claim about what an agent is "doing"
(it cannot see that):

| Mark | ASCII | Means |
|---|---|---|
| `✖` | `x` | the process behind the pane is gone |
| `⚠` | `!` | waiting on you, blocked, throttled, or an unanswered request |
| `●` | `*` | it saw the pane advance |
| `✓` | `+` | declared done or paused |
| `◌` | `?` | stale, or a fact ae could not establish |
| `·` | `-` | no agent, or no verdict yet |

A pane that printed since the last cycle shows a spinner frame in place of `●`. The
frame advances once per watchdog cycle, so it means "this moved since I last looked"
rather than "this is moving now".

The roster is keyed by the agents in session meta, so an agent whose pane vanished still
holds its slot as `⚠` rather than quietly disappearing. The same marks appear per window
in the window list, so attention maps onto the windows you already scan. Everything is
watchdog-published and disappears when the watchdog is stopped.

> **ae draws its own sessions, at session and window scope.** It sets `status-format[0]`
> and `[1]` on its sessions, and the pane-border and menu styles on their windows. Your
> global tmux config is untouched, and nothing outside an ae session changes. tmux's `Z`
> zoom flag is kept in the window list; the `*` and `-` flags are replaced by the mark.
> `theme = off` turns all of that off and still publishes every `@ae_*` value, so your own
> `status-right` can read ae's facts in your own layout.

## Copy modes

How agents access your code:

| Mode | Flag | What it does |
|------|------|------|
| `local` | *(default)* | Agents work directly in your project directory. Simple and fast. |
| `full` | `--copy` | Full copy of the project. Use for complex features where agents need an isolated workspace. |
| `worktree` | `--worktree` | Git worktree. Lightweight branch isolation backed by git. |

## `[prompt]`

Custom instructions injected into every agent's system prompt alongside the ae workspace context. Per-project `.ae/config` overrides the global one.

```toml
[prompt]
instructions = "Always cite the source file you used."
```

## Watchdog defaults

The watchdog reads its tunables from environment variables (set them in the session shell before `ae <name>`, or via your shell rc):

| Variable | Default | Meaning |
|---|---|---|
| `AE_WATCHDOG_INTERVAL_SEC` | 60 | Cycle length in seconds |
| `AE_WATCHDOG_STALE_MIN` | 15 | Idle minutes before a nudge |
| `AE_WATCHDOG_MAX_NUDGES` | 2 | Nudges before escalating to alert |
| `AE_WATCHDOG_THROTTLE_ALERT_CYCLES` | 5 | Cycles of continuous upstream throttle before alert |
| `AE_WATCHDOG_TG_SUPERVISE_SEC` | 120 | Telegram-bridge revive cadence in seconds (`0` disables) |
| `AE_WATCHDOG_SWEEP_SEC` | 300 | Orchestrator/meta-agent sweep cadence in seconds (`0` falls back to the normal watchdog) |
| `AE_WATCHDOG_SWEEP_RETRY_SEC` | 30 | After an UNDELIVERED sweep nudge, retry this soon instead of waiting a full `AE_WATCHDOG_SWEEP_SEC` (clamped to it; floor — lands on the next poll) |
| `AE_WATCHDOG_SWEEP_RETRY_MAX` | 6 | Fast retries allowed before falling back to normal cadence and raising one `meta-agent unreachable` alert |

The legacy `AE_LOOP_*` names are still honoured as fallbacks for each tunable. To turn the watchdog off for a single session, run `~/.ae/sessions/<name>/watchdog stop` once. The setting persists across resume.

## Model tiers (recommended profiles)

Profiles are arbitrary shell commands, so capability tiers are just config.
Name profiles by intent, not by vendor model — the config survives model
generations:

```toml
[profiles]
# tiers (Claude Code) — workers MUST be non-interactive: an approval prompt
# stalls an unattended pane forever. bypassPermissions is the trusting default
# (matches ae's own examples); acceptEdits is the cautious alternative.
# chores at FULL effort on a cheap model — cheap comes from the model, not from
# thinking less: tests, CI runs, caller/usage scans, log triage, scouts
chore  = "codex -m gpt-5.6-luna -c model_reasoning_effort=xhigh -a never"
# dev work: implementation slices, doc syncs — builder-grade. Two peers, pick per
# slice or alternate for cross-vendor diversity on builder seats:
dev    = "claude --permission-mode bypassPermissions --model claude-opus-5 --effort xhigh"
devx   = "grok --always-approve -m grok-4.6 --effort high"
# cross-model review seat (spawned per slice, retired after)
review = "codex --yolo -m gpt-5.6-sol -c model_reasoning_effort=xhigh"
# leads / orchestrator / hardest work
best   = "claude --permission-mode bypassPermissions --model fable --effort xhigh"

[roster]
lead = best
coworker = codex

[workspace]
main = lead
workers = coworker
```

Role guidance: leads and the orchestrator run `optimal`/`best`; cross-model
reviews run `codex` (a different model family sees different bugs);
chores/tests/CI run `fast`; scoped implementation runs `standard`.
Spawned workers must never wait on approval prompts (they stall unattended
panes): Claude tiers carry `--permission-mode bypassPermissions` (or
`acceptEdits`), codex workers carry `-a never` — and read-only scouts add
`--sandbox read-only` on codex.
See [Delegation](../reference/delegation.md) for when to spawn what.

## Where state lives

```
~/.ae/
├── config                          # global config (this file)
├── sessions/<name>/
│   ├── meta                        # session metadata (read-only mostly)
│   ├── events.jsonl                # event log (audit trail)
│   ├── memo.tsv                    # shared session memory
│   ├── workspace.md                # in-session reference for agents
│   ├── send, ask, review, reply…   # session helpers (regenerated on resume)
│   └── ...
└── worktrees/<name>/               # for --worktree mode
```

Nothing in your project directory changes.
