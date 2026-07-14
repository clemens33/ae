# Monitor window

Every ae session has a hidden tmux window named `ae-monitor`. It exists from session start, regardless of whether the watchdog is enabled.

## Panes

| Pane tag | Always present? | What it shows |
|---|---|---|
| `_events` | Yes | Live formatted tail of `events.jsonl` via the `events-tail` helper. |
| `_watchdog` | Only when watchdog is running | Per-cycle decision log from the watchdog body. |

Both panes have input disabled (`tmux select-pane -d`) — read-only inspection only.

## Lifecycle

```mermaid
sequenceDiagram
    participant Launch as ae &lt;name&gt;
    participant Monitor as ae-monitor window
    participant Events as _events pane
    participant Watchdog as _watchdog pane

    Launch->>Monitor: _ensure-monitor (idempotent)
    Monitor->>Events: new-window -d + events-tail
    Note over Monitor,Watchdog: ...time passes...
    Launch->>Watchdog: watchdog start (if enabled)
    Watchdog->>Monitor: split-window above _events
    Note over Watchdog: cycles run, log to pane
    Launch->>Watchdog: watchdog stop
    Watchdog->>Monitor: kill _watchdog pane
    Note over Events: _events keeps streaming
```

## Why the events pane always exists

Earlier the entire `ae-monitor` window was created by `watchdog start` and destroyed by `watchdog stop`. That meant `peek _events` only worked when the watchdog was active. Decoupling them ensures:

- The event stream is always inspectable, even with the watchdog disabled.
- Toggling the watchdog is purely additive — split a pane in, kill a pane out.
- Resume-after-detach always reaches a known state (`_events` pane present).

## Inspecting

From your terminal:

```text
Ctrl+b w   →   pick ae-monitor
```

From any pane via helpers:

```bash
~/.ae/sessions/<name>/peek _events 80   # snapshot
~/.ae/sessions/<name>/peek _watchdog 80     # only when watchdog is running
```

The session's `agents` helper also lists `_events` and `_watchdog` as agents (with `_`-prefixed names) so they show up alongside the real agents.

## Status bar

The status bar is **owned by ae** — `_ae_apply_status_bar` bakes both halves at
session creation, resume, rename, and `doctor --refresh`; the watchdog never rewrites
the bar, it only feeds tmux **user options** (interpolated literally — no shell job,
no `#()` parsing — so any branch/status text is safe with zero escaping). Shape:

```text
[ae aedev]  0:leads 1:workers 99:ae-monitor  [~/projects/clemens33/ae main*] [watch ● 2/2]
```

- **`status-left`** is the session **name only** (`[ae <session>] `) — tmux's window
  list renders after it over the role-named windows. The name is user text in tmux
  format syntax and is escaped (`#` → `##`, `%` → `%%`), so a session `a#b` renders
  verbatim.
- **`status-right`** carries **location + git branch + watchdog health**:
  `[<path><branch>] [watch …]`. The path(s) (by mode: local one path, copy `→`,
  worktree `⌁`) are static escaped text; the branch + tracked-dirty marker arrives
  live via the `@ae_branch_status` user option (watchdog, each cycle; empty when the
  watchdog is off or the dir isn't a repo), and the health block via
  `@ae_watchdog_status` at the very end (empty when the watchdog is off). `$HOME` is
  abbreviated to `~`; long paths are left-truncated with a leading `…`; `main*` is
  the branch (or detached short commit), `*` = tracked changes.
- A **second status line** shows the focused pane's `@ae_agent`: tmux renders pane
  borders (which carry the agent name) only in windows with 2+ panes, so a lone
  agent in its own window would otherwise have no visible identity.

The `[watch …]` bracket is watchdog health:

| Glyph | Meaning |
|---|---|
| `●` | All registered agents look healthy |
| `⚠` | One or more agents are stale (waiting in nudge / max-nudges) |
| `✖` | One or more agents are dead |

`2/2` means two watched agent panes are currently okay out of two total. Monitor
panes such as `_watchdog` and `_events` are excluded.

`watchdog stop` unsets the `@ae_branch_status` / `@ae_watchdog_status` user options —
the baked bar stays, its live segments simply render empty.

## Restarting

Refreshing helpers via `ae doctor --refresh <name>` updates the on-disk scripts but does NOT restart the running watchdog process — it keeps using the body it loaded at start time. To pick up watchdog-body changes, stop and restart:

```bash
~/.ae/sessions/<name>/watchdog stop
~/.ae/sessions/<name>/watchdog start
```

The `_events` pane survives the restart because `watchdog stop` only kills the `_watchdog` pane.
