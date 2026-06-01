# Session helpers

Every ae session has a directory at `~/.ae/sessions/<name>/` filled with generated bash scripts. Agents call them by absolute path (they're deliberately not on `PATH`); humans can too. All helpers regenerate from the running ae binary on every start, resume, and `ae doctor --refresh`.

## Communication

| Helper | Purpose |
|---|---|
| `send <agent> <message>` | Fire-and-forget message to another pane (serialized with flock). |
| `ask <agent> <question>` | Tracked request — embeds your identity and an exact reply command with a request id. |
| `review <agent> <request>` | Like `ask`, but with the critical-review prompt template (findings-first, BLOCKER/IMPORTANT/NIT). |
| `reply <request-id> <message>` | Reply to a logged `ask` / `review` by request id. Pass `--as <agent>` to override identity when needed. |
| `requests [mine\|inbox\|all]` | Inspect pending / replied state from `events.jsonl` without peeking panes. |
| `say <text>` | Push a free-text line to the human's Telegram chat (args or piped stdin). Emits a `chat` event the [Telegram bridge](telegram.md) forwards; a Telegram reply routes back to you. Pane output is not forwarded — this is how you answer the human on Telegram. |

All messaging helpers emit a structured event into `events.jsonl` so the morning-after view stays auditable.

### How they compose

`ask` and `review` are thin wrappers over the shared `ae_tracked_send` in `_lib`. Everything ultimately dispatches through `send`, which is the only helper that actually pastes into a pane. `reply` looks up the original request via `ae_find_request` (also in `_lib`) before delegating to `send`.

```mermaid
flowchart LR
    A[Agent A pane] --send--> S[send helper]
    A --ask--> AK[ask helper]
    A --review--> RV[review helper]

    AK --> TS["_lib::ae_tracked_send<br/>(resolve target,<br/>mint req_id,<br/>build message)"]
    RV --> TS
    TS -->|exec env<br/>_AE_EVENT_ACTION=ask/review| S

    S -->|tmux paste| B[Agent B pane]
    S -->|ae_emit_event| EJ[(events.jsonl)]

    B --reply--> RP[reply helper]
    RP -->|ae_find_request<br/>verify pairing| EJ
    RP -->|exec env<br/>_AE_EVENT_ACTION=reply| S
    S -->|tmux paste| A
```

Only one helper touches tmux (`send`). Only one path mints request ids (`ae_tracked_send`). Only one helper validates reply pairing (`ae_find_request`). That symmetry is why the surface is auditable in `events.jsonl` — every interaction passes through the same emit point.

## State

| Helper | Purpose |
|---|---|
| `mark-done [message]` | Signal completion / pause. The loop watchdog stops nudging until a newer ae event mentions the agent. |
| `memo add [--topic t] <text>` | Append to durable shared session memory. |
| `memo read [--topic t]` | Read shared memory. |
| `memo tail [n]` | Show latest entries. |

`memo` is the right place for findings, decisions, and handoffs that should survive agent restarts. Don't dump chat transcripts.

## Inspection

| Helper | Purpose |
|---|---|
| `peek <agent> [lines]` | Capture recent output from another agent's pane (default 80, max 2000). Inspection only — never a reply mechanism. |
| `peak <agent> [lines]` | Alias for `peek` (common typo). |
| `agents` | List session agents with pane ids and current process. |
| `agents --all` | List agents across every running ae session. |

## Lifecycle

| Helper | Purpose |
|---|---|
| `focus <agent>` | Switch tmux focus to another pane. |
| `interrupt <agent> [message]` | Cancel current generation, optionally send a replacement instruction. |
| `spawn <alias:name> [prompt]` | Add a new agent to the workspace. Always pass a descriptive role name. |
| `retire <agent>` | Remove a spawned agent — kills the pane, cleans meta, updates `workspace.md`. |

## Internal

Helpers prefixed `_` are launched by ae itself, not by you or by other agents.

- `_register-sid` — Codex self-registers its session UUID on launch (so ae can resume the exact conversation later).
- `_loop` / `_events` (pane tags, not scripts) — the panes inside the `ae-monitor` window.

## Name resolution

Every helper that takes an agent argument resolves it flexibly:

- `codex:reviewer` — exact match (alias + name).
- `codex` — alias only, when unique in the session.
- `reviewer` — bare name, when unique.
- `%42` — raw tmux pane id.
- `@other-session:codex:reviewer` — cross-session.

## Cross-session

Prefix any target with `@<session>:` to reach an agent in a different ae session:

```bash
~/.ae/sessions/<your-session>/send @other-feature:claude:lead "check my API changes"
~/.ae/sessions/<your-session>/peek @other-feature:reviewer 50
~/.ae/sessions/<your-session>/agents --all
```

The receiving agent gets the message exactly as if it came from a same-session sender.

## Reply contract

When an agent receives an `ask` / `review` request, the message includes the exact reply command they MUST run when done:

```text
REQUEST ae-20260518T064807Z-bedad2f3 from claude:lead: <your question>

REQUIRED: When you have finished, you MUST run this exact command to reply:
/home/ckriech/.ae/sessions/<name>/reply --as "codex:coworker" "ae-2026..." "<your reply>"
Do not reply any other way. Do NOT use peek/peak as a reply mechanism.
```

Agents are instructed to run that command verbatim. If they do, `requests` picks up the pending → replied transition automatically.
