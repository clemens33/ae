# Batch L — section L-PURGE artifacts — MANIFEST

Worker: `opus5:lexec`. This file maps **arm → row ids → artifact paths → fixtures →
mutation diffs** for the L-PURGE (purge inversion + validator taxonomy) evidence run.

Captures only: bytes, hashes, recursive manifests, byte diffs, rc values, barrier
orderings, logs, together with the manipulation that produced them. No verdicts, no
expected-vs-actual statements, no classification. Seats classify.

## Run-wide provenance

Identical to L-END and recorded there: frozen commit `72c7293…`, frozen `ae` sha256
`b7b8aa9f…`, instrumented `ae` sha256 `84f68ac8…`, hook patch sha256 `00b06eaa…`,
host/tool hashes in `_harness/env-record.txt`, `LANG=LC_ALL=en_US.UTF-8`, `TZ=UTC`,
no live models, no network. The admissibility proofs in `_admissibility/` (inactive
hook, tmux shim, flock spy, git shim, plus three known-difference controls) cover the
same instruments this section uses; `L-PURGE/HARNESS-SHA256SUMS.txt` hashes the harness
snapshot as this section ran it.

Every arm ran in its own **disposable sandbox** under `/tmp/aelx/L-PURGE/<arm>/` with a
private `HOME`, `AE_HOME`, `TMPDIR`, `TMUX_TMPDIR` and tmux socket. No arm shares a
sandbox with another.

## Fixtures: REAL archives, produced in each arm's own sandbox

Every fixture is produced by a REAL frozen `ae end`, cut at a barrier, **inside the arm
that consumes it** — never copied between sandboxes and never hand-fabricated
(`_harness/purge-lib.sh :: purge_template`):

| cut barrier | what the cut leaves |
|---|---|
| `b_pre_cleanup` | archive published, the publisher's own claim ALREADY RELEASED by the product, session directory still on disk |
| `b_post_rename` | archive published, the publisher's `.publishing.<uuid>` claim STILL STANDING, session directory still on disk |

Both are the natural session↔archive pairing of one real run, so no arm binds a session
to an archive by mutation. Each arm records `template.txt` (session, uuid, cut barrier,
archive present, claim present, session dir present, end rc), `template.aehome.tsv`,
`template.archive.tsv`, `template.archive-meta.txt` and `template.session-meta.txt`.
`purge_template` refuses to proceed — writing `ARM-INVALID.txt` — if the cut did not
leave both an archive and a session directory, so no arm can run against a fixture that
is not what it claims.

**Per-consumer separate clones are honoured by construction.** Every arm that exercises
both the purge path and a `--from` attempt is TWO arms on TWO sandboxes, each of which
produced its own template and applied the same named mutation independently. No arm's
outcome can turn another arm's fixture into something else.

## Mutation discipline

Every controller rewrite of a product file goes through
`_harness/arm.sh :: l_rewrite_preserving_mode` — temp, chmod back to the target's
ORIGINAL mode, rename — so only the NAMED bytes change. This is the standing harness
rule adopted after the L-END correction (see `L-END/MANIFEST.md`, "Correction"): a
content diff is structurally blind to mode, and an archive member's mode is asserted by
the frozen validator. Mutations that are THEMSELVES mode changes (cases c1, c2, d, e1-e3)
say so explicitly in their `mutation.txt`. Every arm records `mutation.txt`,
`mutation.diff` (content diff plus the directory manifest diff) and, where a mode is
involved, `mutation.mode.before` / `mutation.mode.after`.

## Common capture set (every arm)

