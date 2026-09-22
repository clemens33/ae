# Configuration

Run `ae init` to discover supported harness executables on `PATH` and propose `~/.ae/config`.
It never runs a harness or checks login/model access. A first session launch still auto-creates
the broad default when the file is absent. Per-project overrides go in `.ae/config` inside your
project directory (the project file shadows the global file key by key).

## Example

```toml
[clients]
claude = claude
codex = codex
grok = grok

[profiles]
opus5 = "claude --permission-mode bypassPermissions --model claude-opus-5 --effort xhigh"
gpt56sol = "codex --yolo -m gpt-5.6-sol -c model_reasoning_effort=xhigh"
fable5 = "claude --permission-mode bypassPermissions --model fable --effort xhigh"
gpt6astra = "codex --yolo -m gpt-6-astra -c model_reasoning_effort=xhigh"
gpt56luna = "codex -m gpt-5.6-luna -c model_reasoning_effort=xhigh -a never"
grok46 = "grok --always-approve -m grok-4.6 --effort high"
muse = "muse"

[prices]
sol_discount = gpt-5.6-sol,1.25,2.5,0.125,10

[roster]
lead = fable5
colead = gpt6astra
orchestrator = gpt56luna

[workspace]
main = lead
workers = colead
layout = lead-pair
watchdog = true
quota = on
quota_every_secs = 300
idle_nudge_secs = 300
done_confirmations = 2
# auto_upgrade = on

[prompt]
instructions = "Always write tests. Prefer TypeScript."
```

## `[clients]`

A client names one installed CLI instance: its executable plus, optionally, its account/config
directory. Profiles build flags and permissions on that name. The default entries are no-op
aliases such as `claude = claude`; this keeps the executable choice in one place without changing
existing profile commands.

The value is one executable word followed by at most one `config_home=<path>` assignment. The
path must be absolute or rooted at `$HOME`/`${HOME}`. ae expands it once when it resolves the
profile; other variables are refused because a client path must not depend on a pane's ambient
environment. Only Claude Code and Codex have verified account-directory variables:

| Tool | `config_home` sets | Default store |
|---|---|---|
| Claude Code | `CLAUDE_CONFIG_DIR` | `$HOME/.claude` |
| Codex | `CODEX_HOME` | `$HOME/.codex` |

`config_home` is refused for Grok, agy, Muse, OpenCode, and Gemini until their account-directory
contracts are verified.

## `[profiles]`

Register any CLI tool as a profile — a reusable launch recipe. The value is the shell command to launch it. ae extracts the executable name from the command and verifies it's on `PATH` during `ae doctor`.

### Multiple identities of one CLI

One binary can serve several logins/subscriptions. Give each non-default account a client, then
use that client name as the executable word in its profiles:

```toml
[clients]
cc = claude
cc-mic = claude config_home=$HOME/.claude-mic
codex = codex
codex-mic = codex config_home=${HOME}/.codex-mic

[profiles]
fable5 = "cc --permission-mode bypassPermissions --model fable --effort xhigh"
fablemic = "cc-mic --permission-mode bypassPermissions --model fable --effort xhigh"
solmic = "codex-mic --yolo -m gpt-5.6-sol -c model_reasoning_effort=xhigh"
```

For a one-off seat on another account, skip the duplicate profile and say so at launch instead:
`ae myproject --lead fable5@cc-mic` runs the `fable5` flags against the `cc-mic` account. The
`@` spelling works on `--lead`, `--colead` and `--seat`, and only there — `spawn --using`
takes a bare profile. Both sides must be the same known tool, and the recorded client is
write-once for the session: a flagless resume honors it by itself, while an explicit
re-pairing repeats its exact label. See
[Commands](../reference/commands.md) for the refusal rules.

A client row also takes `manual_resets=<0-9>`, in any order beside `config_home=`:

```toml
[clients]
codex = codex manual_resets=1
codex-mic = codex config_home=${HOME}/.codex-mic manual_resets=0
```

