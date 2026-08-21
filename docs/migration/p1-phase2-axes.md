# Which axes the phase-2 evidence varies — and which it holds constant

**By `opus5:lexec`.** Subject: phase-2 gate blob `29db943aa85319534301332052105ba16df03b4d`
(verified, 20382 bytes, the blob at `HEAD`) and `tests/it/phase2.rs`. `src/` not read beyond the
symbols the tests import; **no function body in `src/` was opened**, so every finding below is a
statement about what the evidence can discriminate, not about how the product happens to be
written.

**The event axis is excluded by instruction** — already found, already being repaired, and the
`Scratch::events` doc comment now carries the lesson.

Working rule: **a differential test discriminates exactly over the axes it moves.** Anything
identical in every arm is invisible to it.

---

## Axes the evidence DOES vary

| axis | where | arms |
|---|---|---|
| status | c13, c16 | running / stopped / unknown |
| degradation | c13 | clean / damaged, flipped independently of status |
| read-loss mechanism | c13 | readable-no-selector vs absent-meta, both yielding `missing` |
| inventory completeness | c23 | complete / incomplete, and earned through real discovery in the second arm |
| schema version | c17, c19 | 1 / 2 |
| render route and filter | c15 | every filter, human and JSON |
| filesystem after inventory | c14 | unchanged / grown / deleted |
| server availability | c9, c14 | live / down |
| entitlement | c22 | named-by-phase-1 / unentitled-but-present |
| selector kind **at the tmux primitive** | c20 | `Selector::Name` / `Selector::Socket` against two real servers |

---

## Finding 1 — OWNERSHIP IS CONSTANT IN EVERY CLASSIFICATION ARM *(admits a wrong implementation)*

Measured across the whole file: **15 owned pairs, and in every one the marker equals the session
name.** Zero pairs where the marker is present but names something else. There is exactly **one**
unowned pair — `("alpha", None)` at line 307 — and it sits inside
`criterion_20_the_recorder_tells_success_absence_ownership_and_failure_apart`, the **recorder
calibration** test, not a classification cell.

So across every arm that reaches `classify()`, ownership is held constant at *present and
matching*.

**The wrong implementation this admits:** `positively_owned(name, marker)` implemented as
`marker.is_some()` — checking **presence** and never comparing to the name. Every test in the file
passes, because no fixture ever supplies a marker that disagrees with its session name.

That implementation is wrong against the gate's own criterion 5, which requires four cells —
exact+owned → `running`, exact+**ownership missing** → `unknown`, exact+**ownership mismatched** →
`unknown`, exact absent → `stopped`. Two of those four cells are unreachable from the current
fixtures, and the mismatched cell is the one that separates *comparing* from *checking presence*.

**This is the hazard shape exactly: the distinguishing input is ABSENT rather than OPPOSED.** `None`
is absence; a marker naming a different session is the opposed form, and it is the only arm that
discriminates. Same structure as an absent event log — the axis never moves, so nothing about it
can be observed.

## Finding 2 — A SOCKET SELECTOR NEVER REACHES `classify()` *(admits a wrong implementation)*

`ServerSelector::Positive(Selector::Socket)` occurs **zero times** in the file. Every durable
fixture is written by `Scratch`, and every `Scratch` meta carries `tmux_server_kind=name` — the only
two `tmux_server_kind` occurrences in 1427 lines are both `name`. The `positive()` helper builds
`ServerSelector::Positive(Selector::Name(..))` and nothing else.

Socket selectors appear only as a `ServerId` inside the criterion-20 routing tests, which exercise
`ae::tmux::list_sessions_args` against two real servers. **That proves routing, not classification.**

**The wrong implementation this admits:** any classification path that mishandles
`Selector::Socket` — routes it to the wrong server, or treats it as `missing` and returns `unknown`.
Criterion 20 still passes, because it never calls `classify()`; every classification test still
passes, because every candidate it classifies carries a `Name`.

SC-405l makes `positive(name)` and `positive(socket)` two distinct states of one typed fact, and the
gate's criterion 3 asks for the relation "once with a positive named selector and once with a
positive socket selector." The socket half of that pairing exists nowhere in the classification
evidence.

## Finding 3 — TWELVE OF TWENTY-THREE CRITERIA HAVE NO TEST *(the direct answer to the question)*

Tests exist for criteria **1, 9, 13, 14, 15, 16, 17, 19, 20, 22, 23**. Nothing anywhere in `tests/`
references **2, 3, 4, 5, 6, 7, 8, 10, 11, 12, 18, 21**.

Verified as a genuine absence rather than a naming difference, by probing the distinctive vocabulary
each would need:

| term | hits in `tests/it/phase2.rs` |
|---|---|
| `prefix` (c4) | **0** |
| `grouping` (c11) | **0** |
| `same name` (c12) | **0** |
| `mismatch` (c5) | **0** |
| `two servers` (c8/c12) | **0** |

These are the core SC-017k/SC-017l criteria — recorded-server-beats-ambient, exact-vs-prefix,
the ownership table, success/failure over identical bytes, the nine-cell non-proof table,
per-server failure locality, dual provenance, grouping, and same-name-different-servers. **Their
axes are varied nowhere**, which is why findings 1 and 2 are sharper than this one: those two name
a specific wrong implementation that passes the suite *as it stands*, whereas the rest are
unproven.

---

## What I checked and found clean

- **The criterion-13 matrix is honestly built.** Every cell is produced by the product reader, the
  degraded cells earn degradation from an independent record fact rather than from a broken query,
  and `criterion_13_the_degraded_cells_are_reachable_rather_than_asserted` is a real reachability
  control — it exists precisely to stop the matrix describing an impossible state. Its comment
  about a fresh recorder, because "the double answers with the FIRST world it holds for a server",
  is a fixture hazard caught by its author.
- **Criterion 23's second test answers the authoring hazard I raised last pass.** Both roots are
  empty, so "the candidate sets are equal — neither source had anything to lose" is true by
  construction rather than by luck, and the delta is earned through a real `read_dir` failure.
- **Criterion 14's manifest comparison** captures path, kind and content length, so a same-length
  overwrite is the only mutation it would miss — and criterion 14's own record-change test covers
  that separately.
