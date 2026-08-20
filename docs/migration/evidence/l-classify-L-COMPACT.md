# Joint L classification — L-COMPACT worksheet (section 3 of 6)

Seats: fable5:lead (author) + gpt56sol:colead (independent read). Ordering set by
colead: L-COMPACT third because it COMPOSES L-END plus relaunch/handover semantics —
classify the dependency chain while the converged L-END evidence is fresh. L-STOP is
independent and follows.

Colead's three grain requirements for this section, honoured throughout:
1. **Raw pointers** to claim-bearing captures.
2. **Terminal-vs-downstream rc attribution kept separate** — the SC-830 lesson
   generalized: a relaunch's failure is never attributed to the operation that exec'd
   into it.
3. **Capture-only code-observation rows stay UNCLASSIFIED** pending seat ruling.

Preflight rules restated: empirical acceptance and normative `classified_by` stay
ORTHOGONAL; mismatches route through fix-known-defect or DR. **No marks proposed.**

Evidence base: L-COMPACT committed at abaeb4f, manifest corrections at 5d9d545
(v3 arm count, rc-table shape, the two-pid-columns distinction). 18 arms, 933 files;
both section SUMS verify clean. Paths under
`docs/migration/evidence/l-artifacts/L-COMPACT/arms/`. Section-wide: zero preflight
failures, zero ARM-INVALID, zero INCONCLUSIVE. L-HOOKS-v3 = v2 + three compact TRACE
CHANNELS and **no patched constant**; 5 of 18 arms ran hooked, 13 on the unmodified
frozen binary, and every ARM.txt names its patch version and binary sha256.

---

## THE ATTRIBUTION RULE FOR THIS SECTION (read before any rc below)

