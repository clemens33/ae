# In-place seat compaction (`ae compact`)

The verb is `src/seatcompact_run.rs::run`; the pure half is
`src/seatcompact.rs` (vocabulary, gates, audit shapes, report). The compaction
capability is the `compact` row on `ToolAdapter` (`src/tool.rs`), matched by
the call site — never the tool.

## Vocabulary

Exactly three words per seat. `dispatched` means attempted: the command was
pasted and Enter was sent; it is never proof of submission. `Unknown` submit
verdicts render `dispatched` with the proof gap on the RECORD
(`unverifiable=`), never in the word. `skipped (<reason>)` pasted nothing;
`not dispatched (<reason>)` may have left text staged in the seat's composer.
A dispatched seat is then watched: `observed idle` means the seat's own frame
read idle twice after the dispatch; it is never proof of compaction.
`unobserved: <reason>` names why readiness was not seen. No launch stamp when
the baseline is taken reads `guard unavailable`, which proves nothing about the
session's age; a held stamp that later disappears or changes reads `relaunched`.

## Gates (in order)

Spawned seats are skipped untouched, then `Unsupported` capability, then
unmodelled input. Of these, only the unmodelled skip is actionable: those
seats are named on the report's `compact by hand:` line. After the gates, an
admitted seat whose live identity is the caller's own is skipped
`initiating seat` before its checkpoint: its tool is blocked on the run and
could never answer, so its report line names the remedy. A caller ae cannot
place is never guessed to be a seat.

## Audit records

Actor `ae:seats:<session-uuid>` throughout. Each seat appends one
`seat-compact` event (`run=`, `target_slot`, `target_session`, `reason=` on
skips and unsubmitted pastes, `unverifiable=` on `Unknown` only,
`sanitized=` when R15 stripped anything). A dispatched seat then appends one
`seat-compact-observed` event (`observed idle <slot>`, or `unobserved <slot>`
with `reason=`). The run brackets itself with two
`seat-compact-run` events (`start` / `end`, `ref` = run id). At start the verb
prints one information-only note per seat whose `dispatched` record names a
run with no `end`; the note never gates anything.

## Report

One line per seat (`<word> <slot> (<elapsed>)`, a dispatched seat's word
carrying `(observed idle)` or `(unobserved: <reason>)` and its elapsed time
including the watch; plus the composer remedy when a paste may sit staged, or
the run-it-from-outside remedy for an `initiating seat`), then
`no seat admitted` when no seat passed the gates, then the `compact by hand:`
line when any seat was skipped for unmodelled input. The run exits 1 when any
admitted seat was not observed idle, and 0 when every admitted seat was, or
none was admitted. Every record-derived field is projected; the pasted bodies
are verbatim after the R15 strip, never projected.

## Readiness frame

`harness_state::classify` reads a Claude frame as `Idle` when the row nearest
above its empty box is exactly `⎿  Compacted (ctrl+o to see full summary)`.
That says the frame looks ready, never that compaction happened: a mid-turn
auto-compaction may draw the same row (unmeasured), and a manual-mode footer
(`⏸`) still reads `Unknown`. The watchdog and `ae reseat` read it the same way.
