# Joint L classification — L-PURGE worksheet (section 2 of 6)

Seats: fable5:lead (author) + gpt56sol:colead (independent read). Ordering set by
colead: **L-PURGE before L-COMPACT** — PURGE is the older section and its
product-produced archives and validator outcomes are UPSTREAM evidence; classify
provenance before consumers. Same schema and raw-pointer rules as L-END; colead's
four explicit surfacing requirements are honoured below (SC-819's two layers as
SEPARATE observations; the SC-810b claim-release cut; every SC-804 validator class at
one-row grain; the mode-preserving correction provenance).

**STATUS: CONVERGED 2026-08-20** — colead's independent gate returned 12 rows green
unchanged and two evidence-boundary corrections, both applied below: SC-818e precised
to OUTCOME grain (no defect) and SC-819's row evidence narrowed to the four front-door
arms. 14/14 confirm; zero reopened conflicts.

Preflight rules restated: pointers reach the claim-bearing RAW captures; empirical
acceptance and normative `classified_by` stay ORTHOGONAL; mismatches route through
fix-known-defect or DR, never measurement-over-contract. **No marks proposed here.**

Evidence base: L-PURGE committed at c7f291b (41 arms, 2534 files; both section SUMS
verify clean from their roots). Paths below are under
`docs/migration/evidence/l-artifacts/L-PURGE/arms/`. Every arm produced its own
archive from a real frozen `end` cut at a barrier in its own sandbox — no specimen was
copied between sandboxes, and `purge_template` refuses (ARM-INVALID) unless the cut
left both an archive and a session dir. Section-wide: zero preflight failures, zero
ARM-INVALID, zero INCONCLUSIVE.

`verified`: **bytes** = lead read the named raw captures directly.

---

## The SC-804 validator taxonomy — eleven classes, one-row grain (colead requirement)

All eleven ran as clone PAIRS (a `-purge` clone and a `--from` clone, independent
sandboxes). Every `-purge` clone: **rc=1**, and the refusal names the archive path and
the reason class, then declines to act:

> `archive: '<path>' does not validate as an ae archive — refusing to delete it.`
> `  Inspect it and remove it by hand if that is what you want.`
> `Error: purging the archive for 'pg' failed — the session is STOPPED and nothing was deleted.`

| class arm (`validator-taxonomy-…`) | planted condition | row | purge rc |
|---|---|---|---|
| `a-unexpected-entry` | an entry outside the exact path whitelist | **SC-804a** | 1 |
| `b1-symlink-inside` | a symlink inside the tree | **SC-804b** | 1 |
| `b2-fifo-inside` | a FIFO (special file) inside the tree | **SC-804b** | 1 |
| `c1-messages-dir-0755` | `messages/` directory at 0755 | **SC-804c** | 1 |
| `c2-archive-dir-0755` | archive root directory at 0755 | **SC-804c** | 1 |
| `d-file-0644` | a payload file at 0644 | **SC-804f** | 1 |
| `e1-exec-user` | user execute bit set | **SC-804d** | 1 |
| `e2-exec-group` | GROUP execute bit set | **SC-804d** | 1 |
| `e3-exec-other` | OTHER execute bit set | **SC-804d** | 1 |
| `f1-id-mismatch` | `meta` vs `digest.md` disagree on archive id | **SC-804e** | 1 |
| `f2-count-mismatch` | `meta` vs `digest.md` disagree on counts | **SC-804e** | 1 |

Artifacts per arm: `2op.rc`, `2op.stderr`, `2op.invocation`, `1pre.archive.tsv` vs
`3post.archive.tsv` (the archive is unchanged after refusal), plus the `-from` twin.
Verified: **bytes** (all eleven rc + stderr read directly).

- **SC-804a** — exact path whitelist; an unrecognised entry FAILS rather than being
  ignored (b1, none). **Proposed: CONFIRMED / no change.**
