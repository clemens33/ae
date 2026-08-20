# Open-issue migration dispositions — DRAFT (lead proposal, not ratified)

Pre-ratification gate of #81: every open bash-era issue carries a disposition, recorded on
#79 at ratification. Quadrant (ruling ae-20260820T075423Z-7c2ce445): `rust-requirement` |
`migration-enabler(owner, phase-needed, gate, gate-impact)` | `wontfix-by-policy` |
`stays-python-contrib`. This file is the lead's proposal for joint classification —
**colead stress-test pending; nothing here is final.**

`RR(Pn)` = rust-requirement landing at phase n; `+B3` = also a bucket-3 contract row
(known defect with intended Rust behavior); `byC` = fixed by construction in Rust.

Schema note: `owner` (role lane) and `protected gate` fields for migration-enabler rows
are FILLED below (gate ruling 2026-08-20): integrity rows protect the first P1 811-suite
invocation; cost-only rows carry `protected gate: none (cost)`.

## rust-requirement

| # | Disposition | Note |
|---|---|---|
| 1 | RR(P4) | bridge design input for the daemon fold-in |
| 5 | RR(P1) | list perf; batching is free in one process |
| 6 | RR(P2) byC | escape-copy duplication dies with generated helpers |
| 9 | RR(P3) | doctor verify-session against typed state |
| 11 | RR(P2) | request lifecycle states, lands in request domain |
| 12 | RR(P1) | agents --json, read side |
| 13 | RR(P1) | list early warnings, display only |
| 14 | RR(P1) | per-agent task line (needs D07 to persist reason) |
| 15 | RR(post-core) | checkpoint/handoff helper |
| 16 | RR(P3) | worktree lifecycle hooks |
| 17 | RR(post-core) | runtime namespace per session+agent |
| 18 | RR(P2) | done→review-ready, state domain |
| 19 | RR(P2) | ae explain, needs typed state authority trace |
| 20 | RR(P1) | ae find, read side |
| 21 | RR(P2) | epic-named: schema_version at first format evolution |
| 22 | RR(post-core) | CLI drift canaries; adapter test lane |
| 23 | RR(P1) | replay harness merges into parity-harness design |
| 40 | RR(P2) +B3 | positive arrival evidence; epic-folded |
| 45 | RR(P4) +B3 | requirement ports: nudges go through send protocol; aewatch itself retires (B3 per colead, accepted) |
| 46 | RR(P3) +B3 | manifest must use recorded agent_bin |
| 49 | RR(P2) +B3 | epic-folded; ask-vs-done join |
| 50 | RR(P3) +B3 | resume preflights recorded agent_bin |
| 51 | RR(P4) | origin evidence for quiet stabilization |
| 26 | RR(P2/send) +B3 | fix-known-defect(#26): measured locale defect with clear intended Rust behavior — DR-candidate label removed (dispositions gate) |
| 54 | RR(post-core) | fleet message board |
| 61 | RR(P1) +B3 | pre-dispatch bootstrap = mechanism M2 in ownership.md; reads never write |
| 82 | RR(P2) +DR-004 | durable inbox + coalesced notification — DR-004 RATIFIED (contract S3 SC-200; implementation at P2) |
| 83 | RR(P4) +B3 | fix-known-defect: explicit-start single-sender (SC-963) |
| 84 | RR(P4) +B3 | fix-known-defect: proven-complete takeover; DR-002 implementation (SC-964) |
| 85 | RR(P4) +B3 | fix-known-defect: exact tmux identity before destruction (SC-965) |
| 86 | RR(P4) +B3 | durable-clear fix (SC-966) + DR-003 outbound policy + DR-002 single writer |
| 87 | RR(P4) +B3 | one effective-config authority + atomic setup publication (SC-967/969) |
| 88 | RR(P4) +B3 | daemon control/state committed-truth findings A/B/C/G (SC-926/927/928/968) |
| 89 | RR byC | SC2329 gives ae no dead-top-level-function coverage (scope check by design, sourceable script) — rustc dead_code closes the gap structurally per domain cutover, complete by P5; bash-side spend bounded per the issue's own framing |
| 90 | RR | doctor-owned dead-socket sweep (the un-closed human half of #70; suite half done) — rust doctor at its phase; report-first/--fix opt-in, ae-created sockets only, answer+liveness rule per the issue |
| 94 | RR(P2/send) +B3 | fix-known-defect: canonical tool kind transported into every delivery call, process title = liveness only (SC-705/706); found live by B0 Design 8 |
| 96 | RR(P1) +B3 | fix-known-defect: cross-dimension list filter intersection is literal — frozen ae force-resets scope to running after selector parse, silently overriding --stopped (SC-521a); found live by Batch C A2 |
| 55 | RR(P4) | watchdog fairness test contract |
| 56 | RR(P2/adapter) +B3 | opencode capture disambiguation |
| 57 | RR(P5) +B3 | installer flip defines installed-artifact contract (B3 per colead, accepted) |
| 60 | RR(P2) byC | triple-listing tax dies with generated helpers |
| 62 | RR(P3) +B3 | compact bases on git_final_commit |
| 63 | RR(P2) +B3 | epic-folded; stale-terminal-event sensor |
| 65 | RR(P3) +B3 | end -f double-confirmation |
| 66 | RR(P2) +B3 (≥2 rows) | epic-folded; tracked-request protocol |
| 71 | RR(P3) +B3 | claims — first Rust-native feature; invariants re-ratified (B3 per colead, accepted) |
| 72 | RR(P3) +B3 | compact child inherits goal |
| 73 | RR(P3) +B3 | internal panes excluded from roster |
| 74 | RR(P3) +B3 | transfer under lifecycle serialization |
| 75 | RR(P2) +B3 | epic-folded; no hard flock dependency |
| 76 | RR(P2) byC | helper drift class dies; epic-folded |
| 77 | RR(P3) +B3 | compact roster must satisfy launch grammar |
| 78 | RR byC | empty-key class unrepresentable; epic-folded; closure by construction per-domain as each cuts over, complete by P5 |

## migration-enabler (owners = role lanes below; gate = 811 integration suite unless noted)

Owner lanes (gate ruling): `gate-tooling`, `integration-harness`, `CI/toolchain`,
`test-infra` — spawned as worker lanes at P1 entry. Protected gate for every
gate-integrity row: the FIRST P1 811-suite invocation; integrity rows resolve first.

| # | Owner lane | Phase needed | Gate-impact | Note |
|---|---|---|---|---|
| 10 | CI/toolchain | opportunistic (target P1 entry; hard deadline P5 flip) | gate-cost (protected gate: none — cost) | recast (gate): the P0-close label was already missed — #80's Rust-native Linux proof landed without the issue's bash just-check+integration acceptance; remaining scope is the bash CI half |
| 37 | integration-harness | pre-P1 gate | gate-integrity | SIGPIPE flake, undiagnosed |
| 41 | integration-harness | pre-P1 gate | gate-integrity | SPAWN INCOMPLETE seed swallows status |
| 58 | gate-tooling | pre-P1 gate | gate-integrity | always-red just check is not a gate |
| 64 | test-infra | none (cost-only) | gate-cost (protected gate: none — cost) | name filter; inner-loop tax |
| 67 | gate-tooling | pre-P1 gate | gate-integrity | shellcheck wedge on agent-run gates |
| 68 | integration-harness | pre-P1 gate | gate-integrity | same class as #37, 30 sites |
| 69 | integration-harness | pre-P1 gate | gate-integrity | dummy agent executes pastes — suite may measure its own echo; may also spawn a B3 row |
| 70 | integration-harness | pre-P1 gate | gate-integrity | socket-dir hermeticity |
| 92 | integration-harness | pre-P1 gate | gate-integrity | suite non-determinism + unrecoverable-by-design failure labels (end-only ERRORS flush); #37-family, explicitly not #37; streaming-FAIL remedy proposed in-issue |

## wontfix-by-policy (bash frozen / superseded)

| # | Note |
|---|---|
| 2 | aedev-close tracking, superseded by #79 handoff; close with pointer |
| 3 | shellcheck-closure lint tier — bash-era tooling, class dies with the core |
| 25 | tmux facelift — feature work in frozen bash; revisit post-P5 |
| 42 | superseded — #58's acceptance delivers it (colead correction, accepted); class dies in Rust regardless |
| 52 | ruled (both seats): steward NAME retained through P5, CLOSED — any later rename is a new post-rewrite decision; the P4-revisit clause is removed as internally contradictory |

## administrative (tracking, not bash behavior)

| # | Note |
|---|---|
| 79 | the epic itself — tracking; no disposition |
| 81 | the P0 gate deliverable — closes on ratification |

## stays-python-contrib

(none currently open — aemonitor analytics has no open issue; aewatch issues port as
requirements and the sidecar retires at P4)

## Notes for joint pass

- Colead's round-3 corrections (#26, #42, #45, #52, #57, #61, #71) are applied above;
  the joint pass confirms them.
- #10's original P0-close phase was missed and honestly recast (see its row) — the
  rust CI half landed under #80; the bash half is opportunistic.
- Owners for enabler fixes: propose spawning builders per item post-P0; gate-integrity
  items must land before the first P1 gate invocation (ruling 7c2ce445).
