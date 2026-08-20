# Batch C design — read-side fixture cluster (59 assignments)

Lead draft for both-seat approval; NO evidence worker runs before that approval.
Rules inherited and binding: value-blind arms (manipulation/barriers/captures only —
expected values omitted, seats classify); deterministic (no timing races, no
live-model queries); every arm fails independently; captures are CANDIDATE
observations, never a builder oracle, until seat acceptance; bucket-3/4 rows capture
the INCUMBENT baseline only, never a successor SHOULD; provenance on every artifact
(frozen commit, environment, command, rc).

## Fixture topology — nine session directories, one AE_HOME sandbox

Built ONCE per run by the probe script into an isolated `AE_HOME`; every fixture dir
is thereafter IMMUTABLE for the run (a read-only chmod after build is the barrier).
The suite operates on SNAPSHOTS, never live sessions (#81 parity rule).

| Fixture | Purpose (rows it feeds) |
|---|---|
| FX1 healthy-multi | 2 agents, full meta (mode/origin/work_dir/goal), events with all six attention reasons ABSENT — baseline for schema/field rows |
| FX2 attention-ladder | 6 sessions in one home, one per attention reason at documented severity — rollup/severity rows |
| FX3 degraded-meta | unreadable meta (mode 000) + separately a malformed-complete event line — degradation rows |
| FX4 quiet-fresh | session dir with NO events.jsonl; sibling with zero-byte events.jsonl — absent/empty rows |
| FX5 request-pairs | ask/reply chains: correct mirror pair; same-ref wrong-target reply; mixed identity (one routed, one display); threshold-boundary ages (exactly at, strictly past) |
| FX6 stopped-set | stopped sessions incl. one with attention-shaped history — filter rows |
| FX7 unknown-extras | meta with unknown keys; events with unknown keys and unknown action — tolerance rows |
| FX8 dr001-tail | events.jsonl WITHOUT trailing newline + a partial trailing record — reader boundary |
| FX9 goal-history | multiple goal events over time — goal_set_epoch derivation |

**Producer-bytes rule (colead B2, binding):** every `events.jsonl` byte in FX1–FX9 is
EXTRACTED from real frozen producers, never hand-authored: a builder subrun invokes the
generated helpers (`state`, `goal`, `memo`, `say`, `ask`, `review`, `reply`) of a
scratch session at 72c7293 and the probe HARVESTS those emitted lines into the
fixtures, recording per-line provenance (producing helper + command). Meta bytes are
harvested from a real `ae` launch of the scratch session (then edited ONLY by the
documented mutation of the arm, e.g. chmod for FX3 — each mutation is the arm's named
manipulation). The extraction script and its provenance manifest ship with the
artifacts.

## Reader invocations

Two readers per arm where applicable, captured separately with identical inputs:
1. **Frozen bash** `ae list --json` / `status` / `next` etc. at 72c7293 (the IS lane —
   dispatcher invoked with the sandbox AE_HOME; the M2 bootstrap effect is part of the
   capture, per SC-1202's known conflict).
2. **Rust slice binary** where the seam exists (candidate validation only — its output
   is NEVER the expected value; divergence between the two lanes is a REPORT for the
   seats, resolved row-by-row as bash-IS vs contract-SHOULD vs builder-defect).

## Arms (59, grouped; each independently failable; captures = stdout+stderr+rc+
fixture-dir listing before/after — before/after diff proves read-only rows)

D01/D02/D03/D04a/D04b + SC-1306a-e (concurrency cuts) use the four B0-approved
concurrency designs (named mutation barriers, before/after input fingerprints, tmux
snapshots, repeated assertions) — those designs gate separately and this batch
consumes them as approved.

- **A1 schema/document** (SC-509, 509b, 506, 510a-d, 511a-c): FX1 baseline capture;
  FX3 for 509b/506 (degraded entry present, document closes, identity survives); FX7
  for 511b tolerance; FX8 for 510-shape at the boundary. 510c's state-in-ref via FX1's
  harvested state events.
- **A2 filters** (SC-017a-i, 521): FX1+FX2+FX6 in one home; one invocation per flag
  and per documented alias; the two intersection invocations (stopped+needs-attn,
  stopped+active); ls alias invocation.
- **A3 rollup/severity** (SC-017g, 017h, 524): FX2 captures per severity; one
  future-timestamped event fixture (harvested line, timestamp mutation named) for 524.
- **A4 status/next** (SC-016a-d, 513a-c, 019, 020a-c): status named/default (inside
  via a scripted tmux client in the sandbox server), never-attaches proven by tmux
  client-list before/after; next with/without attention; --attach inside vs outside
  tmux; gone-session re-check (fixture removed between resolve and attach via the
  B0 barrier design); unknown-arg and no-attention rc captures; jump alias.
- **A5 exits** (SC-514): doctor run against the sandbox with one planted FAIL
  (missing dependency on PATH) and one clean run.
- **A6 requests/pairing** (SC-518, 522, 523a-b, SC-212c signature): FX5 captures via
  the requests helper and list unanswered derivation; threshold arms at equality vs
  past; defaults confirmed against unset env.
- **A7 meta grammar** (SC-405a-g): FX1 for the ratified keys; FX7 for 405d unknown
  keys (probe capture, UNCLASSIFIED row - observation only); a malformed/duplicate-key
  fixture for 405e (same status); FX9 for 405f; 405g's two subarms — a running scratch
  session with @ae_branch_name set (tmux-primary) and a stopped fixture with only git
  (fallback).
- **A8 modes** (SC-100, 101, 102a-b, 018, 018b): default-name derivation from a
  hostile-adjacent cwd (grammar-guarantee capture); fast-path attach capture with
  before/after state diff (101's mutation-exclusion IS); resume-of-stopped capture
  (102a regeneration set diff); inside-session invocation capture (102b); use-arm
  captures (018 documented start; 018b against-existing decision surface).
- **A9 quiet-vs-degraded** (SC-519, 520): FX4 absent + empty (quiet); FX3 malformed
  complete line (degraded + skip observability capture incl. the public JSON marker).

## Ordering & environment

Fixed order A1→A9 (any inter-arm dependence is a defect — the read-only before/after
diffs prove isolation); single-threaded; `TZ=UTC`, fixed `LANG`; sandbox AE_HOME under
mktemp with the suite-marker discipline from the harness slice; frozen-commit checkout
verified by hash before any run; every artifact under
`docs/migration/evidence/batch-c-artifacts/` with the provenance manifest.

## Deliverables

Per arm: captured stdout/stderr/rc + fixture provenance + before/after diffs, one
manifest file mapping arm → row ids → artifact paths. No verdicts — the seats classify
each capture against its row (contradictions become bucket-3/DR reopenings; measurement
never rewrites SHOULD).