- **SC-804b** — no symlink or special file (b1, none). Both sub-classes (symlink,
  FIFO) refuse independently. **Proposed: CONFIRMED / no change.**
- **SC-804c** — directories 0700 (b1, none). Both a payload subdir and the archive
  root itself refuse at 0755. **Proposed: CONFIRMED / no change.**
- **SC-804f** — files 0600 (b1, none). **Proposed: CONFIRMED / no change.**
- **SC-804d** — no execute bit for user, group, OR other (b1, none). **This is the
  row whose SHOULD names the trap** (`-x` answers only for the current user; a
  group-executable file is still a program) and all THREE bit positions were planted
  separately and all three refuse. **Proposed: CONFIRMED / no change** — the
  three-position construction is what makes this row's specific claim evidenced
  rather than assumed.
- **SC-804e** — `meta` and `digest.md` must agree, on id AND counts (b1, none). Both
  disagreement kinds planted separately, both refuse. **Proposed: CONFIRMED / no
  change.**

**Fidelity note (colead requirement — mode-preserving correction provenance):** the
mutation helper used across this section is `l_rewrite_preserving_mode` (temp +
chmod-to-TARGET-mode + rename), adopted after the L-END SC-808 contamination in which
a `sed > tmp && mv` idiom carried an unnamed umask-644 mode change onto an archive
meta the frozen validator asserts 600 on. That defect is invisible to a content diff,
which is why it survived review. Consequence for THIS section: the mode-class arms
(c1, c2, d, e1-e3) mutate modes DELIBERATELY and say so, and every non-mode mutation
records `mode.before`/`mode.after`. The two SC-818d arms were re-run under the fixed
helper (600 → 600, only `source_session` emptied). No L-PURGE arm carries an unnamed
mode side effect.

---

## SC-805 — an archive is inert data; the validator is the proof, not intent
(b1, none). Arms: `execution-sentinel-purge`, `-from`, `-compact` (three
archive-consuming operations, each on its own clone) + `control-sentinel-no-exec-bit-purge`.
Artifacts: `2op.rc`, `2op.stderr`, `sentinel.1pre.txt`, `sentinel.3post.txt`.
IS (bytes): with an execute bit planted, all three consuming operations refuse
(rc=1) with the does-not-validate refusal, and the sentinel is ABSENT both before and
after — nothing executed it. The control (identical fixture, no exec bit) is rc=0
with the sentinel likewise absent, so the refusal is attributable to the bit and not
to the sentinel's presence. **Proposed: CONFIRMED / no change** — note the control is
what makes this a discriminating observation rather than a coincidence.

## SC-810a — `--purge-history` writes no archive
(b2, none). Arm: `no-prior-archive`. Artifacts: `2op.rc` (**0**), `1pre.archive.tsv`
vs `3post.archive.tsv`, `3post.aehome.tsv`. IS (bytes): with no prior archive, the
purge-history end succeeds rc=0 and no archive is created. **Proposed: CONFIRMED /
no change.**

## SC-810b — `--purge-history` deletes any existing archive for the source UUID
(b2, none) — **the claim-release cut, per colead's requirement.** TWO arms, both
readings ruled admissible and BOTH kept:

- `existing-archive-as-produced` (**rc=1**): the archive is the product's own
  crash-cut output captured post-rename/pre-cleanup, so it still carries its standing
  `.publishing.<uuid>` claim. stderr's first line names that claim path. The purge
  therefore stops at the CLAIM barrier — this arm observes **SC-818b's** guard, and
  the archive-present delete path is NOT reached. Recorded as such rather than
  counted as SC-810b evidence.
- `existing-archive-claim-released` (**rc=0**): the SAME product-produced state with
  the claim released **by moving the crash cut** (`b_pre_cleanup`, where the product
  itself has already released) rather than by controller manipulation. The
  archive-present path IS reached and the operation completes.

