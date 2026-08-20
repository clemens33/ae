# Stage-2 corpus import — design PROPOSAL (#93)

**Status: PROPOSAL. Nothing here is ratified.** For `fable5:lead` to rule on.
Author: `opus5:s2builder`. Design only — no code was written and no file outside
this one was touched. Stage 2 is seat-gated and is not being entered by this
document.

**What this rests on, as of 2026-08-20.** Stage 1 is under review round 5 and is
NOT settled; `opus5:fixer2` holds the pen on both parity files, `gpt56sol:reviewer4`
is read-only, and at least one more structural change (the raw-`Output` newtype,
§4) is expected. This design is therefore keyed to the **on-disk artifact
layout** rather than to Rust type paths — endorsed independently by the lead and
reviewer4 on the grounds that the layout survived four review rounds unchanged
while the type organisation did not. Every type-level reference below is
illustrative and marked as such.

---

## 0. Reading discipline — what the author did and did not look at

I did **not** open the contents of any batch-C or batch-L artifact. I listed
directory **names** under `docs/migration/evidence/` and nothing else.

Two reasons, and the second is the important one:

1. My standing brief prohibits reading batch artifacts and bash-produced output.
2. **Contamination.** A builder who has read recorded bash outputs cannot
   afterwards implement Rust behavior value-blind. The rows would be built by
   someone who knows what the incumbent printed, and "built from the row" would
   quietly become "built to match what I saw". That is the failure the whole
   anti-oracle discipline exists to prevent, and it does not stop applying just
   because the artifact is ours.

The consequence is deliberate: **this design is structural.** It says what an
import must do, not what any recorded value is. Reconciling it against the real
schemas is Gap **G1** below, and it is work for someone whose contamination
budget can afford it.

What the directory names alone establish (names only, no contents):
`batch-c-artifacts/` holds `arms/{A1,A2,A3,A3b,A4,D}/` each with a `harness/`
and a `SHA256SUMS.txt`, plus `templates/`, `MANIFEST.md` and `PATH-CITES.tsv`;
`l-artifacts/` holds `_admissibility/`, `_harness/` and per-surface arm
directories. That is enough to design against — per-arm checksums and an
admissibility ledger already exist, so the import should **consume** them rather
than invent a parallel scheme.

---

## 1. The shape, in one sentence

**Stage 2 is the inverse of stage 1's writer.** Stage 1 runs a lane and writes
its raw capture into a known on-disk layout. Stage 2 reads a *historical*
capture and materialises it into that same layout. Import is a **materialiser,
never a comparator**.

### The interface is the layout, not the Rust types

Stage 1 already publishes:

```
<root>/pair.json                  the template, and the lane names in order
<root>/<lane>/clone/              the tree that lane ran against, as it was left
<root>/<lane>/command.json        program, args, cwd, environment
<root>/<lane>/exit                `code <n>` or `signalled`
<root>/<lane>/stdout              raw bytes
<root>/<lane>/stderr              raw bytes
<root>/<lane>/manifest.json       the recursive listing
```

I propose this layout — not any Rust module path — is the stage-1/stage-2
contract. It survived the stage-1 review refactor unchanged while the type
organisation did not, which is the empirical argument for keying to it. Type
references below are **illustrative only**.

---

## 2. Two run modes; the seats must choose which is admissible

| Mode | Lane A | Lane B | Uses the committed corpus? |
|---|---|---|---|
| **A — live/live** | executes now | executes now | no (needs a live bash `ae`) |
| **B — replay** | imported historical capture | executes now | **yes** |

Only Mode B consumes batch-C/L. The two are not interchangeable: in Mode A both
sides are reproducible and re-runnable; in Mode B one side is frozen history
that can never be re-derived, only checked for integrity.

**Row needed (R1): is Mode B admissible evidence at all, and for which
question?** A replayed capture proves what the incumbent *did on the capture
host at the capture commit*, not what it does now. That may be exactly what is
wanted for a frozen-at-`72c7293` incumbent — but it is a seat call, not mine.