`ARM.txt` · `template.*` (the fixture's own production record) ·
`0launch.*` / `0template-end.*` / `2op.*` (`stdout`, `stderr`, `rc`, `invocation` with
full argv and env) · `1pre.{aehome,archive,sessions}.tsv` and
`3post.{aehome,archive,sessions}.tsv` recursive manifests (path, type, mode, nlink, size,
symlink target, sha256) · `3post.tmux.txt` · `preflight-tab.txt` (the blocking
environment-as-instrument proof, run inside the arm's own `env -i`) ·
`consumer-inproc.txt` + `tmux-argv.log` + `_ae_install_tmux_shim.extracted.sh` (brief
rule (e)) · `barrier-order.tsv` and per-barrier manifests from the template cut ·
`SHA256SUMS.txt`.

## Arms

### Purge inversion

| arm | roster ids | construction | key artifacts |
|---|---|---|---|
| `no-prior-archive` | SC-810a | a real `--local` session with a session id and NO archive anywhere is ended with `--purge-history` | `1pre.archive.tsv`, `3post.archive.tsv`, `2op.{stdout,stderr,rc}` |
| `existing-archive-as-produced` | SC-810b | the named specimen shape EXACTLY as produced — the `b_post_rename` cut, whose still-standing `.publishing.<uuid>` claim is part of what that cut produces — ended with `--purge-history` unmodified | `template.txt` (`claim_present yes`), `1pre.archive.tsv`, `3post.archive.tsv`, `2op.stderr` |
| `existing-archive-claim-released` | SC-810b | the same natural pairing produced one barrier later (`b_pre_cleanup`), where the product has ALREADY RELEASED its own claim — no controller manipulation was needed to clear it | `template.txt` (`claim_present no`), `1pre.archive.tsv`, `3post.archive.tsv`, `2op.stdout` |
| `claim` | SC-818b | a `.publishing.<uuid>` claim for the archive's own uuid is planted mode 0700 under the archive root, then `end --purge-history` runs | `mutation.{txt,diff}`, `mutation.dir.{before,after}.tsv`, `2op.stderr` |

The two SC-810b readings are run because the design NAMES the crash-cut output as this
row's specimen, and that specimen carries a standing claim which the purge path acquires
first. Running both readings on separate clones records the overlap with SC-818b instead
of hiding it behind a choice.

### Validator taxonomy — one named mutation per case, purge clone AND `--from` clone

Every row below is TWO arms: `…-purge` (drives `ae end -f --purge-history pg`) and
`…-from` (drives `ae --local pgchild --from <uuid>`), on independent sandboxes.
Each records `archive.pre-mutation.tsv`, `archive.post-mutation.tsv`,
`archive.mutation.diff`, `archive-meta.pre-mutation.txt`, and the full pre/post
`AE_HOME`, archive-root and sessions manifests (SC-818c's post-state material).

| case | roster ids | named mutation |
|---|---|---|
| `a-unexpected-entry` | SC-804a, SC-818c | an unexpected extra entry — a regular file mode 0600 named `EXTRA.txt` — is created inside the archive directory |
| `b1-symlink-inside` | SC-804b, SC-818c | the archive member `memo.tsv` is replaced by a SYMLINK pointing outside the archive (`/etc/hosts`) |
| `b2-fifo-inside` | SC-804b, SC-818c | sibling of b1: `memo.tsv` is replaced by a FIFO of mode 0600 |
| `c1-messages-dir-0755` | SC-804c, SC-818c | the archive's `messages/` directory mode 0700 → 0755 |
| `c2-archive-dir-0755` | SC-804c, SC-818c | the archive directory's own mode 0700 → 0755 |
| `d-file-0644` | SC-804f, SC-818c | the archive member `meta` mode 0600 → 0644 (a NAMED mode mutation, deliberate here) |
| `e1-exec-user` | SC-804d, SC-818c | `digest.md` gains the USER executable bit (0600 → 0700) |
| `e2-exec-group` | SC-804d, SC-818c | `digest.md` gains the GROUP executable bit (0600 → 0610) |
| `e3-exec-other` | SC-804d, SC-818c | `digest.md` gains the OTHER executable bit (0600 → 0601) |
| `f1-id-mismatch` | SC-804e, SC-818c | the archive meta's `archive_id` value is replaced by a different well-formed uuid, mode preserved |
| `f2-count-mismatch` | SC-804e, SC-818c | on a SECOND independent clone: the archive meta's `handover_count` value is replaced by `42` so meta and digest disagree, mode preserved |

SC-804e's two conjuncts are on two independent clones per side, exactly as the design
requires — `f1` and `f2` never share a fixture.

### Execution sentinel (SC-805)

The archive member `memo.tsv` is replaced by a shebang script whose body would write a
sentinel file at `<sandbox>/SENTINEL-WAS-EXECUTED`, OUTSIDE the archive, and is given
executable bits (0600 → 0700). **Each archive-consuming operation runs on ITS OWN clone.**

| arm | roster ids | archive-consuming operation |
|---|---|---|
| `execution-sentinel-purge` | SC-805 | `ae end -f --purge-history pg` |
| `execution-sentinel-from` | SC-805 | `ae --local pgchild --from <uuid>` |
| `execution-sentinel-compact` | SC-805 | `ae compact -f --digest-only pg` |
| `control-sentinel-no-exec-bit-purge` | none — control, captures only | the SAME shebang body with the member's ORIGINAL mode kept (no executable bit), purge path |

Per arm: `mutation.txt` (target, description, mode before/after, whether exec bits were
granted, the sentinel path), `mutation.diff`, `member.{before,after}.txt`,
`archive.post-mutation.tsv`, and `sentinel.1pre.txt` / `sentinel.3post.txt` recording the
sentinel path's existence, size and content at both points. The control exists so the
exec-bit arm's sentinel reading is not the only reading on the table.

### Lineage parent (SC-818e) — both readings, per the lead's ruling

A REAL parent archive is produced by a real `ae end`, and a REAL `--from` child is
launched from it. Then:

| arm | roster ids | reading |
|---|---|---|
| `lineage-parent-mutated` | SC-818e | (a) ONE named mutation sets the child meta's `session_id` EQUAL to its `parent_archive_id` (mode preserved), then `end -f --purge-history ch` |
| `lineage-parent-literal` | SC-818e | (b) the real `--from` child is left exactly as the product created it, then `end -f --purge-history ch` |

**Code observation recorded for the seats, not a verdict** (`mutation.txt`,
`reachability.note` in the mutated arm): at 72c7293 `_ar_purge_archive:5404-5408` the
refusal fires only when the aid being purged equals the session's own
`parent_archive_id`, i.e. when meta `session_id` == meta `parent_archive_id`. A real
`--from` child receives a FRESH `session_id` on the launch path, so no sequence of real
operations produces that equality. The mutated arm reaches it by the single named
mutation; the literal arm shows the reachable behaviour. Whether the row's SHOULD tracks
the guard or the reachable behaviour is seat work.
Per arm: `parent-archive-meta.txt`, `child-meta.before.txt`, `child-meta.after.txt`,
`mutation.diff`, `lineage.txt` (parent archive presence after, archive and session dirs
after, the child's lineage keys at the time of the op).

### Unidentifiable session (SC-819) — FOUR fresh clones, ONE invocation each

Two classes crossed with two policies. In every clone a real `--local` session is
launched and then stopped, one named mutation is applied to its state, and exactly ONE
end invocation runs.

| arm | class | policy | invocation |
|---|---|---|---|
| `unidentifiable-missing-meta-keep` | the session meta file is REMOVED; memo, events, helpers and messages are left intact | keep | `ae end -f un` |
| `unidentifiable-missing-meta-purge` | same | purge | `ae end -f --purge-history un` |
| `unidentifiable-unparseable-id-keep` | the session meta's `session_id` value is replaced by an UNPARSEABLE token (`not-a-uuid--0000`), mode preserved | keep | `ae end -f un` |
| `unidentifiable-unparseable-id-purge` | same | purge | `ae end -f --purge-history un` |

**Flag-bearing subclass (added on the lead's SC-819 ruling, colead concurring).**
`--assume-stopped` is a real frozen per-target flag a user can pass, so reaching the
archive-plan layer with it is a REACHABLE construction rather than an invented one. Two
further clones, same value-blind pattern, labelled as the flag-bearing subclass; the
four arms above stay exactly as they are and their front-door captures stand on their
own.

| arm | class | policy | invocation |
|---|---|---|---|
| `unidentifiable-missing-meta-keep-assume-stopped` | session meta REMOVED, memory intact | keep | `ae end -f --assume-stopped un` |
| `unidentifiable-missing-meta-purge-assume-stopped` | same | purge | `ae end -f --assume-stopped --purge-history un` |

All four missing-meta arms carry `frozen-cites.txt`, two pointers supplied by
`gpt56sol:colead` through the lead and recorded verbatim as POINTERS for the seats,
with no verdict attached by this worker: 72c7293 `ae:2911-2955` (the flag acknowledges
ONLY stopped-state absence after the full enumerable sweep — it authorizes neither
identity fabrication nor memory deletion) and `ae:8039-8052` (`_end_archive_plan`
classifies missing-meta-with-memory as `unavailable`). Which gate each invocation
actually reached is in that arm's own `2op.stderr`; classifying it against those
semantics is seat work.

The unparseable class is deliberately DISTINCT from the legacy MISSING-`session_id` mint
path (SC-826, L-FROM's territory): the key is PRESENT here and its value is unparseable.
Per arm: `session-meta.before.txt`, `session-meta.after.txt`, `mutation.{txt,diff}`,
`sessiondir.before.tsv`, `sessiondir.after.tsv`, `3post.sessions-full.tsv`.

### Non-roster control

| arm | note |
|---|---|
| `control-symlinked-archive-root` | SC-818a is an ALREADY-OBSERVED non-roster safety control, referenced only. The archive root is replaced by a symlink to a directory outside `AE_HOME` holding the same real archive, then `end -f --purge-history` runs. `manipulation.txt`, `1pre.linktarget.tsv`, `3post.linktarget.tsv`. **NOT a coverage arm.** |

## Roster coverage as executed

All 14 L-PURGE roster ids have a primary arm above: SC-804a, SC-804b, SC-804c, SC-804d,
SC-804e, SC-804f, SC-805, SC-810a, SC-810b, SC-818b, SC-818c, SC-818d, SC-818e, SC-819.
SC-818a appears only as the referenced non-roster control.

## Known limits of this section, stated

- The four front-door `unidentifiable-missing-meta-*` / `-unparseable-id-*` clones run
  exactly the single invocation the design specifies (no `--assume-stopped`), so what
  they observe is whatever gate the frozen code reaches first. The two flag-bearing
  clones were added afterwards on an explicit ruling; each arm's `subclass` field in
  `ARM.txt` says which layer it addresses. Neither set was re-run to make the other
  look tidier.
- `execution-sentinel-*` mutates a member that is itself mode-asserted, so the exec bit
  the design calls for is also a validator-visible property. The `control-sentinel-no-exec-bit-purge`
  arm is there so the exec-bit arm's sentinel reading is not the only one available.
- `existing-archive-as-produced` and the `claim` arm reach the same claim primitive by
  different routes (one product-produced, one controller-planted). Both are kept; the
  overlap is recorded rather than resolved.
- The watchdog is left ENABLED (the launch default) in every arm.

## Harness stability at the section boundary

`L-PURGE/harness-snapshot/` is a byte copy of the shared `_harness/` exactly as this
section ran, hashed by `L-PURGE/HARNESS-SHA256SUMS.txt`; `L-PURGE/ADMISSIBILITY-SHA256SUMS.txt`
hashes the admissibility proofs it rests on. Nothing under `L-PURGE/` changes after this
point. Later sections may extend the shared `_harness/` libraries additively.