It is the one fact no client reports: how many manual window resets that subscription has in hand.
`ae quota` spreads the window percentage over `1 + n` windows in its `EFFECTIVE` column, and the
watchdog advisory and delegation guidance judge that number instead of the raw window, so a 95%
window with one reset declared no longer pushes work off a client that is not constrained. An
unusable value is ignored with one visible note rather than refusing the config.

The count is yours to maintain: ae does not decrement it when a reset is used, so update the row
after each manual reset. A positive count is a claim about headroom that only holds while the
declaration is current; `0` declares that there is none.

Each distinct config home isolates local login state, settings, and conversation files, so one
workspace can mix work and personal identities seat by seat. It does not create an independent
provider quota: two homes may authenticate the same account, while two client labels may share one
home. `ae quota` groups profiles by the resolved client home and lists the client labels sharing
each scope. A client no profile names is read as its own scope, so its declaration is never dead
config — the row is there before you launch anything against it. Bind the profiles to different `[roster]` names and run the tool's login flow once per
new home.

**Claude default-state trap:** never set `CLAUDE_CONFIG_DIR` (directly or through `config_home`) to
the default `$HOME/.claude` directory. Without the variable, Claude Code reads its account state
from `$HOME/.claude.json`; with the variable set, it reads
`$CLAUDE_CONFIG_DIR/.claude.json` instead. The default client must stay `claude = claude` with no
`config_home`, or the same-looking path selects a different state file and can appear logged out.

The config home becomes seat identity on first start: ae records its canonical path and whether the
tool-specific variable was explicit or the default was derived from an unset variable. It uses that
recorded value and mode for resume probes, execution, Codex session-id capture, and optional history
purge. For a default-derived store, ae also records the effective `HOME`; this keeps the tool's login
and state files aligned even when the default store is a symlink. Changing `cc-mic` from directory A
to B while a session is retained therefore prints:

```text
ae: seat <slot>: config now points claude at <B>; the retained conversation lives in <A>, resuming there — end the session to adopt <B>
```

Stopping and resuming keeps A. End that session and start a new one to adopt B. A client entry
also avoids shell functions and renamed wrapper binaries: ae expands the client label to the real
executable before it classifies and launches the tool.

## `[prices]`

Override an exact model's API reference price without extending the INI grammar:

```toml
[prices]
sol_discount = gpt-5.6-sol,1.25,2.5,0.125,10
```

The key is an arbitrary config-safe alias because model ids may contain dots. The value is
`model,input,cache_write,cache_read,output`; rates are USD per one million tokens with at most six
decimal places. Project rows overlay global rows by alias. If two surviving aliases name the same
model, `ae usage` refuses with exit 2 and names both aliases rather than choosing one silently.

An override wins over the bundled exact model row. Bundled rows also match an exact `-YYYYMMDD`
release suffix; broader prefixes do not match. These are API-equivalent reference prices for the
base service tier, base context window and five-minute cache writes. Long-context surcharges and
priority or flex tiers are not modelled. Subscription access incurs no additional charge from an
`ae usage` report: ae reads local transcripts offline and is not a billing system.

## `[roster]`

Bind names to profiles: `name = profile`. The NAME is the agent's identity — it's
what you address with `send`/`ask`/`spawn`, and what pane titles, borders, and
`ae list` show; the profile is metadata (`ae list` shows it alongside the name),
and the same profile can back more than one name. Every seat in
`[workspace] main`/`workers` must be bound here — ae refuses the launch
otherwise and lists every violation. A name bound here but not seated is legal:
`ae <session> use <name>` starts it as main instead of the configured one. Spawn
on demand with `spawn <name> --using <profile>`. The optional orchestrator seat
also uses this global roster: add `orchestrator = <profile>` before running
`ae orchestrator`; its dedicated local overlay no longer chooses the profile.
Old seat files that still carry `[profiles]`/`[roster]` are ignored for identity;
`[workspace]` and `[prompt]` still overlay.

## `[workspace]`

