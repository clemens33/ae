# Monitor window

Every ae session has a hidden tmux window named `ae-monitor`. It exists from session start, regardless of whether the loop watchdog is enabled.

## Panes

| Pane tag | Always present? | What it shows |
|---|---|---|
| `_events` | Yes | Live formatted tail of `events.jsonl` via the `events-tail` helper. |
| `_loop` | Only when loop watchdog is running | Per-cycle decision log from the loop body. |

Both panes have input disabled (`tmux select-pane -d`) — read-only inspection only.

## Lifecycle

```mermaid
sequenceDiagram
    participant Launch as ae &lt;name&gt;
    participant Monitor as ae-monitor window
    participant Events as _events pane
    participant Loop as _loop pane

    Launch->>Monitor: _ensure-monitor (idempotent)
    Monitor->>Events: new-window -d + events-tail
    Note over Monitor,Loop: ...time passes...
    Launch->>Loop: loop start (if enabled)
    Loop->>Monitor: split-window above _events
    Note over Loop: cycles run, log to pane
    Launch->>Loop: loop stop
    Loop->>Monitor: kill _loop pane
    Note over Events: _events keeps streaming
```

## Why the events pane always exists

Earlier the entire `ae-monitor` window was created by `loop start` and destroyed by `loop stop`. That meant `peek _events` only worked when the watchdog was active. Decoupling them ensures:

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
~/.ae/sessions/<name>/peek _loop 80     # only when loop is running
```

The session's `agents` helper also lists `_events` and `_loop` as agents (with `_`-prefixed names) so they show up alongside the real agents.

## Status indicator

When the loop is running it also overlays a glyph plus the active/total agent count into tmux's `status-right`, refreshed every 5 seconds:

```text
[loop ● 2/2]  ckriech@host  10:42
```

| Glyph | Meaning |
|---|---|
| `●` | All registered agents look healthy |
| `⚠` | One or more agents are stale (waiting in nudge / max-nudges) |
| `✖` | One or more agents are dead |

`loop stop` restores the previous `status-right`.

## Restarting

Refreshing helpers via `ae doctor --refresh <name>` updates the on-disk scripts but does NOT restart the running watchdog process — it keeps using the body it loaded at start time. To pick up loop-body changes, stop and restart:

```bash
~/.ae/sessions/<name>/loop stop
~/.ae/sessions/<name>/loop start
```

The `_events` pane survives the restart because `loop stop` only kills the `_loop` pane.
