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
headers with bodies; every body line is indented two spaces, and one blank
line closes each row:

```text
scope: current conversations only (phase 1b) — a seat that resumed keeps only its current transcript
coverage incomplete: demo:colead — opencode: not read
## 2026-09-16T09:00:00.500000Z demo:lead
  ship the slice today

```

`--lines <n>` clips each text body to its first `<n>` lines; the dropped
remainder prints one indented marker, `  … +k lines` (k = dropped line count). A
body of at most `<n>` lines prints whole and gets no marker. `--lines` is
text-only: with `--json` it is a usage error, because NDJSON always carries the
whole body.

`--json` prints NDJSON — one scope line, coverage lines, then row lines:

```json
{"kind":"scope","scope":"current-conversations","phase":"1b"}
{"kind":"row","ts":1789549200500000,"actor":"demo:lead","role":"human","body":"ship the slice today","source":"claude","file":"/tmp/demo.jsonl#1:2","offset":0}
```

## Follow

`--follow` prints the one-shot board, then keeps printing NEW human rows and
coverage changes every 5 seconds until Ctrl-C ends the process. The selection
is fixed at start: the running sessions (or the named ones) are chosen once,
and a session started later is not picked up — restart the follow. Every poll
does re-read each selected session's roster and re-locate each seat's
transcript, so a re-launched seat or a rotated file is caught immediately.

Offsets bind to the transcript file's identity — dev+inode, never the path —
and per seat the follow holds the identity, the commit point after the last
complete line, and the mtime it last saw:

- **append** — same identity, more bytes: stream from the commit point; only
  the new rows print, with absolute offsets.
- **rescan** — a new identity, a shrunken file, or the same length rewritten
  (mtime changed): stream from zero and print EVERY row again, behind one
  coverage line, `transcript replaced — rescanned` or `transcript rewritten —
  rescanned`. Rescans are loud and complete; nothing is silently deduplicated
  across generations. A torn last record holds the commit point at the last
  newline and is retried next poll.
- **hold** — nothing new: no read, no output.

After the first pass, coverage prints on CHANGE only: a seat that becomes
readable prints nothing, a seat that becomes unreadable prints its new reason
once. Rescan lines always print. Batches are sorted internally by
`(ts, file, offset)` but never merged across batches — a late seat's older row
prints later. Every batch prints its rows through the same renderer as the
one-shot — same indented shape, same `--lines` clip. Every row keeps its
durable identity (`file` = `path#dev:ino` plus `offset`), so a consumer that
wants dedup across generations can have it.

## Phases

1b Claude CLI · 2 codex · 3a grok · 3b muse · 4 `--follow` (this slice) ·
5 agy · 6 OpenCode (ruled out — not read) · 7 assistant rows · 8 predecessors.

Codex reads the seat's current rollout only, and only the `response_item`
record of each user turn — never its older `event_msg` twin. The project-doc
turn Codex injects is harness plumbing, excluded: by its `agents_md.instructions`
kind when the rollout carries one, else by its `# AGENTS.md instructions`
first line.

Grok reads the seat's current `updates.jsonl` only, one `user_message_chunk`
per turn, time from `_meta.agentTimestampMs` millis when present.

Muse reads the seat's current `session.jsonl` only — each accepted intent's
model text, never its materialized twin, `recorded_at` micros native.
