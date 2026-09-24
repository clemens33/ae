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

## Gates (in order)

Spawned seats are skipped untouched, then `Unsupported` capability, then
unmodelled input. Only the unmodelled skip is actionable: those seats are
named on the report's `compact by hand:` line.

## Audit records

Actor `ae:seats:<session-uuid>` throughout. Each seat appends one
`seat-compact` event (`run=`, `target_slot`, `target_session`, `reason=` on
skips and unsubmitted pastes, `unverifiable=` on `Unknown` only,
`sanitized=` when R15 stripped anything). The run brackets itself with two
`seat-compact-run` events (`start` / `end`, `ref` = run id). At start the verb
prints one information-only note per seat whose `dispatched` record names a
run with no `end`; the note never gates anything.

## Report

One line per seat (`<word> <slot> (<elapsed>)`, plus the composer remedy when
a paste may sit staged), then the `compact by hand:` line when any seat was
skipped for unmodelled input. Every record-derived field is projected; the
pasted bodies are verbatim after the R15 strip, never projected.

## Readiness frame

`harness_state::classify` reads a Claude frame as `Idle` when the row nearest
above its empty box is exactly `⎿  Compacted (ctrl+o to see full summary)`.
That says the frame looks ready, never that compaction happened: a mid-turn
auto-compaction may draw the same row (unmeasured), and a manual-mode footer
(`⏸`) still reads `Unknown`. The watchdog and `ae reseat` read it the same way.