---

## 3. What a corpus entry is

Three parts, kept strictly separate because they have different epistemic
status:

1. **INPUTS** — a template tree, plus an invocation spec (program, args,
   environment, whether the environment was cleared). Reproducible. Re-clonable.
2. **RECORDED OBSERVATIONS** — per-producer raw capture: stdout bytes, stderr
   bytes, exit outcome, and the post-run tree. Historical. Not reproducible.
3. **PROVENANCE** — where this came from and whether it was admitted (§6).

**What a corpus entry is NOT**, and this is the value-blindness rule in
structural form: it carries no statement about what the output *should* be.
There is no `expected/` directory, no `golden` field, no tolerance, no
normaliser. Recorded output is an OBSERVATION of one producer. Whether two
observations agree is a **seat ruling (R5)**, and no code in stage 1, stage 2 or
the import path may encode it.

---

## 4. Producer-neutral naming — reviewer4's rule, generalised

Reviewer4's standing rule is that lane names must not appear in harness code.
Stage 1 complies: `tests/it/parity.rs` contains no lane-name literal; names
arrive only through `Lane::new`. I propose stage 2 inherit the rule one level
up, because the corpus is where it would most easily be broken:

- a corpus entry records a **producer id** in its provenance (which arm, which
  commit, which binary) — never a lane name;
- the **operator** maps producer → lane at run time;
- therefore no importer function may be named for a producer. There is no
  `import_bash_lane()`. There is `import(entry, producer_id) -> lane artifacts`.

The reason is the same one behind capture-never-judges: a harness that knows
which lane is the incumbent has a direction in which to fail, and a difference
would get read as "the new one is wrong" before anyone decided that. Keeping the
identity in *data* and out of *code* means the harness cannot form the opinion.

**Scope of the rule, confirmed by reviewer4 and by the lead** (2026-08-20): it
scans `harness_source()` — `parity.rs` — and nothing else. The synthetic
self-tests are deliberately outside it, because a test that forbids a name must
be able to write it; the `["bash", "rust"]` literals in the self-test file are
**rule 6's own implementation**, the guard scanning decommented harness source
for the incumbent's lane names. The enforcer necessarily names what it forbids.
I raised this as a possible finding; it is not one, and nothing needs to change.

**Row needed (R2)?** — possibly not; it may just be reviewer4's rule restated
for stage 2. Flagging rather than assuming.

### The raw-handle lesson generalises to import

Stage 1's round-5 finding is worth carrying forward, because import will meet it
again in the same shape. The guard was broken by judging a **raw
`std::process::Output`** inside the capture path: a raw handle carries more
surface than the three legal consumptions, `Output` implements `Debug` natively,
and status never appears in a derived field list — so the capacity to judge
arrived through a type nobody chose to give it. The fix is a private newtype
exposing only the legal consumptions.

The same applies to an imported capture. **Import must not hand back a raw
handle to a recorded observation**; it should expose only what a caller is
allowed to do with it, so that the ability to compare cannot arrive by accident
through a derive or a `Debug` impl. Structural denial beats a rule that a
future implementer has to remember.

---

## 5. Trust versus re-derive

An imported capture is **evidence, not an oracle** — and per the lead, this is
how #97 was found: the anti-oracle rule applies to our own artifacts too.

| Fact | Status on import |
|---|---|
| recorded stdout / stderr bytes | **TRUSTED** (historical; unreproducible) — integrity-checked only |
| recorded exit outcome | **TRUSTED**, same basis |
| the post-run tree's **bytes** | **TRUSTED**, same basis |
| `manifest.json` | **RE-DERIVED** — re-walk the imported tree; see below |
| every content digest | **RECOMPUTED**, never read from the record |
| the template tree | **RE-CLONED** per lane, never reused in place |
| whether the lanes agree | **NEVER DERIVED — seat ruling R5** |

