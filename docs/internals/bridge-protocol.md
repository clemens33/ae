# Bridge protocol

> Stable surface for out-of-band consumers — chat bridges, webhooks, mirrors — to read ae activity and write back into a session without owning a tmux pane.

ae ships exactly one bridge daemon in `ae-core`: the Rust Telegram bridge runs in the `ae-telegram` tmux session. The protocol below is the stable event/send contract used by that shipped bridge and by external consumers such as chat bridges, webhooks, and mirrors. External consumers can read ae activity and write back into a session without owning a tmux pane; they are not additional ae bridge owners. This contract lives here so every consumer has a fixed surface to write against, and so changes are deliberate.

For background context and roadmap, see [issue #1 — Chat bridge for ae sessions](https://github.com/clemens33/ae/issues/1).

## What ae provides

Four pieces of substrate, shipped and stable:

1. **`events.jsonl`** — append-only JSON-lines stream of everything that happens in a session. See [events.md](events.md) for the full schema. Bridges read this.
2. **`AE_SENDER_OVERRIDE`** — env var that lets a non-pane caller identify itself when invoking `send` / `ask` / `review`. Bridges set this when writing back.
3. **Allow-listed external-actor target prefixes** — `telegram:*` and `discord:*` are treated as event-only sinks: `send` emits the event and exits without touching tmux. Bridges read those events and forward them to the chat platform.
4. **`session_id`** — stable UUID per ae session, stored in `~/.ae/sessions/<name>/meta` as `session_id=<uuid>`. Survives rename and `ae transfer`. Bridges bind to this rather than the human-readable name.

## Identities

An *actor* in ae is anyone who can write an event. Today there are three classes:

| Class | Example | How they identify |
|---|---|---|
| **Agent pane** | `claude:lead`, `codex:coworker` | Detected from the tmux pane (`@ae_agent` user-option). |
| **Human** | `human` | Fallback when no agent and no override. |
| **External actor** | `telegram:clemens`, `discord:#general` | Sets `AE_SENDER_OVERRIDE=<id>` when calling helpers. |

External-actor identifiers are strings the bridge chooses and the bridge owns the mapping from chat-platform user id → external-actor id. ae does not maintain that mapping.

External-actor identifiers SHOULD use the form `<platform>:<id>`. The `:` separator matches the agent-pane convention (`alias:name`) but the value is not parsed by ae beyond the prefix check in `send` and `ae_tracked_send`.

## Writing back into ae

A bridge writing into a session calls the same helpers an agent would, with two changes:

```bash
export AE_SENDER_OVERRIDE=telegram:clemens

# Plain message into an agent's pane
~/.ae/sessions/<name>/send claude:lead "deploy is green, proceed"

# Tracked request expecting a reply
~/.ae/sessions/<name>/ask claude:lead "status check?"

# Critical review
~/.ae/sessions/<name>/review codex:coworker "audit the migration"

# Reply to a tracked request (when the chat user replies)
~/.ae/sessions/<name>/reply --as telegram:clemens ae-20260528T130000Z-abc123 "approved"
```

`AE_SENDER_OVERRIDE` is honored by `send` and `ae_tracked_send` (the shared body behind `ask` / `review`). The override becomes the `actor` field on emitted events; replies will route to it via the request-id pairing rules in [events.md](events.md#how-requests-reads-events).

`reply` takes a separate `--as <external-actor>` flag for caller identity — it does NOT read `AE_SENDER_OVERRIDE` directly, because outside a tmux pane there is no implicit identity to fall back on. Internally `reply` sets `AE_SENDER_OVERRIDE` itself when it delegates the underlying write to `send`.

For targets the bridge does NOT have a pane to send to — e.g. the bridge is itself addressed via `telegram:*` / `discord:*` — see *Event-only target prefixes* below.

## Event-only target prefixes

`send` and `ae_tracked_send` short-circuit when the target matches an allow-listed external-actor prefix:

- `telegram:*`
- `discord:*`

For these targets, the helper emits the event into `events.jsonl` and exits. No tmux paste. No `ae_resolve` call. The literal target string is preserved end-to-end so the bridge can route it.

```text
05-28T13:00:00  send     claude:lead          → telegram:clemens     stage 0 review came back clean
05-28T13:00:00  ask      codex:coworker       → discord:#review     please confirm before push
```

Other target strings that don't match an existing pane and don't match the allow-list still fail via `ae_resolve` so typos surface loudly. To add a new platform prefix, ae's source must be updated — there is no runtime configuration knob.

## Reading events

Bridges read `~/.ae/sessions/<name>/events.jsonl` directly. Two common patterns:

**Tail.** Watch a single live session:

```bash
tail -F ~/.ae/sessions/<name>/events.jsonl
```

**Multi-session fan-in.** A bridge serving multiple sessions can tail all of them:

```bash
tail -F ~/.ae/sessions/*/events.jsonl
```

ae writes through a single `flock`-serialized writer per session, with each record terminated by a newline. Readers do not take the lock, so a tailer should still buffer until newline and treat a malformed or unterminated trailing line as "wait for more". Recovery hygiene a bridge implementer should plan for:

- The events file may not exist until the first write into a freshly created session. Tailing it should tolerate `ENOENT` and pick up when it appears.
- If the bridge persists offsets to avoid replay on restart, key them by `(session_id, inode)`. The path can change (rename, transfer); the inode changes if the session is recreated.
- ae does not rotate `events.jsonl`. The file grows for the lifetime of the session. Bridges should not load it whole; tail or back-scan only.
- A session directory can disappear (`ae end` / `ae rm`), be renamed (`ae rename`), or move between machines (`ae transfer`). Treat the session as the durable identity (`session_id`), not the path.

For schema and per-action semantics, see [events.md](events.md). The actions a bridge typically cares about as of the Stage 0 substrate are `send`, `ask`, `review`, `reply`, `done`, `alert`, `nudge`, `chat`, and possibly `memo`. `chat` is a first-class bridge action: an agent's free-text reply to the human (via the `say` helper), carrying its text in `summary`. Internal actions like `focus`, `spawn`, `retire`, `recover`, `throttled` are usually noise for a chat surface.

## Binding by session identity

A session's human-readable name can change (`ae rename`) and the workspace can move between machines (`ae transfer`). Bridges that need a stable handle should bind to `session_id` instead of the name.

```bash
grep '^session_id=' ~/.ae/sessions/<name>/meta | cut -d= -f2-
```

The session UUID is generated at first session creation, preserved on resume, preserved across rename, and copied during transfer. Bridges discovering sessions on a host can scan `~/.ae/sessions/*/meta` for `session_id` and key their internal state on that value.

## Bridge ownership

> **Superseded at P4.3 (2026-08-29).** ae has **one** bridge: the Rust core, run
> in the `ae-telegram` tmux session. The bash telegram daemon and the in-process
> aewatch bridge are both retired, so there is no second owner to coordinate
> with, no `bridge-owner` marker and no handoff. Everything in this section is
> **archival protocol history**, kept because it is the contract an *external*
> bridge implementation was told to honour — see the closing note.

ae once drove a session's outbound path from either of two bridge implementations — the bash `ae-telegram` daemon or the in-process aewatch bridge. Only one could send at a time, coordinated by a durable marker rather than a lock — a lock can't span the bash/Python boundary or a process handoff:

- The owning bridge writes `$AE_HOME/aewatch/bridge-owner` and keeps `$AE_HOME/aewatch/heartbeat` fresh (touched each tick).
- Any bridge — or a bash reviver — treats the marker as authoritative only while the heartbeat is fresh (age ≤ 90s). A stale heartbeat means the owner is gone and the marker suppresses no one.
- Handoff order is strict: claim the marker → stop the other bridge → only then send. There is no window in which both send, and the shared `~/.ae/telegram/` offset files mean the taking-over bridge resumes from the last durable offset.

A bridge implementer coexisting with ae's own bridge no longer has a marker to read: ae's Rust bridge does not write `bridge-owner`, and nothing in ae reads it. An external bridge that wants to coexist should key on the single `ae-telegram` session and on the durable state it owns — `~/.ae/telegram/tg_offset` for the inbound offset and `<meta>/telegram-outbound.cursor` per session for outbound — rather than on the retired marker protocol above.

## Allowed and disallowed assumptions

**Allowed:**

- `events.jsonl` is append-only. Past lines never change.
- One line per event. No multi-line records.
- ISO-8601 UTC timestamps, second precision.
- `actor` and `target` are strings. They use `:` as a separator but bridges should treat them as opaque except when matching the external-actor prefix allow-list.
- `session_id` is stable for the lifetime of a session — including across transfer.

**Not allowed:**

- Assuming the file size stays bounded. Long sessions grow without rotation. Bridges should not load the whole file into memory; tail or back-scan.
- Assuming the human-readable session name is stable. Use `session_id`.
- Assuming `actor` always matches a tmux pane. External actors won't.
- Writing to `events.jsonl` directly. Always go through `send` / `ask` / `review` / `reply` / `memo` — the schema is enforced by `_lib::ae_emit_event`, not by file convention.
- Assuming this list of write helpers is final. New helpers add new event actions over time (the `state` helper emits `state` events for `working` / `waiting-user` / `blocked` / `done`; the `say` helper emits `chat`). Bridges should ignore unknown event actions.

## Versioning

The protocol has no explicit version field today. The schema is the version. Additive changes (new optional event keys, new action types) are non-breaking; bridges should ignore unknown fields and actions. Renames, removals, or semantic changes to existing fields are breaking changes and will be called out in ae release notes when they happen.

## Open work

This document covers the bridge substrate: `events.jsonl` as a tail target, `AE_SENDER_OVERRIDE` for caller identity, the `telegram:` / `discord:` event-only target prefixes, `session_id`, and bridge ownership. New write helpers add new event actions as they ship (bridges ignore unknown actions).

The bridge daemon itself — its deploy story, auth model, rate-limit handling, and platform priority — is tracked in [issue #1](https://github.com/clemens33/ae/issues/1) and out of scope here.