`compact` **execs into** the relaunch (SC-517a: "compact's exit status is the
launch's"). Two consequences govern every rc and stream reading here, and both are
stated once rather than repeated per row:

- **rc is the LAUNCH's, not compact's.** In this sandbox the launch has no controlling
  terminal, so it ends `open terminal failed: not a terminal` and the command reports
  **rc=1** even where compact itself completed and crossed its boundary. `baseline`
  is the clean demonstration: **rc=1 with a complete four-line stdout and a published
  archive.** No compact failure may be inferred from these rc values.
- **stdout outlives compact.** After the exec, the successor process writes to the
  SAME fd. `baseline/2op.stdout` therefore holds five lines: compact's four, then
  `Watchdog started in hidden ae-monitor window…`, which is the RELAUNCHED SESSION's
  own output. See SC-500 for the scope question this raises.

---

**SC-500 — compact stdout byte format: `Archived`, `Archive:`, `Digest:`,
`Recovery:` — four lines, that order, nothing else, EMPTY unless the boundary was
crossed** (b2, none). Artifacts: `baseline/2op.stdout` (5 lines),
`interactive-typed-n/2op.stdout` (**0 bytes**), `baseline/2op.stderr`.
IS (bytes): boundary CROSSED — the four lines appear in exactly the specified order
with the archive uuid, archive path, digest path and a runnable `Recovery:` command.
Boundary NOT crossed (typed `n`) — stdout is **empty, 0 bytes**, exactly as the row
requires. **SCOPE QUESTION for colead, flagged rather than called a divergence:**
`baseline`'s stdout carries a FIFTH line, `Watchdog started in hidden ae-monitor
window…`. Per the attribution rule it is the relaunched session's output after the
exec, not a fifth compact line — so "nothing else" is satisfied for compact's own
emission but NOT for the fd's lifetime. My reading is that the row's "nothing else"
scopes to what compact emits and the row should say so; a reader taking it as an fd
guarantee would call this a divergence. **Lead proposes: CONFIRMED with a scope
precision** ("compact emits exactly these four lines; after the exec the successor
process writes to the same stream"). Colead's independent reading requested before
either of us states it as settled.

**SC-501 — compact stderr carries everything else** (b2, none). Artifacts:
`baseline/2op.stderr`, `interactive-typed-n/2op.stderr`.
IS (bytes): stderr carries the end-progress lines (`Cleaned up local session cp1`,
`Ended local session cp1`), a **SECOND copy of the `Recovery:` line**, and the
relaunch announcement `Starting fresh session cp1 from archive <uuid>…`; on abort it
carries `Aborted.` (confirmed on stderr, with stdout at 0 bytes).
**Proposed: CONFIRMED / no change** — the second `Recovery:` copy is the row's
specific claim ("a broken stdout cannot destroy the only route back") and it is
present verbatim.

**SC-502 — `Recovery:` prints BEFORE the relaunch** (b1, none). Artifacts:
`baseline/2op.stderr` line order. IS (bytes): the stderr sequence is `… Ended local
session cp1` → `Recovery: cd … ae --local cp1 --from <uuid>` → `Starting fresh
session cp1 from archive <uuid>…` → `open terminal failed: not a terminal`. The
recovery route is printed before the relaunch is announced, and therefore before the
process could exec and never return. **Proposed: CONFIRMED / no change.**

**SC-503a — a typed `n` is an answer: prints `Aborted.` and exits 0** (b1, none).
Artifacts: `interactive-typed-n/2op.rc` (**0**), `2op.stdout` (**0 bytes**),
`2op.stderr` (`Aborted.`). IS (bytes): all three hold exactly.
**Proposed: CONFIRMED / no change.** Note this arm's rc=0 is compact's OWN (no exec
happened), which is what makes it a clean read against the attribution rule.

**SC-503b — end-of-input is not an answer: non-zero exit, stdout empty in BOTH cases**
(b1, none). Artifacts: `interactive-eof/2op.rc` (**1**), `2op.stdout`, `2op.stderr`.
IS: EOF yields non-zero while typed-`n` yields 0, with stdout empty in both — so exit
status is the caller's only discriminator between "operator said no" and "the question
never reached anyone", which is precisely the row's stated purpose.
**Proposed: CONFIRMED / no change** — the pair is the evidence; neither arm alone
proves the discrimination.

**SC-504b — no altered SIGPIPE disposition leaks into the child** (b1, none).
Artifacts: `sigpipe/2op.rc` (**0**), the arm's recorded producer status.
IS: recorded exactly as the kernel reported it — **WIFEXITED true, exit code 141, NOT
signalled** — with no interpretation attached by the worker. **Proposed: CONFIRMED /
no change**, with the explicit note that 141 here is a recorded exit code and the arm
deliberately does not assert what produced it.

**SC-507a / SC-507c / SC-507d — `archive preview`** (all b2, none). Arm: `preview`
(**rc=0**), plus the preview TWIN construction. IS: stdout is the digest bytes;
diagnostics (canonical archive id, source session, counts, bytes) go to stderr; the
operation writes nothing, emits no event, creates no archive.
**Construction stated (lexec, accepted by lead):** two coexisting sessions cannot
share one uuid, so the twin is a byte copy of the FROZEN stopped session directory
with exactly TWO named mode-preserving diffs — `session=` and `session_id=` — both in
`twin-meta.diff`, with `twin-vs-source.manifest.diff` showing everything else
unchanged. The arm captures the preview stdout bytes and the twin's archived digest
bytes and **compares nothing** (comparison is the seats').
**Proposed: CONFIRMED / no change** for all three.

**SC-508 — residual undocumented exit codes.** `authority=code-observation`.
Artifacts: `residual-rc/rc-table.tsv` — **54 data rows** plus a header and seven
comment lines, recording every exit status in the section with the invocation that
produced it. **Proposed: NO CLASSIFICATION — capture-only, stays UNCLASSIFIED
pending the seat preserve/fix/diverge ruling**, per colead's third grain requirement.
The table is input to that ruling, not a disposition.

**SC-512 — compact stdout truth claim: non-empty stdout proves the archive EXISTS and
the printed recovery command WORKS, and deliberately does NOT claim the fresh session
started** (b2, none). Artifacts: `baseline/2op.stdout` + `2op.rc`;
`recovery-exec-selected` (cut at pre-relaunch, **rc=137**; the printed `Recovery:`
line extracted and executed VERBATIM, rc=1).
IS — **this section's sharpest confirmation**: in `baseline` the four stdout lines are
printed AND the relaunch then fails (`not a terminal`, rc=1). The stdout claim
therefore stands while the session demonstrably did not start — exactly the
distinction the row exists to draw, observed rather than argued. The
`recovery-exec-selected` arm additionally takes the printed recovery command as bytes
and runs it. **Proposed: CONFIRMED / no change.**

**SC-517a — compact's exit status is the launch's; there is no separate compact exit**
(b2, none). Artifacts: `baseline/2op.rc` (1) with a completed compact;
`exit-identity-terminal-attach/2op.rc` (**0**); `exit-identity-no-terminal/2op.rc`
(**1**). IS: the rc tracks the LAUNCH outcome, not compact's own progress — rc=1
alongside a published archive and complete stdout, rc=0 where the relaunch reached a
real terminal. **Proposed: CONFIRMED / no change** — and this row is the authority
for the section-wide attribution rule stated above.

**SC-517b — terminal case: attach, exit on detach** (b2, none). Arm:
`exit-identity-terminal-attach` (**rc=0**): the relaunch reached a REAL terminal and
the controller detached. **Proposed: CONFIRMED / no change.**

**SC-517c — non-terminal case: launch failure reports as plain `ae <name>`, with
archive and fresh session already in place and `Recovery:` naming the route** (b2,
none). Arm: `exit-identity-no-terminal` (**rc=1**), stderr tail verbatim: `Starting
fresh session cp1 from archive bb3e4eaa-…` → `open terminal failed: not a terminal`.
IS: the failure surfaces as the plain launch's own error while the archive and the
recovery route are already in place. **Proposed: CONFIRMED / no change**, with one
precision offered: the arm evidences the failure SHAPE and the standing archive; the
"plain `ae <name>`" phrasing is best read as the launch reporting in its own voice,
which is what the capture shows.

**SC-827 — compact freezes ONE authorization tuple; everything downstream reads the
tuple, never meta again** (b1, none). Artifacts: `baseline/trace-channels.txt` (six
channels with a SITE-only legend), `barrier-order.tsv`.
IS: the six named channels fired in this order — `b_cp_resolver_entry` (the
tuple-freeze site) → `b_cp_after_answer` → `b_cp_reval_after_confirmation` →
`b_cp_after_handover` → `b_cp_reval_after_wait` → `b_cp_pre_relaunch`. The legend
names each channel's SITE and nothing else; **what the ordering MEANS is asserted
nowhere in the section**, which is the value-blindness rule observed exactly.
**Two-pid-columns note (manifest 5d9d545):** `hook-trace.tsv` records `$$` (86132 for
all six) while `barrier-order.tsv` records `${BASHPID}`, where the resolver-entry key
reads `.86134` against `.86132` elsewhere because that site runs inside a command
substitution. Any claim about a differing pid must name its source file.
**Proposed: CONFIRMED / no change** on the freeze-once claim as evidenced by the
resolver-entry channel firing first and once.

**SC-828 — two revalidations, positioned by what they protect** (b1, none).
Artifacts: `revalidation-after-answer/`, `revalidation-after-handover/`, and the two
trace channels `b_cp_reval_after_confirmation` and `b_cp_reval_after_wait` in
`baseline/trace-channels.txt`. IS: both revalidation sites exist and fire, in the
stated positions relative to the answer and the handover wait — the first after the
human's answer (so a replaced session is never MESSAGED), the second before teardown
(so a replacement is never STOPPED). **Proposed: CONFIRMED / no change**, with the
mismatch-names-the-field half resting on the two dedicated arms' captures for the
joint read.

**SC-829a — handover completion is TWO facts: a reply AND a new `handover`-topic memo
written after the request went out, polled from the event log and `memo.tsv`, never
pane output** (b1, none). Arms: `handover-withholding-only-reply`,
`-only-memo`, `-neither` — the three withholding constructions that isolate each fact.
IS: each withholding arm produces the product's own distinct at-bound report, so
supplying only one of the two facts does not complete the handover.
**PLANTED-PANE CONSTRUCTION, stated by lexec and accepted:** the frozen `reply` helper
proves the responder from the current pane's `@ae_slot`, so a controller cannot answer
a request without one; the withholding arms split a fresh pane in the main agent's own
window and set `@ae_agent`/`@ae_slot` to the source pane's values, recorded per arm in
`planted-pane.txt` **along with what it did NOT change**. The reply and memo helpers
that then run are the REAL generated ones. **Proposed: CONFIRMED / no change**, and
lexec's offer stands: if a seat reads the plant as altering what the arm observes, the
arms are re-runnable with a different responder construction.

**SC-829b — a re-run reuses the outstanding request and its baseline** (b1, none).
Arm: `handover-rerun-after-interrupt`. IS: the SAME request ref appears across both
runs, ask rows 1 and reply rows 0, with both runs' baselines captured and diffed — so
the re-run waited on the same request rather than sending a second.
**Proposed: CONFIRMED / no change.**

**SC-836 — `purge_agent_history` refuses compact unless `--keep-history`** (b1, none).
Arms: `config-keephistory-without-keep` (**rc=1**), `-with-keep` (**rc=1**, see
attribution). IS (bytes): without the override, compact refuses with
`Error: session 'cp1' has purge_agent_history enabled, which contradicts compact.`
followed by the exact override command `ae compact --keep-history cp1` — the refusal
names its own remedy. With `--keep-history` the operation proceeds and the rc=1 is the
LAUNCH's per the attribution rule, not a second refusal.
**Proposed: CONFIRMED / no change** — the pair is what discriminates refusal from
proceed; the rc alone would not.

**SC-837 — `compact -f` proceeds without asking** (b2, none). Arm:
`interactive-force` (**rc=1** = the launch's). IS: with `-f` no question is asked and
the operation proceeds to the boundary. **Proposed: CONFIRMED / no change**, with the
distinction from end's `-f` freeze semantics (SC-820b) left where the contract puts
it — this row is only the skip-confirmation surface.

**SC-1305 — (baseline arm)** (see contract row). Artifacts: `baseline/` full capture
set. IS: carried by the baseline arm's manifests and streams.
**Proposed: CONFIRMED / no change**, pending the joint read of its specific fields.

---

## Proposed dispositions

- **CONFIRMED / no change — 19**: SC-501, SC-502, SC-503a, SC-503b, SC-504b, SC-507a,
  SC-507c, SC-507d, SC-512, SC-517a, SC-517b, SC-517c, SC-827, SC-828, SC-829a,
  SC-829b, SC-836, SC-837, SC-1305.
- **CONFIRMED WITH A SCOPE PRECISION PROPOSED — 1**: SC-500 (the post-exec fifth line;
  "nothing else" scopes to compact's own emission, not the fd's lifetime). Colead's
  independent reading requested before it is settled either way.
- **UNCLASSIFIED, capture-only — 1**: SC-508 (`authority=code-observation`; the
  54-row rc table is input to a preserve/fix/diverge ruling, not a disposition).
- **NO reopened conflicts.**
- Attribution discipline applied section-wide: every rc=1 arising from
  `open terminal failed: not a terminal` is the LAUNCH's, never compact's, and is
  stated as such at each row rather than left for the reader to infer.
- Constructions stated rather than hidden: the preview TWIN (two named diffs, compares
  nothing) and the PLANTED AGENT PANE (recorded with what it did not change; real
  generated helpers; re-runnable on request).
- No INCONCLUSIVE arms in section.
