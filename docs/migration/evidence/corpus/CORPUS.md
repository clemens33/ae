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

## 5. Invocations: partitioned by READ vs WRITE, and the normaliser proven both ways

**The axis is read versus write, not binary versus helper** (seat ruling). My first partition was
on invocation *shape*, and it was wrong: a generated helper can be a P1 surface — VISION:93 names
`requests` explicitly — and the binary can carry a write. Shape now governs **normalisation only**,
never scope.

`classify-invocations.py` partitions all 1424 consumer rows into `INVOCATIONS.tsv`:

| phase | rows | surfaces | parity-gating for P1 entry? |
|---|---|---|---|
| **P1** | 1065 | `ae list` 743, `helper:requests` 168, `ae ls` 116, `helper:events-tail` 38 | **yes** |
| **P1-ADJACENT** | 349 | `ae status` 140, `ae next` 140, `helper:agents` 62, `ae doctor` 7 | **no** — captured, frozen, kept |
| **P2 (write)** | 10 | `ae next --attach` 6, `ae <session>` launch 4 | no — P2 parity inputs |
| **UNRESOLVED** | 0 | — | — |

**P1 is accountable for what VISION names** (seat ruling). VISION:93 reads "Read side: `list
--json`, requests, events queries", so `list`, its alias `ls`, `requests` and `events-tail` gate
P1 entry. The other reads are **P1-ADJACENT**: real reads, captured and frozen, but not gating.

**The reason is phase hygiene, not difficulty.** A phase whose scope widens by reasonable-sounding
increments never closes, and "these are also reads" is exactly such an increment. VISION drew the
line; the ruling makes it explicit rather than letting it drift, so P1 closes on a named surface
and P2 inherits the rest.

**These are LABELS, not notes on a P1 label.** Leaving 349 rows classed `P1` with a caveat beside
them would be a label that disagrees with the ruling — the stale-pointer class, where the text and
the truth part company and only the text is machine-readable. Nothing is discarded and nothing is
re-captured: all 1424 rows stay frozen, and the distinction is a column, so re-deciding costs a
re-run rather than a rebuild.

**Read/write is decided against the frozen script's own documented contract, never inferred from
shape.** `ae next` is classed read because its usage text says "**Read-only by default**" and
scopes the action to `--attach` ("switch-client inside tmux, attach-session outside"). Plain
`ae doctor` is classed read because its 224-line span contains no filesystem write — every `>` in
it is `>&2`, and `doctor_report` is a `printf`; `--refresh` is the write path and no corpus row
uses it. **P2 rows are frozen and KEPT**, labelled P2 parity inputs — never deleted, never quietly
dropped, for the same reason `twd-precursor` is frozen as out-of-scope.

### Normalisation, and the two-sided proof

Stripped as host- or run-specific: absolute scratch prefixes, the capture host's `.ae` home,
per-case directories, and the interpreter token. Preserved as semantic: subcommand, flags, **flag
order**, session names, env prefixes, and the shape distinction. **Which binary produced a capture
— `frozen` vs `hooked` — is provenance, not invocation**: those are the same invocation through
different instruments, so they normalise together and the distinction is kept in an `instrument`
column rather than discarded.

`verify-invocations.py` proves both arms the ruling requires, and **has no write path**:

- **Arm A — convergence.** 160 normalised forms; **49 of them were reached from more than one host
  prefix**, so convergence is exercised rather than vacuous. The gate fails explicitly if that
  count is zero, because a convergence arm that never converges anything proves nothing.
- **Arm B — no collision.** 160 normalised forms against 160 distinct semantic invocations
  computed by an **independent method** (suffix-after-marker, not regex substitution): zero forms
  cover more than one invocation. **Red-proofed**: an over-normaliser that erases `--json`
  produces 26 collisions and fails the arm.

**One honesty note about that independent method.** I corrected it twice. The first was a genuine
bug — it kept only a path's basename, so `sessions/ta1b/requests` and `sessions/ta1c/requests`
collapsed and it reported the normaliser for correctly distinguishing them; an instrument that
discards a distinction cannot judge whether that distinction was preserved. The second added two
**declared equivalences** (an interpreter token carries no meaning; a binary path's meaning is its
basename), stated as rules rather than tuned until the two methods agreed — because tuning an
independent instrument to match the thing it checks destroys the independence, which is precisely
the circular self-test this programme has already been caught by once.

## 6. What this promotion does not claim

- **It does not re-verify the captures.** They were seat-accepted; this records and pins them.
- **It does not decide P1 SCOPE.** The P1 / P1-ADJACENT line is a seat ruling recorded in §5, not
  a worker judgment; this promotion applies it as a label.
- **It does not assert that the corpus is sufficient for P1 parity.** It is complete, pinned and
  reconstructible; whether it *covers* the P1 surface is a coverage question against rows this
  worker does not read.
- **It adds no new capture.** Nothing here was run against the product.
