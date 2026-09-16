# The message board

`ae board` shows the filtered cross-fleet record: genuine human turns from
every ae seat's harness transcript, oldest first, ae plumbing excluded. It
answers "what did the humans actually say" at review-after, handoff, and
brief-writing time.

## Source

Harness transcripts, derived on read. The board opens the transcript files the
agents' own CLIs wrote — no hooks, no writer, no board database. Each seat is
located through the conversation identity and config home ae recorded at
first start.

## Scope

Current conversations only (phase 1b): a seat that resumed into a new
conversation shows that conversation alone. Predecessor transcripts arrive in
phase 8, behind a durable predecessor record written at resume. Turns written
before provenance shipped (`v2026.9.79`) read as human — an accepted gap,
never a shim.

## What it never reads

- The OpenCode `credential` table or any non-`message`/`session` table, forever
- Any `auth.json` or token file
- Live `~/.ae` session state beyond the roster (meta only)

## Output

Text (default): the scope line, coverage rows, then `## <ts> <session:seat>`
headers with bodies:

```text
scope: current conversations only (phase 1b) — a seat that resumed keeps only its current transcript
coverage incomplete: demo:colead — grok: phase 3a
## 2026-09-16T09:00:00.500000Z demo:lead
ship the slice today

```

`--json` prints NDJSON — one scope line, coverage lines, then row lines:

```json
{"kind":"scope","scope":"current-conversations","phase":"1b"}
{"kind":"row","ts":1789549200500000,"actor":"demo:lead","role":"human","body":"ship the slice today","source":"claude","file":"/tmp/demo.jsonl#1:2","offset":0}
```

## Phases

1b Claude CLI · 2 codex (this slice) · 3a grok · 3b muse · 4 `--follow` ·
5 agy · 6 OpenCode (after its ruling) · 7 assistant rows · 8 predecessors.

Codex reads the seat's current rollout only, and only the `response_item`
record of each user turn — never its older `event_msg` twin.