| Key       | Description                                          | Default       |
|-----------|------------------------------------------------------|---------------|
| `main`    | `[roster]` name for the standing main seat. Under `lead-pair` this is a *technical* lifecycle anchor (reboot handover, non-retirable), not a rank | `lead` |
| `workers` | Comma-separated `[roster]` names launched at startup. Under `lead-pair` the FIRST worker is the colead seat — an equal leadership peer of the lead (interchangeable, same level). Recommended default: the colead ONLY — builders/reviewers are spawned on demand per slice and retired when done | `colead` |
| `layout`  | `lead-pair` (lead and colead each get 50% in window 0, other workers in window 1), `lead-solo` (lead alone in window 0, workers in window 1), `vertical` (side-by-side splits), `horizontal` (stacked splits) | `lead-pair`   |
| `copy`    | Working directory mode (see below)                   | `local`       |
| `watchdog`    | Auto-start the watchdog (`true` / `false`)            | `true`        |
| `quota` | Whether ae acts on vendor quota at all (`on` / `off`); absent means `on`. When `off`, agents are never told about quota, the watchdog books no quota advisory, sends no checkpoint ask and renders no quota throttle line, and the settings menu carries no quota entry. While `on`, a scope entering `low` or worse asks every seat on it, once, to write a durable checkpoint before its subscription runs dry. `ae quota` works identically in both states. `off` wins over `quota_every_secs` | `on` |
| `quota_every_secs` | Watchdog cadence in seconds for the quota observation, rounded to whole watchdog cycles (`0` disables it) | `300` |
| `idle_nudge_secs` | Continuous positively observed empty-input time before the watchdog reminds the seat (`0` disables) | `300` |
| `done_confirmations` | Delivered proof challenges a later `done` must answer before confirmation (`0` disables; range `0`–`9`) | `2` |
| `orchestrator` | Mark this session as the fleet overview seat (`true`); grants its panes the bare human-authority `relay` helper | `false`       |
| `sweep` | Persist this orchestrator's changed-overview minimum spacing in seconds (`0` disables; positive values below `60` become `60`) | `AE_WATCHDOG_SWEEP_SEC`, then `120` |
| `auto_upgrade` | Let an installed ae quietly check for and apply strictly newer releases (`on` / `off`); global config only | `on` |
| `fleet_order` | The order your sessions are drawn in on the fleet strip, as a comma-separated list of session names; global config only | creation order |
| `palette` | `darcula` (the JetBrains dark), `a` (neutral dark), `b` (warmer neutrals) | `darcula` |
| `icons`   | `off` draws the ASCII fallback instead of the glyph set | `on`          |
| `theme`   | `off` leaves your own status line, pane borders and menu styles alone | `on`  |
| `motion`  | `off` freezes the working `●` at its accent colour   | `on`          |

Set `orchestrator = true` only in the dedicated overview seat. It authorizes
unenveloped `relay` delivery, whose target treats the text as human input.

`fleet_order` is how you choose where a session sits on the second status
line, instead of taking the order tmux happened to create the sessions in —
which reshuffles every time you restart the fleet:

```toml
[workspace]
fleet_order = aedev, thinking, infra
```

The sessions you name come first, in the order you named them; everything else
falls in behind them in creation order, as before. The orchestrator stays
pinned at the front either way. Naming a session that is not running right now
costs nothing — that is the point, so the fleet comes back the same way after a
restart. The match is exact and case-sensitive.

This is machine policy like `auto_upgrade`, so ae reads it only from
`~/.ae/config` and a project's `.ae/config` cannot reorder your fleet. Edit it
while sessions are running: each session's watchdog re-reads the file and
redraws its strip within one cycle, so nothing needs relaunching. An entry that
is not a legal session name, one you listed twice, or one matching no session ae
has a record of is skipped silently on the bar — `ae doctor` names it once on
the `workspace.fleet_order` row. With no `fleet_order` set, the strip is exactly
what it always was.

The same order decides two more things: which session a client is handed to when
the one it is watching is killed, and how the running rows of the fleet picker
(`<prefix> a`) are sorted underneath attention — the picker still puts whatever
needs you first, and an orchestrator you named there is simply first among rows
of equal attention rather than pinned as it is on the strip.

`auto_upgrade` is machine policy, so ae reads it only from `~/.ae/config`;
a project's `.ae/config` cannot override it. Absence means `on`. Any explicit
value other than `on` or `off`, including an empty value, is invalid and leaves
automatic upgrades disabled until corrected. Checkout builds never
auto-upgrade, and `AE_NO_AUTOSTART=1` suppresses scheduling along with the other
companions. `ae version` and `ae doctor` report policy plus last check/result;
they never trigger a check.

