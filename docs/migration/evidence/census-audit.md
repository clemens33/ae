# Census audit — ownership evidence adequacy

Audit date: 2026-08-20.  Frozen empirical source: `72c729343a0117af2968b66e1c43f89ad25fc0b2`.
Inputs: `ownership.md`, `ratification-critical.md`, and `locks-census.md`,
`locks-census-2.md`, `locks-census-3-aewatch.md`.

This is an evidence-adequacy audit, not a build or a seat classification.  A census
pointer proves an empirical field only when the cited section states that field and
retains its `ae:<line>` citation to `72c7293`.  The empirical fields are:

`E = effects | current writer/call path | locks + order | atomicity | current owner`.

The normative lane is deliberately narrower:

`N = planned owner/fate | flip gate | rollback`.

An N field closes when the record has an epic-phase, DR, or seat-ruling citation that
names the fate.  The ownership document's epic default gate and rollback rule count for
records that do not state a deviation.  `OBSERVED` requires every E and N field.  A
`PARTIAL` row lists every missing field.  `D05` is governed by its special grouping rule
and therefore has `GROUPING-FAIL`, not a normal record verdict.

## Calibration

`D17` was checked first against its accepted `pin-audit.md` PROVES entry.  The pointer
`C11` below (census-2, “Session launch”) states the launch effects and residues, launch
call path, lifecycle lock and order, rollback/partial-failure boundary, and Bash source
ownership, with the cited `ae:<line>` ranges intact.  The ownership record supplies its
P3 open-design fate, default phase gate, and epic rollback rule.  This reaches the
accepted observed-equivalent result:

`CALIBRATION=D17  STATUS=PASS  VERDICT=OBSERVED-EQUIVALENT`

The audit proceeds because calibration did not fail.

`R1` is the one non-census closure used below: the standing both-seats ruling recorded
on D24 in `ownership.md` (`current effects / writer / owner: none — unimplemented; no
bash writer exists`, with the absence explicitly classified) closes its current-state
lane by ruling.  It is not a census pointer and does not invite a nonexistent-code
probe.  `OBSERVED-BY-RULING` is kept distinct from empirical `OBSERVED` in the summary.

## Census pointer register

Each pointer names a census heading whose body contains the frozen-source line citations.
`C` means Bash `ae`; `W` means `contrib/aewatch/aewatch`.

| Pointer | Census section (source citations retained) |
|---|---|
| C1 | `locks-census.md` §Helper `send` (`ae:12993-13000`, `ae:13171-13176`, `ae:13322-13359`, `ae:14235-14283`) |
| C2 | `locks-census.md` §`ask`, `review`, and `reply` (`ae:13490-13538`, `ae:14287-14403`) |
| C3 | `locks-census.md` §`state` (`ae:12836-12871`, `ae:13171-13176`, `ae:13263-13295`) |
| C4 | `locks-census.md` §`goal` (`ae:14127-14153`, `ae:14558-14589`) |
| C5 | `locks-census.md` §`memo add` (`ae:13171-13176`, `ae:14503-14523`) |
| C6 | `locks-census.md` §`say` (`ae:13171-13176`, `ae:14470-14485`) |
| C7 | `locks-census.md` §`spawn` (`ae:11916-12114`, `ae:12554-12581`, `ae:12602-12605`) |
| C8 | `locks-census.md` §`retire` (`ae:12213-12262`, `ae:12339-12353`) |
| C9 | `locks-census.md` §`_register-sid` (`ae:14755-14824`) |
| C10 | `locks-census-2.md` §`focus` (`ae:14648-14653`, `ae:13171-13176`) |
| C11 | `locks-census-2.md` §Session launch (`ae:16941-18233`, especially `ae:17282-18179`, `ae:12587-12698`) |
| C12 | `locks-census-2.md` §End/rm (`ae:2879-3137`, `ae:5285-5463`, `ae:8126-8210`) |
| C13 | `locks-census-2.md` §`stop` (`ae:7015-7952`, `ae:7420-7465`, `ae:12092-12115`) |
| C14 | `locks-census-2.md` §`rename` (`ae:11547-11671`, `ae:152-171`) |
| C15 | `locks-census-2.md` §`transfer` (`ae:10832-11537`, especially `ae:11003-11023`, `ae:11097-11114`) |
| C16 | `locks-census-2.md` §`compact` (`ae:5647-6459`, `ae:5912-5969`, `ae:6272-6390`) |
| C17 | `locks-census-2.md` §`doctor --refresh` (`ae:8536-8934`, `ae:12267-12339`, `ae:14934-15105`) |
| C18 | `locks-census-2.md` §`_recover-pending` (`ae:8690-8872`, `ae:16528-16536`) |
| C19 | `locks-census-2.md` §Request WITHDRAW (`ae:5961-5969`, `ae:14180-14194`) |
| C20 | `locks-census-2.md` §Watchdog controls and daemon loop (`ae:9145-9182`, `ae:14910-15105`, `ae:15981-16659`) |
| C21 | `locks-census-2.md` §Telegram setup/start/stop and daemon loop (`ae:9305-10678`) |
| C22 | `locks-census-2.md` §Steward (`ae:10357-10400`, `ae:12705-12802`, `ae:16722-18179`) |
| C23 | `locks-census-2.md` §Pre-dispatch config bootstrap (`ae:344-352`) |
| C24 | `locks-census-3-aewatch.md` §Shared event log (`aewatch:1463-1508`, `aewatch:2357-2406`) |
| C25 | `locks-census-3-aewatch.md` §Runtime singleton, heartbeat, log, and backoff (`aewatch:2798-3232`) |
| C26 | `locks-census-3-aewatch.md` §Bridge ownership handoff (`aewatch:2859-3348`, `ae:10469-10524`) |
| C27 | `locks-census-3-aewatch.md` §Shared Telegram stores (`aewatch:893-1149`, `aewatch:1411-1620`, `ae:9327-10254`) |
| C28 | `locks-census-3-aewatch.md` §Shared session metadata and recovery (`aewatch:1806-2450`, `ae:2068-2075`, `ae:8717-8732`) |
| C29 | `locks-census-3-aewatch.md` §Shared event/tmux state and tmux mutation paths (`aewatch:553-700`, `aewatch:1973-2006`, `ae:15524-15669`) |

