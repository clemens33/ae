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

## Status indicator

When the watchdog is running it overlays compact ae session facts plus a watchdog
health summary into tmux's `status-right`, refreshed from a cached status file:

```text
[ae aedev local main*] [watch ● 2/2]  ckriech@host  10:42
```

The first bracket is session context:

| Field | Meaning |
|---|---|
| `aedev` | ae session name |
| `local` | workspace mode: `local`, `copy` (`full` mode), or `git` (`--worktree`) |
| `main*` | git branch or detached commit, when the work dir is a git repo; `*` means tracked changes |

The second bracket is watchdog health:

| Glyph | Meaning |
|---|---|
| `●` | All registered agents look healthy |
| `⚠` | One or more agents are stale (waiting in nudge / max-nudges) |
| `✖` | One or more agents are dead |

`2/2` means two watched agent panes are currently okay out of two total. Monitor
panes such as `_watchdog` and `_events` are excluded.

`watchdog stop` restores the previous `status-right`.

## Restarting

Refreshing helpers via `ae doctor --refresh <name>` updates the on-disk scripts but does NOT restart the running watchdog process — it keeps using the body it loaded at start time. To pick up watchdog-body changes, stop and restart:

```bash
~/.ae/sessions/<name>/watchdog stop
~/.ae/sessions/<name>/watchdog start
```

The `_events` pane survives the restart because `watchdog stop` only kills the `_watchdog` pane.