### Why the manifest is re-derived rather than trusted

`manifest.json` is a *description* of the tree beside it. If import trusts the
description, then a corrupted, truncated, or hand-edited manifest silently
becomes evidence, and the thing it describes is never actually consulted.
Re-walking the imported tree makes the recorded manifest **checkable rather than
authoritative**: it demotes from "the answer" to "a second observation", which
is what it always was.

Note the asymmetry and that it is principled, not arbitrary: bytes are trusted
because they *cannot* be re-derived, descriptions are re-derived because they
*can*. Trust exactly what you have no way to check, and check everything else.

---

## 6. Provenance, and the one place import is allowed to say NO

### Proposed minimum record, one per entry

| Field | Why |
|---|---|
| `source_commit` | the tree the producer ran from |
| `producer_id` | which arm/binary produced it — **not** a lane name (§4) |
| `admissibility_ref` | the ledger row admitting this capture |
| `checksums` | per-file, cryptographic |
| `capture_platform` | GNU vs BSD userland changes behavior; a capture without it cannot be interpreted |
| `captured_at` | ordering, and staleness against `source_commit` |

Per-arm `SHA256SUMS.txt` and an `_admissibility/` directory already exist, so
the proposal is to **consume them**, not to invent a second scheme. That keeps
one ledger rather than two that can disagree.

**The stage-1 FNV-1a/64 manifest digest is NOT this** and must not be confused
with it. FNV is a fast in-run lookup aid with acknowledged collisions; evidence
integrity needs a cryptographic digest. They answer different questions and
should stay separate fields with separate names.

### Integrity refusal is not judgement

Import must refuse a corpus entry whose checksums do not match. I want to be
explicit about why that is not a verdict, because it looks like one:

- **integrity** asks "is this the artifact that was admitted?" — a question
  about *provenance*, answerable without knowing what the bytes mean;
- **agreement** asks "do the two lanes match?" — a question about *behavior*,
  and the seats' alone.

Import may answer the first. It may never answer the second.

**Row needed (R3): what a failed integrity check does.** I propose it mirrors
SC-506/SC-509b's doctrine, which was ratified for exactly this shape of problem:
damage must be **visible** and must never render identically to legitimate
sparsity. A corpus entry that fails its checksum must not import as an empty or
partial entry — that would be a machine digest hiding loss, which SC-509b calls
lying by omission. Either refuse the entry loudly, or import it flagged
`degraded` so a reader can see the difference. Not silently.

---

## 7. Fidelity gaps that would make a parity run measure the wrong thing

These are the findings I most want ruled on, because each one lets a run look
clean while comparing something other than behavior.

**F1 — directory modes are not reproduced.** Stage 1's `clone_to` preserves
*file* permission bits but creates directories with the process umask. A corpus
whose fidelity depends on directory modes cannot round-trip today. For ae this
is not hypothetical: the whole `_publish_executable_artifact` chokepoint story
is about modes. **Blocks faithful import until decided.**

**F2 — mtimes are not captured at all.** The stage-1 manifest records path,
kind, length, mode, symlink target and digest — no mtime. Bash ae reads mtimes
(`_ae_stat mtime`, the staleness and activity paths). A replayed corpus whose
mtimes are all "whenever import ran" can drive a lane down a different branch
than the producer took, and the run would still look clean. **I rate this the
highest-risk gap in the design.** It may need a manifest field, and therefore a
stage-1 change after the review.

**F3 — absolute symlinks escape the corpus.** Stage 1 copies symlinks verbatim
rather than following them. Correct for capture; for *import* it means a corpus
containing an absolute link (`/Users/...`, `~/.ae/...`) would point a lane at the
real machine instead of at the clone. That is both a fidelity hole and a safety
one. Proposal: import must detect corpus-escaping links and refuse or quarantine
them — an integrity question (§6), not a judgement.