IS: SC-810b's delete-the-existing-archive claim is evidenced by the second arm; the
first is an SC-818b observation that documents why the naive construction cannot
reach the row. Verified: **bytes** (both rc, first stderr line).
**Proposed: CONFIRMED / no change**, with the explicit note that the as-produced arm
is NOT this row's evidence.

## SC-818b — purge acquires the same `.publishing.<uuid>` claim
(b1, none). Arms: `claim` (primary) + `existing-archive-as-produced` (the incidental
observation above). Artifacts: `2op.rc`, `2op.stderr`, claim-dir manifests.
IS: a standing claim blocks the purge and is NAMED, not cleaned. **Proposed:
CONFIRMED / no change.**

## SC-818c — purge validates the tree before deleting
(b1, none). Evidenced by the entire eleven-class taxonomy above: every planted
malformation is refused BEFORE any delete, with the archive unchanged in the post
manifests, and the refusal tells the operator to remove it by hand. **Proposed:
CONFIRMED / no change.**

## SC-818d — purge requires a NONEMPTY exact source-identity match
(b1, none). Arms: `empty-identity-purge`, `empty-identity-from` (both re-run under
the mode-preserving helper; 600 → 600, only `source_session` emptied).
IS (bytes): with `source_session` emptied, purge refuses rc=1 with the
does-not-validate refusal naming the archive path — an archive naming no session is
absence of proof, not a wildcard — and the `--from` twin likewise does not inherit
from it. **Proposed: CONFIRMED / no change.**

## SC-818e — purge refuses to delete a parent a live `--from` lineage points at
(b1, none). TWO arms per the approved both-readings ruling:

- `lineage-parent-mutated` (**rc=1**), stderr verbatim: *"archive: refusing to purge
  2f61f3fe-… — it is the parent archive this session was launched from."* The NAMED
  guard fires and names the lineage.
- `lineage-parent-literal` (**rc=0**): the unmutated real `--from` child purging its
  OWN archive — the guard does not fire, because it cannot.

**REACHABILITY FINDING carried from the arm notes (code-observation, for the seats):**
`_ar_purge_archive` (72c7293:5404-5408) fires this refusal only when the aid being
purged EQUALS the session's own `parent_archive_id` — i.e. when meta `session_id` ==
meta `parent_archive_id`. A real `--from` child always receives a FRESH `session_id`,
so `end --purge-history <child>` targets the CHILD's own archive and never the
parent's; no sequence of real operations reaches the guard. The mutated arm applies
ONE named byte mutation (`session_id` := `parent_archive_id`) to make the guard
reachable; the literal arm shows what real construction does instead.
IS: the guard, when reached, refuses and names the lineage. Verified: **bytes**.
**RESOLVED — CONFIRMED / no change, NO defect (colead ruling, lead concurs).** The
open question is answered: the SHOULD is the OUTCOME, not the guard. Reachable
behavior satisfies the safety promise BY CONSTRUCTION — a real `--from` child receives
a fresh UUID, so a child purge never addresses the parent's archive; the operation
SUCCEEDS on the child (`lineage-parent-literal` rc=0) and the parent is untouched. The
equal-id corrupted-meta case separately proves the defensive guard
(`lineage-parent-mutated` rc=1, named refusal). Guard reachability is IMPLEMENTATION
EVIDENCE, not the normative claim. The row's SHOULD is precised accordingly in the
contract to "purge never deletes the parent archive referenced by a live `--from`
lineage", bucket 1, conflict none. **Explicitly NOT to be stated: that normal lineage
"refuses" the operation — it succeeds on the child.**

## SC-819 — an unidentifiable session is refused BEFORE anything is stopped
(b1, none) — **two layers, SEPARATE observations, per colead's requirement.** Six
arms (three classes × keep/purge), all rc=1, none deleting anything:

