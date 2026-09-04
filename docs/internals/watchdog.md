# Watchdog

The watchdog is a per-session monitor that lives inside the hidden `ae-monitor` tmux window. Its live loop is the Rust core's `_watchdog-run`, launched by the generated watchdog helper (the `ae` helper region around `ae:16204-16254`). It walks every registered agent pane on a fixed cycle, classifies each agent's state, and reacts: nudges idle agents, alerts on dead ones, pauses nudging when upstream rate limits are visible, and respects explicit completion signals.

Bash is process start/stop/tick glue only: it starts and stops the core child, refreshes the git-branch display, and runs the deferred pending-session-id recovery and Telegram-supervision ticks. The old Bash loop remains only as a rollback path when no usable core is pinned; it is not part of the live path.

## Lifecycle

- **On by default.** Created with the session, unless `workspace.watchdog = false` in config or the session meta says otherwise (`watchdog = false`). Explicit `false`/`no`/`off`/`0` disables it.
- **Manual control:** `~/.ae/sessions/<name>/watchdog start|stop|status` (alias: `loop`).
- **Persists across resume.** The state is recorded in session meta.
- **Self-terminates** if the tmux session or `meta` file disappears.

The `_watchdog` pane runs the core directly: its command is the session's `watchdog` link, which is the core binary under another name, dispatching to `_watchdog-run`. There is no wrapper around it any more.

## Implementations: Rust core (live), Bash glue, and archived alternatives

The live implementation is the Rust core. `_watchdog-run` owns the fixed cycle,
all liveness/stale/throttle/quiet decisions, and their effects. The surrounding
Bash wrapper is live only for process start/stop and the narrow tick glue listed
above. The old Bash loop is rollback-only; it is retained for a session whose
pinned core is unavailable, not selected as a second live implementation.

| Component | Runtime | Ownership |
|---|---|---|
| Rust core watchdog (live) | `_watchdog-run` child in the session's `_watchdog` pane | Per-cycle observation, state-machine decisions, nudges, alerts, and status publication |
| Bash wrapper/glue (live) | Generated helper wrapper around the core child | Process start/stop, git-branch refresh, pending tool-session-id recovery, and Telegram-supervision ticks |

### Archived implementations: Bash loop and aewatch (P4.3 history; not active)

> **ARCHIVAL / ROLLBACK ONLY.** The following comparison and history record the
> former Bash/aewatch arrangement. The Bash loop is retained only as the
> rollback path described above; aewatch is retired and is not selectable.

> **RETIRED at P4.3 (2026-08-29).** The aewatch sidecar is no longer selectable:
> `AE_WATCHDOG_IMPL=uv` selects nothing, and the **bash per-session watchdog
> described below was the one watchdog**. Its Telegram-bridge half was
> superseded by the Rust core, which is now the single bridge. The rest of this
> section is kept as archival design history — read it as what the two-backend
> arrangement WAS, not as a choice available today.

Two implementations reproduced the same watchdog behavior. The **archived Bash watchdog** described in this comparison was active then and needed nothing beyond `bash` + `tmux`. The **aewatch sidecar** was an optional Python reproduction of the watchdog *and* the [Telegram bridge](../reference/telegram.md), once enabled per shell with `AE_WATCHDOG_IMPL=uv`.

| | archived Bash watchdog (rollback-only) | aewatch sidecar (retired; `AE_WATCHDOG_IMPL=uv` selects nothing) |
|---|---|---|
| Runtime | a `bash` subprocess in each session's `_watchdog` pane | one `uv` / PEP 723 Python process (`contrib/aewatch/aewatch`, stdlib-only) |
| Scope | per session | one daemon per `AE_HOME`, sweeping every discovered session |
| Home | the session's `ae-monitor` window | a dedicated `ae-aewatch` tmux session on the root server |
| Liveness | the `_watchdog` pane + pid | a heartbeat file (`$AE_HOME/aewatch/heartbeat`) touched each tick |

*Archival, all three paragraphs below — the selection they describe no longer exists.*

**Selection was exclusive and decided once, at session start.** `_start_session_watchdog` read the effective `AE_WATCHDOG_IMPL`: `uv` started (or reused) the aewatch daemon and did *not* start the bash `_watchdog`; anything else started the bash watchdog. A component — watchdog or bridge — never ran twice against the same session. Since P4.3 the selection is gone: every session starts the bash watchdog, and a stale `ae-aewatch` session is killed when the bridge spawns.

**Reuse was heartbeat-aware.** Launching under `=uv` reused a running `ae-aewatch` session only when its heartbeat was fresh; a stale or wedged daemon was replaced, not trusted.

