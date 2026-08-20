# Pin audit — evidence adequacy

- contract-head: `fdef6d11dc2cdee78433f79dbde83cd9f344157f`
- ownership/semantic source: `HEAD=e70bc24`
- closure-map source: `HEAD=e70bc24`
- frozen-baseline: `72c729343a0117af2968b66e1c43f89ad25fc0b2`
- input: all `398` SC + `42` D stable IDs (`440` total), from `/opt/homebrew/bin/bash docs/migration/evidence/sweep-check.sh`
- sweep-check observed: `rc=1` on the known outstanding closure fields, while its canonical-set summary reported `SC_ROWS=398`, `D_RECORDS=42`, `CLOSURE_ORPHANS=0`, `CLOSURE_MISSING=0`, `SET_EXTRA=0`, `SET_MISSING=0`, and `GRAIN_VIOLATIONS=0`.
- evidence policy: TEST pins were read from `git show 72c7293:<path>`; CENSUS/CODE pins were checked at their artifact commit and every cited `ae:<line>` was rechecked against frozen `72c7293:ae`.

Calibration was performed first and passed the required shape check: SC-510a FALSE, SC-510c FALSE, SC-511a PARTIAL, SC-513c FALSE, SC-514 FALSE, SC-1209 PARTIAL. Bulk verdicts below are trusted only after that calibration. No replacement probe designs are authored here; those remain seat-gated.

## Machine-countable summary

```
TOTAL=440
PROVES=24
PARTIAL=24
FALSE=54
MISSING-OR-STALE=338
ROW-GRAIN-ERROR=0
TEST=81
CENSUS/CODE=21
PROBE_PENDING=338
```

The nonzero-shaped result is intentional: `MISSING-OR-STALE=338` is the mechanically required routing result for placeholder PROBE routes, not a claim that 338 probe designs were attempted or lost.

## Calibration records

- **SC-510a — FALSE.** TEST `tests/unit:1764` is the events infrastructure/source-and-escape block; no assertion requires ISO-8601 UTC second precision plus actor plus action.
- **SC-510c — FALSE.** TEST `tests/integration:1268` churns display names and routes a reply by slot; it does not exercise the action-table ref polysemy.
- **SC-511a — PARTIAL.** TEST `tests/integration:1268` asserts target_slot in one ask and slot-routed reply; it omits actor_slot/actor_session/target_session, four actions, and omission when empty.
- **SC-513c — FALSE.** TEST `tests/integration:726` extracts a backgrounded delivery branch and checks attach-word absence indirectly; it does not observe tmux focus. The closer guard is `tests/unit:2134-2139`.
- **SC-514 — FALSE.** TEST `tests/integration:532` is an orphan teardown comment whose nearby doctor case expects a rc0 WARN summary; it does not inject a FAIL checklist item and assert rc!=0.
- **SC-1209 — PARTIAL.** TEST `tests/integration:2652` checks unverified marking/envelope ordering only; it does not establish the full outermost-envelope authority rule covered by unit #39 at 9820ff/9930ff.

## Deep audit — TEST and CENSUS/CODE

### SC-500 — PARTIAL

Claim (exact):
```
**SC-500 — compact stdout byte format.** Bucket 2 — `Archived`, `Archive:`, `Digest:`,
`Recovery:`: four lines, that order, nothing else, and EMPTY unless the boundary was
crossed. Authority: architecture.md + commands.md:673-676. Empirical: pending.
Conflict: none.
```