**F4 — non-UTF-8 paths are lossy.** Stage 1 records paths through
`to_string_lossy` and declares non-UTF-8 names out of scope. If any real corpus
entry carries one, import cannot round-trip it and must say so rather than
mangle it.

**F5 — uid/gid are not captured.** Probably out of scope; recorded so it is a
decision rather than a discovery.

---

## 8. How stage 2 gets accepted without touching stage 3

Stage 2 must be verifiable on its own, or its acceptance silently depends on a
comparison nobody has authorised yet. I propose the acceptance test is a
**round-trip against synthetic fixtures**:

> Write a synthetic capture into the layout; import it; assert the materialised
> layout is byte-identical to what was written.

This is value-blind by construction — it compares an import against **its own
input**, never against an expectation of what any producer should emit. It needs
no real corpus, so stage 2's plumbing can be reviewed and accepted before the
contamination gate is opened. It is the same move stage 1 made with synthetic
self-tests, applied one stage later.

A round-trip that loses mtimes, directory modes, or a symlink target fails
visibly — which is why F1–F4 above are worth settling first: the acceptance test
is exactly the thing that would catch them.

---

## 9. Contamination boundary — who may read what

Stage 2 is the first stage that reads recorded bash output, so it is the first
that can contaminate a builder. Proposal:

- whoever implements the importer **does not** implement contract rows
  afterwards, or
- the importer is written entirely against synthetic fixtures, and only the
  **operator** ever points it at the real corpus.

This document is a small proof the second is workable: it was written without
opening a single artifact.

---

## 10. What I explicitly cannot design yet

Dependent on the **stage-1 review outcome** (reviewer4's verdict is outstanding,
and the harness is being refactored under it as I write):

- **whether the on-disk layout survives the verdict.** Everything above keys to
  it. If the verdict changes the layout, §1 changes with it.
- **whether the manifest gains fields** (F2's mtime especially). Import cannot
  round-trip a field stage 1 does not record.
- **the digest decision.** If the verdict replaces FNV-1a/64, §6's separation of
  lookup-aid from integrity-checksum may collapse into one field.
- **the raw-`Output` newtype now landing in round 5.** It changes how a capture
  is handed around; the layout it writes is unaffected, but the analogous
  discipline for import (§4) should match whatever shape fixer2 settles on.

Resolved since first draft, recorded so the gap list stays honest: the self-test
lane literals are **not** a finding (§4), and the on-disk layout is endorsed as
the stage-1/stage-2 interface by both the lead and reviewer4.

Dependent on **schemas I have not read (G1)**: the real shape of `MANIFEST.md`,
`SHA256SUMS.txt`, `PATH-CITES.tsv` and `_admissibility/`. Everything in §6 is a
proposal for what import *needs*; reconciling it to what those files *are* is
unstarted and is not something I should do myself (§0, §9).

Dependent on **seat rulings**: R1–R5 below.

---

## 11. Contract rows this would need

| Id | Row |
|---|---|
| **R1** | Is replay-mode evidence admissible, and for which question? |
| **R2** | Producer/lane neutrality in the corpus (may just restate reviewer4's rule). |
| **R3** | What a failed integrity check does — refuse, or import visibly degraded? Proposed to follow SC-506/SC-509b. |
| **R4** | What a corpus entry MUST carry to be importable (the §6 minimum). |
| **R5** | **What counts as agreement.** Explicitly not mine, not the harness's, not the importer's. Named here only so that its absence is a recorded gap rather than a silent one. |

---

## 12. Questions for the lead

1. Mode A, Mode B, or both (R1)? It decides whether the committed corpus is on
   the critical path at all.
2. Do F1 (directory modes) and F2 (mtimes) get fixed in stage 1 post-verdict, or
   accepted as declared corpus limitations? I recommend fixing F2 — a lane that
   branches on mtime makes a clean-looking run meaningless.
3. Who reconciles G1 against the real schemas, given §9's contamination boundary?
4. Is the round-trip acceptance test in §8 sufficient for stage 2, so that stage
   3 stays a separate gate?