**There is no longer a bridge fallback, because there is no longer a second bridge.** This is the paragraph P4.3 invalidated: aewatch used to own the Telegram bridge while it held the `bridge-owner` marker with a fresh heartbeat (age ≤ 90s), and a dead daemon let the next bash `telegram _supervise` revive the bash bridge. **The Rust core is now the bridge** — one implementation, no marker, no handoff, nothing to fall back to or from. See [the bridge protocol](bridge-protocol.md), whose two-owner section is likewise archival.

## Live Rust core loop

The sections below describe the Rust core's per-cycle state machine and effects. They preserve the behavior formerly cross-checked with the retired Python sidecar; the historical source remains only for rollback.

## Tunables

| Variable | Default | Meaning |
|---|---|---|
| `AE_WATCHDOG_INTERVAL_SEC` | 60 | Cycle length in seconds |
| `AE_WATCHDOG_STALE_MIN` | 15 | Idle minutes before a nudge fires |
| `AE_WATCHDOG_MAX_NUDGES` | 2 | Nudges before escalating to alert |
| `AE_WATCHDOG_THROTTLE_ALERT_CYCLES` | 5 | Continuous throttle cycles before throttle-alert |
| `AE_WATCHDOG_TG_SUPERVISE_SEC` | 120 | Telegram-bridge revive cadence in seconds (`0` disables) |
| `AE_WATCHDOG_SWEEP_SEC` | 300 | Orchestrator/meta-agent sweep cadence in seconds (`0` falls back to the normal watchdog) |
| `AE_WATCHDOG_SWEEP_RETRY_SEC` | 30 | After an UNDELIVERED sweep nudge, retry this soon instead of waiting a full `AE_WATCHDOG_SWEEP_SEC` (clamped to it; floor — lands on the next poll) |
| `AE_WATCHDOG_SWEEP_RETRY_MAX` | 6 | Fast retries allowed before falling back to normal cadence and raising one `meta-agent unreachable` alert |

Set them in the shell before `ae <name>`, or via your shell rc.

## Per-cycle state machine

For each agent pane, the watchdog walks a fixed branch order. First match wins; later branches don't fire.

```mermaid
flowchart TD
    Start([cycle start]) --> Dead{Dead?}
    Dead -- yes --> AlertDead[alert + skip forever]
    Dead -- no --> PreCheck[Capture pane buf<br/>hash, quiet_reason, is_throttled]
    PreCheck --> Done{Quiet state<br/>latest?}
    Done -- yes --> SkipDone[skip — honor quiet<br/>done: event-only<br/>waiting/blocked: until pane touched]
    Done -- no --> Throttled{Throttle phrase<br/>in pane?}
    Throttled -- yes --> SkipThrottle[skip + emit throttled<br/>escalate after N cycles]
    Throttled -- no --> Active{Hash<br/>changed?}
    Active -- yes --> MarkActive[skip — active]
    Active -- no --> RecentVis{Last change<br/>&lt; 15min?}
    RecentVis -- yes --> SkipVis[skip — recently visible]
    RecentVis -- no --> RecentAlive{Recent<br/>ae event?}
    RecentAlive -- yes --> SkipAlive[skip — recently alive]
    RecentAlive -- no --> Stale[NUDGE / ALERT]
```

In source order:

1. **Dead** — pane's foreground command is a shell AND no agent binary is in the descendant process tree. Alert once, mark dead, ignore in future cycles.
2. **Declared quiet state** *(strongest)* — agent's latest relevant event is its own `state` declaration of `done`, `waiting-user`, or `blocked` (`mark-done`/`done` events count as `done`), AND no newer ae event mentions them as actor or target. `done` is skipped silently and is event-only (pane churn never revives it). `waiting-user`/`blocked` are also skipped, but yield to pane activity: if the pane changed since the declaration (e.g. the human replied directly in it, leaving no event), the quiet state no longer holds and the normal branches resume — so a post-reply hang is still caught.
3. **Throttled** — pane buffer contains a known upstream rate-limit / overload phrase for the agent's binary. Skip nudge, emit `throttled` event first time per streak, escalate to `alert` after `THROTTLE_ALERT_CYCLES` continuous cycles.
4. **Active** — pane content hash differs from last cycle. Update hash, reset nudge counter.
5. **Recently visible** — pane changed within the stale window. Skip.
6. **Recently alive** — agent's latest event in `events.jsonl` is younger than the stale window. Skip.
7. **Stale** — none of the above. Send "Status check" message via `send`. Up to `MAX_NUDGES` nudges. At `MAX_NUDGES` exactly, emit `alert` + tmux display banner. After that, silent waiting.

After the per-pane pass:

8. **Missing pane check** — agents registered in `meta` whose tmux panes have vanished. Alert once each.
9. **Recover pending session ids** — retry codex/gemini/opencode post-launch session capture for slots still marked `pending`.

## Quiet states and how they're invalidated