Names show in pane borders and are how agents address each other. Each window
keeps its first agent's name as its stable tmux routing name; later splits do
not rename it.

The status bar has two lines. The first is this session: windows led by their
live marks and named agents, then the branch, goal,
shortened path and watch segment. One agent is `0:✓lead`; multiple agents are
`0:[✓lead ●colead]`. The selected window and current fleet row use the
palette's selection colours.
The second is the **fleet strip** — every ae session on this tmux server in the order it
was created, each with its live mark and clickable to switch to it. Every pane
also carries a border title: `<name> · <mark> <reason>`.

Right-click a session tab in the fleet strip to open its context menu. **Flip
lead/colead panes** swaps the clicked session's current window when it has exactly
two panes and is not zoomed; otherwise ae leaves it alone and explains why. The
tmux keys `prefix {` and `prefix }` provide the same pane swap directly.
Click the quiet `≡` menu glyph (`=` with `icons = off`) or the matching `+N`
overflow count with either mouse button, or press `prefix a`, to open the fleet
picker. Its title starts with `ae session`, then counts
running sessions and those needing you; up to 30
attention-ordered rows show name, mark, state, branch and goal. Choosing a row
drops the invoking client into that session's lead pane. On tmux 3.4 use the
row shortcut keys; tmux 3.5 and newer also supports mouse selection.

The bottom-right three-cell range (` ⚙ `, or ` * ` with `icons = off`) opens
settings against that corner of the client that clicked it. Both surrounding
spaces are clickable and highlight with the glyph. The range is always
version-free; the menu title alone shows the running `ae` version when the
session reports a valid one. While the menu is open, selection colours
highlight this button only. The settings menu shows one live quota entry
opening a centred per-window dialog (absent entirely when `[workspace] quota = off`),
then the recorded orchestrator role and
exactly one of Start, Resume or Pause when that action is safe; otherwise it
shows why the control is unavailable. Very small clients may show a quota
overflow notice instead of the entry; `ae quota` remains the full view.

The marks are the **watchdog's verdict**, never a claim about what an agent is "doing"
(it cannot see that):

| Mark | ASCII | Means |
|---|---|---|
| `✖` | `x` | the process behind the pane is gone |
| `⚠` | `!` | waiting on you, blocked, throttled, or an unanswered request |
| `●` | `*` | working according to the latest liveness verdict |
| `◔` | `~` | waiting on another agent — quiet, no human needed |
| `✓` | `+` | declared done or paused |
| `◌` | `?` | stale, or a fact ae could not establish |
| `·` | `-` | positively idle at an empty modeled input, no agent, or no verdict yet |

While somebody is attached, a Working verdict shows a pulsing `●` in its place.
The pulse is the cached verdict, not terminal motion; silence past the liveness
window changes the next watchdog verdict to stale. A positively recognized
empty Claude Code or Codex input is idle immediately and does not pulse.

Session attention is keyed by the agents in session meta, so an agent whose
pane vanished still holds its slot as `⚠` rather than quietly disappearing.
Live agents and their marks appear in the window list, so attention maps onto
the windows you already scan. Everything is watchdog-published and disappears
when the watchdog is stopped.

> **ae draws its own sessions, at session and window scope.** It sets `status-format[0]`
> and `[1]` on its sessions, and the pane-border and menu styles on their windows. Your
> global tmux config is untouched, and nothing outside an ae session changes. tmux's `Z`
> zoom flag is kept in the window list; the `*` and `-` flags are replaced by
> named agents and marks.
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

Custom instructions transported beside the ae workspace context through each harness's supported launch channel. Per-project `.ae/config` overrides the global one.

```toml
[prompt]
instructions = "Always cite the source file you used."
```

