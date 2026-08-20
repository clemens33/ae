# Stage-2 corpus import — design PROPOSAL (#93)

**Status: PROPOSAL. Nothing here is ratified.** For `fable5:lead` to rule on.
Author: `opus5:s2builder`. Design only — no code was written and no file outside
this one was touched. Stage 2 is seat-gated and is not being entered by this
document.

**G1 is discharged** — this document is reconciled against
`s2-schema-inventory.md` (§1a), which corrected it in three places, one of them
substantially.

**The contamination boundary in §0 and §9 is ratified house doctrine** (lead,
2026-08-20) and binds every future builder, with the §9 narrowing: it attaches
to recorded output **values**, never to schema.

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

## 1a. Reconciliation against the real corpus — **G1 DISCHARGED**

`docs/migration/evidence/s2-schema-inventory.md` (structure-only, 45 grouped file
kinds, 11,593 files) closed G1. I read it after an **independent** leak check of
my own — no long hex runs but the declared 64-zero stand-in, no absolute user
paths, no UUID-shaped tokens, and the synthetic vocabulary counts matching the
lead's independently (`987654321` ×29, `2999-12-31T23:59:59Z` ×12). Three
separate checks agreeing is the control; a worker certifying its own containment
would not have been.

It corrects this document in three places, one of them badly.

### C1 — Import is a TRANSLATION, not an inverse. §1 was too clean.

There is **no `manifest.json` in either tree**, and the corpus does not use
stage 1's layout at all. It uses two *different* layouts, which are also
different from each other:

| | manifest dialect | capture layout | invocation record |
|---|---|---|---|
| **stage 1** | JSON array | `<lane>/{stdout,stderr,exit,manifest.json}` | `command.json` |
| **batch C** | 5-col positional TSV: `entry_type, mode, sha256-or-dash, link_target-or-dash, relative_path` | `out/<LABEL>.{stdout,stderr,tmuxtrace}` indexed by `consumers.tsv` | `argv` column in `consumers.tsv` |
| **L** | 7-col headed TSV with a `# manifest-root` preamble: `path, type, mode, nlink, size, link, sha256` | sibling `<STEP>.{invocation,rc,stdout,stderr}` | `.invocation` files |

So import is a **translation from two foreign dialects into stage 1's layout**,
and §1's "inverse of stage 1's writer" describes the destination only. The
layout is still the right interface — it is the target of the translation — but
the mapping is not 1:1 and no single decoder serves both trees (inventory #2:
"Do not reuse one decoder for both").

### C2 — The dialects carry DIFFERENT fields. Import must take the union, never the intersection.

L records `nlink` and `size`; batch C records neither. Normalising to a common
shape would either **lose** L's fields or **fabricate** them for batch C.
Neither is acceptable, and the rule is one this project already ratified:
SC-509b — a fact that was never recorded must be **omitted and visible as
omitted**, never defaulted into a value that reads as measured. An imported
entry from batch C has no `nlink`; it does not have `nlink = 1`.

### C3 — The corpus's own emptiness vocabulary is three-valued (inventory #8).

`-` (non-applicable), `ABSENT` (a two-column record for an absent
archive/worktree), and a genuinely zero-byte file are **three different facts**.
Import must not collapse them. This is R3's doctrine meeting real data before
any code was written, which is the best possible time to meet it.

---

## 2. Two run modes — **RULED: both, and they answer different questions**

| Mode | Lane A | Lane B | Uses the committed corpus? |
|---|---|---|---|
| **A — live/live** | executes now | executes now | no (needs a live bash `ae`) |
| **B — replay** | imported historical capture | executes now | **yes** |

Only Mode B consumes batch-C/L. The two are not interchangeable: in Mode A both
sides are reproducible and re-runnable; in Mode B one side is frozen history
that can never be re-derived, only checked for integrity.

**R1 — RULED (lead, 2026-08-20): both are admissible, and neither substitutes
for the other.**

### Why replay is admissible HERE, which is not the same as replay being fine

A replayed capture proves what the incumbent *did on the capture host at the
capture commit*, not what it does now. In general that gap makes replay weak
evidence. It does not here, and the reason is specific and load-bearing: **the
incumbent is FROZEN at `72c7293`.** "What does bash do" therefore has ONE
immutable answer, and a recorded answer is as good as a fresh one.