| R1 | `ownership.md` D24 (`current effects / writer / owner: none — unimplemented`, classified absence and standing seat ruling; planned P3 design gate) |

For compactness, a field map cell is `field: Cn` when proven.  `—` means no census
pointer proves the whole field.  A pointer that covers only one half of a multi-path
record does not close that field.

## Per-record field maps

| Record | E: effects | E: writer / path | E: locks + order | E: atomicity | E: owner | N: fate / gate / rollback | Missing fields | Verdict |
|---|---|---|---|---|---|---|---|---|
| D01 | — (C23 proves inherited bootstrap only) | — | — | — | — | `ownership.md` P1 + epic defaults | E: effects, writer/call path, locks+order, atomicity, owner | PARTIAL |
| D02 | — | — | — | — | — | P1 + epic defaults | E: effects, writer/call path, locks+order, atomicity, owner | PARTIAL |
| D03 | — (event append mechanism is not the query) | — | — | — | — | P1 + epic defaults | E: effects, writer/call path, locks+order, atomicity, owner | PARTIAL |
| D04a | — (C23 proves inherited bootstrap only) | — | — | — | — | P1 + epic defaults | E: effects, writer/call path, locks+order, atomicity, owner | PARTIAL |
| D04b | — (C10 is helper `focus`, not `cmd_next`) | — | — | — | — | P1 + epic defaults | E: effects, writer/call path, locks+order, atomicity, owner | PARTIAL |
| D05 | **GROUPING-FAIL**; see split map below | **GROUPING-FAIL**; see split map below | **GROUPING-FAIL**; see split map below | **GROUPING-FAIL**; see split map below | **GROUPING-FAIL**; see split map below | P2 + epic defaults, but not a group verdict | grouping, not lane omission | GROUPING-FAIL |
| D06 | C1 | C1 | C1 | C1 | C1 (Bash `ae`) | P2 + epic defaults | — | OBSERVED |
| D07 | C3 | C3 | C3 | C3 | C3 (Bash `ae`) | P2 + epic defaults | — | OBSERVED |
| D12 | C10 | C10 | C10 | C10 | C10 (Bash `ae`) | P2 + epic defaults | — | OBSERVED |
| D13 | C9 + C11 + C18 | C9 + C11 + C18 | C9 + C11 + C18 | C9 + C11 + C18 | C9 + C11 (Bash `ae`) | P2, with D23 seat ruling + epic defaults | — | OBSERVED |
| D14 | — (umbrella has no independently stated field set) | — | — | — | — | no record-level fate/gate/rollback citation | E: all five; N: fate, flip gate, rollback | PARTIAL |
| D14a | C11 + C17 | C11 + C17 (`sync_session_assets` / declaration emission) | C11 + C17 | C11 + C17 (temp+chmod+mv per artifact) | C11 + C17 (Bash `ae`) | P2 logic dies, #76/seat ruling + epic defaults | — | OBSERVED |
| D14b | C11 (launch side) + C17 (refresh side only partially names helpers) | C11 + C17 (refresh launch-artifact path not stated) | C11 + C17 (refresh launch-artifact lock path not stated) | C11 + C17 (refresh launch-artifact publication not stated) | C11 + C17 (Bash `ae`) | stays Bash, epic end-state + epic defaults | E: effects, writer/call path, locks+order, atomicity (doctor launch-artifact half) | PARTIAL |
| D18 | C12 | C12 | C12 | C12 | C12 (Bash `ae`) | P3 + epic defaults | — | OBSERVED |
| D19b | C13 | C13 | C13 | C13 | C13 (Bash `ae`) | P3 + epic defaults | — | OBSERVED |
| D19c | C13 | C13 | C13 | C13 | C13 (Bash `ae`) | P3 + epic defaults | — | OBSERVED |
| D23 | C18 | C18 | C18 | C18 | C18 (Bash `ae`) | P2 with D13 seat ruling + epic defaults | — | OBSERVED |
| D24 | R1 (ruled classified absence) | R1 (ruled classified absence) | R1 (ruled absent; no protocol exists yet) | R1 (ruled absent; no implementation boundary exists yet) | R1 (none/unimplemented, classified) | Rust-born P3 + explicit pre-build design gate/seat ruling + epic rollback | — | OBSERVED-BY-RULING |
| D31 | C17 | C17 | C17 | C17 | C17 (Bash `ae`) | P2 + epic defaults | — | OBSERVED |
| D25 | C20 + C24-C29 | C20 + C24-C29 | C20 + C24-C29 | C20 + C24-C29 | C20 (Bash) + C24-C29 (aewatch) | Rust P4 fate named; flip gate blocked by B1-B3 addenda | N: flip gate | PARTIAL |
| D26a | C20 | C20 | C20 | C20 | C20 (Bash `ae`) | P4 with D25 + epic defaults | — | OBSERVED |
| D26b | C20 | C20 | C20 | C20 | C20 (Bash `ae`) | P4 + epic defaults | — | OBSERVED |
| D27 | C21 + C26 + C27 | C21 + C26 + C27 | C21 + C26 + C27 | C21 + C26 + C27 | C21 (Bash) + C26-C27 (aewatch) | Rust P4 fate named; flip gate blocked by B1-B3 addenda | N: flip gate | PARTIAL |
| D28c | — (C21 is start/stop/daemon, not status) | — | — | — | — | P4 + epic defaults | E: effects, writer/call path, locks+order, atomicity, owner | PARTIAL |
| D29a | C22 | C22 | C22 | C22 | C22 (Bash `ae`) | P4 + epic defaults | — | OBSERVED |
| D30a | — (C22 proves runtime scaffold writes, not static source ownership) | — | — | — | — | stays-python-contrib on #79; no flip, rollback N/A | E: effects, writer/call path, locks+order, atomicity, owner | PARTIAL |
| D30b | — (aemonitor explicitly excluded from these censuses) | — | — | — | — | stays contrib indefinitely, epic fate; no flip, rollback N/A | E: effects, writer/call path, locks+order, atomicity, owner | PARTIAL |
| D30c | C24-C29 | C24-C29 | C24-C29 | C24-C29 | C24-C29 (Python `aewatch`) | split fates named; final row blocked by B1-B3 addenda | N: flip gate | PARTIAL |

