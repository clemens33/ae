# The golden corpus — promotion record

**This executes a P0 deliverable that was never performed.** VISION lists a golden corpus under
P0 and P1 promises "snapshot parity against the corpus". No corpus existed: `cluster-plan.md`
calls batch C's artifacts golden-corpus **candidates** "seat-accepted before corpus freeze", and
the phrase *corpus freeze* occurs exactly once in the tree — in that sentence, describing a step
nobody ran. The material was accepted; the promotion was not executed. This is that promotion,
not a new capture.

## 1. What "frozen" means here

**A named, hash-pinned view over already-accepted artifacts — not a copy.** The bytes stay in
`batch-c-artifacts/`. What makes them a corpus is `CORPUS-MANIFEST.tsv` (role, sha256, size, path
for every file) and `FREEZE.txt` (source, commit, file count, root digest, role census). Copying
29 MB to create a second set of the same bytes would introduce the one thing a corpus must not
have: two copies someone has to keep in agreement.

Frozen therefore means four things, each mechanically checkable:

1. **Complete** — every file under the source is listed. A file on disk that the manifest does not
   list is a named failure, so material cannot be added quietly.
2. **Content-addressed** — each file carries its sha256, and one **root digest** covers the whole
   listing, so any edit, addition or removal moves a single value.
3. **Immutable in practice** — consumers read it; nothing that consumes it may write to it.
4. **Superseded, never deleted** — the source already contains
   `templates/FINGERPRINTS.superseded-pre-locale-fix.tsv`, kept and marked rather than removed.
   That is the pattern: a corpus that edits its own history is worth less than one that shows
   where it changed.

**A GATE READS, A GENERATOR WRITES, AND THEY ARE DIFFERENT PROGRAMS RUN AT DIFFERENT TIMES.**
`freeze-corpus.py` writes and never verifies; `verify-corpus.py` verifies and **has no write path
at all** — no `open(..., "w")`, no temp file, no rename. This is not stylistic. A gate that
regenerates the table it reports on overwrites the drift it exists to detect, and "regenerate"
then silently means "repair". That failure has already occurred once in this evidence tree, in a
different lane, and is the reason the two programs are separate files.

## 2. What the corpus contains

6862 files across 11 arms and 177 cases. **Roles are derived from path shape by the generator, not
hand-assigned**, so a new file lands in a role by construction rather than by someone remembering:

| role | files | what it is | how parity uses it |
|---|---|---|---|
| `FIXTURE` | 785 | `templates/*/fixture-bytes` and per-case `manifest.before.tsv` | materialise the input tree, then confirm the clone fingerprint matches the template |
| `INVOCATION` | 531 | `case.txt`, `consumers.tsv`, `env.txt` | what was run, with which argv and environment, and the rc it returned |
| `EXPECTED` | 3118 | `out/*.stdout`, `*.stderr`, `*.tmuxtrace` | the bytes the frozen implementation produced |
| `NO-MUTATION` | 690 | `manifest.after.tsv`, `manifest.diff.txt`, `tmux.before/after.txt` | evidence the read side changed nothing — a read-side parity run must reproduce that too |
| `PROVENANCE` | 1143 | admissibility ledgers, env self-checks, shim-equivalence records, hook patches | why the capture is admissible; not consumed by parity |
| `OUT-OF-SCOPE` | 595 | `twd-precursor/` | P4 watchdog precursor material. Frozen so it cannot drift, **explicitly not a P1 parity input** |

`consumers.tsv` is the spine: 1424 rows carrying consumer name, **rc**, per-stream sha256 and byte
counts, a bounded flag, and full argv. Measured rc distribution: `0` × 1256, `1` × 130, `143` × 38
(SIGTERM, the bounded consumers).

## 3. Where it lives

```
docs/migration/evidence/corpus/
  CORPUS.md              this file — the definition and the promotion record
  CORPUS-MANIFEST.tsv    role, sha256, bytes, path — one row per file
  FREEZE.txt             source, commit, file count, root digest, role census
  freeze-corpus.py       GENERATOR — writes the two files above, nothing else
  verify-corpus.py       GATE — reads them, recomputes, reports drift, never writes
```

The corpus **content** stays at `docs/migration/evidence/batch-c-artifacts/`.

## 4. How a parity run consumes it

1. **Verify first.** `verify-corpus.py` must exit 0 before any comparison. Parity against an
   unverified corpus measures nothing.
2. **Materialise** the case's template `fixture-bytes` into a scratch tree — never into the
   corpus — and confirm the clone fingerprint equals `clone_fingerprint` in `case.txt`.
3. **Invoke** the Rust binary with the normalised argv (§5) and the recorded environment.
4. **Compare** rc, stdout and stderr **byte for byte** against `consumers.tsv` and the `out/`
   files. Not "equivalent", not "semantically the same" — the corpus records bytes.
5. **Assert no mutation** by recomputing the tree manifest and comparing to `manifest.after.tsv`
   and `manifest.diff.txt`.
6. **Never write into the corpus.** A parity run that can update its own baseline is not a
   comparison; it is a recording with extra steps.

## 5. THE BLOCKING FINDING: argv is host- and run-specific

**A parity run cannot use the recorded argv as-is, and this must be settled before parity is
written.** The argv column embeds absolute paths from the capture host, including a scratch
directory named after a session UUID that will never exist again. Measured shapes across the 1424
rows:

| shape | rows | example prefix |
|---|---|---|
| frozen `ae` via bash | 1141 | `/opt/homebrew/bin/bash /private/tmp/claude-501/…/scratchpad/frozen/ae` |
| generated session helper | 259 | `/tmp/aecx/arms/A1/c01-healthy-ro/home/.ae/sessions/tg1/agents` |
| bare `ae` | 14 | `ae …` |
| template-path helper | 9 | `/tmp/aecx/tpl/a1405k/home/.ae/sessions/ta1k/agents` |
| env-prefixed | 1 | `AE_HOOK=H_NEXT_SELECTED …` |

So normalisation is not one rule but at least five shapes, and two of them (`helper`,
`template-path helper`) invoke **generated session helpers rather than the binary** — those are
not `ae` subcommands at all and may not be in P1's parity scope. **Recorded as UNRESOLVED and
flagged for a seat**: which of the five shapes are in P1 parity scope, and what the normalisation
rule is for each. This worker is not deciding it, because the answer changes what P1 is
accountable for reproducing.

## 6. What this promotion does not claim

- **It does not re-verify the captures.** They were seat-accepted; this records and pins them.
- **It does not decide argv normalisation** (§5), which is a scope question for a seat.
- **It does not assert that the corpus is sufficient for P1 parity.** It is complete, pinned and
  reconstructible; whether it *covers* the P1 surface is a coverage question against rows this
  worker does not read.
- **It adds no new capture.** Nothing here was run against the product.
