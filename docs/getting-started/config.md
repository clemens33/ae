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
workers = codex:reviewer
layout = vertical
watchdog = true

[prompt]
instructions = "Always write tests. Prefer TypeScript."
```

## `[agents]`

Register any CLI tool as an agent alias. The value is the shell command to launch it. ae extracts the executable name from the command and verifies it's on `PATH` during `ae doctor`.

## `[workspace]`

| Key       | Description                                          | Default       |
|-----------|------------------------------------------------------|---------------|
| `main`    | `alias:name` for the primary agent                   | `claude:lead` |
| `workers` | Comma-separated agents launched at startup           | *(empty)*     |
| `layout`  | `vertical` (side-by-side) or `horizontal` (stacked)  | `vertical`    |
| `copy`    | Working directory mode (see below)                   | `local`       |
| `watchdog`    | Auto-start the watchdog (`true` / `false`)            | `true`        |

Names show in pane borders and are how agents address each other.

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
| `AE_WATCHDOG_SWEEP_SEC` | 300 | Hub/meta-agent sweep cadence in seconds (`0` falls back to the normal watchdog) |

The legacy `AE_LOOP_*` names are still honoured as fallbacks for each tunable. To turn the watchdog off for a single session, run `~/.ae/sessions/<name>/watchdog stop` once. The setting persists across resume.

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