A long policy is easier as a multi-line block. The opener line is exactly
`instructions = """`; a line that is exactly `"""` closes the block. Between them
every line is raw text — a `[section]` header, a `key = value` line and a `#`
comment are all just text, and no escapes are interpreted. A raw line that is
exactly `"""` cannot appear inside the block, because that line closes it; no
escape exists. The single-line form above stays valid and unchanged, and the
block is accepted for `prompt.instructions` alone.

```toml
[prompt]
instructions = """
SPEND POLICY: keep replies short.
Cite the file for every claim.
"""
```

Every harness receives the value on its own channel — a system-prompt flag, a
developer-instructions value, an initial user turn, or a generated config file —
so the newlines and quotes reach the agent's system prompt verbatim.

## Watchdog defaults

The watchdog reads its tunables from environment variables (set them in the session shell before `ae <name>`, or via your shell rc):

| Variable | Default | Meaning |
|---|---|---|
| `AE_WATCHDOG_INTERVAL_SEC` | 60 | Cycle length in seconds |
| `AE_WATCHDOG_STALE_MIN` | 15 | Idle minutes before a nudge |
| `AE_WATCHDOG_MAX_NUDGES` | 2 | Nudges before escalating to alert |
| `AE_WATCHDOG_THROTTLE_ALERT_CYCLES` | 5 | Cycles of continuous upstream throttle before alert |
| `AE_WATCHDOG_TG_SUPERVISE_SEC` | 120 | Telegram-bridge revive cadence in seconds (`0` disables) |
| `AE_WATCHDOG_SWEEP_SEC` | 120 | Orchestrator changed-overview minimum-spacing fallback when `[workspace] sweep` is absent or invalid (`0` falls back to the normal watchdog; positive values below `60` become `60`) |
| `AE_WATCHDOG_SWEEP_RETRY_SEC` | 30 | After an UNDELIVERED sweep nudge, retry this soon instead of waiting a full `AE_WATCHDOG_SWEEP_SEC` (clamped to it; floor — lands on the next poll) |
| `AE_WATCHDOG_SWEEP_RETRY_MAX` | 6 | Fast retries allowed before falling back to normal cadence and raising one `meta-agent unreachable` alert |

The legacy `AE_LOOP_*` names are still honoured as fallbacks for each tunable. To turn the watchdog off for a single session, run `~/.ae/sessions/<name>/watchdog stop` once. The setting persists across resume.

Quota observation is configured by `[workspace] quota_every_secs`, not an environment fallback.
Launch persists the value in session meta so rename and resume keep the same cadence. It accepts
unsigned integer seconds only; `0` disables quota advisories. `[workspace] quota = off` disables
the advisories (and the injected quota guidance, the throttle line, and the settings quota entry)
regardless of the cadence; `ae quota` itself is unaffected in both states.

The cadence's due counter advances on whole watchdog cycles, so the
effective observation period is the requested seconds rounded up to that grid.

Idle reminders use `[workspace] idle_nudge_secs`, also persisted at launch and
validated as unsigned integer seconds. The default is 300; `0` disables idle
reminders, done challenges included, without disabling legacy stale detection for unmodeled frames.
`done_confirmations` is persisted too; bad input falls back to 2 with a note. A resumed session
keeps its recorded value, so config changes apply on relaunch; a session without the row records 2.

## Model tiers (recommended profiles)

- **STRONG DEV (build slices):** use `gpt56sol` xhigh or `opus5` xhigh.
- **BRAINPOWER (lead/colead seats, plans, rulings, hard debugging):** use `fable5` xhigh or `gpt6astra` xhigh.
- **CHORES/tests/simple slices:** use `gpt56luna` xhigh; it also runs the orchestrator seat.
- **REVIEWER:** use `grok46` when usage allows.

**OpenCode Go seats (only if listed in workspace.md Available profiles or
configured [profiles]):** `spark13gom` (Meta-served Muse Spark 1.3 Contributor)
ranks as a peer of `gpt56solx`/`opus5x` for strong build slices and judgment
work; `deepseek41flashgom` (DeepSeek-served DeepSeek V4.1 Flash) is the fast
alternative for bounded build slices and review lanes. A shared OpenCode scope
proves nothing about provider — declare the served provider for rule 11.
Chores still go to `gpt56luna`; simple work never takes `fablex`/`astrax`.

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