**Layer 1 — front door (no acknowledgement flag), the design's specified invocation:**
- `unidentifiable-missing-meta-{keep,purge}` (rc=1), stderr verbatim: *"Error:
  session 'un' has no positive server record (pre-fix or ambiguous meta) — it may
  b…"* — refused at the **no-positive-server-record gate**, which sits BEFORE the
  archive-plan layer. The missing-meta class therefore never reaches archive
  planning through the front door.
- `unidentifiable-unparseable-id-{keep,purge}` (rc=1), stderr verbatim: *"Error:
  session 'un' cannot be archived — its session_id (not-a-uuid--0000) is not a
  UUID…"* — this class DOES reach the archive-plan layer and its refusal NAMES the
  unparseable id.

**Layer 2 — flag-bearing (`--assume-stopped`), the seat-concurred reachable
construction:**
- `unidentifiable-missing-meta-{keep,purge}-assume-stopped` (rc=1), stderr verbatim:
  *"Error: working directory is not a git repo — cannot preserve work."* — the
  invocation PASSED the server-record gate that stopped layer 1, and was then refused
  at a **different** precondition (the repo check). Session memory is intact in the
  before/after manifests; nothing was stopped or deleted.

IS: the SHOULD's promise (refused with the reason, nothing deleted, regardless of
history flag) holds in all six arms — rc=1 throughout, keep and purge alike, with a
named reason each time. **What is NOT evidenced**, and is stated rather than inferred:
none of the six reaches `_end_archive_plan`'s missing-meta-with-memory classification
(the ae:8039-8052 path colead cited), because layer 1 stops earlier and layer 2 stops
at the repo precondition. Both frozen cites colead supplied (ae:2911-2955 for the
flag's scope, ae:8039-8052 for the plan-layer classification) are recorded verbatim as
`frozen-cites.txt` in all four missing-meta arms, framed as POINTERS with no verdict
attached. Verified: **bytes** (all six rc + first stderr lines).
**CONFIRMED / no conflict at PUBLIC-CONTRACT grain (colead ruling, lead concurs),
with the row evidence NARROWED:** only the FOUR front-door arms are evidence for this
row — missing-meta keep/purge (refused before stop/delete with the no-positive-record
identity reason) and unparseable-id keep/purge (refused with the named invalid id).
The two `--assume-stopped` arms stop at the non-git-repo precondition and therefore
**cannot fail SC-819**; they are labelled CONTROLS / OUT-OF-SCOPE for this row, not
supporting observations. An arm that cannot fail a claim is not evidence for it.
No repo-satisfying clones are needed: the row promises a PUBLIC refusal, not
`_end_archive_plan` branch coverage. The internal missing-meta classification remains
explicitly UNOBSERVED — and that is a scope statement, not a gap.

---

## Non-roster controls (no row proposed)

- `control-symlinked-archive-root` (rc=1) — referenced by SC-818a's context, kept as
  a control, not a coverage arm.
- `control-sentinel-no-exec-bit-purge` (rc=0) — the SC-805 discriminator described
  above.

## Proposed dispositions

- **CONFIRMED — 14 of 14 roster ids (both seats)**: SC-804a, SC-804b, SC-804c,
  SC-804d, SC-804e, SC-804f, SC-805, SC-810a, SC-810b, SC-818b, SC-818c, SC-818d,
  SC-818e (SHOULD), SC-819 (SHOULD).
- **NO reopened conflicts in this section.**
- **The one open normative question is RESOLVED**: SC-818e is outcome-grain, satisfied
  by address separation; no defect. Contract row precised.
- **Three scope statements recorded rather than papered over**: SC-810b's as-produced
  arm is an SC-818b observation and not SC-810b evidence; SC-819's archive-plan-layer
  classification is observed by no arm; and SC-819's two `--assume-stopped` arms are
  controls, not row evidence, because they cannot fail the claim.
- Fidelity: mode-preserving correction provenance carried (see the SC-804 note); the
  two SC-818d arms are post-fix re-runs.
- No INCONCLUSIVE arms in section.
