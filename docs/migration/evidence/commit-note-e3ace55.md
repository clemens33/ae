# Commit provenance note — e3ace55

`e3ace55`'s message describes only the SC-818e outcome-grain precision. It in fact
contains **133 files**: that one contract edit plus **all 132 Batch-L `*SHA256SUMS.txt`
files**, rewritten by lexec's end-of-batch checksum-integrity fix, which landed in the
working tree while the contract edit was being staged with a too-broad
`git add -A docs/migration`.

Nothing is wrong with the CONTENT — both changes are correct and verified (see below) —
but the commit message does not describe what the commit carries, so this note is the
durable correction rather than a rewritten history on a shared branch.

## What the checksum fix was

lexec swept every checksum file they had written across Batch L and classified each
failure by cause. Measured: 132 files, exactly ONE verified clean.

- **119** failed on their own SELF-REFERENCE — the listing included `SHA256SUMS.txt`
  itself, whose hash necessarily changes when it is written. Structurally impossible
  to satisfy.
- **12** failed because the listing root differed from where the file sits:
  `HARNESS-SHA256SUMS.txt` listed paths relative to `harness-snapshot/` and
  `ADMISSIBILITY-SHA256SUMS.txt` relative to `_admissibility/`, while both files live
  in the section directory.
- **0 content mismatches.** Every artifact hash was correct; the defect was entirely
  in how the lists were built.

Fix applied to all 132: each file excludes itself and verifies from its own directory;
the two section-level files list paths that resolve from the section directory; each
carries a leading comment naming the directory to verify from (`shasum -c` ignores
comment lines, verified).

## Why it mattered

Lead gate-reads had reported both section-level SUMS clean — true, because they were
run from the roots the listings happened to use. But a reader running the check the
obvious way, from the directory the file lives in, would have hit a wall of failures,
none of which meant anything. **A verification artifact that cannot be verified is
worse than none.** The lead gate-read also had a coverage gap it should not have had:
it verified 2 section-level checksum files per section and never the ~22 per-arm ones.

## Independent verification (lead, post-fix)

All 132 files, each verified from its own directory: **TOTAL=132 CLEAN=132 FAILING=0**,
and `git diff --name-only` confirms only `*SHA256SUMS.txt` files changed — no manifest,
no arm artifact, no capture.

## Rules reinforced

- Per-path adds at phase boundaries; `git add -A <dir>` is banned in this program
  precisely because it makes a commit message a lie about its own contents (this is
  the second occurrence — the first swept worker artifacts mid-write at 305b16e).
- A gate-read must state its COVERAGE, not only its result: "both SUMS verify clean"
  should have read "the 2 section-level SUMS verify clean; the 22 per-arm files were
  not checked."
