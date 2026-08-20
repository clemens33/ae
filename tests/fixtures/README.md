# Fixture session directories

Hand-built session directories, one per contract row, read by `tests/it/fixtures.rs`.

**These are placeholders with a known expiry.** #81 deliverable 3 replaces them with the
golden corpus: immutable snapshots from the C-cluster probe batch, each naming its
contract-row authority with a red-proof mutation. Until that lands, these exist so the P1
reader has something shaped like a real session directory to read.

**What they are NOT.** No line here was captured from a bash run. Every byte was written
from the SHOULD text of the row named in the directory name — the anti-oracle rule, applied
to fixtures: a measured value cannot define an expected value. If a bash capture later
disagrees with one of these files, that is a finding for the seats, not a fixture to fix.

Each directory holds a `meta` (SC-405a–c) and, unless the fixture is about its absence, an
`events.jsonl`. Most `meta` files carry only the keys SC-405b and SC-405c name, which keeps
each fixture about one row. A real bash-era meta holds many more keys; SC-405d ratified
that those are tolerated silently and never degrade, and SC-405h was REJECTED so no row
enumerates them — `sc-405d-unknown-key/` pins the tolerance, and no fixture here tries to
mirror the live key population, because a census is evidence and never contract.

| Fixture | Row | What it exercises |
|---|---|---|
| `sc-510a-required-keys/` | SC-510a | `ts` / `actor` / `action` present on every record |
| `sc-510b-optional-keys-omitted/` | SC-510b | `target`/`ref`/`summary` absent rather than empty |
| `sc-510c-ref-polysemy/` | SC-510c (amended) | `ref` as request id, memo topic, captured session id, **declared state**, and an action the table leaves undefined |
| `sc-510d-escaped-strings/` | SC-510d | the escape set `\"` `\\` `\n` `\t` `\r` in a `summary` |
| `sc-511a-routing-keys/` | SC-511a, SC-511b | the four routing-key fields, and a record carrying none |
| `sc-511c-additive-keys/` | SC-511c | unknown keys of unknown types, which a reader must step over |
| `sc-017e-activity-clock/` | SC-017e | the newest event is the activity clock, even out of file order |
| `sc-017g-unanswered-request/` | SC-017g | an `ask` whose target never replied — a stray reply from a third agent does not close it |
| `sc-017g-answered-request/` | SC-017g, SC-511b | a `review` closed by a replier whose display name changed but whose routing key did not |
| `sc-518-reply-to-someone-else/` | SC-518 | the full mirror: the right responder replying to a **third party** closes nothing |
| `sc-519-absent-event-log/` | SC-519 | a session with a `meta` and no `events.jsonl` — quiet, **not** degraded |
| `sc-509b-meta-missing/` | SC-509b | a directory with no `meta` at all — degraded, and the loss reaches the JSON |
| `sc-520-malformed-record/` | SC-520, DR-001 | malformed complete lines: skipped, reported with generation+offset+reason, session degraded, good records still read |
| `dr-001-partial-tail/` | DR-001, SC-975b | a record the writer has not finished; the cursor must not land mid-record, and a buffered tail is **not** damage |
| `sc-405c-roster/` | SC-405b/c, SC-510c | the roster keys, `agent_bin`, an optional session id, and per-agent declared states that roll up |
| `sc-405d-unknown-key/` | SC-405d | a key outside SC-405b/c: seen by the reader, tolerated, **not** degrading |
| `sc-405e-malformed-meta/` | SC-405e, SC-509b | a meta line the reader could not take — actual loss, so it degrades |
| `sc-405f-goal-event/` | SC-405f | `goal` text from the meta, `goal_set_epoch` from the **latest goal event** |
| `sc-405j-stale-session/` | SC-405j | a routed event whose session was renamed: unassociated, never attributed by display name |

`dr-001-partial-tail/events.jsonl` deliberately has **no trailing newline**. The integration
test asserts that byte fact first, so an editor or hook that normalises the file fails the
test loudly instead of quietly turning the fixture into a copy of the happy path.

`sc-509b-meta-missing/` holds only `.gitkeep`: git cannot store an empty directory, and the
absence of both files is the whole fixture.

A guard test pins the corpus at **19** directories, so a fixture added without a place in
this map fails the suite.
