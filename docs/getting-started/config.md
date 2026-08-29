# Configuration

`~/.ae/config` is auto-created on first run. Per-project overrides go in `.ae/config` inside your project directory (the project file shadows the global file key by key).

## Example

```toml
[agents]
claude = "claude --permission-mode bypassPermissions --model claude-opus-4-8"
codex = "codex --yolo -m gpt-5.5 -c model_reasoning_effort=high"
gemini = "gemini --yolo -m gemini-2.5-pro"
grokbuild = "grok --always-approve -m grok-4.6 --effort high"
opencode = "opencode -m google/gemini-3-pro-preview"

[workspace]
main = claude:lead
workers = codex:colead
layout = lead-pair
watchdog = true

[prompt]
instructions = "Always write tests. Prefer TypeScript."
```

## `[agents]`

Register any CLI tool as an agent alias. The value is the shell command to launch it. ae extracts the executable name from the command and verifies it's on `PATH` during `ae doctor`.

### Multiple identities of one CLI

One binary can serve several logins/subscriptions — Claude Code selects its
identity via `CLAUDE_CONFIG_DIR`, so an env prefix in the alias command is all
it takes:

```toml
[agents]
fable5   = "claude --permission-mode bypassPermissions --model fable --effort xhigh"
fablemic = "CLAUDE_CONFIG_DIR=$HOME/.claude-mic claude --permission-mode bypassPermissions --model fable --effort xhigh"
claude2  = "CLAUDE_CONFIG_DIR=$HOME/.claude2 claude --permission-mode bypassPermissions --model opus --effort xhigh"
```

Each alias gets its own login (macOS keychain entries are keyed by the config
dir), its own usage pool, and its own history — so one workspace can mix a work
subscription and a personal one seat by seat. `/login` once per identity.

Two traps:

- **Inline the env var — don't rely on a shell function.** A fish/zsh wrapper
  like `claude-mic` doesn't exist in the bash that launches agent panes.
- **Don't create wrapper binaries** (`claude2`, `claude-mic` on `PATH`): ae's
  session machinery keys on the exact executable name, and a renamed binary is
  deliberately treated as an unknown tool — no session IDs, no exact resume.

Current limitation: env-prefixed commands are classified by a raw prefix match
today, so the identity aliases launch fine but degrade to generic-tool handling
(no `--session-id`, heuristic resume) until
[#32](https://github.com/clemens33/ae/issues/32) lands. Track that issue if you
adopt this pattern.

## `[workspace]`

| Key       | Description                                          | Default       |
|-----------|------------------------------------------------------|---------------|
| `main`    | `alias:name` for the primary agent                   | `fable5:lead` |
| `workers` | Comma-separated agents launched at startup. Under `lead-pair` the FIRST worker is the colead seat. Recommended default: the colead ONLY — builders/reviewers are spawned on demand per slice and retired when done | `gpt56sol:colead` |
| `layout`  | `lead-pair` (lead + colead share window 0, other workers in window 1), `lead-solo` (lead alone in window 0, workers in window 1), `vertical` (side-by-side splits), `horizontal` (stacked splits) | `lead-pair`   |
| `copy`    | Working directory mode (see below)                   | `local`       |
| `watchdog`    | Auto-start the watchdog (`true` / `false`)            | `true`        |

Names show in pane borders and are how agents address each other. Under `lead-pair`
the windows carry role names (`0:leads`, `1:workers`); under `lead-solo` window 0 keeps
the session name and only window 1 is role-named (`workers`).

The status bar reads `[ae <session>]  0:leads●● 1:workers◌ 99:ae-monitor  [<path> <git>] [watch …]`
— session name left, the window list in the middle, path + git branch + watchdog health
right. A second status line shows the focused pane's agent identity (pane borders only
render in windows with two or more panes, so a lone agent would otherwise have no
visible name), and on its right a roster of every registered agent:
`lead● colead✔ builder◌ grok⚡`.

The glyphs are the **watchdog's verdict** for each agent — never a claim about what an
agent is "doing" (it cannot see that): `●` it saw the pane advance, `✔` declared done,
`⏳` waiting on you, `⛔` blocked, `◌` stale/nudged, `⚡` throttled, `✖` dead or its pane
is gone, `👁` the orchestrator swept recently, `·` no verdict this cycle. The roster is keyed
by the agents in session meta, so an agent whose pane vanished still holds its slot as
`✖` rather than quietly disappearing from the line. The same glyphs appear per window in
the window list, so attention maps onto the windows you already scan. Everything is
watchdog-published and disappears when the watchdog is stopped.

> **ae owns `window-status-format` inside its own sessions** (and `status-format[0]`/`[1]`,
> `status-left`, `status-right`). If you theme tmux, note that ae overrides these at
> **session scope** for the sessions it creates — your global config is untouched, and
> tmux's own window flags (`#F`) are preserved. Nothing outside an ae session changes.

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

## Model tiers (recommended aliases)

Aliases are arbitrary shell commands, so capability tiers are just config.
Name aliases by intent, not by vendor model — the config survives model
generations:

```toml
[agents]
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

[workspace]
main = best:lead
workers = codex:coworker
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
│   ├── _lib                        # shared library sourced by helpers
│   ├── send, ask, review, reply…   # session helpers (regenerated on resume)
│   └── ...
└── worktrees/<name>/               # for --worktree mode
```

Nothing in your project directory changes.
