# Configuration

`~/.ae/config` is auto-created on first run. Per-project overrides go in `.ae/config` inside your project directory (the project file shadows the global file key by key).

## Example

```toml
[agents]
claude = "claude --permission-mode bypassPermissions --model claude-opus-4-7"
codex = "codex --yolo -m gpt-5.5 -c model_reasoning_effort=high"
gemini = "gemini --yolo -m gemini-2.5-pro"
opencode = "opencode -m google/gemini-3-pro-preview"

[workspace]
main = claude:lead
workers = codex:colead, claude:builder, codex:reviewer
layout = lead-pair
watchdog = true

[prompt]
instructions = "Always write tests. Prefer TypeScript."
```

## `[agents]`

Register any CLI tool as an agent alias. The value is the shell command to launch it. ae extracts the executable name from the command and verifies it's on `PATH` during `ae doctor`.

## `[workspace]`

| Key       | Description                                          | Default       |
|-----------|------------------------------------------------------|---------------|
| `main`    | `alias:name` for the primary agent                   | `fable5:lead` |
| `workers` | Comma-separated agents launched at startup. Under `lead-pair` the FIRST worker is the colead seat | `gpt56sol:colead, opus48:builder` |
| `layout`  | `lead-pair` (lead + colead share window 0, other workers in window 1), `lead-solo` (lead alone in window 0, workers in window 1), `vertical` (side-by-side splits), `horizontal` (stacked splits) | `lead-pair`   |
| `copy`    | Working directory mode (see below)                   | `local`       |
| `watchdog`    | Auto-start the watchdog (`true` / `false`)            | `true`        |

Names show in pane borders and are how agents address each other. Under `lead-pair`
the windows carry role names (`0:leads`, `1:workers`); under `lead-solo` window 0 keeps
the session name and only window 1 is role-named (`workers`).

The status bar reads `[ae <session>]  0:leads 1:workers 99:ae-monitor  [<path> <git>] [watch …]`
— session name left, the window list in the middle, path + git branch + watchdog health
right. A second status line shows the focused pane's agent identity (pane borders only
render in windows with two or more panes, so a lone agent would otherwise have no
visible name).

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
| `AE_WATCHDOG_SWEEP_SEC` | 300 | Steward/meta-agent sweep cadence in seconds (`0` falls back to the normal watchdog) |
| `AE_WATCHDOG_SWEEP_RETRY_SEC` | 30 | After an UNDELIVERED sweep nudge, retry this soon instead of waiting a full `AE_WATCHDOG_SWEEP_SEC` (clamped to it; floor — lands on the next poll) |
| `AE_WATCHDOG_SWEEP_RETRY_MAX` | 6 | Fast retries allowed before falling back to normal cadence and raising one `meta-agent unreachable` alert |

The legacy `AE_LOOP_*` names are still honoured as fallbacks for each tunable. To turn the watchdog off for a single session, run `~/.ae/sessions/<name>/watchdog stop` once. The setting persists across resume.

## Model tiers (recommended aliases)

Aliases are arbitrary shell commands, so capability tiers are just config.
Name aliases by intent, not by vendor model — the config survives model
generations:

```toml
[agents]
# tiers (Claude Code) — workers MUST be non-interactive: an approval prompt
# stalls an unattended pane forever. bypassPermissions is the trusting default
# (matches ae's own examples); acceptEdits is the cautious alternative.
fast     = "claude --permission-mode bypassPermissions --model sonnet --effort low"    # chores, tests, CI, scouts
standard = "claude --permission-mode bypassPermissions --model sonnet"                 # scoped features (default effort: high)
optimal  = "claude --permission-mode bypassPermissions --model opus --effort xhigh"    # leads
best     = "claude --permission-mode bypassPermissions --model fable --effort xhigh"   # leads, steward, hardest work
# codex
codex     = "codex -m gpt-5.5 -c model_reasoning_effort=xhigh"
codexfast = "codex -m gpt-5.4-mini -c model_reasoning_effort=low -a never"

[workspace]
main = best:lead
workers = codex:coworker
```

Role guidance: leads and the steward run `optimal`/`best`; cross-model
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
│   ├── _lib                        # shared library sourced by helpers
│   ├── send, ask, review, reply…   # session helpers (regenerated on resume)
│   └── ...
└── worktrees/<name>/               # for --worktree mode
```

Nothing in your project directory changes.