State the justification that way. "Replay is fine" would be a claim about replay;
what is true is a claim about the freeze, and it stops being true the moment the
incumbent unfreezes.

### The constraint: replay is SILENT about the second platform

The corpus was captured on **macOS/arm64**. Replay therefore cannot say anything
about Linux/musl — and an arm that cannot fail a claim is not evidence for it.
Replay cannot fail a platform-divergence claim, so it must never be cited for
one.

| Lane | What it is for |
|---|---|
| **live/live** | the **PLATFORM** lane, and the **acceptance** lane for any parity claim |
| **replay** | the **BREADTH** lane — many surfaces cheaply, one platform only |

**Every replay-backed row must carry `capture_platform` in its evidence
string**, so a reader can see which question the evidence answered rather than
having to reconstruct it. This is why `capture_platform` is a required
provenance field in §6 and not a nice-to-have. (§6 flags an open problem: the
schema inventory surfaces no per-entry source for that field.)

### A second narrowing, from G1: replay is also silent about anything mtime-dependent

F2 establishes that **neither corpus dialect records mtimes at all**. Replay
therefore cannot reproduce a file's modification time, and by the same
arm-that-cannot-fail rule it is not evidence for any row whose behavior turns on
one — which, per the A2 finding, includes the activity clock and staleness.

So the replay lane has **two** blind spots, and they should be stated together
wherever it is cited:

| Replay cannot speak to | Because |
|---|---|
| Linux/musl behavior | the corpus is macOS/arm64 only |
| anything reading a file mtime | no dialect recorded one |

Both route to live/live. Neither is a defect in the corpus — the captures were
taken for their own purposes — but both are limits on what replay may be cited
for, and a limit nobody wrote down is a limit that gets exceeded.

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
| `captured_at` | ordering, staleness against `source_commit`, **and the base a replay harness needs to compute an mtime offset (F2)** |
| `mtime_granularity` | the capture filesystem's real timestamp resolution. A per-capture fact, not a per-file one: APFS and ext4 differ, and a corpus that rounded has destroyed the difference before anyone could measure it (F2) |

Per-arm `SHA256SUMS.txt` and an `_admissibility/` directory already exist, so
the proposal is to **consume them**, not to invent a second scheme. That keeps
one ledger rather than two that can disagree.

**The stage-1 FNV-1a/64 manifest digest is NOT this** and must not be confused
with it. FNV is a fast in-run lookup aid with acknowledged collisions; evidence
integrity needs a cryptographic digest. They answer different questions and
should stay separate fields with separate names.

### Three corrections G1 forces on this section

**The exit record needs a third class.** Stage 1 writes `code <n>` or
`signalled`. The inventory says L's `.rc` "is not guaranteed to be numeric text
because harness failures and cut records are represented too". A two-valued
enum cannot hold that, and coercing it to a number would fabricate a status
nobody recorded. Import needs an unparsed-but-preserved class carrying the raw
bytes.