D14 is an umbrella whose closure rides D14a/D14b; pending an explicit seats ruling at
the marks pass, its record-level field map remains `PARTIAL` even though D14a is
evidence-complete and D14b identifies its refresh launch-artifact gap.

### D05 split map

The census disproves the one-group premise.  `withdraw` is the event-only path
`ae:5958-5969`; `reply` performs an unlocked `ae_find_request` lookup; and `ask`/`review`
traverse target lock → body artifact → event append.  Those are not one shared lock/store
protocol with one atomic cutover.

| Provisional split | Census pointer | Fields that split would prove |
|---|---|---|
| D05a — ask/review | C2 | E all five: effects, writer/call path, locks+order, atomicity, owner; N all three via P2 + epic defaults |
| D05b — reply | C2 | E all five: unlocked lookup plus send/body/event path; N all three via P2 + epic defaults |
| D05c — withdraw | C19 | E all five: event-only effects/path, lifecycle→event lock order, append residue, Bash owner; N all three via P2 + epic defaults |

The split fields are evidence-complete individually; the parent D05 remains
`GROUPING-FAIL` until a new census proves one shared store, one lock protocol, and one
atomic cutover across all four verbs.

## Machine-countable summary

```text
RECORDS_AUDITED=28
CALIBRATION_RECORDS=1
CALIBRATION_D17=OBSERVED-EQUIVALENT
OBSERVED=13
OBSERVED-BY-RULING=1
PARTIAL=13
GROUPING-FAIL=1
NORMATIVE_MISSING_FIELD_OCCURRENCES=6
NORMATIVE_MISSING.fate=1
NORMATIVE_MISSING.flip_gate=4
NORMATIVE_MISSING.rollback=1
EMPIRICAL_MISSING_FIELD_OCCURRENCES=49
EMPIRICAL_MISSING.effects=10
EMPIRICAL_MISSING.writer_call_path=10
EMPIRICAL_MISSING.locks_order=10
EMPIRICAL_MISSING.atomicity=10
EMPIRICAL_MISSING.current_owner=9
SKIPPED_ALREADY_OBSERVED=D08,D09,D10,D11,D15,D16,D17,D19a,D20,D21,D22,D28a,D28b,D29b
SKIPPED_NONCRITICAL=none (all D records not listed above were either audited or already-observed)
```

The empirical count excludes D05 because its parent is a grouping failure, not a field
omission, and excludes D24 because R1 is a standing ruled absence.  D14b has four
missing E fields (the refresh launch-artifact half); the other partial rows with
empirical gaps have five.  The normative count is six: D14 lacks a record-level N lane,
while D25, D27, and D30c retain unresolved B1-B3 flip gates.
