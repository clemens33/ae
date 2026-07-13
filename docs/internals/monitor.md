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

ae splits its tmux status bar by **side**: repo context (where you are) on the left,
watchdog health (monitoring state) on the right.

- **`status-left`** carries the session **location + git branch** — the repo context.
  The path(s) the session works in (by mode) are static, set at launch and re-applied
  by `ae doctor --refresh`; the branch + tracked-dirty marker is appended live by the
  watchdog, which writes it to the `@ae_branch_status` tmux **user option** each cycle
  (status-left renders it as `#{@ae_branch_status}`). A user-option value is
  interpolated literally — no shell job, no `#()` parsing — so a branch with a `#`,
  `)`, space, or any character is safe with zero escaping; it's empty (left falls back
  to just the name + path) when the watchdog is off or the work dir isn't a repo. The
  session *name* leads the segment — under `lead-pair` the window label carries a role
  name (`0:lead`), so status-left is where the session name lives. The
  name and the static path text are both user text in tmux format syntax and are
  escaped — `#` → `##` (format vars) and `%` → `%%` (strftime) — so a session `a#b` or
  a directory like `/tmp/%Y` renders verbatim.

  ```text
  [ae aedev ~/projects/clemens33/ae main*]                         # local: name + path + branch
  [ae aedev ~/projects/clemens33/ae → ~/.ae/copies/aedev main*]    # copy (full): origin → copy
  [ae aedev ~/projects/clemens33/ae ⌁ ~/.ae/worktrees/aedev feat*] # git: origin ⌁ worktree
  ```

  `$HOME` is abbreviated to `~`; long paths are truncated from the left (keeping the
  tail) with a leading `…`. `status-left-length` is raised to fit. `main*` is the git
  branch (or detached short commit); `*` = tracked changes
  (`git status --porcelain --untracked-files=no`); omitted for non-git work dirs.

- **`status-right`** carries only the watchdog's per-cycle **health**, rendered from
  the `@ae_watchdog_status` user option (set by the watchdog each cycle, same
  literal-interpolation safety as the branch):

  ```text
  [watch ● 2/2]  ckriech@host  10:42
  ```

The `[watch …]` bracket is watchdog health:

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