Evidence (TEST schema): `SC-500 | TEST | assertion=git show 72c7293:tests/integration | line=5379 | label=assert_eq "compact-out: line 1 names the archive id" "1" \`
Frozen assertion body (exact pinned line, tests/integration:5379): `assert_eq "compact-out: line 1 names the archive id" "1" \`

Ability-to-fail / coverage: line 5379 only checks line 1; deleting/mangling lines 2–4 or the empty-boundary rule stays green.

Verdict: **PARTIAL**

### SC-501 — FALSE

Claim (exact):
```
**SC-501 — compact stderr carries everything else.** Bucket 2 — frozen facts,
confirmation + question, end's progress, handover chatter, `Aborted.`, the relaunch
announcement, and a SECOND copy of the `Recovery:` line (a broken stdout cannot destroy
the only route back). Authority: commands.md:678-683. Empirical: pending. Conflict: none.
```

Evidence (TEST schema): `SC-501 | TEST | assertion=git show 72c7293:tests/integration | line=5379 | label=assert_eq "compact-out: line 1 names the archive id" "1" \`
Frozen assertion body (exact pinned line, tests/integration:5379): `assert_eq "compact-out: line 1 names the archive id" "1" \`

Ability-to-fail / coverage: the same line asserts the archive-id line on stdout, not the stderr-only chatter contract; any stderr loss stays green.

Verdict: **FALSE**

### SC-502 — FALSE

Claim (exact):
```
**SC-502 — `Recovery:` prints BEFORE the relaunch.** Bucket 1 — past the relaunch the
archive is published, the source is gone, and the process may exec and never return.
Authority: architecture.md. Empirical: pending. Conflict: none.
```

Evidence (TEST schema): `SC-502 | TEST | assertion=git show 72c7293:tests/integration | line=5379 | label=assert_eq "compact-out: line 1 names the archive id" "1" \`
Frozen assertion body (exact pinned line, tests/integration:5379): `assert_eq "compact-out: line 1 names the archive id" "1" \`

Ability-to-fail / coverage: the same line checks only archive-id text; moving Recovery after exec or deleting it stays green.

Verdict: **FALSE**

### SC-503a — FALSE

Claim (exact):
```
**SC-503a — a typed `n` is an answer.** Bucket 1 — prints `Aborted.` and exits **0**.
Authority: commands.md:692-697. Empirical: pending. Conflict: none.
```

Evidence (TEST schema): `SC-503a | TEST | assertion=git show 72c7293:tests/integration | line=1639 | label={"ts":"2026-08-01T00:03:00Z","actor":"ae:compact:op1","action":"ask","ref":"canary-withdrawn","target":"a:one","summary":"handover please"}`
Frozen assertion body (exact pinned line, tests/integration:1639): `{"ts":"2026-08-01T00:03:00Z","actor":"ae:compact:op1","action":"ask","ref":"canary-withdrawn","target":"a:one","summary":"handover please"}`

Ability-to-fail / coverage: line 1639 is a fixture JSON record, not a confirmation path; changing typed-n handling stays green.

Verdict: **FALSE**

### SC-503b — FALSE

Claim (exact):
```
**SC-503b — end-of-input is not an answer.** Bucket 1 — with no stdin, compact reports
it could not obtain confirmation and exits **non-zero**; stdout is empty in both cases,
so exit status is the caller's only way to tell "operator said no" from "the question
never reached anyone". Authority: commands.md:692-697. Empirical: pending. Conflict: none.
```

Evidence (TEST schema): `SC-503b | TEST | assertion=git show 72c7293:tests/integration | line=1639 | label={"ts":"2026-08-01T00:03:00Z","actor":"ae:compact:op1","action":"ask","ref":"canary-withdrawn","target":"a:one","summary":"handover please"}`
Frozen assertion body (exact pinned line, tests/integration:1639): `{"ts":"2026-08-01T00:03:00Z","actor":"ae:compact:op1","action":"ask","ref":"canary-withdrawn","target":"a:one","summary":"handover please"}`

Ability-to-fail / coverage: line 1639 is a fixture record, not EOF handling; accepting EOF as no is untested.

Verdict: **FALSE**

### SC-504a — PROVES

Claim (exact):
```
**SC-504a — a reporting failure never suppresses the relaunch.** Bucket 1 — a consumer
that exits early (closed/broken stdout) must not kill the operation between archive and
launch. Authority: commands.md:685-686 + architecture.md. Empirical: pending.
Conflict: none.
```

Evidence (TEST schema): `SC-504a | TEST | assertion=git show 72c7293:tests/integration | line=5071 | label=assert_eq "compact-pipe: the archive was published" "1" \`
Frozen assertion body (exact pinned line, tests/integration:5071): `assert_eq "compact-pipe: the archive was published" "1" \`

Ability-to-fail / coverage: the pipe-consumer failure fixture is executed; archive publication, child start, and stderr recovery are asserted, so removing relaunch or stderr recovery fails.

Verdict: **PROVES**

### SC-504b — PARTIAL

Claim (exact):
```
**SC-504b — no altered SIGPIPE disposition leaks into the child.** Bucket 1 — semantic
SHOULD, narrowed per fold guard: the child sees normal/unmodified SIGPIPE behavior; the
authority guarantees restoration of SIGPIPE specifically, not that every disposition is
default. The parent's mechanism (ignore/restore) is implementation, not contract.
Authority: architecture.md. Empirical: pending. Conflict: none.
```

Evidence (TEST schema): `SC-504b | TEST | assertion=git show 72c7293:tests/unit | line=13109 | label=assert_eq "compact-boundary: SIGPIPE is ignored across the report and RESTORED before the exec" "1" \`
Frozen assertion body (exact pinned line, tests/unit:13109): `assert_eq "compact-boundary: SIGPIPE is ignored across the report and RESTORED before the exec" "1" \`

Ability-to-fail / coverage: the assertion is source-order only (trap/restore/exec); a child-level SIGPIPE behavior change can stay green.

Verdict: **PARTIAL**

### SC-507c — FALSE

Claim (exact):
```
**SC-507c — `archive preview` diagnostics go to stderr.** Bucket 2 — canonical archive
id, source session, file counts and bytes. Authority: commands.md:554-556.
Empirical: pending. Conflict: none.
```

Evidence (TEST schema): `SC-507c | TEST | assertion=git show 72c7293:tests/integration | line=4152 | label=assert_contains "archive-preview: stdout is the digest" "# ae session archive" "$a48_prev_out"`
Frozen assertion body (exact pinned line, tests/integration:4152): `assert_contains "archive-preview: stdout is the digest" "# ae session archive" "$a48_prev_out"`

Ability-to-fail / coverage: line 4152 asserts digest stdout; removing or relocating stderr diagnostics stays green.

Verdict: **FALSE**

### SC-507d — FALSE

Claim (exact):
```
**SC-507d — `archive preview` is read-only by construction.** Bucket 2 — writes
nothing, emits no event, creates no archive, never enters the lifecycle. Authority:
commands.md:546-548. Empirical: pending. Conflict: none.
```

Evidence (TEST schema): `SC-507d | TEST | assertion=git show 72c7293:tests/integration | line=4152 | label=assert_contains "archive-preview: stdout is the digest" "# ae session archive" "$a48_prev_out"`
Frozen assertion body (exact pinned line, tests/integration:4152): `assert_contains "archive-preview: stdout is the digest" "# ae session archive" "$a48_prev_out"`

Ability-to-fail / coverage: line 4152 asserts digest stdout; writes/events/archive creation can change while it stays green.

Verdict: **FALSE**

### SC-509 — FALSE

Claim (exact):
```
**SC-509 — `list --json` versioned object schema.** Bucket 2 — a single JSON object:
`schema_version` (1), `generated_at`, `sessions[]` with the documented session fields
(name/status/mode/origin/work_dir/goal/goal_set_epoch/branch/last_active_epoch/
needs_attention/attention/attention_rank) and `agents[]` fields (ref/alias/name/
session_id/alive/state/reason); `schema_version` lets consumers gate on shape.
Authority: commands.md:97-132. Empirical: pending. Conflict: none.
```

Evidence (TEST schema): `SC-509 | TEST | assertion=git show 72c7293:tests/unit | line=386 | label=_r43_bare() { # -> every arm's marker, or a short list ending where it died`
Frozen assertion body (exact pinned line, tests/unit:386): `_r43_bare() { # -> every arm's marker, or a short list ending where it died`

Ability-to-fail / coverage: the pinned helper exercises launch-rerun command containment, not list --json fields or JSON validity.

Verdict: **FALSE**

### SC-510a — FALSE

Claim (exact):
```
**SC-510a — event required keys.** Bucket 2 — every event carries `ts` (ISO 8601 UTC,
second precision), `actor`, `action`. Authority: events.md:47-60. Empirical: pending.
Conflict: none.
```

Evidence (TEST schema): `SC-510a | TEST | assertion=git show 72c7293:tests/unit | line=1764 | label=# ── events.jsonl infrastructure ─────────────────────────────────────`
Frozen assertion body (exact pinned line, tests/unit:1764): `# ── events.jsonl infrastructure ─────────────────────────────────────`

Ability-to-fail / coverage: calibration: event infrastructure/source and escaping checks do not require ts precision, actor, and action.

Verdict: **FALSE**

### SC-510c — FALSE

Claim (exact):
```
**SC-510c — `ref` polysemy follows the action table.** Bucket 2 — request id for
ask/review/reply, topic for memo, captured session id for recover, absent otherwise.
Authority: events.md:62-68. Empirical: pending. Conflict: none.
```

Evidence (TEST schema): `SC-510c | TEST | assertion=git show 72c7293:tests/integration | line=1268 | label=# ── request-integrity: slot-routed reply survives a display-name CHURN (codex B1) ──`
Frozen assertion body (exact pinned line, tests/integration:1268): `# ── request-integrity: slot-routed reply survives a display-name CHURN (codex B1) ──`

Ability-to-fail / coverage: calibration: the churn fixture tests slot-routed reply, not action-dependent ref meanings.

Verdict: **FALSE**

### SC-510d — FALSE

Claim (exact):
```
**SC-510d — string values are JSON-escaped.** Bucket 2 — the escape set is `\"` `\\`
`\n` `\t` `\r`. Authority: events.md:70. Empirical: pending. Conflict: none.
```

Evidence (TEST schema): `SC-510d | TEST | assertion=git show 72c7293:tests/unit | line=1764 | label=# ── events.jsonl infrastructure ─────────────────────────────────────`
Frozen assertion body (exact pinned line, tests/unit:1764): `# ── events.jsonl infrastructure ─────────────────────────────────────`

Ability-to-fail / coverage: the pinned section header has no escaping assertion body; malformed JSON escaping can pass.

Verdict: **FALSE**

### SC-511a — PARTIAL

Claim (exact):
```
**SC-511a — messaging events carry optional routing-key fields.** Bucket 2 —
`actor_slot`/`actor_session`/`target_slot`/`target_session` on send/ask/review/reply
when known, omitted when empty. Authority: events.md:71-84. Empirical: pending.
Conflict: none.
```

Evidence (TEST schema): `SC-511a | TEST | assertion=git show 72c7293:tests/integration | line=1268 | label=# ── request-integrity: slot-routed reply survives a display-name CHURN (codex B1) ──`
Frozen assertion body (exact pinned line, tests/integration:1268): `# ── request-integrity: slot-routed reply survives a display-name CHURN (codex B1) ──`

Ability-to-fail / coverage: calibration: only target_slot is asserted; actor/target slot+session across four actions and omission are untested.

Verdict: **PARTIAL**

### SC-511b — PARTIAL

Claim (exact):
```
**SC-511b — readers prefer slot+session over display name.** Bucket 2 — pairing and
delivery use the churn-proof routing key where present; unknown keys are ignored.
Authority: events.md:84. Empirical: pending. Conflict: none.
```

Evidence (TEST schema): `SC-511b | TEST | assertion=git show 72c7293:tests/integration | line=1268 | label=# ── request-integrity: slot-routed reply survives a display-name CHURN (codex B1) ──`
Frozen assertion body (exact pinned line, tests/integration:1268): `# ── request-integrity: slot-routed reply survives a display-name CHURN (codex B1) ──`

Ability-to-fail / coverage: the churn round-trip proves slot routing over a renamed display name, but unknown-key tolerance is not exercised.

Verdict: **PARTIAL**

### SC-512 — PARTIAL

Claim (exact):
```
**SC-512 — compact stdout truth claim.** Bucket 2 — non-empty stdout proves exactly:
the archive EXISTS and the printed recovery command WORKS. It deliberately does NOT
claim the fresh session started (the relaunch can still refuse; a stdout line asserting
a launch that then failed would be worse than no line). Authority: commands.md:673-676 +
architecture.md. Empirical: pending. Conflict: none.
```

Evidence (TEST schema): `SC-512 | TEST | assertion=git show 72c7293:tests/integration | line=5379 | label=assert_eq "compact-out: line 1 names the archive id" "1" \`
Frozen assertion body (exact pinned line, tests/integration:5379): `assert_eq "compact-out: line 1 names the archive id" "1" \`

Ability-to-fail / coverage: only the Archived line is pinned; a broken recovery command can leave that assertion green.

Verdict: **PARTIAL**

### SC-513a — FALSE

Claim (exact):
```
**SC-513a — `next` exits non-zero when nothing needs attention.** Bucket 2 — with a
message; composes in scripts. Authority: commands.md:150-152. Empirical: pending.
Conflict: none.
```

Evidence (TEST schema): `SC-513a | TEST | assertion=git show 72c7293:tests/integration | line=726 | label=        infn && /if \[\[ -n "\$prompt" \]\]; then/ { buf = $0 "\n"; cap = 1; want = 0; next }`
Frozen assertion body (exact pinned line, tests/integration:726): `        infn && /if \[\[ -n "\$prompt" \]\]; then/ { buf = $0 "\n"; cap = 1; want = 0; next }`

Ability-to-fail / coverage: the pinned extractor is the fd8 inheritance probe, unrelated to next's no-attention exit.

Verdict: **FALSE**

### SC-513b — FALSE

Claim (exact):
```
**SC-513b — `next` exits non-zero on an unknown argument.** Bucket 2. Authority:
commands.md:158-159. Empirical: pending. Conflict: none.
```

Evidence (TEST schema): `SC-513b | TEST | assertion=git show 72c7293:tests/integration | line=726 | label=        infn && /if \[\[ -n "\$prompt" \]\]; then/ { buf = $0 "\n"; cap = 1; want = 0; next }`
Frozen assertion body (exact pinned line, tests/integration:726): `        infn && /if \[\[ -n "\$prompt" \]\]; then/ { buf = $0 "\n"; cap = 1; want = 0; next }`

Ability-to-fail / coverage: the pinned extractor is the fd8 inheritance probe, unrelated to unknown-argument handling.

Verdict: **FALSE**

### SC-513c — FALSE

Claim (exact):
```
**SC-513c — `next` is read-only by default.** Bucket 2 — no tmux focus change without
`--attach`. Authority: commands.md:150. Empirical: pending. Conflict: none.
```

Evidence (TEST schema): `SC-513c | TEST | assertion=git show 72c7293:tests/integration | line=726 | label=        infn && /if \[\[ -n "\$prompt" \]\]; then/ { buf = $0 "\n"; cap = 1; want = 0; next }`
Frozen assertion body (exact pinned line, tests/integration:726): `        infn && /if \[\[ -n "\$prompt" \]\]; then/ { buf = $0 "\n"; cap = 1; want = 0; next }`

Ability-to-fail / coverage: calibration: this is absence-of-attach-words/extractor plumbing, not a focus-state proof; unit 2134–2139 is the closer guard.

Verdict: **FALSE**

### SC-514 — FALSE

Claim (exact):
```
**SC-514 — `doctor` exit contract.** Bucket 2 — non-zero if any checklist item FAILed.
Authority: commands.md:168. Empirical: pending. Conflict: none.
```

Evidence (TEST schema): `SC-514 | TEST | assertion=git show 72c7293:tests/integration | line=532 | label=# doctor-orphan test far below (teardown can lag under full-suite contention).`
Frozen assertion body (exact pinned line, tests/integration:532): `# doctor-orphan test far below (teardown can lag under full-suite contention).`

Ability-to-fail / coverage: calibration: the pinned orphan comment/rc0 summary has no injected FAIL item and rc!=0 assertion.

Verdict: **FALSE**

### SC-515a — PARTIAL

Claim (exact):
```
**SC-515a — `stop all` folds per-target result records into its exit.** Bucket 2 —
bounded (~30s) wait on the per-session stop-result events. Authority:
commands.md:365-373. Empirical: pending. Conflict: none.
```

Evidence (TEST schema): `SC-515a | TEST | assertion=git show 72c7293:tests/integration | line=3536 | label=assert_eq "outside caller (F7): a target that could not be stopped makes the rc non-zero" "1" \`
Frozen assertion body (exact pinned line, tests/integration:3536): `assert_eq "outside caller (F7): a target that could not be stopped makes the rc non-zero" "1" \`

Ability-to-fail / coverage: only the failure-direction rc is asserted; per-target record folding and bounded wait can regress while it stays green.

Verdict: **PARTIAL**

### SC-515b — FALSE

Claim (exact):
```
**SC-515b — result-wait timeout is not a failure.** Bucket 2 — reports `results
pending` and keeps the handoff status rather than calling a still-working supervisor a
failure. Authority: commands.md:370-372. Empirical: pending. Conflict: none.
```

Evidence (TEST schema): `SC-515b | TEST | assertion=git show 72c7293:tests/unit | line=5089 | label=# F7 — HANDING OFF THE LOOP MUST NOT COST THE CALLER ITS EXIT CODE. F4 put the`
Frozen assertion body (exact pinned line, tests/unit:5089): `# F7 — HANDING OFF THE LOOP MUST NOT COST THE CALLER ITS EXIT CODE. F4 put the`

Ability-to-fail / coverage: the pinned line is a prose section comment; timeout return behavior is not asserted there.

Verdict: **FALSE**

### SC-515c — FALSE

Claim (exact):
```
**SC-515c — an unowned ae-tagged session is named, not stopped.** Bucket 2 — visible on
the server but absent from meta: not killed, run becomes a partial failure (non-zero),
message names both ways out. Authority: commands.md:392-395. Empirical: pending.
Conflict: none.
```

Evidence (TEST schema): `SC-515c | TEST | assertion=git show 72c7293:tests/integration | line=3380 | label=# (e) `ae stop all` FROM INSIDE one of the sessions, at fleet size 3. Rounds 2-6`
Frozen assertion body (exact pinned line, tests/integration:3380): `# (e) \`ae stop all\` FROM INSIDE one of the sessions, at fleet size 3. Rounds 2-6`

Ability-to-fail / coverage: the pinned line is a prose scenario header; unmanaged-session naming/nonzero behavior is not asserted there.

Verdict: **FALSE**

### SC-516 — FALSE

Claim (exact):
```
**SC-516 — `end` fails non-zero when the archive cannot be written.** Bucket 1 —
capture-then-delete: publication happens after verified stop and git, before any live
state is removed; a failed archive fails the end with the whole session still on disk.
Authority: commands.md:499-501 + architecture.md publication protocol.
Empirical: pending (census-2 end section). Conflict: none.
```

Evidence (TEST schema): `SC-516 | TEST | assertion=git show 72c7293:tests/integration | line=4275 | label=assert_eq "archive-claim: a standing claim FAILS the end rather than being cleaned up" "1" \`
Frozen assertion body (exact pinned line, tests/integration:4275): `assert_eq "archive-claim: a standing claim FAILS the end rather than being cleaned up" "1" \`

Ability-to-fail / coverage: the pinned assertion is standing-claim refusal, not archive-write failure with state preservation.

Verdict: **FALSE**

### SC-702 — FALSE

Claim (exact):
```
**SC-702 — a readiness timeout fails loud and durable.** Bucket 1 — the pane text is
preserved next to the session and an event is emitted, because launch delivery runs
detached where stderr reaches nobody. Authority: AGENTS.md readiness ruling.
Empirical: pending. Conflict: none.
```

Evidence (TEST schema): `SC-702 | TEST | assertion=git show 72c7293:tests/unit | line=2880 | label=# (#36 extracted the poll loop into _wait_input_ready so the launch path could share`
Frozen assertion body (exact pinned line, tests/unit:2880): `# (#36 extracted the poll loop into _wait_input_ready so the launch path could share`

Ability-to-fail / coverage: the pinned line is a prose comment; the actual readiness assertion is line 2882, so the map anchor cannot fail on a readiness mutation.

Verdict: **FALSE**

### SC-801 — PARTIAL

Claim (exact):
```
**SC-801 — staging is private by construction.** Bucket 1 — payload populated under
umask 077 with every mode set explicitly. Authority: architecture.md:89.
Empirical: pending. Conflict: none.
```

Evidence (TEST schema): `SC-801 | TEST | assertion=git show 72c7293:tests/unit | line=11574 | label=assert_eq "archive-publish: the final path is the canonical UUID under \$AE_HOME/archive" \`
Frozen assertion body (exact pinned line, tests/unit:11574): `assert_eq "archive-publish: the final path is the canonical UUID under \$AE_HOME/archive" \`

Ability-to-fail / coverage: only final UUID path is asserted at the pinned line; umask/modes/private staging and complete tree require separate assertions.

Verdict: **PARTIAL**

### SC-803 — PARTIAL

Claim (exact):
```
**SC-803 — a standing claim is refused and named, never cleaned.** Bucket 1 — from the
outside a stale claim and a live publisher are indistinguishable; the next run refuses
with the claim's name. Authority: architecture.md:95-97. Empirical: pending.
Conflict: none.
```

Evidence (TEST schema): `SC-803 | TEST | assertion=git show 72c7293:tests/integration | line=4275 | label=assert_eq "archive-claim: a standing claim FAILS the end rather than being cleaned up" "1" \`
Frozen assertion body (exact pinned line, tests/integration:4275): `assert_eq "archive-claim: a standing claim FAILS the end rather than being cleaned up" "1" \`

Ability-to-fail / coverage: only nonzero refusal is pinned; claim naming and preservation are separate assertions not covered by this pin.

Verdict: **PARTIAL**

### SC-804a — FALSE

Claim (exact):
```
**SC-804a — validator: exact path whitelist.** Bucket 1 — an entry ae does not
recognise FAILS validation rather than being ignored. Authority: architecture.md:99-104.
Empirical: pending. Conflict: none.
```

Evidence (TEST schema): `SC-804a | TEST | assertion=git show 72c7293:tests/unit | line=11574 | label=assert_eq "archive-publish: the final path is the canonical UUID under \$AE_HOME/archive" \`
Frozen assertion body (exact pinned line, tests/unit:11574): `assert_eq "archive-publish: the final path is the canonical UUID under \$AE_HOME/archive" \`

Ability-to-fail / coverage: final archive path assertion does not validate exact whitelist or unknown-entry refusal.

Verdict: **FALSE**

### SC-804b — FALSE

Claim (exact):
```
**SC-804b — validator: no symlink or special file.** Bucket 1. Authority:
architecture.md:100. Empirical: pending. Conflict: none.
```

Evidence (TEST schema): `SC-804b | TEST | assertion=git show 72c7293:tests/unit | line=4562 | label=# café or a deep tree produced a name its own validator refuses: the validator`
Frozen assertion body (exact pinned line, tests/unit:4562): `# café or a deep tree produced a name its own validator refuses: the validator`

Ability-to-fail / coverage: default-name hostile-directory comment does not exercise symlink/special-file rejection.

Verdict: **FALSE**

### SC-804c — FALSE

Claim (exact):
```
**SC-804c — validator: directories 0700.** Bucket 1. Authority: architecture.md:100-101.
Empirical: pending. Conflict: none.
```

Evidence (TEST schema): `SC-804c | TEST | assertion=git show 72c7293:tests/unit | line=4562 | label=# café or a deep tree produced a name its own validator refuses: the validator`
Frozen assertion body (exact pinned line, tests/unit:4562): `# café or a deep tree produced a name its own validator refuses: the validator`

Ability-to-fail / coverage: default-name hostile-directory comment does not exercise directory mode 0700.

Verdict: **FALSE**

### SC-804d — FALSE

Claim (exact):
```
**SC-804d — validator: no executable bit for user, group, OR other.** Bucket 1 — `-x`
answers only for the current user; a group-executable file is still a program.
Authority: architecture.md:101-103. Empirical: pending. Conflict: none.
```

Evidence (TEST schema): `SC-804d | TEST | assertion=git show 72c7293:tests/unit | line=4562 | label=# café or a deep tree produced a name its own validator refuses: the validator`
Frozen assertion body (exact pinned line, tests/unit:4562): `# café or a deep tree produced a name its own validator refuses: the validator`

Ability-to-fail / coverage: default-name hostile-directory comment does not exercise all-user/group/other execute-bit rejection.

Verdict: **FALSE**

### SC-804e — FALSE

Claim (exact):
```
**SC-804e — validator: `meta` and `digest.md` must agree.** Bucket 1 — on the archive
id and the counts they report. Authority: architecture.md:103-104. Empirical: pending.
Conflict: none.
```

Evidence (TEST schema): `SC-804e | TEST | assertion=git show 72c7293:tests/unit | line=4562 | label=# café or a deep tree produced a name its own validator refuses: the validator`
Frozen assertion body (exact pinned line, tests/unit:4562): `# café or a deep tree produced a name its own validator refuses: the validator`

Ability-to-fail / coverage: default-name hostile-directory comment does not exercise meta/digest agreement.

Verdict: **FALSE**

### SC-804f — FALSE

Claim (exact):
```
**SC-804f — validator: files 0600.** Bucket 1. Authority: architecture.md:100-101.
Empirical: pending. Conflict: none.
```

Evidence (TEST schema): `SC-804f | TEST | assertion=git show 72c7293:tests/unit | line=4562 | label=# café or a deep tree produced a name its own validator refuses: the validator`
Frozen assertion body (exact pinned line, tests/unit:4562): `# café or a deep tree produced a name its own validator refuses: the validator`

Ability-to-fail / coverage: default-name hostile-directory comment does not exercise file mode 0600.

Verdict: **FALSE**

### SC-805 — FALSE

Claim (exact):
```
**SC-805 — an archive is inert data.** Bucket 1 — never an executable file; the
validator is the proof, not intent. Authority: AGENTS.md rules bullet + architecture.md.
Empirical: pending. Conflict: none.
```

Evidence (TEST schema): `SC-805 | TEST | assertion=git show 72c7293:tests/integration | line=4209 | label=assert_contains "archive-end: the outcome names the canonical UUID" "Archived $A48_UUID" "$a48_end"`
Frozen assertion body (exact pinned line, tests/integration:4209): `assert_contains "archive-end: the outcome names the canonical UUID" "Archived $A48_UUID" "$a48_end"`

Ability-to-fail / coverage: archive-end output UUID assertion does not inspect inertness/executable bits or validator behavior.

Verdict: **FALSE**

### SC-806a — PARTIAL

Claim (exact):
```
**SC-806a — archive identity is the session UUID, never the mutable name.** Bucket 1 —
addressable independently of a name that is neither unique over time nor stable.
Authority: architecture.md:81-83. Empirical: pending. Conflict: none.
```

Evidence (TEST schema): `SC-806a | TEST | assertion=git show 72c7293:tests/integration | line=4209 | label=assert_contains "archive-end: the outcome names the canonical UUID" "Archived $A48_UUID" "$a48_end"`
Frozen assertion body (exact pinned line, tests/integration:4209): `assert_contains "archive-end: the outcome names the canonical UUID" "Archived $A48_UUID" "$a48_end"`

Ability-to-fail / coverage: output names a UUID but does not prove name-independent archive addressing through rename/reuse.

Verdict: **PARTIAL**

### SC-806b — PARTIAL

Claim (exact):
```
**SC-806b — canonical lowercase key; legacy uppercase normalized.** Bucket 2.
Authority: architecture.md:81-82. Empirical: pending. Conflict: none.
```

Evidence (TEST schema): `SC-806b | TEST | assertion=git show 72c7293:tests/unit | line=11338 | label=assert_eq "archive-uuid: lowercase passes through" "ae3aa692-e177-4798-9ba0-d14e0d084061" "$(_ar_canonical_uuid ae3aa692-e177-4798-9ba0-d14e0d084061)"`
Frozen assertion body (exact pinned line, tests/unit:11338): `assert_eq "archive-uuid: lowercase passes through" "ae3aa692-e177-4798-9ba0-d14e0d084061" "$(_ar_canonical_uuid ae3aa692-e177-4798-9ba0-d14e0d084061)"`

Ability-to-fail / coverage: the pinned lower-case identity case does not fail if uppercase/mixed-case normalization is removed.

Verdict: **PARTIAL**

### SC-807 — PARTIAL

Claim (exact):
```
**SC-807 — the lifecycle lock is released before the relaunch.** Bucket 1 — the child
takes the same lock under the same name; holding across both would deadlock ae against
itself. Authority: architecture.md:230-232. Empirical: pending. Conflict: none.
```

Evidence (TEST schema): `SC-807 | TEST | assertion=git show 72c7293:tests/unit | line=4820 | label=assert_eq "lifecycle-lock (12th I1): released BEFORE the async SID-capture children inherit fd 8" "1" \`
Frozen assertion body (exact pinned line, tests/unit:4820): `assert_eq "lifecycle-lock (12th I1): released BEFORE the async SID-capture children inherit fd 8" "1" \`

Ability-to-fail / coverage: release-before-capture is asserted, not release-before-relaunch lock safety; a relaunch deadlock can remain.

Verdict: **PARTIAL**

### SC-808 — FALSE

Claim (exact):
```
**SC-808 — the child re-proves the exact parent archive before publishing its state.**
Bucket 1 — semantic invariant: re-prove immediately before publication, roll the launch
back on mismatch rather than creating a child with no lineage. (The bash transport
variable is mechanism — ownership/evidence, not SHOULD.) Authority:
architecture.md:232-234. Empirical: pending. Conflict: none.
```

Evidence (TEST schema): `SC-808 | TEST | assertion=git show 72c7293:tests/integration | line=5031 | label=assert_eq "compact-rollback: (setup) the parent archive exists to inherit from" "1" \`
Frozen assertion body (exact pinned line, tests/integration:5031): `assert_eq "compact-rollback: (setup) the parent archive exists to inherit from" "1" \`

Ability-to-fail / coverage: the pinned setup assertion only establishes a parent archive exists; it does not test child re-proof/rollback.

Verdict: **FALSE**

### SC-809 — PARTIAL

Claim (exact):
```
**SC-809 — lineage is never inferred from a name.** Bucket 1 — a session continues an
archive only via explicit `--from <uuid>`. Authority: AGENTS.md "How it works" (ruling
text) + architecture.md. Empirical: pending. Conflict: none.
```

Evidence (TEST schema): `SC-809 | TEST | assertion=git show 72c7293:tests/integration | line=4198 | label=assert_eq "lineage: an invalid --from on a fresh home refuses" "1" \`
Frozen assertion body (exact pinned line, tests/integration:4198): `assert_eq "lineage: an invalid --from on a fresh home refuses" "1" \`

Ability-to-fail / coverage: invalid --from refusal proves one error path, not that a valid continuation can never be inferred from a mutable name.

Verdict: **PARTIAL**

### SC-810a — FALSE

Claim (exact):
```
**SC-810a — `--purge-history` writes no archive.** Bucket 2. Authority: AGENTS.md "How
it works" + architecture.md:131-133. Empirical: pending. Conflict: none.
```

Evidence (TEST schema): `SC-810a | TEST | assertion=git show 72c7293:tests/integration | line=4694 | label=assert_eq "end-all: (setup) the purge target has a REAL archive to lose" "1" \`
Frozen assertion body (exact pinned line, tests/integration:4694): `assert_eq "end-all: (setup) the purge target has a REAL archive to lose" "1" \`

Ability-to-fail / coverage: the pinned line only seeds a purge archive; no no-archive behavior is asserted.

Verdict: **FALSE**

### SC-810b — FALSE

Claim (exact):
```
**SC-810b — `--purge-history` deletes any existing archive for the source UUID.**
Bucket 2 — a purge that left memo and request payloads would only have looked like
privacy. Delete PROOFS are SC-818a-e (bucket 1). Authority: architecture.md:131-133.
Empirical: pending. Conflict: none.
```

Evidence (TEST schema): `SC-810b | TEST | assertion=git show 72c7293:tests/integration | line=4694 | label=assert_eq "end-all: (setup) the purge target has a REAL archive to lose" "1" \`
Frozen assertion body (exact pinned line, tests/integration:4694): `assert_eq "end-all: (setup) the purge target has a REAL archive to lose" "1" \`

Ability-to-fail / coverage: the pinned line only seeds a purge archive; deletion of an existing source archive is not asserted.

Verdict: **FALSE**

### SC-811a — FALSE

Claim (exact):
```
**SC-811a — `launch.<slot>.sh` re-run: first run creates, later runs resume.** Bucket 2
— the `.started` marker decides. Authority: AGENTS.md launch-rerun bullet (ruling).
Empirical: pending. Conflict: none.
```

Evidence (TEST schema): `SC-811a | TEST | assertion=git show 72c7293:tests/integration | line=2722 | label=# ── #27: launch.<slot>.sh survives being re-run ──────────────────────`
Frozen assertion body (exact pinned line, tests/integration:2722): `# ── #27: launch.<slot>.sh survives being re-run ──────────────────────`

Ability-to-fail / coverage: the pinned line is the rerun section header, not the create-then-resume assertions.

Verdict: **FALSE**

### SC-811b — FALSE

Claim (exact):
```
**SC-811b — ae clears the marker whenever it rewrites the script.** Bucket 2 — a fresh
launch always creates. Authority: AGENTS.md launch-rerun bullet. Empirical: pending.
Conflict: none.
```

Evidence (TEST schema): `SC-811b | TEST | assertion=git show 72c7293:tests/integration | line=2722 | label=# ── #27: launch.<slot>.sh survives being re-run ──────────────────────`
Frozen assertion body (exact pinned line, tests/integration:2722): `# ── #27: launch.<slot>.sh survives being re-run ──────────────────────`

Ability-to-fail / coverage: the pinned line is the rerun section header, not marker clearing on rewrite.

Verdict: **FALSE**

### SC-812 — FALSE

Claim (exact):
```
**SC-812 — the resume decision happens BEFORE exec.** Bucket 1 — a `cmd || fallback`
chain leaves bash as the pane process and `pane_current_command` reports `bash`,
silently disabling the send path's TUI modelling. Authority: AGENTS.md launch-rerun
bullet + #30-family ruling (commit 32719f5). Empirical: pending. Conflict: none.
```

Evidence (TEST schema): `SC-812 | TEST | assertion=git show 72c7293:tests/unit | line=95 | label=    /^(_ae_tac|_ae_stat|_ae_epoch|_ae_sed_inplace|_ae_json_first|_ae_json_first_num|_ae_md5|_ae_inside_tmux|daemon_session_running|launch_rerun_command|write_launch_script|_publish_executable_artifact|_emit_launch_script|_validate_session_name|_validate_agent_name|_dedup_worker_names|_session_name_usable|_session_path_is_safe|_require_session_path_safe|_transfer_remote_path_is_safe|_canonical_dir|_path_is_direct_child|_spawn_rollback|parse_config|get_config|build_ae_context|strip_session_flags|strip_opencode_session_flags|strip_gemini_prompt_flags|strip_grok_session_flags|resume_cmd_from_cmd|inject_session_id|inject_ae_context|initial_prompt_for_cmd|gen_uuid|resolve_agent_session_id|resolve_launch_tmux_server|_end_verify_gone|_srv_tmux_args|_end_target_server|_end_target_kind|_tmux_err_is_clean_dead|_ae_install_tmux_shim|_end_sweep_servers|read_session_meta|_cmd_split_binary|_cmd_strip_env_prefix|_cmd_binary_kind|_cmd_env_prefix|ae_json_escape|_publish_data_artifact|_opencode_context_files|tool_kind_from_cmd|tool_name_from_cmd|tool_kind_supports_launch_id|read_launch_id_for_slot|read_launch_time_for_slot|default_session_name|sanitize_branch_name|shell_quote|build_launch_command|_launch_injected_head|_launch_is_resume|_launch_id_probeable|_launch_probe_claude|_launch_probe_codex|_launch_resume_decider|extract_binary_from_cmd|_transfer_validate_uuid|_transfer_find_claude_session_file|_transfer_find_codex_session_file|_transfer_json_escape|_transfer_bash_quote|_transfer_session_summary|_transfer_session_running|_transfer_ensure_stopped|_transfer_ensure_stopped_remote|_transfer_ssh_probe|_transfer_local_rsync_supports_protect_args|_transfer_remote_preflight|_transfer_check_remote_path|_transfer_remote_session_exists|_transfer_emit_destination_event|_transfer_read_remote_meta_value|_transfer_find_remote_claude_session_file|_transfer_find_remote_codex_session_file|_transfer_local_session_exists|_transfer_emit_local_event|_cleanup_agent_session_files|_end_target_class|_end_confirm_body|_end_tmux|_end_live_id|_end_target_server|_end_history_lines|_ae_session_id|_end_target_server|_end_live_id|_roster_glyph|_roster_label|_roster_compose|_roster_slot_refs|_roster_window_compose|_roster_session_id|cmd_transfer)\(\) \{/ { printing=1 }`
Frozen assertion body (exact pinned line, tests/unit:95): `    /^(_ae_tac|_ae_stat|_ae_epoch|_ae_sed_inplace|_ae_json_first|_ae_json_first_num|_ae_md5|_ae_inside_tmux|daemon_session_running|launch_rerun_command|write_launch_script|_publish_executable_artifact|_emit_launch_script|_validate_session_name|_validate_agent_name|_dedup_worker_names|_session_name_usable|_session_path_is_safe|_require_session_path_safe|_transfer_remote_path_is_safe|_canonical_dir|_path_is_direct_child|_spawn_rollback|parse_config|get_config|build_ae_context|strip_session_flags|strip_opencode_session_flags|strip_gemini_prompt_flags|strip_grok_session_flags|resume_cmd_from_cmd|inject_session_id|inject_ae_context|initial_prompt_for_cmd|gen_uuid|resolve_agent_session_id|resolve_launch_tmux_server|_end_verify_gone|_srv_tmux_args|_end_target_server|_end_target_kind|_tmux_err_is_clean_dead|_ae_install_tmux_shim|_end_sweep_servers|read_session_meta|_cmd_split_binary|_cmd_strip_env_prefix|_cmd_binary_kind|_cmd_env_prefix|ae_json_escape|_publish_data_artifact|_opencode_context_files|tool_kind_from_cmd|tool_name_from_cmd|tool_kind_supports_launch_id|read_launch_id_for_slot|read_launch_time_for_slot|default_session_name|sanitize_branch_name|shell_quote|build_launch_command|_launch_injected_head|_launch_is_resume|_launch_id_probeable|_launch_probe_claude|_launch_probe_codex|_launch_resume_decider|extract_binary_from_cmd|_transfer_validate_uuid|_transfer_find_claude_session_file|_transfer_find_codex_session_file|_transfer_json_escape|_transfer_bash_quote|_transfer_session_summary|_transfer_session_running|_transfer_ensure_stopped|_transfer_ensure_stopped_remote|_transfer_ssh_probe|_transfer_local_rsync_supports_protect_args|_transfer_remote_preflight|_transfer_check_remote_path|_transfer_remote_session_exists|_transfer_emit_destination_event|_transfer_read_remote_meta_value|_transfer_find_remote_claude_session_file|_transfer_find_remote_codex_session_file|_transfer_local_session_exists|_transfer_emit_local_event|_cleanup_agent_session_files|_end_target_class|_end_confirm_body|_end_tmux|_end_live_id|_end_target_server|_end_history_lines|_ae_session_id|_end_target_server|_end_live_id|_roster_glyph|_roster_label|_roster_compose|_roster_slot_refs|_roster_window_compose|_roster_session_id|cmd_transfer)\(\) \{/ { printing=1 }`

Ability-to-fail / coverage: the pinned unit line is an emission-list regex, unrelated to pre-exec resume decision.

Verdict: **FALSE**

### SC-814 — FALSE

Claim (exact):
```
**SC-814 — transfer validates both endpoint names before any side effect.** Bucket 1 —
before any path construction, SSH probe, mkdir, or rsync. Authority: AGENTS.md
session-name bullet. Empirical: pending. Conflict: none.
```

Evidence (TEST schema): `SC-814 | TEST | assertion=git show 72c7293:tests/integration | line=2095 | label=# ── ae transfer — phase 1 failure paths ──────────────────────────────`
Frozen assertion body (exact pinned line, tests/integration:2095): `# ── ae transfer — phase 1 failure paths ──────────────────────────────`

Ability-to-fail / coverage: the pinned line is a transfer section header; endpoint validation-before-side-effect is not asserted.

Verdict: **FALSE**

### SC-815a — FALSE

Claim (exact):
```
**SC-815a — the confirmed fleet is the fleet acted on.** Bucket 1 — `stop all` hands
over the exact confirmed list and does not re-enumerate; a session started during
confirmation is left alone. Authority: commands.md:382-386. Empirical: pending.
Conflict: none.
```

Evidence (TEST schema): `SC-815a | TEST | assertion=git show 72c7293:tests/unit | line=5045 | label=# (The LOOP's spelling changed in round 8 — F9 made the fleet an argv list, so it`
Frozen assertion body (exact pinned line, tests/unit:5045): `# (The LOOP's spelling changed in round 8 — F9 made the fleet an argv list, so it`

Ability-to-fail / coverage: the pinned unit line is a loop comment; confirmed-list immutability is not asserted.

Verdict: **FALSE**

### SC-815b — FALSE

Claim (exact):
```
**SC-815b — fleet entries carry session identity, not names.** Bucket 1 — ending a
session and starting a new one under the same name mid-operation leaves the newcomer
running, with a recorded failure explaining the name changed hands. Authority:
commands.md:386-389. Empirical: pending. Conflict: none.
```

Evidence (TEST schema): `SC-815b | TEST | assertion=git show 72c7293:tests/integration | line=3661 | label=assert_eq "ABA (F10): the same name now carries a different instance" "1" \`
Frozen assertion body (exact pinned line, tests/integration:3661): `assert_eq "ABA (F10): the same name now carries a different instance" "1" \`

Ability-to-fail / coverage: the pinned assertion is only a fixture precondition that old/new ids differ; it does not assert the acted-on set.

Verdict: **FALSE**

### SC-815c — FALSE

Claim (exact):
```
**SC-815c — each fleet run has a unique operation identity and consumes ONLY its own
results.** Bucket 1 — cross-run result folding is never permitted; the label alone is
not the mechanism. Authority: commands.md:389-390. Empirical: pending. Conflict: none.
```

Evidence (TEST schema): `SC-815c | TEST | assertion=git show 72c7293:tests/unit | line=5096 | label=# BASELINE PINS RETIRED — SUPERSEDED BY F8. Round 7 pinned "the baseline is taken`
Frozen assertion body (exact pinned line, tests/unit:5096): `# BASELINE PINS RETIRED — SUPERSEDED BY F8. Round 7 pinned "the baseline is taken`

Ability-to-fail / coverage: the pinned line is a retired-pin comment; operation-id result isolation is not asserted.

Verdict: **FALSE**

### SC-815d — FALSE

Claim (exact):
```
**SC-815d — the visible representation is `[op <uuid>]` in the events.** Bucket 2.
Authority: commands.md:389-390. Empirical: pending. Conflict: none.
```

Evidence (TEST schema): `SC-815d | TEST | assertion=git show 72c7293:tests/unit | line=5096 | label=# BASELINE PINS RETIRED — SUPERSEDED BY F8. Round 7 pinned "the baseline is taken`
Frozen assertion body (exact pinned line, tests/unit:5096): `# BASELINE PINS RETIRED — SUPERSEDED BY F8. Round 7 pinned "the baseline is taken`

Ability-to-fail / coverage: the pinned line is a retired-pin comment; [op uuid] event representation is not asserted.

Verdict: **FALSE**

### SC-816 — FALSE

Claim (exact):
```
**SC-816 — an unverifiable session is still a target.** Bucket 1 — if its recorded tmux
server is unreachable, ae does not know it is stopped: it is carried into the fleet and
fails loudly in its own log rather than being silently counted as gone. Authority:
commands.md:378-381. Empirical: pending. Conflict: none.
```

Evidence (TEST schema): `SC-816 | TEST | assertion=git show 72c7293:tests/unit | line=4998 | label=# ── F4/F5/F6: THE INTERACTION FACTS (caller IS one of the fleet targets) ─`
Frozen assertion body (exact pinned line, tests/unit:4998): `# ── F4/F5/F6: THE INTERACTION FACTS (caller IS one of the fleet targets) ─`

Ability-to-fail / coverage: the pinned line is an interaction header; unreachable-server target handling is not asserted.

Verdict: **FALSE**

### SC-818a — PROVES

Claim (exact):
```
**SC-818a — purge requires ae's REAL archive root, never a symlink.** Bucket 1.
Authority: commands.md:534-535 + architecture.md:134-137. Empirical: pending.
Conflict: none.
```

Evidence (TEST schema): `SC-818a | TEST | assertion=git show 72c7293:tests/unit | line=12161 | label=assert_eq "archive-root: a SYMLINKED archive root is refused, not followed" "1" \`
Frozen assertion body (exact pinned line, tests/unit:12161): `assert_eq "archive-root: a SYMLINKED archive root is refused, not followed" "1" \`

Ability-to-fail / coverage: the pinned mutation invokes _ar_require_real_root against a symlink and requires nonzero; removing the root refusal fails.

Verdict: **PROVES**

### SC-818b — FALSE

Claim (exact):
```
**SC-818b — purge acquires the same `.publishing.<uuid>` claim.** Bucket 1 — a delete
cannot race a publisher's rename. Authority: commands.md:534-536. Empirical: pending.
Conflict: none.
```

Evidence (TEST schema): `SC-818b | TEST | assertion=git show 72c7293:tests/unit | line=11679 | label=assert_eq "archive-purge: removes the archive for THIS source session" "0" \`
Frozen assertion body (exact pinned line, tests/unit:11679): `assert_eq "archive-purge: removes the archive for THIS source session" "0" \`

Ability-to-fail / coverage: normal purge success does not exercise the shared publishing claim lock.

Verdict: **FALSE**

### SC-818c — FALSE

Claim (exact):
```
**SC-818c — purge validates the tree as an ae archive before deleting.** Bucket 1 — a
tree ae cannot validate is a tree ae cannot claim to own; a hand-edited archive is
refused (remove it yourself). Authority: commands.md:536-542. Empirical: pending.
Conflict: none.
```

Evidence (TEST schema): `SC-818c | TEST | assertion=git show 72c7293:tests/unit | line=11679 | label=assert_eq "archive-purge: removes the archive for THIS source session" "0" \`
Frozen assertion body (exact pinned line, tests/unit:11679): `assert_eq "archive-purge: removes the archive for THIS source session" "0" \`

Ability-to-fail / coverage: normal purge success does not exercise archive validation or hand-edited refusal.

Verdict: **FALSE**

### SC-818d — FALSE

Claim (exact):
```
**SC-818d — purge requires a NONEMPTY exact source-identity match.** Bucket 1 — an
archive naming no session is absence of proof, not a wildcard; refused as malformed
(and `--from` will not inherit from it either). Authority: commands.md:537-540 +
architecture.md:134-137. Empirical: pending. Conflict: none.
```

Evidence (TEST schema): `SC-818d | TEST | assertion=git show 72c7293:tests/unit | line=11679 | label=assert_eq "archive-purge: removes the archive for THIS source session" "0" \`
Frozen assertion body (exact pinned line, tests/unit:11679): `assert_eq "archive-purge: removes the archive for THIS source session" "0" \`

Ability-to-fail / coverage: normal purge success does not exercise nonempty exact source identity.

Verdict: **FALSE**

### SC-818e — FALSE

Claim (exact):
```
**SC-818e — purge refuses to delete a parent a live `--from` lineage points at.**
Bucket 1. Authority: architecture.md:137-138. Empirical: pending. Conflict: none.
```

Evidence (TEST schema): `SC-818e | TEST | assertion=git show 72c7293:tests/unit | line=11679 | label=assert_eq "archive-purge: removes the archive for THIS source session" "0" \`
Frozen assertion body (exact pinned line, tests/unit:11679): `assert_eq "archive-purge: removes the archive for THIS source session" "0" \`

Ability-to-fail / coverage: normal purge success does not exercise live-parent lineage refusal.

Verdict: **FALSE**

### SC-819 — PARTIAL

Claim (exact):
```
**SC-819 — an unidentifiable session is refused BEFORE anything is stopped.** Bucket 1
— meta gone with memory remaining, or `session_id` unparseable: refused with the
reason, nothing deleted, regardless of history flag ("delete it" is not an answer to
"which session is this"). Authority: commands.md:513-518 + architecture.md:139-143.
Empirical: pending. Conflict: none.
```

Evidence (TEST schema): `SC-819 | TEST | assertion=git show 72c7293:tests/integration | line=4343 | label=assert_eq "archive-nometa: a meta-less session that still holds memory is NOT ended silently" "1" \`
Frozen assertion body (exact pinned line, tests/integration:4343): `assert_eq "archive-nometa: a meta-less session that still holds memory is NOT ended silently" "1" \`

Ability-to-fail / coverage: only nonzero refusal is pinned; reason, pre-stop ordering, and retained memo are separate assertions.

Verdict: **PARTIAL**

### SC-820a — PARTIAL

Claim (exact):
```
**SC-820a — end freezes the confirmed plan and re-proves it under the lock.** Bucket 1
— each target resolved exactly ONCE; the prompt renders from those fields and the
freeze captures the same observation (a fork cannot carry a freeze back); re-proof
under the lifecycle lock refuses on mismatch and prints both versions. Authority:
commands.md:526-532 + architecture.md:146-149,158-166. Empirical: pending.
Conflict: none.
```

Evidence (TEST schema): `SC-820a | TEST | assertion=git show 72c7293:tests/integration | line=4566 | label=assert_eq "end-freeze: a confirmed KEEP is not carried out as a PURGE" "1" \`
Frozen assertion body (exact pinned line, tests/integration:4566): `assert_eq "end-freeze: a confirmed KEEP is not carried out as a PURGE" "1" \`

Ability-to-fail / coverage: the pinned mismatch rc proves one KEEP/PURGE flip refusal, not every-target freeze and under-lock re-proof.

Verdict: **PARTIAL**

### SC-820b — PROVES

Claim (exact):
```
**SC-820b — `-f` freezes nothing.** Bucket 2 — nothing was promised, so nothing is
frozen or re-proved. Authority: commands.md:526-532. Empirical: pending. Conflict: none.
```

Evidence (TEST schema): `SC-820b | TEST | assertion=git show 72c7293:tests/unit | line=12290 | label=assert_eq "end-freeze: the plan the human confirmed proceeds" "0" "$(_ar48_frozen_step match; echo $?)"`
Frozen assertion body (exact pinned line, tests/unit:12290): `assert_eq "end-freeze: the plan the human confirmed proceeds" "0" "$(_ar48_frozen_step match; echo $?)"`

Ability-to-fail / coverage: the pinned helper exercises match/differ/none, including the explicit -f no-freeze arm; mutating the no-freeze behavior fails.

Verdict: **PROVES**

### SC-821a — FALSE

Claim (exact):
```
**SC-821a — `end all` acts on the confirmed target set only.** Bucket 1 — the set can
never grow between question and answer. Authority: architecture.md:150-155.
Empirical: pending. Conflict: none.
```

Evidence (TEST schema): `SC-821a | TEST | assertion=git show 72c7293:tests/integration | line=4694 | label=assert_eq "end-all: (setup) the purge target has a REAL archive to lose" "1" \`
Frozen assertion body (exact pinned line, tests/integration:4694): `assert_eq "end-all: (setup) the purge target has a REAL archive to lose" "1" \`

Ability-to-fail / coverage: the pinned line is archive setup, not confirmed-set execution.

Verdict: **FALSE**

### SC-821b — PARTIAL

Claim (exact):
```
**SC-821b — "a prompt ran" is its own fact, never a count.** Bucket 1 — an empty
confirmed list means end NOTHING, which a count cannot distinguish from
nobody-was-asked. Authority: architecture.md:150-155. Empirical: pending.
Conflict: none.
```

Evidence (TEST schema): `SC-821b | TEST | assertion=git show 72c7293:tests/unit | line=12486 | label=assert_eq "end-all: execution iterates the confirmed list, not a new enumeration" "1" \`
Frozen assertion body (exact pinned line, tests/unit:12486): `assert_eq "end-all: execution iterates the confirmed list, not a new enumeration" "1" \`

Ability-to-fail / coverage: confirmed-list iteration is pinned, but the independent prompt-ran fact is not.

Verdict: **PARTIAL**

### SC-822 — PARTIAL

Claim (exact):
```
**SC-822 — `--from` is valid only for a session that does not exist in any form.**
Bucket 1 — no tmux session, no session state, no worktree; onto an existing session it
refuses ("resume this AND inherit that" has two meanings and no safe default).
Authority: commands.md:580-584. Empirical: pending. Conflict: none.
```

Evidence (TEST schema): `SC-822 | TEST | assertion=git show 72c7293:tests/integration | line=4198 | label=assert_eq "lineage: an invalid --from on a fresh home refuses" "1" \`
Frozen assertion body (exact pinned line, tests/integration:4198): `assert_eq "lineage: an invalid --from on a fresh home refuses" "1" \`

Ability-to-fail / coverage: invalid --from on a fresh home is covered; existing session/state/worktree refusal is not.

Verdict: **PARTIAL**

### SC-823 — PARTIAL

Claim (exact):
```
**SC-823 — the parent is proved before anything is created.** Bucket 1 — a refusal
leaves no tmux session, no session state, no worktree. Authority: commands.md:586-592.
Empirical: pending. Conflict: none.
```

Evidence (TEST schema): `SC-823 | TEST | assertion=git show 72c7293:tests/integration | line=4198 | label=assert_eq "lineage: an invalid --from on a fresh home refuses" "1" \`
Frozen assertion body (exact pinned line, tests/integration:4198): `assert_eq "lineage: an invalid --from on a fresh home refuses" "1" \`

Ability-to-fail / coverage: invalid parent pre-mutation is covered; valid-parent proof ordering before all creation is not.

Verdict: **PARTIAL**

### SC-824a — FALSE

Claim (exact):
```
**SC-824a — proof facts are recorded as proved, never re-read.** Bucket 1 — id and
handover/pending counts come back from the one proof, not from a file another process
may be deleting. Authority: commands.md:589-592. Empirical: pending. Conflict: none.
```

Evidence (TEST schema): `SC-824a | TEST | assertion=git show 72c7293:tests/unit | line=11891 | label=assert_eq "from-preflight: a valid archive resolves to its canonical id" "ae3aa692-e177-4798-9ba0-d14e0d084061" \`
Frozen assertion body (exact pinned line, tests/unit:11891): `assert_eq "from-preflight: a valid archive resolves to its canonical id" "ae3aa692-e177-4798-9ba0-d14e0d084061" \`

Ability-to-fail / coverage: the pinned valid-id lookup does not prove proof facts are recorded and never reread.

Verdict: **FALSE**

### SC-824b — FALSE

Claim (exact):
```
**SC-824b — an archive mid-publication or mid-purge is refused outright.** Bucket 1.
Authority: commands.md:589-592. Empirical: pending. Conflict: none.
```

Evidence (TEST schema): `SC-824b | TEST | assertion=git show 72c7293:tests/unit | line=11891 | label=assert_eq "from-preflight: a valid archive resolves to its canonical id" "ae3aa692-e177-4798-9ba0-d14e0d084061" \`
Frozen assertion body (exact pinned line, tests/unit:11891): `assert_eq "from-preflight: a valid archive resolves to its canonical id" "ae3aa692-e177-4798-9ba0-d14e0d084061" \`

Ability-to-fail / coverage: the pinned valid-id lookup does not exercise mid-publication/mid-purge refusal.

Verdict: **FALSE**

### SC-825a — FALSE

Claim (exact):
```
**SC-825a — the child records lineage durably.** Bucket 2 — `parent_archive_id` +
parent handover/pending counts, preserved across resumes. Authority:
commands.md:594-598. Empirical: pending. Conflict: none.
```

Evidence (TEST schema): `SC-825a | TEST | assertion=git show 72c7293:tests/integration | line=4198 | label=assert_eq "lineage: an invalid --from on a fresh home refuses" "1" \`
Frozen assertion body (exact pinned line, tests/integration:4198): `assert_eq "lineage: an invalid --from on a fresh home refuses" "1" \`

Ability-to-fail / coverage: invalid --from refusal does not assert durable parent lineage fields on a child.

Verdict: **FALSE**

### SC-825b — PARTIAL

Claim (exact):
```
**SC-825b — the parent path is derived, never stored.** Bucket 2 — from archive root +
id, so moving `AE_HOME` cannot rot it. Authority: commands.md:594-598.
Empirical: pending. Conflict: none.
```

Evidence (TEST schema): `SC-825b | TEST | assertion=git show 72c7293:tests/unit | line=11923 | label=assert_eq "from-prompt: the MAIN agent is told to read the digest first" "1" \`
Frozen assertion body (exact pinned line, tests/unit:11923): `assert_eq "from-prompt: the MAIN agent is told to read the digest first" "1" \`

Ability-to-fail / coverage: the main prompt contains a derived archive path, but the pin does not assert absence of a stored path.

Verdict: **PARTIAL**

### SC-825c — FALSE

Claim (exact):
```
**SC-825c — a deleted parent warns and continues on resume.** Bucket 2 — the lineage
fact is still true; workspace.md says the digest is gone. Authority:
commands.md:594-598. Empirical: pending. Conflict: none.
```

Evidence (TEST schema): `SC-825c | TEST | assertion=git show 72c7293:tests/integration | line=4198 | label=assert_eq "lineage: an invalid --from on a fresh home refuses" "1" \`
Frozen assertion body (exact pinned line, tests/integration:4198): `assert_eq "lineage: an invalid --from on a fresh home refuses" "1" \`

Ability-to-fail / coverage: invalid --from refusal does not exercise deleted-parent warn-and-continue resume.

Verdict: **FALSE**

### SC-826 — PARTIAL

Claim (exact):
```
**SC-826 — a pre-id session gets one minted at end, recorded on both sides.** Bucket 2
— `session_id_origin=minted-at-end` in live meta AND `archive_id_origin=minted-at-end`
in the archive; the live record keeps a retry after failed publication honest.
Authority: commands.md:520-524. Empirical: pending. Conflict: none.
```

Evidence (TEST schema): `SC-826 | TEST | assertion=git show 72c7293:tests/integration | line=4493 | label=assert_contains "archive-legacy: a pre-session-id session is minted one rather than stranded" \`
Frozen assertion body (exact pinned line, tests/integration:4493): `assert_contains "archive-legacy: a pre-session-id session is minted one rather than stranded" \`

Ability-to-fail / coverage: minted output/archive provenance is exercised nearby, but the pinned output assertion alone does not cover both live and archive records.

Verdict: **PARTIAL**

### SC-828 — FALSE

Claim (exact):
```
**SC-828 — two revalidations, positioned by what they protect.** Bucket 1 — first
immediately after the human's answer (a replaced session is never MESSAGED); second
under the lifecycle lock immediately before teardown (a replacement is never STOPPED);
a mismatch names the field that moved. Authority: architecture.md:186-191 +
commands.md:650-655. Empirical: pending. Conflict: none.
```

Evidence (TEST schema): `SC-828 | TEST | assertion=git show 72c7293:tests/integration | line=5244 | label=assert_contains "compact-gate2: (control) the handover completed, so the wait was passed" \`
Frozen assertion body (exact pinned line, tests/integration:5244): `assert_contains "compact-gate2: (control) the handover completed, so the wait was passed" \`

Ability-to-fail / coverage: the pinned control only proves handover completion; it does not prove either post-wait identity revalidation.

Verdict: **FALSE**

### SC-829a — PARTIAL

Claim (exact):
```
**SC-829a — handover completion is two facts.** Bucket 1 — a reply to the request AND a
new `handover`-topic memo written after the request went out, polled from the event log
and `memo.tsv`, never pane output. Authority: architecture.md:193-199. Empirical:
pending. Conflict: none.
```

Evidence (TEST schema): `SC-829a | TEST | assertion=git show 72c7293:tests/integration | line=4908 | label=assert_eq "compact-wait: with neither fact, it times out" "1" "$([[ $b48_to_rc -ne 0 ]] && echo 1 || echo 0)"`
Frozen assertion body (exact pinned line, tests/integration:4908): `assert_eq "compact-wait: with neither fact, it times out" "1" "$([[ $b48_to_rc -ne 0 ]] && echo 1 || echo 0)"`

Ability-to-fail / coverage: timeout and the nearby two-fact diagnostic are related, but the pinned rc assertion alone cannot distinguish one fact from two.

Verdict: **PARTIAL**

### SC-829b — FALSE

Claim (exact):
```
**SC-829b — a re-run reuses the outstanding request and its baseline.** Bucket 1 — the
memo baseline travels in the request's own stored body, so re-running waits on the SAME
request instead of sending a second, and the fact survives into the archive.
Authority: architecture.md:198-201. Empirical: pending. Conflict: none.
```

Evidence (TEST schema): `SC-829b | TEST | assertion=git show 72c7293:tests/integration | line=4908 | label=assert_eq "compact-wait: with neither fact, it times out" "1" "$([[ $b48_to_rc -ne 0 ]] && echo 1 || echo 0)"`
Frozen assertion body (exact pinned line, tests/integration:4908): `assert_eq "compact-wait: with neither fact, it times out" "1" "$([[ $b48_to_rc -ne 0 ]] && echo 1 || echo 0)"`

Ability-to-fail / coverage: the pinned timeout assertion does not exercise rerun reuse or stored baseline.

Verdict: **FALSE**

### SC-830 — FALSE

Claim (exact):
```
**SC-830 — `--digest-only` is the one explicit degradation.** Bucket 2 — withdraws
anything outstanding and treats the digest as the handover. Authority:
commands.md:634-638 + architecture.md:201-203. Empirical: pending. Conflict: none.
```

Evidence (TEST schema): `SC-830 | TEST | assertion=git show 72c7293:tests/integration | line=4997 | label=assert_eq "compact-withdraw: (setup) a request is outstanding" "1" \`
Frozen assertion body (exact pinned line, tests/integration:4997): `assert_eq "compact-withdraw: (setup) a request is outstanding" "1" \`

Ability-to-fail / coverage: the pinned assertion only seeds an outstanding request; withdrawal/digest-only semantics are not asserted.

Verdict: **FALSE**

### SC-831 — PARTIAL

Claim (exact):
```
**SC-831 — a timed-out handover stops nothing.** Bucket 1 — nothing stopped, nothing
archived, the request stays open so a re-run waits on the SAME request rather than
sending a second. Authority: commands.md:656-658. Empirical: pending. Conflict: none.
```

Evidence (TEST schema): `SC-831 | TEST | assertion=git show 72c7293:tests/integration | line=4908 | label=assert_eq "compact-wait: with neither fact, it times out" "1" "$([[ $b48_to_rc -ne 0 ]] && echo 1 || echo 0)"`
Frozen assertion body (exact pinned line, tests/integration:4908): `assert_eq "compact-wait: with neither fact, it times out" "1" "$([[ $b48_to_rc -ne 0 ]] && echo 1 || echo 0)"`

Ability-to-fail / coverage: timeout rc is pinned and nearby source-untouched checks exist, but request remains-open/reuse semantics are not covered by the pin.

Verdict: **PARTIAL**

### SC-832b — PROVES

Claim (exact):
```
**SC-832b — rename vs concurrent meta writers.** `authority=code-observation` — the
meta rewrite runs without `meta.lock` (census-2, ae:11597-11667); race semantics need
seat closure. UNCLASSIFIED pending closure.
```

Evidence (CENSUS/CODE schema): `SC-832b | CENSUS/CODE | artifact=fdef6d11dc2cdee78433f79dbde83cd9f344157f | path=docs/migration/evidence/locks-census-2.md | heading=## `rename` (`cmd_rename`) | recheck=verify cited ae locations against git show 72c7293:ae`

Ability-to-fail / coverage: artifact heading and ae:11597-11667 citations rechecked against git show 72c7293:ae; they show rename holds lifecycle locks but rewrites meta without meta.lock, exactly the code-observation row.

Verdict: **PROVES**

### SC-832c — PROVES

Claim (exact):
```
**SC-832c — rename crash cuts.** `authority=code-observation` — residue at each cut
point (dir moved / tmux renamed / meta updated) per census-2; seat closure pending.
UNCLASSIFIED pending closure.
```

Evidence (CENSUS/CODE schema): `SC-832c | CENSUS/CODE | artifact=fdef6d11dc2cdee78433f79dbde83cd9f344157f | path=docs/migration/evidence/locks-census-2.md | heading=## `rename` (`cmd_rename`) | recheck=verify cited ae locations against git show 72c7293:ae`

Ability-to-fail / coverage: the same census cites each rename cut (tmux, directory, meta, manifest) and the corresponding residues; the crash-cut row is directly supported.

Verdict: **PROVES**

### SC-833b — PROVES

Claim (exact):
```
**SC-833b — transfer requires the stopped state first.** `authority=code-observation` —
stop-before-rsync ordering; seat closure pending. UNCLASSIFIED.
```

Evidence (CENSUS/CODE schema): `SC-833b | CENSUS/CODE | artifact=fdef6d11dc2cdee78433f79dbde83cd9f344157f | path=docs/migration/evidence/locks-census-2.md | heading=## `transfer` (`cmd_transfer`) | recheck=verify cited ae locations against git show 72c7293:ae`

Ability-to-fail / coverage: transfer census cites stop completion before rsync at ae:11432-11442; reversing that order changes the cited sequence and is caught.

Verdict: **PROVES**

### SC-833c — PROVES

Claim (exact):
```
**SC-833c — per-direction partial-rsync residue.** `authority=code-observation` —
census-2 evidence; seat closure pending. UNCLASSIFIED.
```

Evidence (CENSUS/CODE schema): `SC-833c | CENSUS/CODE | artifact=fdef6d11dc2cdee78433f79dbde83cd9f344157f | path=docs/migration/evidence/locks-census-2.md | heading=## `transfer` (`cmd_transfer`) | recheck=verify cited ae locations against git show 72c7293:ae`

Ability-to-fail / coverage: transfer census cites direct rsync writes and partial/mixed destination residue at ae:11443-11515 for both directions.

Verdict: **PROVES**

### SC-833d — PROVES

Claim (exact):
```
**SC-833d — transfer's audit event is best-effort.** `authority=code-observation` —
warned, success still reported (census-2 addenda, ae:11530-11535); seat closure
pending. UNCLASSIFIED.
```

Evidence (CENSUS/CODE schema): `SC-833d | CENSUS/CODE | artifact=fdef6d11dc2cdee78433f79dbde83cd9f344157f | path=docs/migration/evidence/locks-census-2.md | heading=### Transfer audit event is best-effort | recheck=verify cited ae locations against git show 72c7293:ae`

Ability-to-fail / coverage: addendum heading cites ae:11530-11535; event append failure is warned while transfer still reports success, matching the code observation.

Verdict: **PROVES**

### SC-834b — PROVES

Claim (exact):
```
**SC-834b — recovery meta reconciliation.** `authority=code-observation` — fd200
check-then-set rewriting `agent.<slot>` (census, ae:8717-8732); seat closure pending.
UNCLASSIFIED.
```

Evidence (CENSUS/CODE schema): `SC-834b | CENSUS/CODE | artifact=fdef6d11dc2cdee78433f79dbde83cd9f344157f | path=docs/migration/evidence/locks-census-2.md | heading=## `_recover-pending` (standalone command and watchdog path) | recheck=verify cited ae locations against git show 72c7293:ae`

Ability-to-fail / coverage: recovery census cites fd200 check/rewrite and matching pending slot at ae:8717-8743; the meta reconciliation observation is covered.

Verdict: **PROVES**

### SC-834c — PROVES

Claim (exact):
```
**SC-834c — the `recover` event follows success separately.** `authority=code-observation`
— separate append after the meta write (census); seat closure pending. UNCLASSIFIED.

### S10 — Daemons/sidecars + contrib boundary

Watchdog (nudge rules, quiet-state honoring, footprint exclusion; bash impl vs
`AE_WATCHDOG_IMPL=uv` aewatch), telegram bridge (chat events, reply routing, markdown/jq
injection boundaries; bash daemon vs aewatch runtime handoff via marker + fresh
heartbeat), `ae steward` + ae-monitor window (bash product surfaces) vs contrib
templates/sidecars.

<!-- rows: SC-9xx — claims collected by gpt56luna:s10source (colead's evidence worker,
2026-08-20, frozen-doc citations); buckets proposed by lead; colead confirm pending -->
```

Evidence (CENSUS/CODE schema): `SC-834c | CENSUS/CODE | artifact=fdef6d11dc2cdee78433f79dbde83cd9f344157f | path=docs/migration/evidence/locks-census-2.md | heading=## `_recover-pending` (standalone command and watchdog path) | recheck=verify cited ae locations against git show 72c7293:ae`

Ability-to-fail / coverage: census cites meta update in the child followed by separate watchdog recover-event append at ae:16532-16536; ordering and partial residue are covered.

Verdict: **PROVES**

### SC-1102 — PARTIAL

Claim (exact):
```
**SC-1102 — session/archive UUIDs are canonical lowercase.** Bucket 2 — `gen_uuid`
normalizes (macOS `uuidgen` is uppercase); validators and filenames are
lowercase-only. Authority: AGENTS.md bullet (ruling). Empirical: pending.
Conflict: none.
```

Evidence (TEST schema): `SC-1102 | TEST | assertion=git show 72c7293:tests/unit | line=11338 | label=assert_eq "archive-uuid: lowercase passes through" "ae3aa692-e177-4798-9ba0-d14e0d084061" "$(_ar_canonical_uuid ae3aa692-e177-4798-9ba0-d14e0d084061)"`
Frozen assertion body (exact pinned line, tests/unit:11338): `assert_eq "archive-uuid: lowercase passes through" "ae3aa692-e177-4798-9ba0-d14e0d084061" "$(_ar_canonical_uuid ae3aa692-e177-4798-9ba0-d14e0d084061)"`

Ability-to-fail / coverage: lowercase pass-through is pinned; removal of uppercase/mixed-case normalization remains green.

Verdict: **PARTIAL**

### SC-1105 — PARTIAL

Claim (exact):
```
**SC-1105 — the bash floor is 4.0, scoped to the surviving glue.** Bucket 2 — the
requirement applies to what remains bash after each flip; the binary imposes no bash
requirement at all. Authority: AGENTS.md rules + epic end state. Empirical: pending.
Conflict: none.

### S13 — Identity/provenance security surface

System-prompt interpolation (#59): agent/session name allowlists at every creation
boundary, fresh=fatal vs restored=fail-quiet provenance rule, derived names as grammar
fixed points, message-envelope authority (human = no envelope).

<!-- rows: SC-12xx -->

Authority for all rows is the #59 ruling (closing comment + 72c7293 commit message +
AGENTS.md allowlist bullets) unless noted — normative role. Session-name boundary rows
live in S9 (SC-813/814).

**classified_by: SC-1200..1209 including all splits — fable5:lead + gpt56sol:colead,
2026-08-20 (confirmed on 07e2770), including SC-1202 bucket 3/#61 and the SC-1209 joint
ruling.**
```

Evidence (TEST schema): `SC-1105 | TEST | assertion=git show 72c7293:tests/integration | line=239 | label=assert_eq "helpers: every shebang names a bash >= 4 that can parse the helper" "" "$_sb_bad"`
Frozen assertion body (exact pinned line, tests/integration:239): `assert_eq "helpers: every shebang names a bash >= 4 that can parse the helper" "" "$_sb_bad"`

Ability-to-fail / coverage: helper shebang compatibility is checked, but the surviving glue floor and no-bash binary path are not.

Verdict: **PARTIAL**

### SC-1201 — FALSE

Claim (exact):
```
**SC-1201 — the spawn boundary treats a peer name as hostile.** Bucket 1 — a name
  Authority: #59 ruling (closing comment + 72c7293 commit message + AGENTS.md allowlist bullets).
arriving via `spawn` is validated fatally: violation refuses the spawn (the #59 exploit
was a legal-looking name carrying prose into the identity sentence).
Empirical: pending. Conflict: none.
```

Evidence (TEST schema): `SC-1201 | TEST | assertion=git show 72c7293:tests/unit | line=1525 | label=# ── #59 R1: the agent-name grammar, and the sink it protects ─────────────────`
Frozen assertion body (exact pinned line, tests/unit:1525): `# ── #59 R1: the agent-name grammar, and the sink it protects ─────────────────`

Ability-to-fail / coverage: the pinned section header is not the spawn-boundary assertion; direct validator tests are not reached by this map line.

Verdict: **FALSE**

### SC-1203 — FALSE

Claim (exact):
```
**SC-1203 — enforcement follows provenance, not the variable.** Bucket 1 — FRESH input
  Authority: #59 ruling (closing comment + 72c7293 commit message + AGENTS.md allowlist bullets).
(config, CLI, spawn) is fatal on violation; RESTORED input (saved meta, compact's
frozen roster) is left to the interpolation guard — refusing restored input would make
a pre-grammar session unresumable and kill a compact child whose source is already
archived. Empirical: pending. Conflict: none.
```

Evidence (TEST schema): `SC-1203 | TEST | assertion=git show 72c7293:tests/integration | line=5692 | label=# ── #59 C3-2: a RESTORED roster name is fail-quiet, a FRESH one is fatal ─────`
Frozen assertion body (exact pinned line, tests/integration:5692): `# ── #59 C3-2: a RESTORED roster name is fail-quiet, a FRESH one is fatal ─────`

Ability-to-fail / coverage: the pinned section header is not the restored-vs-fresh runtime assertions.

Verdict: **FALSE**

### SC-1204 — FALSE

Claim (exact):
```
**SC-1204 — the interpolation boundary re-validates and fails quiet.** Bucket 1 —
  Authority: #59 ruling (closing comment + 72c7293 commit message + AGENTS.md allowlist bullets).
semantic SHOULD: at the system-prompt interpolation boundary, alias and name are EACH
revalidated under their respective allowlists; an invalid restored identity omits ONLY
the identity sentence and the launch continues. (The bash function/call path is
empirical/ownership material.) Empirical: pending. Conflict: none.
```

Evidence (TEST schema): `SC-1204 | TEST | assertion=git show 72c7293:tests/unit | line=1525 | label=# ── #59 R1: the agent-name grammar, and the sink it protects ─────────────────`
Frozen assertion body (exact pinned line, tests/unit:1525): `# ── #59 R1: the agent-name grammar, and the sink it protects ─────────────────`

Ability-to-fail / coverage: the pinned section header is not the interpolation fail-quiet assertion.

Verdict: **FALSE**

### SC-1206 — FALSE

Claim (exact):
```
**SC-1206 — a leading underscore is a legal alias but never an agent name.** Bucket 2 —
  Authority: #59 ruling (closing comment + 72c7293 commit message + AGENTS.md allowlist bullets).
`workers = _foo` (alias as its own name) fails the launch with the grammar; internal
`_`-prefixed helpers stay out of the agent namespace. Empirical: pending.
Conflict: none.
```

Evidence (TEST schema): `SC-1206 | TEST | assertion=git show 72c7293:tests/unit | line=1525 | label=# ── #59 R1: the agent-name grammar, and the sink it protects ─────────────────`
Frozen assertion body (exact pinned line, tests/unit:1525): `# ── #59 R1: the agent-name grammar, and the sink it protects ─────────────────`

Ability-to-fail / coverage: the pinned section header is not the leading-underscore roster-boundary assertion.

Verdict: **FALSE**

### SC-1207b — FALSE

Claim (exact):
```
**SC-1207b — meta serializes agents as `alias:name:provider-session-id`.** Bucket 2 —
  Authority: #59 ruling + meta format (S5).
exact on-disk form (cross-link: S5 formats family). Empirical: pending. Conflict: none.
```

Evidence (TEST schema): `SC-1207b | TEST | assertion=git show 72c7293:tests/integration | line=244 | label=# bash. Anything ae then spawns that re-invokes ae through that same shebang`
Frozen assertion body (exact pinned line, tests/integration:244): `# bash. Anything ae then spawns that re-invokes ae through that same shebang`

Ability-to-fail / coverage: the pinned hostile-PATH comment is unrelated to alias:name:provider-session-id serialization.

Verdict: **FALSE**

### SC-1209 — PARTIAL

Claim (exact):
```
**SC-1209 — envelope authority: the outermost helper-emitted line is the only
provenance.** Bucket 1 — the helper-emitted FIRST PHYSICAL line determines peer
provenance; nested/pasted envelopes are data; truly unenveloped interactive input is
the human, who outranks every agent. **Authority: S13 joint seat ruling (fable5:lead +
gpt56sol:colead, 2026-08-20) — recorded here as the normative source, superseding
mutable workspace rules.** Empirical: pending. Conflict: none.

### S14 — Locking / concurrency observable promises

Externally observable ordering/atomicity promises only; protocol detail lives in
`ownership.md`.

<!-- rows: SC-13xx -->
```

Evidence (TEST schema): `SC-1209 | TEST | assertion=git show 72c7293:tests/integration | line=2652 | label=assert_eq "envelope (#39): an unbindable sender is marked unverified, not bare" "1" \`
Frozen assertion body (exact pinned line, tests/integration:2652): `assert_eq "envelope (#39): an unbindable sender is marked unverified, not bare" "1" \`

Ability-to-fail / coverage: calibration: line 2652 proves only unverified marking; the map's later behavioral pin is envelope-before-body, while unit #39 (9820ff/9930ff) covers the full authority rule.

Verdict: **PARTIAL**

### D08 — PROVES

Claim (exact):
```
### D08 — goal (`goal [text|--clear]`)

- effects: meta `goal=` (locked write), `goal` event append
- current writer/call path: `helper_goal_main`
- locks (ordered): TBD
- atomicity boundary: TBD
- current owner: bash
- planned owner/fate: **rust at P2**
```

Evidence (CENSUS/CODE schema): `D08 | CENSUS/CODE | artifact=fdef6d11dc2cdee78433f79dbde83cd9f344157f | path=docs/migration/evidence/locks-census.md | heading=## `goal` (`helper_goal_main` + `ae_meta_set`) | recheck=verify cited ae locations against git show 72c7293:ae`

Ability-to-fail / coverage: goal census fills effects, writer, lock order, temp+rename atomicity, and event-gap residue from ae:14123-14589; cited lines exist in frozen ae.

Verdict: **PROVES**

### D09 — PROVES

Claim (exact):
```
### D09 — memo (`memo add`)

- effects: `memo.tsv` append **and memo event append** (gate finding fe7cfc2e, blocker 5)
- current writer/call path: `helper_memo_main`
- locks (ordered): TBD
- atomicity boundary: TBD — tsv row + event are two writes; torn state possible (contract row)
- current owner: bash
- planned owner/fate: **rust at P2**
```

Evidence (CENSUS/CODE schema): `D09 | CENSUS/CODE | artifact=fdef6d11dc2cdee78433f79dbde83cd9f344157f | path=docs/migration/evidence/locks-census.md | heading=## `memo add` (`helper_memo_main`) | recheck=verify cited ae locations against git show 72c7293:ae`

Ability-to-fail / coverage: memo census fills memo/event effects, lock order, direct append and torn-state residues from ae:13171-14523; frozen citations rechecked.

Verdict: **PROVES**

### D10 — PROVES

Claim (exact):
```
### D10 — chat (`say`)

- effects: `chat` event append (bridge consumes)
- current writer/call path: `helper_say_main`
- locks (ordered): TBD
- atomicity boundary: TBD
- current owner: bash
- planned owner/fate: **rust at P2**
```

Evidence (CENSUS/CODE schema): `D10 | CENSUS/CODE | artifact=fdef6d11dc2cdee78433f79dbde83cd9f344157f | path=docs/migration/evidence/locks-census.md | heading=## `say` (`helper_say_main`) | recheck=verify cited ae locations against git show 72c7293:ae`

Ability-to-fail / coverage: say census fills chat-event-only effect, writer, events lock, direct append, and partial-line residue from ae:13171-14485.

Verdict: **PROVES**

### D11 — PROVES

Claim (exact):
```
### D11 — `interrupt`

- effects: pane key injection (cancel), optional follow-up delivery, `interrupt` event
- current writer/call path: `helper_interrupt_main`
- locks (ordered): **target lock fd9 → event lock fd8, fd9 HELD across the event append**
  — DIVERGES from D06 `send`, which releases fd9 before taking fd8 (colead finding,
  lead-verified 2026-08-20: `ae_lock_target` with no release before `ae_emit_event`).
  The Rust owner must pick ONE order for the pair — contract row candidate
- atomicity boundary: TBD
- current owner: bash
- planned owner/fate: **rust at P2** — one operation including its tmux calls
```

Evidence (CENSUS/CODE schema): `D11 | CENSUS/CODE | artifact=fdef6d11dc2cdee78433f79dbde83cd9f344157f | path=docs/migration/evidence/locks-census-2.md | heading=## `interrupt` (`helper_interrupt_main`) | recheck=verify cited ae locations against git show 72c7293:ae`

Ability-to-fail / coverage: interrupt census fills pane cancellation/follow-up, target→event lock order, direct event append, and failure residue from ae:12993-14700.

Verdict: **PROVES**

### D15 — PROVES

Claim (exact):
```
### D15 — `spawn`

- effects: meta entry, `workspace.md` manifest, tmux window/pane creation, launch script,
  brief delivery (readiness-gated), events
- current writer/call path: `_cmd_spawn` / `helper_spawn_main`
- locks (ordered): TBD (meta lock; event via `_spawn_emit_event`)
- atomicity boundary (citation-audit findings, verified): meta-lock timeout occurs AFTER
  `tmux new-window` — an unregistered live pane with no rollback; `_spawn_emit_event`
  swallows append failure (`return 0`, ae:12113-12114) — spawn can REPORT SUCCESS with no
  event recorded (completion-without-delivery class). Both are contract row candidates
- current owner: bash
- planned owner/fate: **rust at P3**
```

Evidence (CENSUS/CODE schema): `D15 | CENSUS/CODE | artifact=fdef6d11dc2cdee78433f79dbde83cd9f344157f | path=docs/migration/evidence/locks-census.md | heading=## `spawn` (`_cmd_spawn`) | recheck=verify cited ae locations against git show 72c7293:ae`

Ability-to-fail / coverage: spawn census covers tmux/meta/manifest/launch/event effects, lock acquisitions, direct and atomic writes, async capture and rollback residues.

Verdict: **PROVES**

### D16 — PROVES

Claim (exact):
```
### D16 — `retire`

- effects: pane kill, meta removal, manifest update, events (inline writer, ae:12262)
- current writer/call path: `helper_retire_main`
- locks (ordered): TBD
- atomicity boundary: event append failure returns NONZERO — but only after pane, meta,
  and artifact mutations already landed (citation-audit finding): the operation fails
  loudly yet leaves its effects
- current owner: bash
- planned owner/fate: **rust at P3**
```

Evidence (CENSUS/CODE schema): `D16 | CENSUS/CODE | artifact=fdef6d11dc2cdee78433f79dbde83cd9f344157f | path=docs/migration/evidence/locks-census.md | heading=## `retire` (`helper_retire_main` + `_cmd_retire`) | recheck=verify cited ae locations against git show 72c7293:ae`

Ability-to-fail / coverage: retire census covers pane kill, meta/artifact/manifest/event effects, meta→event lock order, direct append and post-mutation event-failure residue.

Verdict: **PROVES**

### D17 — PROVES

Claim (exact):
```
### D17 — session launch (`ae [name]`, modes, `--from <uuid>`)

- effects: session dir + meta + helpers + workspace.md, tmux session/panes, per-session
  ae-monitor window (`_monitor_ensure_events_pane`), worktree/copy creation, launch
  rollback (`rm -rf` of the validated name), archive inheritance. Launch delivery is a
  direct fire-and-forget paste — NO target lock, NO body, NO event; paste failure is
  IGNORED (ae:12608-12613). Contract-row candidates (census-2 audit): launch-script
  publication failure vs rollback; ignored paste failure = reported success with an
  unstarted agent (completion-without-delivery); deferred Codex-resume prompt failure
  leaves durable evidence (`undelivered.launch-*`, `launch-delivery-failed`) but the
  parent launch already reported success
- current writer/call path: launch family
- locks (ordered): `.lifecycle.<name>.lock` on fd 8 (NOT the event lock — fd number
  reuse), `flock -w 15`, and **degrades to UNLOCKED with a one-line note when flock is
  absent** (ae:17288-17298; #75 material) + TBD
- atomicity boundary: rollback contract on failed launch — TBD
- current owner: bash
- planned owner/fate: **rust at P3 — OPEN-DESIGN, due at P3 entry (narrowed per gate
  finding a1358882):** the launch transaction must split into whole operations at a
  defined interface (direction: Rust `prepare-session` owning state + rollback; a second
  operation owning tmux realization). What the design must cover before any ruling:
  op2's REAL effect set is not just attach — tmux session/window/pane creation, agent
  launch, async SID capture (D13), per-session ae-monitor window
  (`_monitor_ensure_events_pane`), watchdog/telegram/steward hooks; `tmux attach` runs
  after `new-session`, so an attach failure can leave a LIVE session (a "no state writes /
  stopped-but-launchable" claim is false without a rollback matrix); durable
  prepared-state marker + idempotent re-entry + full failure/rollback matrix required.
  Until ruled, no record may assert the boundary. Helpers/roster operations that touch
  tmux are unaffected: they own their tmux calls inside their single (Rust) operation.
```

Evidence (CENSUS/CODE schema): `D17 | CENSUS/CODE | artifact=fdef6d11dc2cdee78433f79dbde83cd9f344157f | path=docs/migration/evidence/locks-census-2.md | heading=## Session launch (`ae [name]`, including `--from`) | recheck=verify cited ae locations against git show 72c7293:ae`

Ability-to-fail / coverage: launch census covers modes, lifecycle lock, tmux/meta/assets/manifest/monitor/worktree effects, rollback and fire-and-forget delivery residues.

Verdict: **PROVES**

### D19a — PROVES

Claim (exact):
```
### D19a — external singular `stop`

- effects: tmux teardown without archive/removal; NO event (census-2 audit)
- current writer/call path: `cmd_stop` (resolves via session lookup, not raw paths — measured)
- locks (ordered): lifecycle lock per census-2; TBD detail
- current owner: bash
- planned owner/fate: **rust at P3**
```

Evidence (CENSUS/CODE schema): `D19a | CENSUS/CODE | artifact=fdef6d11dc2cdee78433f79dbde83cd9f344157f | path=docs/migration/evidence/locks-census-2.md | heading=## `stop` (`cmd_stop` and supervisors) | recheck=verify cited ae locations against git show 72c7293:ae`

Ability-to-fail / coverage: stop census covers singular lifecycle lock, exact tmux teardown, no archive/removal, no event, and supervisor ordering/residue.

Verdict: **PROVES**

### D20 — PROVES

Claim (exact):
```
### D20 — `rename`

- effects: meta, tmux session name, session dir move, `.lifecycle.<name>.lock` identity
- current writer/call path: `cmd_rename` (target name strict-validated)
- locks (ordered): BOTH names' lifecycle locks — and then rewrites meta WITHOUT
  `meta.lock` (ae:11597-11667): unserialized against helper meta writers (fd200),
  empirical race window, own contract row
- atomicity boundary: TBD (dir moved but tmux rename fails → ?)
- current owner: bash
- planned owner/fate: **rust at P3**
```

Evidence (CENSUS/CODE schema): `D20 | CENSUS/CODE | artifact=fdef6d11dc2cdee78433f79dbde83cd9f344157f | path=docs/migration/evidence/locks-census-2.md | heading=## `rename` (`cmd_rename`) | recheck=verify cited ae locations against git show 72c7293:ae`

Ability-to-fail / coverage: rename census covers both lifecycle locks, tmux/dir/meta/manifest writes, no meta lock, and each crash cut.

Verdict: **PROVES**

### D21 — PROVES

Claim (exact):
```
### D21 — `transfer`

- effects: rsync both directions, SSH probe, dest `mkdir`, name validation both ends
- current writer/call path: `cmd_transfer`
- locks (ordered): TBD
- atomicity boundary: TBD (partial rsync → ?); audit event is BEST-EFFORT — after
  stop+rsync succeed, event failure on either side is warned and transfer still reports
  success (ae:11530-11535): per-direction contract rows
- current owner: bash
- planned owner/fate: **rust at P3**
```

Evidence (CENSUS/CODE schema): `D21 | CENSUS/CODE | artifact=fdef6d11dc2cdee78433f79dbde83cd9f344157f | path=docs/migration/evidence/locks-census-2.md | heading=## `transfer` (`cmd_transfer`) | recheck=verify cited ae locations against git show 72c7293:ae`

Ability-to-fail / coverage: transfer census covers both directions, stop/rsync/event effects, lock boundaries, direct rsync partial residue and best-effort event.

Verdict: **PROVES**

### D22 — PROVES

Claim (exact):
```
### D22 — compact / handover (self-continue under same name)

- effects (complete set — gate findings fe7cfc2e b5 + b29dac92 imp2): four-line stdout
  contract, frozen roster, archive capture (source handover memo is captured in the
  archive/digest and REFERENCED by the successor — never copied into the child's live
  memo), request-state disposition, events, predecessor teardown, successor launch
- current writer/call path: `cmd_compact`
- locks (ordered): `.lifecycle.<name>.lock` fd8 `flock -w 15` at the
  revalidate-after-confirmation boundary (ae:6272-6280) + TBD
- atomicity boundary: TBD
- current owner: bash
- planned owner/fate: **rust at P3**
```

Evidence (CENSUS/CODE schema): `D22 | CENSUS/CODE | artifact=fdef6d11dc2cdee78433f79dbde83cd9f344157f | path=docs/migration/evidence/locks-census-2.md | heading=## `compact` (`cmd_compact`) | recheck=verify cited ae locations against git show 72c7293:ae`

Ability-to-fail / coverage: compact census covers freeze/handover/end/relaunch effects, lifecycle/meta/events locks, stdout/recovery publication and crash residues.

Verdict: **PROVES**

### D28a — PROVES

Claim (exact):
```
### D28a — telegram setup (`ae telegram setup`)

- effects: token/config writes ONLY (ae:10550-10607, verified — no publication, no tmux)
- current writer/call path: `cmd_telegram_setup`
- locks / atomicity: TBD
- current owner: bash
- planned owner/fate: **rust at P4**
```

Evidence (CENSUS/CODE schema): `D28a | CENSUS/CODE | artifact=fdef6d11dc2cdee78433f79dbde83cd9f344157f | path=docs/migration/evidence/locks-census-2.md | heading=## Telegram setup/start/stop and daemon loop | recheck=verify cited ae locations against git show 72c7293:ae`

Ability-to-fail / coverage: Telegram census setup heading cites token/config-only writes, no lock, direct modes and partial config/token residue.

Verdict: **PROVES**

### D28b — PROVES

Claim (exact):
```
### D28b — telegram start/stop (+ autostart)

- effects: the AUTHORITATIVE config mutation `telegram.enabled` persisted BEFORE
  spawn/kill (ae:10288-10310, ae:10628-10678 — the durable effect and residue; the
  control-lock file is mechanism), machine-global telegram-daemon script publication via
  M3 (ae:9305-9308, ae:10315-10346, ae:10628-10655 — moved here from setup per census-2
  audit), tmux creation, daemon lifecycle
- current writer/call path: `cmd_telegram_start` / `cmd_telegram_stop` / autostart path
- locks / atomicity: TBD
- current owner: bash
- planned owner/fate: **rust at P4**
```

Evidence (CENSUS/CODE schema): `D28b | CENSUS/CODE | artifact=fdef6d11dc2cdee78433f79dbde83cd9f344157f | path=docs/migration/evidence/locks-census-2.md | heading=## Telegram setup/start/stop and daemon loop | recheck=verify cited ae locations against git show 72c7293:ae`

Ability-to-fail / coverage: Telegram census start/stop heading cites control lock, enabled persistence before tmux, artifact publication, daemon lock/lifecycle and crash residues.

Verdict: **PROVES**

### D29b — PROVES

Claim (exact):
```
### D29b — steward session launch

- effects: detached steward session launch (isolated config), autostart hook
  (`AE_NO_AUTOSTART` gate). The per-session ae-monitor window is NOT here: it is created
  by `_monitor_ensure_events_pane` on every session launch — a D17 effect (gate finding
  a1358882)
- current writer/call path: `cmd_steward` family
- locks / atomicity: TBD
- current owner: **bash (product surface)**
- planned owner/fate: **rust at P4** — one operation including its tmux calls
```

Evidence (CENSUS/CODE schema): `D29b | CENSUS/CODE | artifact=fdef6d11dc2cdee78433f79dbde83cd9f344157f | path=docs/migration/evidence/locks-census-2.md | heading=## Steward (`cmd_steward*`, autostart, and runtime) | recheck=verify cited ae locations against git show 72c7293:ae`

Ability-to-fail / coverage: Steward census distinguishes scaffold/help/autostart/generic launch effects and confirms monitor is D17; writer/lock/residue paths are cited.

Verdict: **PROVES**

## Probe-design queue — 338 mechanical MISSING-OR-STALE entries

Every row below has `artifact=not-yet-run` in the closure map. It is not evidence and is not an executable probe design. Each remains in the queue by stable ID; this audit does not invent expected results or replacement specs.

- SC-011 — **MISSING-OR-STALE** — SC-011 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-012 — **MISSING-OR-STALE** — SC-012 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-013 — **MISSING-OR-STALE** — SC-013 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-014 — **MISSING-OR-STALE** — SC-014 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-016a — **MISSING-OR-STALE** — SC-016a | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-016b — **MISSING-OR-STALE** — SC-016b | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-016c — **MISSING-OR-STALE** — SC-016c | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-016d — **MISSING-OR-STALE** — SC-016d | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-017a — **MISSING-OR-STALE** — SC-017a | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-017b — **MISSING-OR-STALE** — SC-017b | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-017c — **MISSING-OR-STALE** — SC-017c | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-017d — **MISSING-OR-STALE** — SC-017d | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-017e — **MISSING-OR-STALE** — SC-017e | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-017f — **MISSING-OR-STALE** — SC-017f | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-017g — **MISSING-OR-STALE** — SC-017g | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-017h — **MISSING-OR-STALE** — SC-017h | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-017i — **MISSING-OR-STALE** — SC-017i | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-018 — **MISSING-OR-STALE** — SC-018 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-018b — **MISSING-OR-STALE** — SC-018b | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-019 — **MISSING-OR-STALE** — SC-019 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-020a — **MISSING-OR-STALE** — SC-020a | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-020b — **MISSING-OR-STALE** — SC-020b | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-020c — **MISSING-OR-STALE** — SC-020c | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-100 — **MISSING-OR-STALE** — SC-100 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-101 — **MISSING-OR-STALE** — SC-101 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-102a — **MISSING-OR-STALE** — SC-102a | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-102b — **MISSING-OR-STALE** — SC-102b | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-200 — **MISSING-OR-STALE** — SC-200 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-201 — **MISSING-OR-STALE** — SC-201 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-202 — **MISSING-OR-STALE** — SC-202 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-203 — **MISSING-OR-STALE** — SC-203 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-204 — **MISSING-OR-STALE** — SC-204 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-205 — **MISSING-OR-STALE** — SC-205 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-206 — **MISSING-OR-STALE** — SC-206 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-207 — **MISSING-OR-STALE** — SC-207 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-208 — **MISSING-OR-STALE** — SC-208 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-209a — **MISSING-OR-STALE** — SC-209a | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-209b — **MISSING-OR-STALE** — SC-209b | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-209c — **MISSING-OR-STALE** — SC-209c | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-209d — **MISSING-OR-STALE** — SC-209d | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-210 — **MISSING-OR-STALE** — SC-210 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-211a — **MISSING-OR-STALE** — SC-211a | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-211b — **MISSING-OR-STALE** — SC-211b | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-211c — **MISSING-OR-STALE** — SC-211c | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-211d — **MISSING-OR-STALE** — SC-211d | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-211e — **MISSING-OR-STALE** — SC-211e | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-211f — **MISSING-OR-STALE** — SC-211f | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-211g — **MISSING-OR-STALE** — SC-211g | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-211h — **MISSING-OR-STALE** — SC-211h | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-211i — **MISSING-OR-STALE** — SC-211i | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-211j — **MISSING-OR-STALE** — SC-211j | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-211k — **MISSING-OR-STALE** — SC-211k | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-211l — **MISSING-OR-STALE** — SC-211l | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-211m — **MISSING-OR-STALE** — SC-211m | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-211n — **MISSING-OR-STALE** — SC-211n | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-211o — **MISSING-OR-STALE** — SC-211o | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-211p — **MISSING-OR-STALE** — SC-211p | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-212a — **MISSING-OR-STALE** — SC-212a | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-212b — **MISSING-OR-STALE** — SC-212b | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-212c — **MISSING-OR-STALE** — SC-212c | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-212d — **MISSING-OR-STALE** — SC-212d | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-212e — **MISSING-OR-STALE** — SC-212e | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-212f — **MISSING-OR-STALE** — SC-212f | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-212g — **MISSING-OR-STALE** — SC-212g | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-212h — **MISSING-OR-STALE** — SC-212h | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-212i — **MISSING-OR-STALE** — SC-212i | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-212j — **MISSING-OR-STALE** — SC-212j | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-212k — **MISSING-OR-STALE** — SC-212k | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-212l — **MISSING-OR-STALE** — SC-212l | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-212m — **MISSING-OR-STALE** — SC-212m | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-212n — **MISSING-OR-STALE** — SC-212n | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-212o — **MISSING-OR-STALE** — SC-212o | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-212p — **MISSING-OR-STALE** — SC-212p | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-212q — **MISSING-OR-STALE** — SC-212q | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-212r — **MISSING-OR-STALE** — SC-212r | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-212s — **MISSING-OR-STALE** — SC-212s | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-300a — **MISSING-OR-STALE** — SC-300a | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-300b — **MISSING-OR-STALE** — SC-300b | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-300c — **MISSING-OR-STALE** — SC-300c | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-301 — **MISSING-OR-STALE** — SC-301 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-302 — **MISSING-OR-STALE** — SC-302 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-303 — **MISSING-OR-STALE** — SC-303 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-304 — **MISSING-OR-STALE** — SC-304 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-305 — **MISSING-OR-STALE** — SC-305 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-306 — **MISSING-OR-STALE** — SC-306 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-307 — **MISSING-OR-STALE** — SC-307 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-400a — **MISSING-OR-STALE** — SC-400a | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-400b — **MISSING-OR-STALE** — SC-400b | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-400c — **MISSING-OR-STALE** — SC-400c | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-401a — **MISSING-OR-STALE** — SC-401a | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-401b — **MISSING-OR-STALE** — SC-401b | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-402 — **MISSING-OR-STALE** — SC-402 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-403 — **MISSING-OR-STALE** — SC-403 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-404 — **MISSING-OR-STALE** — SC-404 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-505a — **MISSING-OR-STALE** — SC-505a | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-505b — **MISSING-OR-STALE** — SC-505b | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-506 — **MISSING-OR-STALE** — SC-506 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-507a — **MISSING-OR-STALE** — SC-507a | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-507b — **MISSING-OR-STALE** — SC-507b | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-508 — **MISSING-OR-STALE** — SC-508 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-510b — **MISSING-OR-STALE** — SC-510b | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-511c — **MISSING-OR-STALE** — SC-511c | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-517a — **MISSING-OR-STALE** — SC-517a | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-517b — **MISSING-OR-STALE** — SC-517b | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-517c — **MISSING-OR-STALE** — SC-517c | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-600 — **MISSING-OR-STALE** — SC-600 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-601 — **MISSING-OR-STALE** — SC-601 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-602 — **MISSING-OR-STALE** — SC-602 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-603 — **MISSING-OR-STALE** — SC-603 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-604 — **MISSING-OR-STALE** — SC-604 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-700 — **MISSING-OR-STALE** — SC-700 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-701 — **MISSING-OR-STALE** — SC-701 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-703 — **MISSING-OR-STALE** — SC-703 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-704 — **MISSING-OR-STALE** — SC-704 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-704a — **MISSING-OR-STALE** — SC-704a | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-704b — **MISSING-OR-STALE** — SC-704b | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-704c — **MISSING-OR-STALE** — SC-704c | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-704d — **MISSING-OR-STALE** — SC-704d | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-704e — **MISSING-OR-STALE** — SC-704e | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-705 — **MISSING-OR-STALE** — SC-705 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-706 — **MISSING-OR-STALE** — SC-706 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-800 — **MISSING-OR-STALE** — SC-800 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-802 — **MISSING-OR-STALE** — SC-802 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-813 — **MISSING-OR-STALE** — SC-813 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-817 — **MISSING-OR-STALE** — SC-817 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-827 — **MISSING-OR-STALE** — SC-827 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-832a — **MISSING-OR-STALE** — SC-832a | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-833a — **MISSING-OR-STALE** — SC-833a | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-834a — **MISSING-OR-STALE** — SC-834a | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-835a — **MISSING-OR-STALE** — SC-835a | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-835b — **MISSING-OR-STALE** — SC-835b | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-835c — **MISSING-OR-STALE** — SC-835c | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-835d — **MISSING-OR-STALE** — SC-835d | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-835e — **MISSING-OR-STALE** — SC-835e | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-835f — **MISSING-OR-STALE** — SC-835f | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-835g — **MISSING-OR-STALE** — SC-835g | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-835h — **MISSING-OR-STALE** — SC-835h | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-836 — **MISSING-OR-STALE** — SC-836 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-837 — **MISSING-OR-STALE** — SC-837 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-838a — **MISSING-OR-STALE** — SC-838a | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-838b — **MISSING-OR-STALE** — SC-838b | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-839a — **MISSING-OR-STALE** — SC-839a | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-839b — **MISSING-OR-STALE** — SC-839b | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-839c — **MISSING-OR-STALE** — SC-839c | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-839d — **MISSING-OR-STALE** — SC-839d | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-839e — **MISSING-OR-STALE** — SC-839e | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-900 — **MISSING-OR-STALE** — SC-900 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-901 — **MISSING-OR-STALE** — SC-901 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-902 — **MISSING-OR-STALE** — SC-902 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-903 — **MISSING-OR-STALE** — SC-903 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-904 — **MISSING-OR-STALE** — SC-904 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-905 — **MISSING-OR-STALE** — SC-905 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-906 — **MISSING-OR-STALE** — SC-906 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-907 — **MISSING-OR-STALE** — SC-907 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-908 — **MISSING-OR-STALE** — SC-908 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-909 — **MISSING-OR-STALE** — SC-909 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-910 — **MISSING-OR-STALE** — SC-910 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-911 — **MISSING-OR-STALE** — SC-911 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-912 — **MISSING-OR-STALE** — SC-912 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-913 — **MISSING-OR-STALE** — SC-913 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-914 — **MISSING-OR-STALE** — SC-914 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-915 — **MISSING-OR-STALE** — SC-915 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-916 — **MISSING-OR-STALE** — SC-916 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-917 — **MISSING-OR-STALE** — SC-917 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-918 — **MISSING-OR-STALE** — SC-918 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-919 — **MISSING-OR-STALE** — SC-919 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-920 — **MISSING-OR-STALE** — SC-920 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-921 — **MISSING-OR-STALE** — SC-921 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-922 — **MISSING-OR-STALE** — SC-922 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-923 — **MISSING-OR-STALE** — SC-923 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-924 — **MISSING-OR-STALE** — SC-924 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-925 — **MISSING-OR-STALE** — SC-925 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-926 — **MISSING-OR-STALE** — SC-926 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-927 — **MISSING-OR-STALE** — SC-927 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-928 — **MISSING-OR-STALE** — SC-928 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-929 — **MISSING-OR-STALE** — SC-929 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-930 — **MISSING-OR-STALE** — SC-930 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-931 — **MISSING-OR-STALE** — SC-931 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-932 — **MISSING-OR-STALE** — SC-932 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-933 — **MISSING-OR-STALE** — SC-933 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-934 — **MISSING-OR-STALE** — SC-934 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-935 — **MISSING-OR-STALE** — SC-935 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-936 — **MISSING-OR-STALE** — SC-936 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-937 — **MISSING-OR-STALE** — SC-937 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-938 — **MISSING-OR-STALE** — SC-938 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-939a — **MISSING-OR-STALE** — SC-939a | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-939b — **MISSING-OR-STALE** — SC-939b | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-939c — **MISSING-OR-STALE** — SC-939c | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-939d — **MISSING-OR-STALE** — SC-939d | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-939e — **MISSING-OR-STALE** — SC-939e | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-939f — **MISSING-OR-STALE** — SC-939f | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-940 — **MISSING-OR-STALE** — SC-940 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-941 — **MISSING-OR-STALE** — SC-941 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-942 — **MISSING-OR-STALE** — SC-942 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-943 — **MISSING-OR-STALE** — SC-943 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-944a — **MISSING-OR-STALE** — SC-944a | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-944b — **MISSING-OR-STALE** — SC-944b | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-944c — **MISSING-OR-STALE** — SC-944c | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-945 — **MISSING-OR-STALE** — SC-945 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-946 — **MISSING-OR-STALE** — SC-946 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-947 — **MISSING-OR-STALE** — SC-947 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-948 — **MISSING-OR-STALE** — SC-948 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-949 — **MISSING-OR-STALE** — SC-949 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-950 — **MISSING-OR-STALE** — SC-950 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-951 — **MISSING-OR-STALE** — SC-951 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-952 — **MISSING-OR-STALE** — SC-952 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-953 — **MISSING-OR-STALE** — SC-953 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-954 — **MISSING-OR-STALE** — SC-954 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-955 — **MISSING-OR-STALE** — SC-955 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-956 — **MISSING-OR-STALE** — SC-956 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-957 — **MISSING-OR-STALE** — SC-957 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-958 — **MISSING-OR-STALE** — SC-958 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-959 — **MISSING-OR-STALE** — SC-959 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-960 — **MISSING-OR-STALE** — SC-960 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-961 — **MISSING-OR-STALE** — SC-961 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-962 — **MISSING-OR-STALE** — SC-962 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-963 — **MISSING-OR-STALE** — SC-963 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-964 — **MISSING-OR-STALE** — SC-964 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-965 — **MISSING-OR-STALE** — SC-965 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-966 — **MISSING-OR-STALE** — SC-966 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-967 — **MISSING-OR-STALE** — SC-967 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-968 — **MISSING-OR-STALE** — SC-968 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-969 — **MISSING-OR-STALE** — SC-969 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-970 — **MISSING-OR-STALE** — SC-970 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-971 — **MISSING-OR-STALE** — SC-971 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-972 — **MISSING-OR-STALE** — SC-972 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-973a — **MISSING-OR-STALE** — SC-973a | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-973b — **MISSING-OR-STALE** — SC-973b | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-974a — **MISSING-OR-STALE** — SC-974a | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-974b — **MISSING-OR-STALE** — SC-974b | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-975a — **MISSING-OR-STALE** — SC-975a | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-975b — **MISSING-OR-STALE** — SC-975b | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-976a — **MISSING-OR-STALE** — SC-976a | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-976b — **MISSING-OR-STALE** — SC-976b | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-977 — **MISSING-OR-STALE** — SC-977 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-978a — **MISSING-OR-STALE** — SC-978a | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-978b — **MISSING-OR-STALE** — SC-978b | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-979a — **MISSING-OR-STALE** — SC-979a | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-979b — **MISSING-OR-STALE** — SC-979b | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1000 — **MISSING-OR-STALE** — SC-1000 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1001 — **MISSING-OR-STALE** — SC-1001 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1002 — **MISSING-OR-STALE** — SC-1002 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1003 — **MISSING-OR-STALE** — SC-1003 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1004 — **MISSING-OR-STALE** — SC-1004 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1005 — **MISSING-OR-STALE** — SC-1005 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1006 — **MISSING-OR-STALE** — SC-1006 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1100 — **MISSING-OR-STALE** — SC-1100 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1101a — **MISSING-OR-STALE** — SC-1101a | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1101b — **MISSING-OR-STALE** — SC-1101b | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1103 — **MISSING-OR-STALE** — SC-1103 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1104 — **MISSING-OR-STALE** — SC-1104 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1200 — **MISSING-OR-STALE** — SC-1200 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1202 — **MISSING-OR-STALE** — SC-1202 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1205a — **MISSING-OR-STALE** — SC-1205a | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1205b — **MISSING-OR-STALE** — SC-1205b | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1207a — **MISSING-OR-STALE** — SC-1207a | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1208 — **MISSING-OR-STALE** — SC-1208 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1300 — **MISSING-OR-STALE** — SC-1300 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1301 — **MISSING-OR-STALE** — SC-1301 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1302 — **MISSING-OR-STALE** — SC-1302 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1303 — **MISSING-OR-STALE** — SC-1303 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1304a — **MISSING-OR-STALE** — SC-1304a | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1304b — **MISSING-OR-STALE** — SC-1304b | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1304c — **MISSING-OR-STALE** — SC-1304c | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1304d — **MISSING-OR-STALE** — SC-1304d | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1305 — **MISSING-OR-STALE** — SC-1305 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1306a — **MISSING-OR-STALE** — SC-1306a | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1306b — **MISSING-OR-STALE** — SC-1306b | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1306c — **MISSING-OR-STALE** — SC-1306c | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1306d — **MISSING-OR-STALE** — SC-1306d | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1306e — **MISSING-OR-STALE** — SC-1306e | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1400 — **MISSING-OR-STALE** — SC-1400 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1401 — **MISSING-OR-STALE** — SC-1401 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1402 — **MISSING-OR-STALE** — SC-1402 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1403 — **MISSING-OR-STALE** — SC-1403 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1404a — **MISSING-OR-STALE** — SC-1404a | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1404b — **MISSING-OR-STALE** — SC-1404b | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1405a — **MISSING-OR-STALE** — SC-1405a | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1405b — **MISSING-OR-STALE** — SC-1405b | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1406a — **MISSING-OR-STALE** — SC-1406a | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1406b — **MISSING-OR-STALE** — SC-1406b | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1407a — **MISSING-OR-STALE** — SC-1407a | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1407b — **MISSING-OR-STALE** — SC-1407b | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1408a — **MISSING-OR-STALE** — SC-1408a | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1408b — **MISSING-OR-STALE** — SC-1408b | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1409a — **MISSING-OR-STALE** — SC-1409a | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1409b — **MISSING-OR-STALE** — SC-1409b | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1409c — **MISSING-OR-STALE** — SC-1409c | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1410a — **MISSING-OR-STALE** — SC-1410a | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1410b — **MISSING-OR-STALE** — SC-1410b | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1410c — **MISSING-OR-STALE** — SC-1410c | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1410d — **MISSING-OR-STALE** — SC-1410d | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1410e — **MISSING-OR-STALE** — SC-1410e | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1410f — **MISSING-OR-STALE** — SC-1410f | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1410g — **MISSING-OR-STALE** — SC-1410g | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1410h — **MISSING-OR-STALE** — SC-1410h | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1410i — **MISSING-OR-STALE** — SC-1410i | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1410j — **MISSING-OR-STALE** — SC-1410j | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1410k — **MISSING-OR-STALE** — SC-1410k | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1410l — **MISSING-OR-STALE** — SC-1410l | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1411a — **MISSING-OR-STALE** — SC-1411a | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1411b — **MISSING-OR-STALE** — SC-1411b | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1411c — **MISSING-OR-STALE** — SC-1411c | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1412a — **MISSING-OR-STALE** — SC-1412a | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1412b — **MISSING-OR-STALE** — SC-1412b | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1412c — **MISSING-OR-STALE** — SC-1412c | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1412d — **MISSING-OR-STALE** — SC-1412d | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1412e — **MISSING-OR-STALE** — SC-1412e | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1412f — **MISSING-OR-STALE** — SC-1412f | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- SC-1412g — **MISSING-OR-STALE** — SC-1412g | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the row operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- D01 — **MISSING-OR-STALE** — D01 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the D operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- D02 — **MISSING-OR-STALE** — D02 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the D operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- D03 — **MISSING-OR-STALE** — D03 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the D operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- D04a — **MISSING-OR-STALE** — D04a | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the D operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- D04b — **MISSING-OR-STALE** — D04b | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the D operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- D05 — **MISSING-OR-STALE** — D05 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the D operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- D06 — **MISSING-OR-STALE** — D06 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the D operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- D07 — **MISSING-OR-STALE** — D07 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the D operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- D12 — **MISSING-OR-STALE** — D12 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the D operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- D13 — **MISSING-OR-STALE** — D13 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the D operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- D14 — **MISSING-OR-STALE** — D14 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the D operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- D14a — **MISSING-OR-STALE** — D14a | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the D operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- D14b — **MISSING-OR-STALE** — D14b | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the D operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- D18 — **MISSING-OR-STALE** — D18 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the D operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- D19b — **MISSING-OR-STALE** — D19b | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the D operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- D19c — **MISSING-OR-STALE** — D19c | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the D operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- D23 — **MISSING-OR-STALE** — D23 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the D operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- D24 — **MISSING-OR-STALE** — D24 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the D operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- D31 — **MISSING-OR-STALE** — D31 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the D operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- D25 — **MISSING-OR-STALE** — D25 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the D operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- D26a — **MISSING-OR-STALE** — D26a | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the D operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- D26b — **MISSING-OR-STALE** — D26b | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the D operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- D27 — **MISSING-OR-STALE** — D27 | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the D operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- D28c — **MISSING-OR-STALE** — D28c | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the D operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- D29a — **MISSING-OR-STALE** — D29a | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the D operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- D30a — **MISSING-OR-STALE** — D30a | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the D operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- D30b — **MISSING-OR-STALE** — D30b | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the D operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification
- D30c — **MISSING-OR-STALE** — D30c | PROBE | artifact=not-yet-run | environment=frozen 72c7293 checkout; isolated AE_HOME; fixed fixtures; no live-model query | manipulate=deterministic fault hook at the D operation boundary | barriers=freeze inputs and explicit before/after barriers; no timing races | capture=stdout,stderr,rc,files,tmux state | expected-values=omitted for seat classification

## Exact input ID set

The following is the exact stable-ID input set audited (canonical map order; equality is required, not a count-only claim).

### SC (398)

```
SC-011 SC-012 SC-013 SC-014 SC-016a SC-016b SC-016c SC-016d SC-017a SC-017b SC-017c SC-017d SC-017e SC-017f SC-017g SC-017h SC-017i SC-018 SC-018b SC-019 SC-020a SC-020b SC-020c SC-100 SC-101 SC-102a SC-102b SC-200 SC-201 SC-202 SC-203 SC-204 SC-205 SC-206 SC-207 SC-208 SC-209a SC-209b SC-209c SC-209d SC-210 SC-211a SC-211b SC-211c SC-211d SC-211e SC-211f SC-211g SC-211h SC-211i SC-211j SC-211k SC-211l SC-211m SC-211n SC-211o SC-211p SC-212a SC-212b SC-212c SC-212d SC-212e SC-212f SC-212g SC-212h SC-212i SC-212j SC-212k SC-212l SC-212m SC-212n SC-212o SC-212p SC-212q SC-212r SC-212s SC-300a SC-300b SC-300c SC-301 SC-302 SC-303 SC-304 SC-305 SC-306 SC-307 SC-400a SC-400b SC-400c SC-401a SC-401b SC-402 SC-403 SC-404 SC-500 SC-501 SC-502 SC-503a SC-503b SC-504a SC-504b SC-505a SC-505b SC-506 SC-507a SC-507b SC-507c SC-507d SC-508 SC-509 SC-510a SC-510b SC-510c SC-510d SC-511a SC-511b SC-511c SC-512 SC-513a SC-513b SC-513c SC-514 SC-515a SC-515b SC-515c SC-516 SC-517a SC-517b SC-517c SC-600 SC-601 SC-602 SC-603 SC-604 SC-700 SC-701 SC-702 SC-703 SC-704 SC-704a SC-704b SC-704c SC-704d SC-704e SC-705 SC-706 SC-800 SC-801 SC-802 SC-803 SC-804a SC-804b SC-804c SC-804d SC-804e SC-804f SC-805 SC-806a SC-806b SC-807 SC-808 SC-809 SC-810a SC-810b SC-811a SC-811b SC-812 SC-813 SC-814 SC-815a SC-815b SC-815c SC-815d SC-816 SC-817 SC-818a SC-818b SC-818c SC-818d SC-818e SC-819 SC-820a SC-820b SC-821a SC-821b SC-822 SC-823 SC-824a SC-824b SC-825a SC-825b SC-825c SC-826 SC-827 SC-828 SC-829a SC-829b SC-830 SC-831 SC-832a SC-832b SC-832c SC-833a SC-833b SC-833c SC-833d SC-834a SC-834b SC-834c SC-835a SC-835b SC-835c SC-835d SC-835e SC-835f SC-835g SC-835h SC-836 SC-837 SC-838a SC-838b SC-839a SC-839b SC-839c SC-839d SC-839e SC-900 SC-901 SC-902 SC-903 SC-904 SC-905 SC-906 SC-907 SC-908 SC-909 SC-910 SC-911 SC-912 SC-913 SC-914 SC-915 SC-916 SC-917 SC-918 SC-919 SC-920 SC-921 SC-922 SC-923 SC-924 SC-925 SC-926 SC-927 SC-928 SC-929 SC-930 SC-931 SC-932 SC-933 SC-934 SC-935 SC-936 SC-937 SC-938 SC-939a SC-939b SC-939c SC-939d SC-939e SC-939f SC-940 SC-941 SC-942 SC-943 SC-944a SC-944b SC-944c SC-945 SC-946 SC-947 SC-948 SC-949 SC-950 SC-951 SC-952 SC-953 SC-954 SC-955 SC-956 SC-957 SC-958 SC-959 SC-960 SC-961 SC-962 SC-963 SC-964 SC-965 SC-966 SC-967 SC-968 SC-969 SC-970 SC-971 SC-972 SC-973a SC-973b SC-974a SC-974b SC-975a SC-975b SC-976a SC-976b SC-977 SC-978a SC-978b SC-979a SC-979b SC-1000 SC-1001 SC-1002 SC-1003 SC-1004 SC-1005 SC-1006 SC-1100 SC-1101a SC-1101b SC-1102 SC-1103 SC-1104 SC-1105 SC-1200 SC-1201 SC-1202 SC-1203 SC-1204 SC-1205a SC-1205b SC-1206 SC-1207a SC-1207b SC-1208 SC-1209 SC-1300 SC-1301 SC-1302 SC-1303 SC-1304a SC-1304b SC-1304c SC-1304d SC-1305 SC-1306a SC-1306b SC-1306c SC-1306d SC-1306e SC-1400 SC-1401 SC-1402 SC-1403 SC-1404a SC-1404b SC-1405a SC-1405b SC-1406a SC-1406b SC-1407a SC-1407b SC-1408a SC-1408b SC-1409a SC-1409b SC-1409c SC-1410a SC-1410b SC-1410c SC-1410d SC-1410e SC-1410f SC-1410g SC-1410h SC-1410i SC-1410j SC-1410k SC-1410l SC-1411a SC-1411b SC-1411c SC-1412a SC-1412b SC-1412c SC-1412d SC-1412e SC-1412f SC-1412g
```

### D (42)

```
D01 D02 D03 D04a D04b D05 D06 D07 D08 D09 D10 D11 D12 D13 D14 D14a D14b D15 D16 D17 D18 D19a D19b D19c D20 D21 D22 D23 D24 D31 D25 D26a D26b D27 D28a D28b D28c D29a D29b D30a D30b D30c
```

## Row grain

No `ROW-GRAIN-ERROR` was assigned in this audit. Each audited row head was treated as one independently testable behavior; where a pin covered only one fragment of a compound output/schema, it was marked PARTIAL rather than blessed by adjacent assertions. Pending rows were routed mechanically and not reclassified.