**`admissibility_ref` cannot be one shape** (inventory #4). Batch C records
admissibility **per case** (`admissibility-ledger.txt`); L records it at
**section** level (`_admissibility/equiv-*.txt` plus
`ADMISSIBILITY-SHA256SUMS.txt`). The field must be able to name either
granularity, and it must say which — a section-level admission is a weaker
statement about an individual file than a per-case one, and flattening them
would hide that.

**Checksums are layered, not flat** (inventory #11). Batch C has arm, template,
hook and T-WD `SHA256SUMS.txt`. L adds section-root admissibility and harness
ledgers alongside arm and specimen ones. Import must record **which ledger
admitted a given file**, not merely that some ledger did.

**Open: where does `capture_platform` come from?** §2's ruling makes it required
in every replay-backed evidence string, but the inventory surfaces no explicit
platform key — `ARM.txt` carries `binary`/`binary.sha256`, `construction`,
`fixture`; `env.txt` carries locale (and the existence of
`FINGERPRINTS.superseded-pre-locale-fix.tsv` shows locale already bit once). If
platform is recorded only at batch level rather than per entry, import must
attach it from the batch's own provenance and say that is where it came from.
**Flagged for the lead: this is a required field with no identified source.**

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

**F1 — directory modes. RULED fix — and G1 confirms the corpus HAS the data.**
Both dialects record `mode`, for directories as well as files (L's example row
is literally `dir<TAB>0755<TAB>…`). So this is not a speculative fidelity gap:
the corpus carries directory modes that stage 1 would silently discard on
clone. The fix restores information that genuinely exists.
Stage 1's `clone_to` preserves *file* permission bits but creates directories
with the process umask. Cheap to fix, and load-bearing here specifically:
L-PURGE's validator taxonomy carries explicit **directory-0755 vs file-0644**
classes, and the `_publish_executable_artifact` chokepoint story is entirely
about modes. A corpus that silently umasks directory modes cannot replay a
permission row.

**F2 — mtimes. RULED fix in stage 1 — but G1 shows the fix CANNOT reach the
existing corpus. ESCALATED; needs a seat decision this document cannot make.**

> **The corpus does not record mtimes either.** Batch C's manifest is
> `entry_type, mode, sha256, link_target, relative_path`. L's is `path, type,
> mode, nlink, size, link, sha256`. **Neither dialect carries a timestamp.**
>
> Fixing stage 1 to capture and restore mtimes therefore helps **future**
> captures and the **live/live** lane. It cannot retroactively give the
> *existing* corpus mtime fidelity, because the data was never written down.
>
> By the lead's own ruling — a corpus that cannot reproduce mtimes replays
> staleness and activity rows *silently wrong* — the consequence is direct:
> **any row whose behavior depends on a file mtime is NOT replayable from this
> corpus.** Such rows must be routed to the live/live lane, or the replay lane
> must refuse them outright. What it must not do is run them and report clean.
>
> One lead worth chasing, which I cannot chase myself because it needs value
> access: `batch-c-artifacts/twd-precursor/<ARM>/stamps/` is an opaque snapshot
> channel whose *name* suggests timestamps. If it carries file mtimes, part of
> the gap may be recoverable for the T-WD arms specifically. Someone with a
> spent contamination budget should look. I am recording it as a lead, not a
> finding.

Everything below remains correct for the **forward-looking** half — what stage 1
must record from now on, and what live/live gets. It is the *retrospective* half
that G1 removed.

The stage-1 manifest records path, kind, length, mode, symlink target and digest
— no mtime. The grounds are harder than "ae reads mtimes somewhere": the **A2
work established that `events.jsonl`'s mtime IS the frozen reader's activity
clock**, and staleness reads pane and meta mtimes. A corpus that cannot
reproduce mtimes cannot replay any staleness or activity row — and crucially it
does not FAIL on them, it replays them **silently wrong**, which is this
document's own "looks clean while comparing something other than behavior".

Three refinements from the ruling, all of which change what stage 1 must record:

1. **Capturing an mtime and RESTORING it are different problems, and restoring
   is the one that matters.** A manifest field that is only ever read by humans
   fixes nothing.
2. **Do not assume second granularity.** APFS and ext4 differ. Capture at the
   platform's real granularity and **record that granularity as its own field** —
   a corpus that rounds has destroyed the difference before anyone could measure
   it, and no later stage can recover it.
3. **Staleness is RELATIVE** (`now - mtime`), so a faithful replay may need an
   offset rather than an absolute stamp. Stage 1 owes the **data**, not the
   policy: capture absolute mtime, record granularity, restore absolute on
   import, and keep `captured_at` so a replay harness can compute the offset.
   **Whether to shift the clock is stage 3's decision.** The job here is to make
   sure stage 3 still HAS that decision instead of finding it foreclosed.

**F3 — corpus-escaping symlinks. RULED: HARD REFUSAL, upgraded from fidelity to
safety.** Stage 1 copies symlinks verbatim rather than following them — correct
for capture. On *import* it means a corpus containing an absolute link
(`/Users/…`, `~/.ae/…`) points a replay lane at **the human's real live state**:
the lane could read it, and depending on the row, **write** it.

The ruling goes past this document's first framing, and correctly. I called it a
fidelity hole and a safety one; **the safety half dominates**. This is not a
question about faithful replay, it is a question about blast radius, and it is
answered before fidelity is even considered.

**Import MUST refuse such an entry, loudly** — per R3: damage visible, never
rendered as legitimate sparsity. Quarantine is acceptable; silently following
the link is not. It stays inside what import may answer because it is an
integrity question (§6), not a judgement about behavior.

**G1 makes the check cheap and safe.** Both dialects record the link target as a
field (`link_target-or-dash`; `link`). The escape test therefore runs against
*metadata*, and import can refuse an entry **without ever dereferencing the
link** — the safest possible order of operations, since dereferencing is the
exact act being guarded against.

**F4 — hostile path names. REFRAMED by G1: the live hazard is shell
metacharacters, not non-UTF-8.** Stage 1 records paths through
`to_string_lossy` and declares non-UTF-8 out of scope; the inventory reports no
non-UTF-8 name, but does report (inventory #12) **one L event-capture basename
containing shell metacharacters — an intentional hostile-name case** — with the
explicit instruction: *import by directory entry, never by shell interpolation.*

A Rust importer working in `OsStr`/`Path` is structurally safe here, which is a
reason to keep it in Rust rather than shell out for any part of the walk. The
hostile name graduates from a hypothetical to a **required fixture** (§8).

**F6 — hard links are not preserved. NEW, from G1.** L's manifest records
`nlink`; stage 1's `clone_to` uses `fs::copy`, which turns two names for one
inode into two independent files. Any corpus entry with `nlink > 1` has its
file-identity graph silently changed by the clone. Whether ae behavior depends
on it is unknown to me; recorded so it is a decision rather than a discovery.

**F5 — uid/gid are not captured. RULED: ACCEPTED as a declared limitation.**

### Where F4 and F5 must be declared

**Not only here.** A limitation a future reader can find only by reading a
300-line design document is a limitation nobody will find. F4 and F5 belong in
**the corpus's own README**, next to the data they constrain, so that anyone who
opens the corpus meets them before they trust it.

The corpus does not exist yet — stage 2 creates it — so this is a **stage-2
deliverable requirement**, not an edit anyone can make today: whatever import
produces must carry a README declaring its own fidelity limits. (If an existing
file was meant instead, name it and I will write it; I am not guessing at
someone else's file.)

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

**RULED (lead, 2026-08-20): sufficient for stage-2 acceptance; stage 3 stays a
separate gate.** Three requirements attach, and none is optional.

### (a) Red-proof the round-trip

Deliberately corrupt **one byte, one mode, and one mtime** in the synthetic
fixture and prove the round-trip reports each one. A round-trip that cannot fail
proves nothing. That principle has already cost this session a rerun and five
review rounds; it is not a formality, and "the test passes" is not evidence that
it would ever have failed.

### (b) The fixture set must instantiate every fidelity class we decided to fix

One instance of each, so that each decision is proven rather than documented:

| Fixture | Proves |
|---|---|
| a directory whose mode is **not** 0755 | F1 — directory modes survive |
| a file with a distinctive **non-now** mtime | F2 — mtime is restored, not stamped at import |
| a corpus-**internal** symlink | links round-trip as links |
| a corpus-**escaping** symlink | **F3 — refusal** |
| a hard-linked pair (`nlink > 1`) | F6 — link identity, or its declared loss |
| a basename with **shell metacharacters** | F4 — import by directory entry, never by interpolation (inventory #12) |
| a `-`, an `ABSENT` record, and a **zero-byte** file | C3 — three different facts, not collapsed (inventory #8) |
| a TSV row whose last field contains a **literal tab** | inventory #7 — the invocation tail is joined, not split into phantom columns |
| a non-numeric `.rc` | the exit record has a third class beyond code/signalled (§6a) |
| a JSONL file with a **partial final record** and one with **no final newline** | byte-level states the inventory says import must preserve |

The escaping link is the only fixture whose expected outcome is a **failure**,
which makes it the only one that proves F3 is real rather than merely written
down. A refusal path with no fixture exercising it is a paragraph, not a
behavior.

### (c) The raw-handle denial must be STRUCTURAL, not prose

Per §4's generalisation, and as an acceptance criterion rather than a
recommendation: **the round-trip must fail to COMPILE if import hands back a raw
handle to a recorded observation.** Prose asking a future implementer to
remember is not a guard; a type that cannot express the mistake is.

Match whatever newtype `fixer2` settles on in round 5 — and take the **current**
shape at implementation time, not the shape recorded here.

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

### RULED — the boundary is narrower than this document first drew it

**SCHEMA IS NOT VALUE.** The contamination risk is recorded **output values**,
not field names: knowing that `manifest.json` has a digest field does not tell
anyone what bash printed. This document drew the line too conservatively.

So **G1 is dischargeable by a schema document** — field inventory, types,
cardinality, and one synthetic example per field with **invented** values —
which the design author may read without contamination. A dedicated worker is
being spawned whose only job is that inventory; it becomes contaminated by the
reading and therefore never implements product rows, which costs nothing,
because inventory is all it will ever do. The pen on this design stays here.

The general form is worth keeping: **contamination attaches to values, not to
structure.** A builder can know the shape of the evidence without knowing the
evidence.

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

**G1 is DISCHARGED** (§1a) — `s2-schema-inventory.md` supplied the structure and
this document is reconciled against it. What remains unknown is deliberately
value-shaped, not schema-shaped: whether `twd-precursor/*/stamps/` carries file
mtimes (F2's open lead), and where `capture_platform` is recorded (§6). Both
need value access and therefore belong to someone else.

Dependent on **seat rulings**: R1 and R3 are now ruled (§12); R2 and R4 remain
open in wording; **R5 stays deliberately unassigned**. See §11.

---

## 11. Contract rows this would need

| Id | Row | Status |
|---|---|---|
| **R1** | Replay-mode admissibility, and for which question. | **RULED** — admissible *because the incumbent is frozen*; macOS/arm64 only, so never citable for platform divergence; `capture_platform` required in every replay-backed evidence string (§2). Still owes a written row. |
| **R2** | Producer/lane neutrality in the corpus. | Open — may merely restate reviewer4's rule (§4). |
| **R3** | What a failed integrity check does. | **RULED** — refuse loudly or quarantine; never silent, never rendered as legitimate sparsity, following SC-506/SC-509b. F3's escaping symlink is the hard-refusal case (§6, §7). |
| **R4** | What a corpus entry MUST carry to be importable. | Open in wording; the §6 minimum now also carries `capture_platform` (R1) and the mtime + granularity fields (F2). |
| **R5** | **What counts as agreement.** | **Deliberately unassigned.** Not this document's, not the harness's, not the importer's. Named so its absence stays a recorded gap rather than a silent one. |

---

## 12. Questions for the lead — **ALL FOUR RULED (2026-08-20)**

Recorded inline so this document stands alone.

**1. Which mode? — BOTH, with a provenance constraint.** They answer different
questions and neither substitutes. Replay is admissible *because the incumbent
is frozen at `72c7293`*, which gives "what does bash do" one immutable answer —
not because replay is generally sound. The corpus is macOS/arm64 only, so replay
is silent about Linux/musl and can never be cited for a platform-divergence
claim. **live/live is the platform and acceptance lane; replay is the breadth
lane; every replay-backed row carries `capture_platform`.** See §2.

**2. F1 and F2? — FIX BOTH in stage 1 post-verdict, and F3 too, upgraded.**
F2's grounds are harder than first stated (A2: `events.jsonl` mtime IS the
frozen reader's activity clock) and the ruling adds three requirements —
restoring matters more than capturing, granularity is its own recorded field
because APFS and ext4 differ, and `captured_at` must survive so stage 3 can
still choose whether to shift the clock. F1 is load-bearing via L-PURGE's
directory-0755/file-0644 taxonomy. **F3 is upgraded from fidelity to a hard
refusal on blast-radius grounds.** F4 and F5 are accepted as declared
limitations and must appear in the corpus's own README. See §7.

**3. Who reconciles G1? — not the design author, and the boundary is narrower
than this document drew it.** Schema is not value; contamination attaches to
recorded output values, not to field names. A dedicated worker produces a schema
inventory with invented example values, which the design author may read safely.
See §9.

**4. Is the §8 round-trip sufficient? — YES**, with stage 3 a separate gate, and
with three attached requirements: red-proof it against a corrupted byte, mode
and mtime; instantiate every fidelity class in the fixture set including the
escaping-symlink refusal case; and make the raw-handle denial a compile-time
acceptance criterion rather than prose. See §8.

### Still open

- **R5 — what counts as agreement.** Unassigned by design. Named so its absence
  stays a recorded gap.
- **R2** — whether producer-neutrality needs its own row or merely restates
  reviewer4's rule.
- The **stage-1 verdict**, which can still move the layout (§10).
