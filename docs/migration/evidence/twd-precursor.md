# T-WD producer precursor — subdesign v1 (seat gate PENDING)

Batch C prerequisite 2 (batch-c-design.md): the ONLY legitimate source for G2's
attention-event bytes (dead/stale/throttled) is the REAL frozen watchdog producing
them against controlled panes — SC-980's incumbent-capture task. Narrow scope:
harvest alert-family event bytes with provenance; no row classification happens
here. Sole-writer draft by fable5:lead; per the standing rules this subdesign is
seat-approved before any run.

Value-blindness: arms are named by MANIPULATION, never by the reason string they
are expected to produce; which action/summary bytes appear is the capture. The
manipulation→reason mapping lives in the seat annex only.

## Producer and sandbox

- Isolated AE_HOME/HOME sandbox; a real `ae` launch at 72c7293 whose config's
  agent commands are FIXTURE SHELLS (plain bash), never live models; dedicated
  tmux server per the harness marker discipline.
- The REAL generated watchdog from that launch is the producer. No instrumentation
  patch is needed: pacing rides the documented `AE_WATCHDOG_*` environment knobs
  (`AE_WATCHDOG_INTERVAL_SEC`, `AE_WATCHDOG_STALE_MIN`, `AE_WATCHDOG_MAX_NUDGES`,
  `AE_WATCHDOG_THROTTLE_ALERT_CYCLES`, sweep knobs as needed), with every value
  used recorded in the run manifest. No clock shim: thresholds are shortened, not
  time faked.
- Global artifact contract applies (b0-design.md): frozen SHA, environment/tool
  hashes, per-arm fresh sandbox, recursive manifests, tmux snapshots, bounded
  observation windows (a fixed number of watchdog cycles) whose expiry is recorded
  INCONCLUSIVE — never interpreted by the worker.

## Arms (each a fresh sandbox; capture = events.jsonl bytes + watchdog log + pane
snapshots + knob values, before/after manifests)

1. **Process-killed pane** — controller kills the fixture agent's process so the
   pane drops to (or loses) its shell; observe N cycles.
2. **Idle pane past threshold** — fixture agent emits one initial event, then
   nothing; `AE_WATCHDOG_STALE_MIN` shortened; observe N cycles.
3. **Nudge-cap consumption** — idle pane as in arm 2, `AE_WATCHDOG_MAX_NUDGES`
   set low; observe enough cycles for the cap to be reached and passed.
4. **Recovery after arm-3 state** — from arm 3's end state, the fixture pane
   emits fresh activity (a real helper `state working` invocation); observe N
   further cycles. (Captures any clearing bytes the frozen producer emits —
   `throttle-cleared`/`alert-cleared` are documented actions; whether and when
   they appear is the capture.)

Each arm also records WHICH watchdog code path ran (the generated watchdog's own
log lines), giving every harvested byte its provenance: producing binary, arm,
cycle, knob values.

## Deliverable

A provenance-annotated byte archive: per arm, the exact events.jsonl lines the
watchdog appended (byte ranges + hashes), ready for G2 template-group harvesting
under the producer-derivation rule. The worker never edits or normalizes a byte;
G2's template build consumes these files as-is with any further mutation being a
named Batch C byte diff.

## Ownership and sequencing

Proposed: executed by the Batch C worker as its fixture-harvest step ZERO (it
feeds G2's templates and C owns fixture building), after this subdesign's seat
gate and before any Batch C template group is built. Independent of the four B0
designs; can run in parallel with the B0 worker's SC-507b/511c/1208 execution.

---

## SEAT ANNEX — never included in the worker brief

Expected manipulation→reason mapping for classification: arm 1 → `alert` with a
dead-class summary; arm 2 → `alert` stale-class; arm 3 → `throttled`; arm 4 →
`throttle-cleared`/`alert-cleared` if the frozen producer emits clears on
recovery (its absence is itself an observation for SC-980's incumbent baseline —
seats classify, the worker does not). Alert-consumer semantics under test later:
`_agents_alert_reasons` at ae:3416-3565 (summary substring classes noted at
ae:3522-3526 — dead must not downgrade to throttled on a stray substring match).
