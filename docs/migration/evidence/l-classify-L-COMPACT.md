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
guarantee would call this a divergence. **RESOLVED — CONFIRMED, and my proposed scope precision is WITHDRAWN as
unnecessary.** Colead re-anchored this through the frozen authority and was right:
`docs/reference/commands.md` @72c7293 already states the attribution rule outright at
**:682-684** — *"Anything printed after the contract belongs to the fresh session:
compact `exec`s into the launch, so from there on you are reading the child."* The
fifth `Watchdog started…` line is therefore the CHILD's output by the authority's own
words, not by a precision either seat invented. Writing my precision into the row
would have presented an existing normative statement as a new seat ruling — the row
needs a CITATION, not an amendment.
*Citation correction (lead, verified before adopting):* colead's gate named
`docs/internals/commands.md`; no such file exists at 72c7293 or in the tree. The file
is `docs/reference/commands.md`. Substance unaffected; recorded because an anchor
nobody re-checked is how a wrong citation becomes load-bearing.

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
signalled** — with no interpretation attached by the worker.
**RULED: PARTIAL, not confirmed** (colead's objection, adopted). The capture records a
TERMINATION SHAPE; the row claims an INSTALLED CHILD DISPOSITION, and those are not
the same fact. Exit 141 tells us the child exited 128+13; it does not tell us what
SIGPIPE disposition the child was handed. Worse, the harness itself sets `SIG_DFL`
before the exec — so the arm cannot separate "ae leaked no altered disposition" from
"the harness reset it before anyone could observe a leak". **The arm as built cannot
fail this claim, so by the arm-that-cannot-fail rule it is not evidence for it.**
NEEDED to close: an ability-to-fail control — a variant that deliberately leaks
`SIG_IGN` into the child and is shown to be DETECTED by this same capture. Until that
control exists the row stays PARTIAL.

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
**SEAT COMPARISON, now performed and RECORDED** (colead first; independently rerun by
lead before adoption, same result). `2op.stdout` (1280 B) vs
`twin.archived-digest.md` (1291 B), 26 `- ` fields each. Raw diff rc=1 on exactly
**six line classes**, every one a field that MUST differ between a preview and a
completed archive of a differently-named twin: `Snapshot:` (preview/archived),
`Archive ID:`, `Source session:`, `Source session ID:`, `Archived at:`
(`pending` vs a real stamp), `Push outcome:` (`preview-not-run` vs `not-managed`).
Normalising those six and re-diffing gives **rc=0**.
The confirmation is in the REMAINDER, not the diff: the other **20 fields are
byte-identical**, including `Base commit`, `Final commit`, `Commit range`,
`Commit count`, `Records`, `First` and `Last`. Preview computes the same digest
CONTENT a real archive computes, and differs only where it would be lying not to —
it says `pending` because it has not archived and `preview-not-run` because it has not
pushed. **CONFIRMED** for all three, on that recorded comparison rather than on the
arm's rc alone.

**SC-508 — residual undocumented exit codes.** `authority=code-observation`.
Artifacts: `residual-rc/rc-table.tsv` — **54 data rows** plus a header and seven
comment lines, recording every exit status in the section with the invocation that
produced it. **RULED: SPLIT REQUIRED before any disposition — A CATCH-ALL CANNOT RATIFY**
(colead's objection, adopted). SC-508 as written is a bag labelled "residual", and a
bag cannot be preserved, fixed or diverged: each outcome in it has its own answer.
Ratifying the bag would ratify every unexamined member by association.
MECHANICAL SUBTRACTION FIRST — remove from the 54 rows every outcome already OWNED by
a classified row (**SC-503, SC-507, SC-512, SC-517, SC-828, SC-829, SC-836, SC-837**)
plus every harness-only kill and setup status, which are the instrument's exits and
not the product's. Then split each TRUE residual into its own row at outcome grain and
classify it individually. Only what survives subtraction was ever residual.
Stays UNCLASSIFIED until that split lands; the table is input to the ruling, never the
ruling.

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
**CONFIRMED, and re-labelled COMPOSITE EVIDENCE** (colead's precision, adopted). The
trace alone proves ORDER — that `b_cp_resolver_entry` fired first and once. It cannot
prove the NEGATIVE half of the claim ("everything downstream reads the tuple, never
meta again"), because a channel that did not fire is not a read that did not happen.
That half rests on frozen source plus the census, not on this trace. The row's
evidence is therefore **trace + frozen source/census**, and any future citation of it
must carry both — citing the trace alone would overstate what six ordered channels
can show.

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
**RULED: SEAT CLOSURE — my "CONFIRMED / no change" is WITHDRAWN as inadmissible**
(colead's objection, adopted in full). The contract row was a bare PLACEHOLDER —
"compact: mid-operation observability" — which states no SHOULD at all. **A row that
makes no claim cannot be confirmed**; marking it would have ratified a non-claim and
recorded agreement where nothing had been asserted. This is the code-observation
closure rule doing exactly its job.
**Closed by joint seat ruling instead** (both seats, 2026-08-20), rewritten at
one-invariant grain: *concurrent readers see ONE coherent lifecycle phase, never mixed
predecessor/successor state; a no-session interval between predecessor removal and
successor publication is PERMITTED.* Permitted, **not required** — a successor that
publishes without a visible gap also satisfies it, so the row cannot be read as
mandating the gap our capture happens to show. The requests-helper absence at the
pre-relaunch cut stays empirical MECHANISM, outside the claim.
Bucket 1, authority = joint seat ruling grounded in architecture.md's compact phase
order, conflict none. Empirical support: the five pre-teardown cuts each showing one
coherent running predecessor, and the pre-relaunch cut showing no session.
Landed in `semantic-contract.md` (commit e0f9e3f). **The placeholder is NOT marked as
written** — it was replaced.

---

## Proposed dispositions

*Dispositions below are POST-GATE — colead's independent read moved four of them, and
every move was away from a mark I had proposed. Recorded that way deliberately: a gate
whose findings are folded silently into the totals leaves no evidence it ran.*

- **CONFIRMED / no change — 18**: SC-501, SC-502, SC-503a, SC-503b, SC-507a, SC-507c,
  SC-507d, SC-512, SC-517a, SC-517b, SC-517c, SC-827, SC-828, SC-829a, SC-829b,
  SC-836, SC-837, **SC-500**.
  - SC-500 moved IN: the proposed scope precision was withdrawn once the frozen
    authority was found to state the attribution rule itself
    (`docs/reference/commands.md` @72c7293 :682-684). The row needed a citation, not
    an amendment.
  - SC-507a confirmed on a RECORDED seat comparison (six normalised line classes →
    diff rc=0; 20 of 26 fields byte-identical), not on the arm's rc.
  - SC-827 confirmed but re-labelled **COMPOSITE** (trace + frozen source/census):
    the trace proves order and cannot prove the claim's negative half.
- **PARTIAL — 1**: SC-504b. Moved OUT of confirmed. The capture shows a termination
  SHAPE, not an installed child DISPOSITION, and the harness sets `SIG_DFL` before the
  exec — so the arm cannot fail the claim. Needs an ability-to-fail control.
- **SEAT CLOSURE, contract row REWRITTEN — 1**: SC-1305. Moved OUT of confirmed: the
  row was a placeholder stating no SHOULD, and a non-claim cannot be confirmed.
- **SPLIT REQUIRED, stays UNCLASSIFIED — 1**: SC-508 (`authority=code-observation`).
  A catch-all cannot ratify; the 54-row table needs mechanical subtraction of
  already-owned outcomes and harness-only statuses, then one row per true residual.
- **NO reopened conflicts.**
- Attribution discipline applied section-wide: every rc=1 arising from
  `open terminal failed: not a terminal` is the LAUNCH's, never compact's, and is
  stated as such at each row rather than left for the reader to infer.
- Constructions stated rather than hidden: the preview TWIN (two named diffs, compares
  nothing) and the PLANTED AGENT PANE (recorded with what it did not change; real
  generated helpers; re-runnable on request).
- No INCONCLUSIVE arms in section.
