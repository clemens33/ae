# Events

`~/.ae/sessions/<name>/events.jsonl` is ae's single durable record of what happened. Mutating helpers and ae internals write structured events; inspection helpers (`peek`, `agents`, `requests`, `events-tail`, `watchdog status`) read but don't write. The watchdog reads events to enforce its done-invalidation contract; `requests` derives pending/replied state from them. Append-only, one JSON object per line.

## Producers and consumers

Single writer (`ae_emit_event` in `_lib`), single file, multiple readers. The append-only structure plus flock-serialized writes mean readers can scan safely without coordination.

```mermaid
flowchart LR
    subgraph Writers
        direction TB
        SH[send]
        AKH[ask / review]
        RPH[reply]
        MDH[mark-done]
        MEH[memo]
        SPH[spawn / retire]
        LP1["watchdog<br/>nudge / alert /<br/>throttled / throttle-cleared /<br/>recover"]
        IH[interrupt / focus]
    end
    EE["_lib::ae_emit_event<br/>(JSON escape,<br/>flock + append)"]
    FILE[(events.jsonl)]
    subgraph Readers
        direction TB
        LP2["watchdog<br/>_agent_done_epoch<br/>_buf_shows_throttle"]
        REQ["requests helper<br/>ae_find_request"]
        ET["_events pane<br/>events-tail"]
    end

    SH --> EE
    AKH --> EE
    RPH --> EE
    MDH --> EE
    MEH --> EE
    SPH --> EE
    LP1 --> EE
    IH --> EE
    EE --> FILE
    FILE -.tac scan.-> LP2
    FILE -.tac scan.-> REQ
    FILE -.tail -F.-> ET
```

Inspection helpers (`peek`, `agents`, `requests`, `events-tail`, `watchdog status`) read but don't write. Mutating helpers and the watchdog write through the single `ae_emit_event` choke point.

## Schema

Every event has these keys; `target`, `ref`, and `summary` are optional and omitted when empty.

```json
{
  "ts":      "2026-05-19T07:29:45Z",        // ISO 8601 UTC, second precision
  "actor":   "lead",                        // bare name of the emitter (or 'watchdog', 'human')
  "action":  "done",                        // event type — see below
  "target":  "coworker",                    // bare name of the recipient when applicable
  "ref":     "ae-20260519T072100Z-abc123",  // polysemous — see action table
  "summary": "first 200 chars of payload"   // optional preview, truncated
}
```

`ref` carries different correlation values depending on `action`:

- `ask` / `review` / `reply` — the request id pairing the three.
- `memo` — the topic the memo was filed under.
- `recover` — the captured session id (Codex/Gemini/OpenCode UUID).
- Other actions — usually absent.

String values are JSON-escaped: `\"` `\\` `\n` `\t` `\r`. The flat schema is intentionally cheap to parse from bash (`_event_json_str` in `_lib` is a pure bash walker).

### Routing-key fields

Messaging events (`send` / `ask` / `review` / `reply`) also carry the sender's and recipient's **slot** and **session** when known — the churn-proof routing key that survives a display-name change (see [slot identity](../reference/helpers.md#slot-identity)):

```json
{
  "actor_slot":     "main",         // sender's slot: main | worker.<n> | spawned.<n>
  "actor_session":  "my-feature",   // sender's session
  "target_slot":    "worker.0",     // recipient's slot
  "target_session": "my-feature"    // recipient's session
}
```

Each is optional and omitted when empty. Readers that don't understand them ignore them; readers that do (`reply`, `requests`) prefer slot + session over the display name for pairing and delivery.

## Actions

| Action | Emitted by | Meaning |
|---|---|---|
| `send` | `send` helper | One-way message between agents (or from human / watchdog). |
| `ask` | `ask` helper | Tracked request expecting a reply. Carries `ref`. |
| `review` | `review` helper | Like `ask`, with the critical-review prompt template. Carries `ref`. |
| `reply` | `reply` helper | Reply to an `ask` / `review`. Same `ref`. |
| `state` | `state` helper | Agent declares its work state — `working` / `waiting-user` / `blocked` / `done` (in `ref`). The watchdog honors quiet states. |
| `done` | `mark-done` helper | Completion / pause signal. `mark-done` is a shim over `state done`; both are read as `done`. |
| `chat` | `say` helper | Agent's free-text line to the human, forwarded by the Telegram bridge. Text in `summary`. |
| `memo` | `memo add` helper | Append to shared session memory. |
| `spawn` | ae internal | A new agent joined the workspace. |
| `retire` | ae internal | A spawned agent was removed. |
| `focus` | `focus` helper | tmux focus switch (informational). |
| `interrupt` | `interrupt` helper | Cancel signal sent. |
| `nudge` | watchdog | Stale-agent status check. |
| `alert` | watchdog / ae internal | Attention required (dead, max-nudges, persistent throttle, missing pane). |
| `throttled` | watchdog | First cycle of an upstream throttle streak. |
| `throttle-cleared` | watchdog | Throttle pattern no longer present. |
| `recover` | watchdog | Post-launch session id captured for a previously-pending slot. |

## How `requests` reads events

`requests [mine|inbox|all]` walks `events.jsonl` backward via `tac` and collects:

- The latest `ask` / `review` event per `ref` → request row.
- The latest `reply` event per `ref` → reply row.

A request is `replied` only when the reply's `actor` equals the request's `target` AND the reply's `target` equals the request's `actor`. Stray reply events (wrong actor / wrong target) leave the request `pending`. Without that check a misrouted or manual reply could falsely close a request.

`ae_find_request` returns the matched request as a tab-separated row. When the event carries routing-key fields the row is seven columns — `action  actor  target  actor_slot  actor_session  target_slot  target_session` — and `reply` verifies the responder's live slot against `target_slot` + `target_session` before delivering, so identity survives a name change. An event without those fields yields a three-column row (`action  actor  target`) and pairing falls back to the display name.

## How `_agent_done_epoch` reads events

```bash
_agent_done_epoch <agent>
# Returns the epoch (unix seconds) of the agent's most recent done event,
# IF that done is the latest "relevant" event for the agent — meaning the
# agent is the actor or target of no newer event. Otherwise empty.
```

"Relevant" = `actor == agent` OR `target == agent` OR `target == @<this-session>:<agent>` (cross-session targeting). Scan is newest-first via `tac` and stops at the first relevant match. Unbounded by design so a `done` event stays valid however many unrelated events follow it.

## Reading events live

The hidden `ae-monitor` window has an `_events` pane running the `events-tail` helper. It prints a formatted column view with `MM-DDTHH:MM:SS` UTC timestamps:

```text
05-19T07:29:14  nudge    watchdog               → claude:lead          idle 90m, no recent ae activity
05-19T07:29:45  done     claude:lead                                   All ae streamline + throttle detection...
05-19T09:05:14  nudge    watchdog               → claude:lead          idle 95m, no recent ae activity
```

`peek _events [n]` for a snapshot from any pane.

## Schema stability

Adding new optional keys is fine — readers ignore unknown ones. Renaming or removing keys is breaking; ae has no schema versioning so a breaking change requires a separate migration story. As of this writing, the keys above are the stable surface.

## Disk footprint

`events.jsonl` is append-only with no rotation. Long-running sessions accumulate megabytes. Not catastrophic — the file is read backward with `tac` and matches stop early — but worth knowing for multi-day overnight setups.