`_agent_quiet_reason` returns an agent's current quiet state (`done` / `waiting-user` / `blocked`) plus its declaration timestamp, or empty. It reads the *latest relevant event* for the agent: a `state` declaration (or a `mark-done`/`done` event, mapped to `done`) wins only if no newer event mentions the agent as actor or target. An inbound `send`/`ask`/`review`/`nudge` is newer → quiet state invalidated.

**`done` is event-only.** Pane hash changes, terminal resizes, scrollback churn — none of these revive a done agent. The historical "pane churn after done = agent kept working" heuristic was too noisy and was removed. Trade-off: silent work after `done` (output without ae helpers) is invisible to the watchdog. Acceptable — ae already requires helper discipline; a resuming agent should emit an ae event.

**`waiting-user` / `blocked` yield to pane activity.** These states mean the agent is parked, but the unblocking input often arrives as the human typing *directly in the pane* — which produces no `events.jsonl` entry, only a pane-hash change. The watchdog keeps a per-pane **quiet baseline** (`_quiet_pane_decision`): the first cycle that observes a declaration *arms* the baseline with the current pane hash — already including the declaration's own echo, since the `state` helper prints to the pane — and honors the quiet state. Subsequent cycles *hold* (suppress nudges) while the hash equals that baseline, and *yield* once it changes (human reply / agent output), at which point the normal active/recent/stale branches resume. Baselining on the echoed hash is essential: a naive "no pane change since the declaration timestamp" check would be tripped by the echo itself and never suppress a single nudge.

Concretely:

- Agent emits `state blocked "waiting on X"` → state event in `events.jsonl`.
- The watchdog skips it each cycle while the pane is quiet (step 2 fires).
- Human types unblock info in the pane → pane hash diverges from the armed baseline → `_quiet_pane_decision` yields → normal state machine resumes; if the agent then hangs, it gets nudged.
- Or another agent sends it a message → newer event → quiet invalidated the same way.

### `done` dual-emit

`state done` (and its `mark-done` alias) emit both a `state ref=done` event and an `action=done` event; the watchdog reads either, so a watchdog process started before the `state` helper still recognizes completions.

## Throttle detection

Tool-specific patterns inside the watchdog body. Narrow phrases only — false positives compound badly.

| Tool | Patterns |
|---|---|
| `claude` | `Server is temporarily limiting requests`, `API Error: Overloaded`, `Anthropic API error` |
| `codex` | `Rate limit exceeded`, `RateLimitError`, `ratelimit_exceeded` |
| `gemini` | `RESOURCE_EXHAUSTED`, `Quota exceeded` |
| `opencode` | Union of the three above (TUI wraps configurable providers) |
| generic | `429 Too Many Requests`, `503 Service Unavailable` |

When detected:

1. Skip the nudge. Reset nudge counter (so a previously stale agent's count doesn't carry over).
2. First detection of a streak → emit `throttled` event.
3. After `THROTTLE_ALERT_CYCLES` consecutive throttled cycles → emit `alert` event + tmux banner. Once.
4. When the pattern no longer matches → emit `throttle-cleared` event, reset streak.

The streak state per pane is a small machine:

```mermaid
stateDiagram-v2
    [*] --> NotThrottled
    NotThrottled --> Throttled : pattern matched<br/>emit "throttled"
    Throttled --> Throttled : still matching<br/>streak++
    Throttled --> Alerted : streak == THROTTLE_ALERT_CYCLES<br/>emit "alert" + tmux banner
    Alerted --> Alerted : still matching
    Throttled --> NotThrottled : pattern cleared<br/>emit "throttle-cleared"
    Alerted --> NotThrottled : pattern cleared<br/>emit "throttle-cleared"
```

There is no repeat-alert. Once an agent has been alerted for a streak, it stays in `Alerted` silently until the pattern clears. This is deliberate — paging once per streak is informative; paging every minute would be spam.

## What the watchdog cannot do

- **Restart a dead agent.** Marks it dead and stops checking.
- **Detect CLI-internal hangs that produce no pane output.** The dead-check only fires when the foreground command drops to a shell.
- **Push notifications externally.** Alerts are tmux banners + `events.jsonl` entries. Passive.
- **Distinguish slow-but-progressing from genuinely-stuck.** Both look like a static pane to the hash-based check.

For overnight runs, pair the watchdog with an external tail process on `events.jsonl` if you actually need to be paged:

```sh
tail -F ~/.ae/sessions/<name>/events.jsonl \
  | grep --line-buffered '"action":"alert"' \
  | xargs -L1 -I{} <your-pager-cmd>
```

## Inspection

```sh
~/.ae/sessions/<name>/watchdog status         # is the watchdog running?
~/.ae/sessions/<name>/peek _watchdog 60       # last 60 lines of decisions
~/.ae/sessions/<name>/peek _events 60     # event stream
```

Or via tmux directly: `Ctrl+b w`, pick `ae-monitor`.
