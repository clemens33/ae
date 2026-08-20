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
  agent command is a CONTROLLABLE FAKE AGENT executable, never a live model and
  never a plain shell (gate correction: with agent_bin=bash the pane's own shell
  satisfies the dead-check's descendant search at ae:16298-16318, and a shell
  pane makes real sends hit shell-refusal or echo nudges back as fresh pane
  activity). The fake: a distinct non-shell foreground identity (its own comm/
  `pane_current_command` name), a stable no-echo pane (fixed prompt, reads stdin
  without echoing), accepts real `send` deliveries, and prints controller-driven
  lines on command. Dedicated tmux server per the harness marker discipline.
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

## Arms (three independent fresh sandboxes; capture = events.jsonl bytes +
watchdog log + pane snapshots + send rc + knob values, before/after manifests)

1. **Child-killed pane** — controller kills ONLY the fake agent child, so the
   original pane shell returns as the foreground; observe N cycles. (The
   dead-check resolves the recorded agent_bin and searches for it under the
   pane PID — ae:15895+ `_load_agent_bins`, ae:16298-16318 descendant walk.)
2. **Live-but-static pane past threshold** — the fake stays alive and its pane
   static; no further events; `AE_WATCHDOG_STALE_MIN` and
   `AE_WATCHDOG_MAX_NUDGES` shortened; observe enough cycles for the nudge cap
   to be reached and passed. Record every real-send rc and the nudge bytes the
   pane received.
3. **Pane-phrase two-phase arm (single sandbox, single running watchdog — the
   throttle streak is PROCESS MEMORY in the running watchdog and cannot survive
   a fresh clone; recovery is therefore a named SUBARM here, not a fourth
   sandbox):**
   - *Phase A*: with the fake process LIVE, the controller has it print one
     documented GENERIC phrase (`429 Too Many Requests` — the generic catalog
     applies to every agent_bin, matched over the captured last ~15 pane lines:
     ae:15842-15889) into its pane tail; `AE_WATCHDOG_THROTTLE_ALERT_CYCLES`
     lowered; cross successive cycle barriers, capturing after each.
   - *Phase B*: the controller has the fake print enough nonmatching lines to
     displace the phrase from the captured tail, POSITIVELY captures that pane
     state, then crosses the next cycle(s), capturing whatever bytes the
     producer emits. (Detection and clearing are pane-content facts — an event
     such as `state working` cannot remove pane bytes; clear emission at
     ae:16383-16396.)

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
dead-class summary (agent binary no longer under the pane PID); arm 2 → nudge
sequence then `alert` stale/max-nudges class; arm 3 phase A → `throttled` on the
first matching cycle, then `alert` "throttled for Ns" when the streak reaches
THROTTLE_ALERT_CYCLES; arm 3 phase B → `throttle-cleared` once the captured tail
stops matching (whether/when it appears is the capture — its absence is itself
an observation for SC-980's incumbent baseline; seats classify, the worker does
not). Alert-consumer semantics under test later: `_agents_alert_reasons` at
ae:3416-3565 (summary substring classes noted at ae:3522-3526 — dead must not
downgrade to throttled on a stray substring match).
