# The message board

`ae board` shows the filtered cross-fleet record: genuine human turns from
every ae seat's harness transcript, oldest first, ae plumbing excluded. It
answers "what did the humans actually say" at review-after, handoff, and
brief-writing time.

## Source

Harness transcripts, derived on read. The board opens the transcript files the
agents' own CLIs wrote — no hooks, no writer, no board database. Each seat is
located through the conversation identity and config home ae recorded at
first start. Claude Code, Codex, Grok, Muse and Antigravity seats read today;
every other seat renders an explicit coverage row, never a silent subset.

## Scope

Current conversations plus each seat's recorded predecessors (phase 8b):
every seat reads its current conversation and, newest first, the up-to-4
abandoned conversations its `harness_session_prior.<slot>` row records —
nothing is inferred from time. A predecessor that cannot be read becomes a
coverage row prefixed `predecessor n:` with the reason the current seat would
print. After a resume fallback the current id may be `pending`: then the
current read yields its coverage row as today and the predecessors still read.
Under `--follow` predecessors read ONCE, on the first pass — an abandoned
conversation never grows, so no poll revisits one and the follow holds no
offsets for them. Every row carries its `generation` (0 = current, n = nth
predecessor); text headers read `## HH:MM:SS <session:seat> · prior n` for
n ≥ 1, composed with ` · assistant` as ` · prior 1 · assistant`. Turns written
before provenance shipped (`v2026.9.79`) read as human — an accepted gap,
never a shim.

## What it never reads

- The OpenCode `credential` table or any non-`message`/`session` table, forever
- Any `auth.json` or token file
- Live `~/.ae` session state beyond the roster (meta only)

## Output

Text (default): the scope line, coverage rows, then one `# YYYY-MM-DD UTC`
date divider and `## HH:MM:SS <session:seat>` headers with bodies; every body
line is indented two spaces, and one blank line closes each row. The divider
prints before the first row and again only when the UTC day moves past the
previously printed row's — within one board and across `--follow` batches.
Times are UTC, stated once in the divider, never on the row:

```text
scope: current conversations plus each seat's recorded predecessors (up to 4, newest first) — nothing is inferred from time
coverage incomplete: demo:colead — opencode: not read
# 2026-09-16 UTC
## 09:00:00 demo:lead
  ship the slice today

```

`--lines <n>` clips each text body to its first `<n>` lines; the dropped
remainder prints one indented marker, `  … +k lines` (k = dropped line count). A
body of at most `<n>` lines prints whole and gets no marker. `--lines` is
text-only: with `--json` it is a usage error, because NDJSON always carries the
whole body.

`--assistant` (off by default) adds the model's replies beside the human turns:
one row per transcript record, ` · assistant` after the actor in the text header
and `"role":"assistant"` in JSON, body joined from the record's TEXT parts alone
in order. `thinking`, `tool_use`, reasoning parts and every tool call are never
read, an empty body drops silently, and an unstamped record counts into the
same missing-timestamp coverage. Claude Code excludes `isApiErrorMessage ==
true` records whole and reads `type=="assistant"` records only; Codex reads
`response_item`/`message`/`role=="assistant"` and its `output_text` parts only,
never the `reasoning`, call or `event_msg` twins. Grok joins the
`agent_message_chunk` deltas of one turn into a single row at the run's first
chunk — a user chunk, a `turn_completed` or EOF ends the turn, and a message
boundary inside one turn fuses (the store carries no separator). Muse reads
the whole text of each `assistant_message_committed` event. Antigravity has
no assistant records: each agy seat prints one coverage line saying so. Under
`--follow` a grok stream ending mid-turn holds its commit point at the open
run's first chunk, so the next poll re-reads and joins the whole turn instead
of printing a fragment. The first pass still prints an open run as-is, and the
next poll prints the completed turn again at the same offset — loud,
dedup-able by `file`+`offset`. Markers classify human rows only — a reply may
legitimately quote one. Without the flag the stream is byte-identical to the
human-only board.

```text
scope: current conversations plus each seat's recorded predecessors (up to 4, newest first) — nothing is inferred from time
# 2026-09-16 UTC
## 09:00:00 demo:lead
  ship the slice today
## 09:00:01 demo:lead · assistant
  drafted it; the review is open
```

`--json` prints NDJSON — one scope line, coverage lines, then row lines:

```json
{"kind":"scope","scope":"current-and-recorded-predecessors","phase":"8b"}
{"kind":"row","ts":1789549200500000,"actor":"demo:lead","role":"human","body":"ship the slice today","source":"claude","file":"/tmp/demo.jsonl#1:2","offset":0,"generation":0}
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
one-shot — same indented shape, same `--lines` clip, same divider rule: a
batch on the printed day opens with no divider, a new day opens with one.
Every row keeps its
durable identity (`file` = `path#dev:ino` plus `offset`), so a consumer that
wants dedup across generations can have it.

## Phases

1b Claude CLI · 2 codex · 3a grok · 3b muse · 4 `--follow` · 5 agy · 6 OpenCode
(ruled out — not read) · 7a assistant rows (Claude Code, Codex) · 7b assistant
rows (grok, muse, agy coverage) · 8 predecessors (done: 8a records, 8b reads).

Codex reads the seat's current rollout only, and only the `response_item`
record of each user turn — never its older `event_msg` twin — and, with
`--assistant`, only the `output_text` parts of its `role=="assistant"` message.
The project-doc turn Codex injects is harness plumbing, excluded: by its
`agents_md.instructions` kind when the rollout carries one, else by its
`# AGENTS.md instructions` first line.

Grok reads the seat's current `updates.jsonl` only, one `user_message_chunk`
per turn, time from `_meta.agentTimestampMs` millis when present.

Muse reads the seat's current `session.jsonl` only — each accepted intent's
model text, never its materialized twin, `recorded_at` micros native.

Antigravity reads the seat's own `history.jsonl` records only. One file per home
carries every agy conversation, so attribution is by the captured conversation
id alone — a record of another conversation is skipped silently, `workspace` is
never consulted, and a pre-field CLI record can never match. `display` is the
typed prompt (`"type":"slash_command"` included), `timestamp` integer millis.
