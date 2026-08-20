# Batch C artifacts — MANIFEST

Worker: `opus5:cexec`. This file maps **assignment → row ids → artifact paths →
template fingerprints → mutation diffs** for the Batch C read-side evidence run.

Everything here is a CAPTURE: bytes, hashes, manifests, byte diffs, rc values, logs,
and the manipulation/barrier that produced them. No verdicts, no expected-vs-actual
statements, no classification. Seats classify.

## Run-wide provenance

| field | value |
|---|---|
| frozen commit | `72c729343a0117af2968b66e1c43f89ad25fc0b2` |
| frozen `ae` blob | `50e4f575baf3aa584b98b3eaaeec5264e4916161` |
| frozen `ae` sha256 (extracted) | `b7b8aa9fb77afc0705abdfaadf60cc58911f1cac46fe2ec993578fe5451575fd` |
| frozen tree source | `git archive 72c7293` into worker scratch — the live checkout's `ae` and product files are never touched |
| host | `Darwin 25.6.0 arm64` |
| interpreter | `/opt/homebrew/bin/bash` — GNU bash 5.3.15(1) — sha256 `6ba6319962b59831740a56aa5f65e91d6467c72997f5d0a18be1ba1a6d8d378b` |
| tmux | `/opt/homebrew/bin/tmux` — `tmux 3.7b` |
| per-arm environment | `TZ=UTC`, `LANG=LC_ALL=en_US.UTF-8` (UTF-8 is required — see the correction section; A1–A4 all ran under it), scrubbed `PATH`, fresh `HOME`/`AE_HOME`, own `TMPDIR`/`TMUX_TMPDIR`, dedicated tmux socket, cleaned per arm |
| live models / network | none used |

## Lane status

- **BASH lane**: running, per brief.
- **RUST lane**: UNLOCKED, not yet run. Both gate conditions were delivered by the
  lead: reviewer3 passed the rust slice (round-4 delta verdict, no findings), and the
  frozen reviewed rust tree is commit `2cfbf80` on `rust-rewrite` —
  full id `2cfbf805fa442e2e2d712c81e883fbc7036f0fb2`, resolved locally at record time. The exact
  rust-lane invocation is recorded per arm alongside that commit id before any rust
  capture. Rust lanes clone FRESH from the same pre-lane template fingerprints the
  bash lane cloned from, never from bash-mutated state. Per-lane captures are reported
  raw; no bash-vs-rust comparison or divergence verdict is produced here — divergences
  are paired raw artifacts. Sequencing: the bash lane completes per arm group first.

## Where the admissibility evidence lives

Captures are written DIRECTLY into this committed tree — there is no scratch-then-publish
step for evidence, so no capture can exist here without its admissibility proof beside it.
Only the sandboxes (template clones, tmux tmpdirs, sockets) live outside it.

Every case keeps an append-only `admissibility-ledger.txt`: a monotonic `seq` plus UTC and
epoch for each event — case open, rows, clone verification, the TAB round-trip
START/COMPLETE, the tmux-shim equivalence START/COMPLETE, the inactive-hook equivalence
START/COMPLETE with its measured volatility floor, the before/after manifests, any barrier
ARMED/REACHED/RELEASED and controller mutation, and every consumer START/COMPLETE with its
rc and its stdout / stderr / tmuxtrace sha256. The ledger is written by the checks
themselves as they run, so it establishes ORDER — that the standing checks completed
before the first consumer invocation — from the original durable content, with each
capture's own hash tied into the same record. Filesystem mtimes and the `SHA256SUMS.txt`
list are not relied on for ordering.

### The gate, and exactly what each of its three checks guarantees

`arms/*/harness/manifest-tree-gate.sh` audits a tree given as an ARGUMENT (first
positional, then `$BATCH_C_ARTIFACTS`, then the live tree), so it can be red-proofed
against a candidate copy instead of silently auditing the live one. It runs three checks
that guarantee three DIFFERENT things. None of them subsumes another, and none of them
alone proves the tree is complete.

The SUMS half discovers every directory carrying a `SHA256SUMS.txt` rather than walking a
hardcoded list — the hardcoded list silently skipped `twd-precursor/`, whose checksum files
went unverified for the whole run.

**1. Citation resolution** (`path-cite-resolver.py`) — guarantees that every backticked
path-shaped token in this file RESOLVES somewhere: against the tree root, `arms/`,
`templates/`, `twd-precursor/`, the repository root, the
`<GROUP>/<member>` → `templates/<GROUP>/fixture-bytes/<member>` mapping, or one of the real
context directories a relative citation can legitimately be written against (a case dir, an
arms root, a group dir, a member's fixture-bytes dir, a session dir, a group `_meta` dir, a
T-WD arm dir and its sub-dirs). A wildcard must expand NONEMPTY and every expansion must
exist. It writes `PATH-CITES.tsv`, one row per citation with class, resolving base and
expansion count.
It does NOT prove per-case completeness. Relative tokens are deduplicated globally and
satisfied by ANY matching context, so `case.txt` resolving proves that SOME case has a
`case.txt`, not that every case does.

**2. SHA256SUMS coverage and verification** — guarantees that every published file is
listed in its directory's `SHA256SUMS.txt` and that every listed hash still verifies.
It does NOT prove per-case completeness either: a file deleted TOGETHER with its
`SHA256SUMS` line leaves `files == listed` and every remaining hash intact. That exact
paired deletion passed the two-check gate and is the defect this section was rewritten for.

**3. Per-case artifact schema and case index** (`case-schema-check.py`, `case-schema.tsv`)
— guarantees per-case COMPLETENESS. Each case declares its KIND through its own
admissibility ledger (`barrier`, `twin`, `live`, `staged`/`unstaged`, `pane-follow`,
`consumer-run`, `hooked`, `document`), and the schema table says which artifacts each kind
must contain; a case missing one fails whether or not its `SHA256SUMS` line went with it.
Membership is checked against `arms/<ARM>/CASES.tsv`, a case index written when the arm
completes that binds every case DIRECTORY to the sha256 of that case's own ledger — so a
whole case removed with its hash lines still fails, and a ledger edited and re-hashed into
`SHA256SUMS` fails too, because the index disagrees.

**4. Committed bytes / clone fidelity** (`committed-bytes-check.py`) — guarantees that
the bytes the repository would hand a fresh clone are the bytes that were hashed. The other
three all read the WORKING TREE and cannot see a tree that passes locally and fails on
clone. At commit ce8965e a text filter (`autocrlf=input`) rewrote four D04b pty logs on the
way into the object database: the working file hashed `8320a0a5`, the stored blob hashed
`20fffc72`, and all three working-tree checks passed while a fresh clone would have failed
verification on four evidence artifacts. `.gitattributes` now marks both evidence trees
`-text`, but that is PREVENTION; this check is DETECTION, and it is non-invasive — nothing
is staged, committed, or written to the object database. Part A (pre-commit, the blocking
one) compares the object id git WOULD create from the working bytes with attributes and
clean filters applied against the id of the RAW bytes; any difference means what reaches
the repository is not what was hashed, whatever the cause — autocrlf, a smudge/clean
filter, a new path outside the `-text` globs. Part B (post-commit) compares the sha256 of
`git show HEAD:<path>` against the recorded hash for every file already at HEAD; files
modified since HEAD are counted and named, not failed.

### What the checksum files themselves guarantee

Measured rather than assumed, after a parallel finding in the L-artifacts where 119 of 132
checksum files listed THEMSELVES — a hash that necessarily changes when it is written, so
the file could never verify from its own listing — and 12 more used paths relative to a
different root than they sat in. Batch C: **14 checksum files, zero self-listed, zero entries
that fail to resolve from their own directory** (measured at every gate run). Each now carries
a three-line header naming the exact directory to verify from
(`cd <tree-relative dir> && shasum -a 256 -c SHA256SUMS.txt`) and stating that it is
deliberately not listed in itself; the gate counts checksum lines rather than file lines so
the header cannot inflate coverage. `write-sums.sh` is the single writer, and it writes its
temp file OUTSIDE the directory being hashed — writing it inside is how three phantom
sums-temp entries reached the committed T-WD archive, an entry for a file that never
existed anywhere, which the widened gate found and which is now repaired.

**The honest limit.** These three make an omission require coordinated edits across three
independent records — the ledger-derived schema, the content-bound case index, and the
hash list. They do not, and cannot, defend against an editor who rewrites all three
consistently; that is what git history and seat review are for. The claim here is
completeness against ACCIDENT and against single-record tampering, not against a
determined forger.

All four are red-proved. `arms/*/harness/gate-redproof.sh` runs a green control plus
seventeen injections — one per citation base, the group/member mapping, an empty
wildcard, a case-relative token, a slash-less file citation, a deleted listed file,
tampered bytes, a file deleted with its SUMS line, a case directory removed with its SUMS
lines, a deleted case index, a ledger edited and re-hashed, a file whose bytes change while its
SUMS entry is updated to match, and a phantom entry in `twd-precursor/` (invisible to the
old hardcoded directory list). Every one turns the gate red; the control stays green.
Clone fidelity is red-proved separately by
`arms/*/harness/committed-bytes-redproof.sh`, in an ISOLATED scratch repository so the live
index is never touched: it reproduces the exact ce8965e shape — a CRLF pty log under
`core.autocrlf=input` with no `-text` attribute — shows part A going red on it, shows the
`-text` attribute turning it green, then commits and corrupts one recorded hash to show
part B going red on a HEAD blob that disagrees.

`hook-patch/` holds the ONE hook-only patch used by the barrier arms and the D-record
designs — the unified diff, its generator, and the unmodified / hooked / patch hashes.

## Boundary as executed

65 assignments (batch-c-design.md v3 at `8f21a48`, slice-1d, plus SC-021 — the `ls` alias arm — added to A2): A1 gains SC-510e/f;
A7's SC-405j gains case 5 (empty-member subcases). D01–D04 concurrency cuts execute
the approved b0-design.md Designs 2–6; SC-1306a–e ride those designs per the mapping
note (1306a→D01/Design 2, 1306b→D04a/Design 5, 1306c→D04b/Design 6, 1306d→D02/Design 3,
1306e→D03/Design 4). Designs 1/7/8 are not this worker's.

## Contents

| section | status |
|---|---|
| Step 0 — T-WD producer precursor (feeds G2) | COMPLETE |
| Template groups G1–G11 (+G2b) + the A1/A2/A3/D members | COMPLETE — all fingerprinted and chmod-protected |
| Arm group A1 (SC-509, 509b, 506, 510a–f, 511a–b, 405k) | COMPLETE, bash lane — 39 case runs |
| Arm group A2 (SC-017a–f, 017i, 021, 521a) | COMPLETE, bash lane — 2 case runs x 54 invocations |
| Arm group A3 (SC-017g, 017h, 524) | COMPLETE, bash lane — 20 case runs |
| Arm group A3b (SC-017g adjacent pairs) | COMPLETE, bash lane — 12 case runs |
| Arm group A4 (SC-016a–d, 513a–c, 019, 020a–c) | COMPLETE, bash lane — 7 case runs, incl. SC-020b on D04b's hook |
| Arm group A5 (SC-514) | COMPLETE, bash lane — 7 case runs under a controlled PATH |
| Arm group A6 (SC-518, 522, 523a–b) | COMPLETE, bash lane — 13 case runs |
| Arm group A7 (SC-405a–g, 405j) | COMPLETE, bash lane — 36 case runs |
| Arm groups A8–A9 | not started |
| D-record executions (b0-design Designs 2–6) + SC-1306a–e | COMPLETE, bash lane — D01, D02, D03, D04a, D04b, all with controller-only twins |

---

## Correction — the C-locale tmux defect, and what it forced

**What happened.** The scrubbed environment for consumer runs and for the template-build
sandboxes pinned `LANG=C` / `LC_ALL=C`. tmux decides its output encoding by looking for
the substring `UTF-8` in `LC_ALL` / `LC_CTYPE` / `LANG`; in a non-UTF-8 locale it
SANITISES the TAB in `-F` format output to `_`. The frozen consumer's own pane query is
TAB-separated — `tmux list-panes -s -t "$name" -F '#{@ae_agent}<TAB>#{pane_current_command}'`
at ae@72c7293:4207, and the same query in the attention rollup at :3631 — so
`IFS=$'\t' read -r ae_agent pane_cmd` received ONE field, the `_alive` map was keyed on
`fake:lead_aefake` instead of `fake:lead`, every agent rendered `alive:false`, and the
rollup independently concluded the panes had vanished (`attn:dead`). One cause, two
symptoms, and the cause was the harness environment.

**How it was isolated** (same server, same socket, same format; only the locale varies):

| environment | output |
|---|---|
| `env -i` + `LC_ALL=C LANG=C` | `x:y_sleep` — TAB mangled |
| `env -i` + `LANG=C` | `x:y_sleep` — TAB mangled |
| `env -i` + `LC_ALL=C LANG=C LC_CTYPE=UTF-8` | `x:y<TAB>sleep` — TAB preserved |
| `env -i` + `LC_ALL=en_US.UTF-8 LANG=en_US.UTF-8` | `x:y<TAB>sleep` — TAB preserved |

**Why the first raw probe did not catch it.** The 405k raw probe established that the
panes existed, but it wrote its own output to a file instead of round-tripping through
the consumer's parse, so it proved the topology while missing that the consumer's read of
the same query was being corrupted upstream of `ae`. Necessary, not sufficient.

**What changed.** The documented minimum scrubbed environment now pins a UTF-8 locale
(`LANG=LC_ALL=en_US.UTF-8`) instead of C, in both the arm consumer environment and the
sandbox builder. `ae` is a UTF-8 program — its own output carries the origin envelope,
box drawing and separators — and tmux changes its output encoding with the locale, so C
was the wrong scrub: it silently altered the bytes the product reads. Determinism is
preserved; the locale is still pinned, just pinned to UTF-8.

**Two standing admissibility checks were added, both proven to answer in both directions
before anything is captured through them:**

1. **Consumer TAB round-trip.** Every case proves, in ITS OWN scrubbed environment and
   before any capture, that a real TAB survives the exact query the frozen consumer
   makes. Published per case as `env-tab-selfcheck.txt`, together with a PAIRED RAW
   capture of the identical probe under `LANG=LC_ALL=C` on the same throwaway server —
   both byte strings, no comparison verdict. A failed check is a HARNESS-ABORT: the case
   takes no capture at all.
2. **In-process tmux trace.** A PATH-first `tmux` shim (pure delegate-and-log — it has no
   active mode) records, per consumer invocation, the effective `AE_TMUX_SERVER` /
   `AE_TMUX_SERVER_KIND` the consumer carried, the effective locale, and the DELEGATED
   argv. A leading `-S <socket>` in that argv is the frozen ambient shim
   (ae@72c7293:80-89) having been installed; its absence would mean the call went to the
   default server. Published per consumer as `out/<label>.tmuxtrace`. Its
   inactive-equivalence is proven on each live arm's own stable topology by running the
   read-only queries through the shim and through the real binary and comparing stdout,
   stderr and rc byte for byte (`tmux-shim-equivalence.txt`).

**Blast radius and what was redone.** All template groups were rebuilt and A1 was re-run
in full under the corrected locale; both are republished here and their fingerprints
changed. The superseded fingerprint table is kept verbatim as
`templates/FINGERPRINTS.superseded-pre-locale-fix.tsv`, and the superseded captures remain
in git history rather than being reverted. **Step 0 (T-WD) was NOT rebuilt**, by ruling:
the watchdog's own pane query is `|`-separated (ae@72c7293:16511), its dead/throttle
detection is `capture-pane` based, the generated `_lib` contains no TAB-separated tmux
format at all, and the harvested bytes are hash-verified — so the reasoning is recorded
rather than the rebuild being done silently. `G2/dead`, `G2/stale` and `G2/throttled` are
copies of that archive and their fingerprints are correspondingly unchanged.

**Related observation, recorded not chased.** The b0exec `LANG=C` agent-resolution
observation is plausibly the same tmux behaviour: the frozen script has seven
TAB-separated tmux format sites — the two pane/alive walks above plus five pane-id/agent
resolution sites at :6488, :12151, :12170, :12297 and :12962. No special arm was built
for it. Instead the paired capture falls out of work already being done: every case's
`env-tab-selfcheck.txt` carries the UTF-8 and C probes side by side, and the 405k live
arm runs the ENTIRE consumer battery twice on one unchanged topology —
`consumers.s0-baseline` under UTF-8 and `consumers.s0-baseline-clocale` under C, identical
AE_HOME, socket, topology and argv. Both halves are published raw; which, if either, is
the product's intended behaviour is not decided here.

**Post-fix state.** With the locale corrected, the 405k rendering is coherent with the
topology at every stage: both agents `alive:true` at baseline and after the extra pane
appears, and `alive:false` for exactly the roster slot whose pane was killed, with
`attn:dead` appearing only at that stage. No list/status-versus-topology divergence
remains.

## Step 0 — T-WD producer precursor (fixture harvest for G2)

Design executed: `docs/migration/evidence/twd-precursor.md` v2 (approved 4af7be0).
Producer: the REAL generated watchdog of a real `ae` launch at the frozen commit
(`<meta>/watchdog _run`). No hook patch, no clock shim; pacing rides the documented
`AE_WATCHDOG_*` knobs, recorded per arm. Arms are named by MANIPULATION.

### Arm `a1` — manipulation: kill only the fake-agent child of the pane (pane shell returns to foreground)

| field | value |
|---|---|
| `frozen_sha` | `72c729343a0117af2968b66e1c43f89ad25fc0b2` |
| `frozen_ae_sha256` | `b7b8aa9fb77afc0705abdfaadf60cc58911f1cac46fe2ec993578fe5451575fd` |
| `producer` | `generated-watchdog-from-this-launch (<meta>/watchdog _run)` |
| `interpreter_version` | `GNU bash, version 5.3.15(1)-release (aarch64-apple-darwin25.4.0)` |
| `interpreter_sha256` | `6ba6319962b59831740a56aa5f65e91d6467c72997f5d0a18be1ba1a6d8d378b` |
| `tmux_version` | `tmux 3.7b` |
| `fake_agent_bin` | `/tmp/aecx/bin/aefake` |
| `fake_agent_sha256` | `147363f35ae27912a9fb041dc06648f3d0e8da23111357325a05f49b67135940` |
| `fake_agent_src_sha256` | `9f1d7503f021ff4c091a9736f43975ee1e45f704ab88c90e67baac3d39ecc841` |
| `uname` | `Darwin 25.6.0 arm64` |
| `clock_shim` | `none` |
| `hook_patch` | `none (unmodified 72c7293 copy)` |
| `knob.AE_WATCHDOG_INTERVAL_SEC` | `5` |
| `knob.AE_WATCHDOG_STALE_MIN` | `1` |
| `knob.AE_WATCHDOG_MAX_NUDGES` | `2` |
| `knob.AE_WATCHDOG_THROTTLE_ALERT_CYCLES` | `2` |
| `knob.AE_WATCHDOG_TG_SUPERVISE_SEC` | `0` |
| `knob.AE_SEND_DEFER_SEC` | `5` |
| `env.TZ` | `UTC` |
| `env.LANG` | `C` |
| `env.SHELL` | `/bin/zsh` |
| `launch_rc` | `1` |
| `wd_pane` | `%2` |
| `agent_pane` | `%0` |
| `instrument_selfcheck_positive_rc` | `0 (harness positive control: barrier reached)` |
| `instrument_selfcheck_negative_rc` | `3 (harness negative control: bounded timeout)` |
| `start_utc` | `2026-08-20T14:59:57Z` |
| `end_utc` | `2026-08-20T15:00:46Z` |
| `pre_manipulation_aefake_pids` | `59585 ` |
| `pre_manipulation_pane_current_command` | `aefake` |
| `pre_manipulation_pane_pid` | `59360` |
| `manipulation_utc` | `2026-08-20T15:00:16Z` |
| `post_manipulation_aefake_gone` | `1` |
| `post_manipulation_pane_current_command` | `zsh` |
| `barriers_crossed` | `8` |
| `inconclusive_barriers` | `0` |

Artifact paths (all under `docs/migration/evidence/batch-c-artifacts/twd-precursor/`):

- `a1/run-manifest.txt` — knobs, hashes, barrier ledger
- `a1/events/events.<label>.jsonl` — events.jsonl bytes copied at each barrier
- `a1/watchdog/watchdog.<label>.log` — the producer's own log lines (which code path ran)
- `a1/panes/panes.<label>.txt` — pane snapshots, producer's own capture form
- `a1/tmux/tmux.<label>.txt` — server/session/window/pane/client snapshots
- `a1/fs-manifests/manifest.<label>.txt` — recursive AE_HOME manifest (type/mode/hash/symlink/path)
- `a1/stamps/stamp.<label>.txt` — barrier stamp (epoch, utc, pgrep, byte counts)
- `a1/meta.at-launch.txt`, `a1/meta.final.txt` — session meta bytes
- `a1/ae-launch.out`, `a1/ae-launch.err` — launch stdout/stderr
- `a1/SHA256SUMS.txt` — hash of every file above

Harvested event bytes:

| field | value |
|---|---|
| `source_events_file` | `/tmp/aecx/twd/a1/cap/events.final.jsonl` |
| `source_events_sha256` | `95a330470df9c871d60ee25e540a2d55ccebdcd3b0bfd4cb2c439d09bce49b13` |
| `source_events_bytes` | `141` |
| `total_specimens` | `1` |
| `alert_family_specimens` | `1` |
| `all_actions` | `['alert']` |
| `alert_family_actions` | `['alert']` |

### Arm `a2` — manipulation: none after launch — fake agent alive, pane left static; shortened stale threshold and nudge cap

| field | value |
|---|---|
| `frozen_sha` | `72c729343a0117af2968b66e1c43f89ad25fc0b2` |
| `frozen_ae_sha256` | `b7b8aa9fb77afc0705abdfaadf60cc58911f1cac46fe2ec993578fe5451575fd` |
| `producer` | `generated-watchdog-from-this-launch (<meta>/watchdog _run)` |
| `interpreter_version` | `GNU bash, version 5.3.15(1)-release (aarch64-apple-darwin25.4.0)` |
| `interpreter_sha256` | `6ba6319962b59831740a56aa5f65e91d6467c72997f5d0a18be1ba1a6d8d378b` |
| `tmux_version` | `tmux 3.7b` |
| `fake_agent_bin` | `/tmp/aecx/bin/aefake` |
| `fake_agent_sha256` | `147363f35ae27912a9fb041dc06648f3d0e8da23111357325a05f49b67135940` |
| `fake_agent_src_sha256` | `9f1d7503f021ff4c091a9736f43975ee1e45f704ab88c90e67baac3d39ecc841` |
| `uname` | `Darwin 25.6.0 arm64` |
| `clock_shim` | `none` |
| `hook_patch` | `none (unmodified 72c7293 copy)` |
| `knob.AE_WATCHDOG_INTERVAL_SEC` | `5` |
| `knob.AE_WATCHDOG_STALE_MIN` | `1` |
| `knob.AE_WATCHDOG_MAX_NUDGES` | `2` |
| `knob.AE_WATCHDOG_THROTTLE_ALERT_CYCLES` | `2` |
| `knob.AE_WATCHDOG_TG_SUPERVISE_SEC` | `0` |
| `knob.AE_SEND_DEFER_SEC` | `5` |
| `env.TZ` | `UTC` |
| `env.LANG` | `C` |
| `env.SHELL` | `/bin/zsh` |
| `launch_rc` | `1` |
| `wd_pane` | `%2` |
| `agent_pane` | `%0` |
| `instrument_selfcheck_positive_rc` | `0 (harness positive control: barrier reached)` |
| `instrument_selfcheck_negative_rc` | `3 (harness negative control: bounded timeout)` |
| `start_utc` | `2026-08-20T15:00:48Z` |
| `end_utc` | `2026-08-20T15:06:31Z` |
| `observation_window_cycles` | `64` |
| `agent_stdin_log_sha256` | `bb838301c4c0671d61d3dcf3c4805fb533596e5b770593088069ec0ad2a69f11` |
| `agent_stdin_log_bytes` | `476` |
| `final_aefake_pids` | `70841 ` |
| `barriers_crossed` | `64` |
| `inconclusive_barriers` | `0` |

Artifact paths (all under `docs/migration/evidence/batch-c-artifacts/twd-precursor/`):

- `a2/run-manifest.txt` — knobs, hashes, barrier ledger
- `a2/events/events.<label>.jsonl` — events.jsonl bytes copied at each barrier
- `a2/watchdog/watchdog.<label>.log` — the producer's own log lines (which code path ran)
- `a2/panes/panes.<label>.txt` — pane snapshots, producer's own capture form
- `a2/tmux/tmux.<label>.txt` — server/session/window/pane/client snapshots
- `a2/fs-manifests/manifest.<label>.txt` — recursive AE_HOME manifest (type/mode/hash/symlink/path)
- `a2/stamps/stamp.<label>.txt` — barrier stamp (epoch, utc, pgrep, byte counts)
- `a2/meta.at-launch.txt`, `a2/meta.final.txt` — session meta bytes
- `a2/ae-launch.out`, `a2/ae-launch.err` — launch stdout/stderr
- `a2/agent-stdin.log` — the bytes the pane RECEIVED (fake agent's stdin, no echo)
- `a2/SHA256SUMS.txt` — hash of every file above

Harvested event bytes:

| field | value |
|---|---|
| `source_events_file` | `/tmp/aecx/twd/a2/cap/events.final.jsonl` |
| `source_events_sha256` | `0553a5be84ee5c4b33b4f816447e8207dc7455cd87024ecf872123c9f6cd407c` |
| `source_events_bytes` | `640` |
| `total_specimens` | `3` |
| `alert_family_specimens` | `1` |
| `all_actions` | `['alert', 'nudge']` |
| `alert_family_actions` | `['alert']` |

### Arm `a3` — manipulation: two-phase pane content in ONE sandbox / ONE running watchdog: phase A prints a documented generic phrase; phase B prints nonmatching lines that displace it

| field | value |
|---|---|
| `frozen_sha` | `72c729343a0117af2968b66e1c43f89ad25fc0b2` |
| `frozen_ae_sha256` | `b7b8aa9fb77afc0705abdfaadf60cc58911f1cac46fe2ec993578fe5451575fd` |
| `producer` | `generated-watchdog-from-this-launch (<meta>/watchdog _run)` |
| `interpreter_version` | `GNU bash, version 5.3.15(1)-release (aarch64-apple-darwin25.4.0)` |
| `interpreter_sha256` | `6ba6319962b59831740a56aa5f65e91d6467c72997f5d0a18be1ba1a6d8d378b` |
| `tmux_version` | `tmux 3.7b` |
| `fake_agent_bin` | `/tmp/aecx/bin/aefake` |
| `fake_agent_sha256` | `147363f35ae27912a9fb041dc06648f3d0e8da23111357325a05f49b67135940` |
| `fake_agent_src_sha256` | `9f1d7503f021ff4c091a9736f43975ee1e45f704ab88c90e67baac3d39ecc841` |
| `uname` | `Darwin 25.6.0 arm64` |
| `clock_shim` | `none` |
| `hook_patch` | `none (unmodified 72c7293 copy)` |
| `knob.AE_WATCHDOG_INTERVAL_SEC` | `5` |
| `knob.AE_WATCHDOG_STALE_MIN` | `1` |
| `knob.AE_WATCHDOG_MAX_NUDGES` | `2` |
| `knob.AE_WATCHDOG_THROTTLE_ALERT_CYCLES` | `2` |
| `knob.AE_WATCHDOG_TG_SUPERVISE_SEC` | `0` |
| `knob.AE_SEND_DEFER_SEC` | `5` |
| `env.TZ` | `UTC` |
| `env.LANG` | `C` |
| `env.SHELL` | `/bin/zsh` |
| `launch_rc` | `1` |
| `wd_pane` | `%2` |
| `agent_pane` | `%0` |
| `instrument_selfcheck_positive_rc` | `0 (harness positive control: barrier reached)` |
| `instrument_selfcheck_negative_rc` | `3 (harness negative control: bounded timeout)` |
| `start_utc` | `2026-08-20T15:06:33Z` |
| `end_utc` | `2026-08-20T15:07:39Z` |
| `phaseA_phrase_literal` | `429 Too Many Requests` |
| `phaseA_phrase_sha256` | `9bfee574ec1da5f37ce1af2779f320181faeed87153d2a04799b3cb6e315f0c0` |
| `producer_pane_view_cmd` | `capture-pane -p -J -S -40 -E -` |
| `phaseA_inject_utc` | `2026-08-20T15:06:52Z` |
| `phaseA_producer_view_phrase_occurrences` | `1` |
| `phaseB_fill_lines` | `200` |
| `phaseB_inject_utc` | `2026-08-20T15:07:18Z` |
| `phaseB_producer_view_phrase_occurrences` | `0` |
| `phaseB_producer_view_sha256` | `24344cf608581711a1febc10a80f646fd58d38d1401122fb27436e334443fc6c` |
| `final_aefake_pids` | `54970 ` |
| `barriers_crossed` | `11` |
| `inconclusive_barriers` | `0` |

Artifact paths (all under `docs/migration/evidence/batch-c-artifacts/twd-precursor/`):

- `a3/run-manifest.txt` — knobs, hashes, barrier ledger
- `a3/events/events.<label>.jsonl` — events.jsonl bytes copied at each barrier
- `a3/watchdog/watchdog.<label>.log` — the producer's own log lines (which code path ran)
- `a3/panes/panes.<label>.txt` — pane snapshots, producer's own capture form
- `a3/tmux/tmux.<label>.txt` — server/session/window/pane/client snapshots
- `a3/fs-manifests/manifest.<label>.txt` — recursive AE_HOME manifest (type/mode/hash/symlink/path)
- `a3/stamps/stamp.<label>.txt` — barrier stamp (epoch, utc, pgrep, byte counts)
- `a3/meta.at-launch.txt`, `a3/meta.final.txt` — session meta bytes
- `a3/ae-launch.out`, `a3/ae-launch.err` — launch stdout/stderr
- `a3/agent-stdin.log` — the bytes the pane RECEIVED (fake agent's stdin, no echo)
- `a3/producer-view.phaseA-injected.txt` — positive capture of the producer's pane view at that point
- `a3/producer-view.phaseB-displaced.txt` — positive capture of the producer's pane view at that point
- `a3/SHA256SUMS.txt` — hash of every file above

Harvested event bytes:

| field | value |
|---|---|
| `source_events_file` | `/tmp/aecx/twd/a3/cap/events.final.jsonl` |
| `source_events_sha256` | `785ce345e5eacb2688bc0ddd0cff5526120e05b3c486a84a7c04cd9aa2c08cab` |
| `source_events_bytes` | `441` |
| `total_specimens` | `3` |
| `alert_family_specimens` | `3` |
| `all_actions` | `['alert', 'throttle-cleared', 'throttled']` |
| `alert_family_actions` | `['alert', 'throttle-cleared', 'throttled']` |

### Per-specimen enumeration (every harvested event line, individually hashed)

Machine-readable: `twd-precursor/specimens/specimens.<arm>.jsonl` (ALL lines) and
`twd-precursor/specimens/alert-specimens.<arm>.jsonl` (the alert-family subset:
`alert`, `alert-cleared`, `throttled`, `throttle-cleared`). Each record carries
`arm`, `line_no`, `byte_offset`, `byte_len_with_nl`, `byte_len_no_nl`,
`sha256_line_with_nl`, `sha256_line_no_nl`, the captured `action`/`actor`/`ref`/
`summary`/`ts` byte values, `first_seen_capture_label`, and `raw_line_no_nl`.
A field absent from the producer's bytes is recorded as the sentinel `\u0000ABSENT`,
distinct from an emitted empty string.

Alert-family specimens, by hash:

| arm | line | byte offset | len (+nl) | sha256 (line, no trailing newline) | action | actor | target/ref | first seen at |
|---|---|---|---|---|---|---|---|---|
| `a1` | 1 | 0 | 141 | `eebbacd1a49fff567274f76ad68d644a06248168961cdc257c45339861c82435` | `alert` | `_watchdog` | `ABSENT` | `obs-c1` |
| `a2` | 3 | 484 | 156 | `913a7a3ebed0a14ad34cee292e7bbfa706111ac9c45977849833e6f3dbdc3f9b` | `alert` | `_watchdog` | `ABSENT` | `obs-c13` |
| `a3` | 1 | 0 | 153 | `928a41591fce09a30408eb9a34dae36a90dfc3b4f74f21434a6be2d90f4a9ee9` | `throttled` | `_watchdog` | `ABSENT` | `phaseA-c1` |
| `a3` | 2 | 153 | 142 | `3af565f24c621cbdf3dd1700d811dacc1338516f5ef35e88cd0f78ba9d016370` | `alert` | `_watchdog` | `ABSENT` | `phaseA-c2` |
| `a3` | 3 | 295 | 146 | `7505a850bb7e8b9766c9d4478cee48407827f38f590caf9a63a499af8b999299` | `throttle-cleared` | `_watchdog` | `ABSENT` | `phaseB-c1` |

Alert-family specimen count across all arms: **5**.
Set equality against this enumeration is provable from
`twd-precursor/specimens/alert-specimens.*.jsonl` + `specimens/SHA256SUMS.txt`.


### Harness, provenance, and admissibility records (step 0)

- `twd-precursor/harness/` — the exact harness that produced the arms:
  `aefake.c` (the controllable fake agent), `lib.sh` (sandbox construction, capture
  helpers, bounded waits), `arm.sh` (common runner + run-manifest emission),
  `arm1.sh` / `arm2.sh` / `arm3.sh`, `runall.sh`, `publish.sh`, `enumerate.py`,
  `SHA256SUMS.txt`, and `NOTE-arm.sh-revision.txt` (records the only post-run edit:
  two harness label strings, no behavioral change).
- **Fake agent**: a compiled non-shell executable with its own
  `pane_current_command` identity (`aefake`), a fixed startup banner, terminal echo
  disabled so the pane is stable, real `send` deliveries accepted and written to
  `agent-stdin.log` (never echoed to the pane), and controller-driven lines printed on
  demand through a FIFO. Never a live model, never a plain shell.
- **Producer capture form**: the producer reads its pane with
  `tmux capture-pane -p -J -S -40 -E -` (frozen `ae` `_watchdog_capture_pane`); every
  `panes.<label>.txt` and `producer-view.*.txt` in this archive uses that same form.
- **Instrumentation admissibility** (cluster-plan.md global rule): no hook patch and
  no clock shim were used — the frozen `ae` copy is byte-identical to `72c7293` and
  the arms ride the documented `AE_WATCHDOG_*` knobs, so there is no inactive-hook
  equivalence obligation for this step. Harness artifacts (capture files, the fake
  agent's stdin log, the FIFO) live outside the product state that
  `fs-manifests/manifest.<label>.txt` hashes.
- **Barrier instrument self-check**: each arm proves its cycle barrier answers BOTH
  ways on that run before any capture is taken through it — a positive control (the
  next producer cycle is reached, rc 0) and a negative control (an unreachable cycle
  count times out, rc 3). Recorded per arm in `run-manifest.txt`. Two earlier harness
  revisions were discarded and re-run rather than reported: one whose barrier polled a
  frozen snapshot (could only ever time out) and one whose `grep -c` fallback emitted
  a doubled count. No capture in this archive comes from either.
- **Bounded windows**: every barrier has a fixed timeout and every expiry is recorded
  as `OUTCOME=INCONCLUSIVE` in `run-manifest.txt`. Across a1/a2/a3 the recorded
  INCONCLUSIVE count is 0.
- **Published-copy equality**: `twd-precursor/<arm>/events/events.final.jsonl` is
  byte-identical to the `source_events_file` named in
  `specimens/summary.<arm>.json` (verified by sha256 at publish time); the `/tmp`
  paths in those summaries are the scratch originals.
## Template groups G1-G11 (+G2b) and the A1 members

A template MEMBER is a snapshot of a whole `AE_HOME` (its `config` plus
`sessions/<session>/`), so an arm clone is a working `AE_HOME` and the M2 bootstrap
never runs against a protected tree. Every member is stored chmod-protected (`a-w`) and
fingerprinted twice — before protection and after — over `_meta/<member>.modes.tsv`, the
recursive manifest (type, mode, content hash, symlink target, path for every file). That
manifest also records each path's ORIGINAL producer-written mode, so a WRITABLE clone
restores exactly what the producer wrote instead of inheriting the store's protection.

**Producer-derivation**: every fixture byte comes from a real frozen producer — a real
`ae` launch at 72c7293 for meta and the helper set, the real generated helpers
(`state`/`goal`/`memo`/`say`/`send`/`ask`/`review`/`reply`/`spawn`) for event bytes, the
real generated watchdog for alert and recover bytes, and the T-WD precursor archive for
the dead/stale/throttled families. Nothing is hand-authored. Members marked *byte copy*
differ from their source only by the NAMED mutation in `_meta/<member>.mutation.txt`,
which carries before/after sha256, byte counts, and the removed/inserted byte spans.

**Rebuilt under the corrected locale.** These fingerprints supersede the ones committed at
`305b16e`; the superseded table is kept verbatim as
`templates/FINGERPRINTS.superseded-pre-locale-fix.tsv` and the forcing defect is described
in the correction section above. `G2/dead`, `G2/stale` and `G2/throttled` are the only
members whose fingerprints are UNCHANGED: they are copies of the step-0 archive, which was
ruled not to be rebuilt.

| group | member | session(s) | files | fingerprint (pre-protection) | fingerprint (protected) | superseded pre-protection fp |
|---|---|---|---|---|---|---|
| `A1` | `510b-empty-vs-omitted` | `ta1b` | 42 | `e16decdda8210de95018c5014443cd4a634969320fa3fdbba1a8bd51ff4b1163` | `a6740d4fc379cdb0b65f737fec64aeeebd9b5b8ad020d1e1f1d46cdea8ddaa04` | `(new member)` |
| `A1` | `510c-recover-ref` | `ta1c` | 33 | `616f38cb9e59690b033ee07f6e8c95dacda1b70a72c9ea05f9a8570f2e92b10f` | `68721ca91ce8dc7c8b71859582be718fc5a657668181081992694d4e4dabdbf6` | `(new member)` |
| `A1` | `510e-dupkey-known-reversed` | `tg1` | 42 | `78d1edc6a255d058aa1ed83f46cb7956a9ba14779f16f871fcbdc571a995365b` | `0d586ef284c2ea5502216f81c0310eefc345dfe389de39f3caf730d3fe15dd85` | `(new member)` |
| `A1` | `510e-dupkey-known` | `tg1` | 42 | `bb8d8aeaefeae66e260f0950cca159ccb891b6c858ce7cd138a09e0e1e0135b4` | `588c8d7d688e6bcf90045320b63caf955423750525c9f7ffde870742c2223b10` | `(new member)` |
| `A1` | `510f-dupkey-unknown-reversed` | `tg1` | 42 | `3fc90b2e6ace970f570a3c52f3c0c7bf5482745be24b311a99ab105455dd8f00` | `3a71a1286311ed968b85651dbfe2b6014f4e52e235cf900ea6f5dadb7889632a` | `(new member)` |
| `A1` | `510f-dupkey-unknown` | `tg1` | 42 | `077199567900528338bcfd0cdb0f295ddfd29709bfdea0e5f2366adce18d15d9` | `470da1778b62611b4c09fa61cc19c6d2e71af2e45aba7331c89edc22440ed804` | `(new member)` |
| `A1` | `511a-omitted-routing` | `ta1r` | 39 | `7d3465fc653cabc05248dd804ef354cf535780acca5eceb43a025903e2e03f27` | `d4852d1b4b5f683c55ebc400c4d3e9b316ae9f69d007c660a385d6a50ac13da8` | `(new member)` |
| `A2` | `composite` | `tg1, tg2b, tg2bl, tg2un, tg2wu, tg6a, tg6b, twda1, twda2, twda3` | 317 | `a5b346582285e0b90f6aa3ebab5a5abf0faba841c384a077794b973ae95e4b0b` | `30d1803e4b1dffb513cab1c409cbfda50033411f60e9eefc4e8fe02c1fb2311f` | `(new member)` |
| `A3` | `017g-unanswered-vs-agent-owned` | `ta3g` | 38 | `5a610303e96ad18a7ddbed4245e68e94617184336a290f4c276846e978b9375b` | `ccd721fb59bb9799c11e2a55bcec9906946400408aeb7f3610394a999763edcd` | `(new member)` |
| `A3` | `524a-future-ts-ordinary-mtime` | `tg1` | 42 | `fe7dac214fc310d387f7d983bc5d1e8d17bc272a519efc8efc53b2d14152c49a` | `d56b99e4b85163d19d3967874a21d66f6397225e823a0053596bf459f24dd49b` | `(new member)` |
| `A3` | `524b-ordinary-ts-future-mtime` | `tg1` | 42 | `075d5a2c2b065c8511193a8b16ee9d7785ae91ce8d9ba3020d7bc40a171ff667` | `c940ecaed0bba78bb6b78f7a286ed3c65f601aa18f5a0babda6d2910aa062f18` | `(new member)` |
| `A3b` | `competing-noclear` | `tcompetingnoclear` | 39 | `47bbcb7b8c71b6a6bcb3762879d8a5ddf6444885d18e090bb4d8813e0a09bdbd` | `7596649ff596114dc7e375f6d8cc4648a4355a9ff48bdb81ae5b8c293a049166` | `(new member)` |
| `A3b` | `pair-blocked-over-throttled` | `tpairblockedoverthrottled` | 33 | `b0db024203d8a13b3e326daf1cb9b7214f0fc9d1d41fb3f508e5e83758f487df` | `bb7441fd113cd9dad8e2442b7a734809d57ae824a7826d412b24664e8019cde5` | `(new member)` |
| `A3b` | `pair-dead-over-stale` | `tpairdeadoverstale` | 38 | `4529353c588016259b7b6b7dd7e0af3d2769d64f85959a2a7fb3c95df063430a` | `1e9c59a0ad58d77bb1c092229671c1d33d5903d09d48dc1f135ef099782ad420` | `(new member)` |
| `A3b` | `pair-stale-over-waitinguser` | `tpairstaleoverwaitinguser` | 38 | `1a4b9cce376ac21bb359117a452b6c0955f9623dd82ace1576c6d80b2f0714b4` | `979b9bdcfe49d9f3763975aa321c3de596c4d292cdfdc1553299ffa875e3a48e` | `(new member)` |
| `A3b` | `pair-throttled-over-unanswered` | `tpairthrottledoverunanswered` | 38 | `f397a68bc00ca03191655de0ec6207b07787c3f7149dc5695928c65f8e4afc64` | `e39eaf585b474db0f93ff670efe4de5c1b54a6333daa3d559e15c3d48cd15591` | `(new member)` |
| `A3b` | `pair-waitinguser-over-blocked` | `tpairwaitinguseroverblocked` | 33 | `211b69861f9f331b9664bf277abfe1cd007296067f08aad64fd2de7601561a3e` | `b31945ae8c4612964f2eacc9dddacebe1e90917c836467012618355c0e53825e` | `(new member)` |
| `A5` | `doctor-fixture` | `ta5` | 33 | `e3da566fb1c16e07f0bbdb7af612432fe22f13b9010bec50ae556044ebbb8c4c` | `5cd8a20578b15e0438e095a142d5c9e19439ebe962d8a03a50f980be96d2998c` | `(new member)` |
| `A7` | `405j-all-empty-members` | `ta7j` | 37 | `e87bfc836ee2c8b6554ff3434ae4c15588f455a5a06df48ee9efc354f2b2ea7d` | `4de89389ee74bf6d044465b126df825a9e440e34107d8f8601eb4cbfdd7e160d` | `(new member)` |
| `A7` | `405j-full-fresh` | `ta7j` | 37 | `385a7de811fc015c3d96874c242da2a1bfe59a6802bcb2efae23e018604b03b7` | `b62bd9f1f706794d34f2f1ac617fabfff3e1de78a0a7a7e88912bec492aedad2` | `(new member)` |
| `A7` | `405j-keyless-legacy` | `ta7j` | 37 | `a7b9eb3f36b386c6e7a0c62750bb154625687358cdda49223325b423b3f29176` | `5bfa681ddf302cefb481a27d5e1dfe02226487863708114fd1e57ef7ae0c3dc9` | `(new member)` |
| `A7` | `405j-one-empty-member` | `ta7j` | 37 | `6ae0ec32a3ffd7736d754e9b8f5438c7bcc060ec4262b5cb80c80e3c0d2953f7` | `70447a012db1950007286d433edce85edcae22cc8faaab6939f59e59f9121819` | `(new member)` |
| `A7` | `405j-session-only` | `ta7j` | 37 | `50cc613d6d1af2aebafbdd37ff266a90ac897059cfbc22843aa821af9096a9c4` | `d7a34bfdf2a4114b8303fdbdbd5266f2d7ea592e8a01662f3f01448427a1d3ae` | `(new member)` |
| `A7` | `405j-slot-only` | `ta7j` | 37 | `41fc0e810d6bd95b5e259c25e26a3b068ca721c971397ad2956c057fbf799aea` | `337a8a8cce7cf859bb683d6316d0c92b040a31402550f3a0ad72a58959b44095` | `(new member)` |
| `A7` | `405j-stale-mismatched-keys` | `ta7j` | 37 | `61c1b473e5900455919898f136065545a6eee4c85176b734745ae55bac3852d7` | `ef3776dd8976aecd7c26e88eac53dfde39fd5e016493919e77cb96249aa19899` | `(new member)` |
| `A7` | `branch-two-sources` | `ta7g` | 30 | `cdec74d07c6366c0517b3bbb39b42280f2f94c2e506ea07447568a3a332d78cb` | `3d0d67242624ee3f87cc1ac40810cd61898ffa404791d5e3a48431f0c2e26a55` | `(new member)` |
| `A7` | `goal-order-agreeing` | `ta7goalorderagreeing` | 32 | `8c4c952f273938065279c801e64025e429b5b8b3003185c3c743e326a819e757` | `21ea2c61d47b9df9d16e991a6bb34e3d9a4a9ce4dff439bdaabd606d8bcd5f07` | `(new member)` |
| `A7` | `goal-order-opposed` | `ta7goalorderopposed` | 32 | `b42f26403913bdbb3995a40b3bf5ce750e65bab5bb1b93747d162e73f9855e89` | `3fcccfcdb884cee041ab947a94ef971763226ab1b05660c93727c1de4a1a8a2b` | `(new member)` |
| `A7` | `goal-order-single` | `ta7goalordersingle` | 32 | `3ddc37471ae584993271329477a009edca01380bab9a6135a3a2d7d6270b303f` | `85821f2e87914324545ef84f7bf472fa2e9845d5bf341d9a3d0f5921df8c1f25` | `(new member)` |
| `A7` | `meta-duplicate-key` | `ta7a` | 32 | `98a22d1c6c5cf97cdac15ffa3278293c0feb3d05bb37d77e3764eb163a476321` | `4845e6721e72b08ad0b55e7af3379f64a95ff67960976a1bd0810d375b508ff0` | `(new member)` |
| `A7` | `meta-multi-equals` | `ta7a` | 32 | `7a8388b0822834920abe2242895c92f46c92079c7aeb6f2aa6a545a419220fd8` | `cc25fcd22ba8a493f0899c15fe4ad217e4b2adffbb8be69494ee4975ab3da260` | `(new member)` |
| `A7` | `pair-405j-all-empty` | `tg5` | 42 | `d3c2c20f1a0e16decb11d7911134e3e6af9c45544718f2acb651618d7c6d10b7` | `1a819a2878388c1e3450110792d2c35e199f6e7399c7a3236adebaf86cc6fa48` | `(new member)` |
| `A7` | `pair-405j-full-fresh` | `tg5` | 42 | `28dc10300e4bb7b260b1e9273cf6bddf2189910ff4e783f4d8eb346436b4cc04` | `fad76e71451d28616d5fee1fa330a6ab23bf5a5545b0ea12a3a748f04ad72205` | `(new member)` |
| `A7` | `pair-405j-keyless` | `tg5` | 42 | `dc659e622fd3217bf5276223336db604bc1f14cfd704dd64c38abc3fb4f05078` | `3683b2d418628bc0a004d576a7e3de39752aa4350002aae24a61991358277edc` | `(new member)` |
| `A7` | `pair-405j-one-empty` | `tg5` | 42 | `d56194e44fbe8e622f25657b3c51abfff8732715884fe820f7808ded2427312e` | `534a95fe54e26dc67a29395b82cf592fe7ce4b24cab5ce48f9f06be67328d781` | `(new member)` |
| `A7` | `pair-405j-session-only` | `tg5` | 42 | `95d1ad38acd838a216f08101742d37092660b0e92aae7d1fd6691e48fd61d659` | `08b1dd9a9112a345b10ba396c85888cc20af22a95ed70a43b1c6af8a3f031b15` | `(new member)` |
| `A7` | `pair-405j-slot-only` | `tg5` | 42 | `b5e8c248faaa292a1aab9ed5198015e30b81ebd5ae718b3e300ed5bcfb0493d8` | `f03746c23eb5f84c41e22672f6beb99dffb6d1ae68a74348e21e8ba747f92fe6` | `(new member)` |
| `A7` | `pair-405j-stale-keys` | `tg5` | 42 | `7d7e17f85e00f9ce46652c542964ba71b1227839d5414f17d64befc3f4cd56f9` | `0c1c8e59b9254ea5603eb5002edf689382c36abe05e92ab3fcdfe041479eb28d` | `(new member)` |
| `D` | `d02-pending-with-harvested-reply` | `td02` | 40 | `c5a47d626ffaf77aed1dcfd551e1c945a71d612f3ee9ffaa8d16aa40e8192f14` | `c9863e6ad2a50e4b1e9912b29957bf6d972b3e48425bc8ce6a07154b16f7cd24` | `(new member)` |
| `D` | `d03-31-numbered-events` | `td03` | 39 | `0bd4c21ce2c572b38527d83bc0ea8e8e82b830aee009453bfbfc1a18e9fa9dcb` | `d0d8630ccdb09be5fc6cdbe52960ccc134baea6840f2021c98fa47dd6656aa4d` | `(new member)` |
| `G1` | `healthy` | `tg1` | 42 | `075d5a2c2b065c8511193a8b16ee9d7785ae91ce8d9ba3020d7bc40a171ff667` | `c940ecaed0bba78bb6b78f7a286ed3c65f601aa18f5a0babda6d2910aa062f18` | `bbfdf6957bb62063e2c5c94fc36844bede3eeab190027799b5a11164ddeab5dd` |
| `G10` | `display-only-legacy` | `tg1` | 42 | `6de723ca980e505cb9833c1931ce95474c17192b8aea051cbc906b7fdb78de98` | `6d2ecd219feb5a0e2f77a46f9bee6b2ea5bf01e6841a438e6e096b3a9fe05d52` | `8d30b9a41b173a80a55376c5f840f390e50bd88bdfb5ec4dde08dfea632fa372` |
| `G10` | `same-display-diff-routing` | `tg10` | 39 | `285cd5137c196a605e32088c6ad4657096e6da8375fda6afc2fdf8bd88ed9785` | `296b9224a4903e3b783484f69eb46c286716ce6f1ecd7c88cd479ac2b75cfb06` | `ed4e8f498da25df8aebefec61ef278b4afcf2fbbbce5c2a9567eae7c28496bac` |
| `G11` | `escapes` | `tg11` | 49 | `9d5b086599c350ac99003dbe4baa0b326af0874c421a54e28c58ab8f042f77b0` | `b72f31d61edfe9bcec5aad8dfe1e53c69122edc2cd63c74332366d9d75ca534d` | `aac57e86f0e04b92777d0bd5364470148958dabc79e83cd386a4e5bece0597f3` |
| `G2` | `blocked` | `tg2bl` | 33 | `e8fd157ab8249d2f8cf0edbf33301acfd3d2dc53e11a3ddecdf7171965767087` | `66b8da9282e05e7cb61c70d354d3db1b4abf2d9c2d9e7fd18f4d5b5785d52a51` | `9935089d5c95d32398ba6d7d5ff1a03be2b0be3a0fc35b07215c4d368e248090` |
| `G2` | `dead` | `twda1` | 32 | `cd8a3a64cd530850f5ad8641ab8209344d0afa4ad50b6414cec489255792cb53` | `eff60da558789b3fbac3ae2794105894d058faeeace5bd8399348d31d3b359f5` | `cd8a3a64cd530850f5ad8641ab8209344d0afa4ad50b6414cec489255792cb53` (unchanged) |
| `G2` | `stale` | `twda2` | 37 | `c6085b8190566df8e823fc1189356879e7c183ed925ea5b9329c9a9c33df665a` | `c533c47b12dfb72506a60cf5361b55c0fc0adb1cab8a954fb82ed83f91669eb9` | `c6085b8190566df8e823fc1189356879e7c183ed925ea5b9329c9a9c33df665a` (unchanged) |
| `G2` | `throttled` | `twda3` | 32 | `5d3bae39c512c4415200e3729347171f62e8a6a9d95efbcd498313d5f8718fff` | `23c0a792899cf4bb169a391cdbf1ed1adf7fb2e8df5a2232f37e17c9152a3dc2` | `5d3bae39c512c4415200e3729347171f62e8a6a9d95efbcd498313d5f8718fff` (unchanged) |
| `G2` | `unanswered` | `tg2un` | 37 | `e3b001a06f56b2635329ae424bbd55ce13e796a091ff11df9099ba5be8388981` | `82949f5e4b21565eef74b5e446c9d10723eabe3148c14b19fa15e9b739e262ab` | `50ffdd45792750ecb7ed0c6f682aceaa6c25a7203cd41946c271024045a5c7f1` |
| `G2` | `waiting-user` | `tg2wu` | 37 | `c32946b6a4cb67d93fcbc31a82124ee24873a8c59119b253dc655394532f06c7` | `bc93e334397500ba409957e952ae7d502ca3ca0cdb66f6467293fb54d4815570` | `24c2cbcec497f267cafc3b312fb027b99a0515a605194a21f21ffcb4bd5fd439` |
| `G2b` | `competing` | `tg2b` | 38 | `0d7df5c73481c9aa92d568ac64990a0563e1f59dcce651b5a7237bd6d5c6debe` | `57f784caf0acc08884823490f6242c82d9f002fa3547f4e5e8a06397f89f9a86` | `58311be71fd4a5bc2db5cd2668ffe90cfba530f353dfe4c85b3729e59cbeea66` |
| `G3` | `malformed-complete-line` | `tg1` | 42 | `d10d3366168f38c4cbd6004ccef66ccecd3f7b244c84757f1fa8b7c5436b48fc` | `0be46cd1df0d89453f7ac919c28afc96373c24d4a67ad8aaf41461ecfc20aaa4` | `567dc0a279c86307fd4225dc4981a55b6412db59db3cf876fe21727ba876c964` |
| `G3` | `meta-mode-000` | `tg1` | 42 | `03fc645781ec224d20f5f6d11b0c87280197126af028f6047893d7dc8354a8f6` | `167845758dd6755ea404ebbb53bef4abc3ecdc8e0511d90ed566dd9c0169180f` | `aedf591ba17afa050ae38b2b488c8b98d8dafcce753ae7d38f4940233858aa3c` |
| `G4` | `no-events` | `tg1` | 41 | `a248386e82ffad7f33cb3ef015c058d4098cdc9b8e409e14cc5f70a982f0efed` | `dfce9e3f5e9a4d7b1fa5c26e4080109521702b9eb6f3c0cfdc5722485638e4dc` | `5e18fb79539713c2ac885c5323f0e79b4b64f3bea11917297c3525372caca000` |
| `G4` | `zero-byte-events` | `tg1` | 42 | `98d0044246b078b8bbcadb4459b2ebdfd8a47b0e7ff8fe40c625528523cc9463` | `17ebdc098212cbd2459353d53f67b7dbab1284b852a1b14cc37685b5a4c0cdba` | `dcb21ed0c682902efc637a29a27b8b2db165d7be874b5c261aa450c02e6780eb` |
| `G5` | `m1-control` | `tg5` | 42 | `28dc10300e4bb7b260b1e9273cf6bddf2189910ff4e783f4d8eb346436b4cc04` | `fad76e71451d28616d5fee1fa330a6ab23bf5a5545b0ea12a3a748f04ad72205` | `b8c3259102203f74a460ec56d466a06d068455a692c2490cfda6aee4c581de53` |
| `G5` | `m2-wrong-ref` | `tg5` | 42 | `886e4cca0949bf281e58e287359141b652ab8456c5c92bff9556a9b1b3c0cab1` | `779e7deb29e2138092219022404412d7dab0582d8f826ed41d93697fc2efa394` | `daf9b7375f4c8fa7f011b24f1e7a60dac8c86dcb8e9a6ae3371fbc95b16aca6b` |
| `G5` | `m3-wrong-actor` | `tg5` | 42 | `6757b54db877496486ca4a89d86b0ed084bbad6183bb0bf8d89baf44cd65d257` | `8b6e96f784a004283d32a4dfa03ba9e306af305c0616e333460b461f22e2a8e3` | `707e1807d8750479f87847ad361f160bbe2a4386b3fdaad3bfffc554abbad135` |
| `G5` | `m4-wrong-target` | `tg5` | 42 | `b39dd916d0937584e0bc0e88afb8f206c0ba8dceded78ae69fb83c3f6c8b76ae` | `7910a388cbfd7619f5fdce05d44707214a8d83adf4fa1dbfc9e04b983860eaaf` | `d3dca7169df2847903530850c22960c2a26cedabe2403c27c4c1752daed54d1b` |
| `G5` | `m5-routed-vs-routed-mismatch` | `tg5` | 42 | `99b236f7fa50620ae90d18c88252257a32db3478d74f7567496751f83c5a36af` | `3c8cf713fd7ca99b4ad30c3c99ab08259dbd7d0f6c9a47cfc44cb88d63526b95` | `88f8396d3ae64e9ed4b28c5bd1c3eecff525242bd4a64e4c479a003256aeded9` |
| `G5` | `m6-mixed-routed-display` | `tg5` | 42 | `dc659e622fd3217bf5276223336db604bc1f14cfd704dd64c38abc3fb4f05078` | `3683b2d418628bc0a004d576a7e3de39752aa4350002aae24a61991358277edc` | `c2855c324a8ed5620d618761e0d6e1973aeb84b648eeafffd39c666872d19849` |
| `G6` | `stopped-attention` | `tg6b` | 33 | `568271e35faf9872d49ea35517bc3b0e6f21193fa1f9bdfd0daf293457cb6ce3` | `77273b71fc99506fe4eb2b535ee890470753d2973c96f15bd50d6046e1dfc4f5` | `efe1955bfc91cd0218f0245267896eef75d037554d75d47ad4a2c2892fe07ee6` |
| `G6` | `stopped-plain` | `tg6a` | 37 | `1632a52265ea330162416eb35df2529c72bd59fb1d02cdc353265b08bb7e6b83` | `a9b599015269fb53c8773dc201fa672e3635fe7eab3496b6786b38d222446dd1` | `8120c7cb5b5c457166bc8e61a38006fd1eafb3729bffa1648657c7c9ea44b17c` |
| `G7` | `events-unknown-action` | `tg7a` | 39 | `e1f8c849e17e3ea93939c281aaab642c365b7392ac53fdc6d8bb84719b1d4e52` | `9d39d69a507af1f96ebec0443ecc78cb6c6c19e7ee784fb291b23099807dcb0f` | `fd9ea1661827ec84bd518462beced7e3e0b6bbd2b4b7050f29986753317ffc99` |
| `G7` | `events-unknown-keys` | `tg1` | 42 | `b6bf1820d37210e25d00a91fbc70d8b8117a6430cbde58ce5bac9f12851a318d` | `27f3dcb2d61df9482caab93330f5d4373842007e7d223ad1a8a114627f2c4495` | `2652b58f18f883e6be5facff84f3d156811b2f77fa1730ed4609c58c5e89b926` |
| `G7` | `meta-unknown-keys` | `tg1` | 42 | `2e6d148627736058c398bdeb40169c564a61e784ab51cadb22eb2f62547ffa50` | `e8f8e99e3976a7de9e597dc9bd76de02766171f21908b147d514c13821b97c20` | `693b3ddf0e39655ea3865f3cc4c5d7e982c7d38e0189a207c67c762673fef885` |
| `G8` | `no-trailing-newline` | `tg1` | 42 | `5ad4e8108ee97ba2d72176a1053be7f06d44f7967a47e400881d991b47e3091e` | `d644c1fc58dbfb2769cc2f06ddbf1aaab2d004a8300e8ead920ea1b0688d7b16` | `7981f19b685c649571d0dd6ce67a67700314083671d780a41e758d95b019448d` |
| `G8` | `partial-trailing-record` | `tg1` | 42 | `46ff5fa69ee89099188b7c4e72c59f460618f70c92eaf95aaa2910f7c2638b74` | `f75a4ebe46759eae10217f00cbe1a93b321ceb46f02bab401a791d98accb5b3d` | `837c6bb0547a90bf59dbda97ce6dd6367f8662811478f91bc409a23a5468d53e` |
| `G9` | `goals-distinct-ts` | `tg9` | 32 | `d7e477f10f217dcd10e9ccd418aa9af79e80222cb0e718165c1a208ebd9e1b72` | `48f844ec7dd5cde59a576e706375875462e3a56833bec4f29df0d31b8fbcb808` | `f8a991667313065e7c2db99cffacd14d0d2a51858b055fb85b08f7e535dc4d90` |


### How each member was constructed

| group | member | construction |
|---|---|---|
| `G1` | `healthy` | 2-agent session, full launch-written meta, harvested event history with no attention-family bytes: state/goal/memo/send/ask/reply/state-done, ask answered |
| `G2` | `dead` | AE_HOME of T-WD arm a1 as the producer left it (watchdog alert bytes) |
| `G2` | `stale` | AE_HOME of T-WD arm a2 as the producer left it (nudge + alert bytes) |
| `G2` | `throttled` | AE_HOME of T-WD arm a3 with events.jsonl set to the producer state captured at barrier post-phaseA (named manipulation, byte diff recorded) |
| `G2` | `waiting-user` | fresh 2-agent launch; the worker runs the real `state waiting-user` helper |
| `G2` | `blocked` | fresh 2-agent launch; the worker runs the real `state blocked` helper |
| `G2` | `unanswered` | fresh 2-agent launch; a real `ask` produced under the clock hook (aged past the 30-minute default) and never replied to |
| `G2b` | `competing` | 3-agent session, one reason per agent, produced in arrival order T0<T1<T2 that is DESCENDING in the frozen `_attn_rank` ladder (ae@72c7293:3571-3581, comment at :3586): dead first (a real watchdog alert after the fake child under bravo's pane was killed), waiting-user second, unanswered ask last; clock hook throughout |
| `G3` | `meta-mode-000` | G1/healthy byte copy; named mutation: session meta mode -> 000, content bytes untouched |
| `G3` | `malformed-complete-line` | G1/healthy byte copy; named mutation: one closing quote deleted inside a COMPLETE, newline-terminated event line |
| `G4` | `no-events` | G1/healthy byte copy; named mutation: events.jsonl removed |
| `G4` | `zero-byte-events` | G1/healthy byte copy; named mutation: events.jsonl truncated to zero bytes |
| `G5` | `m1-control` | harvested ask->reply mirror pair plus a third-agent ask and a review, harvested so their genuine routing-key and ref bytes exist as donors. The mutations READ those donor values out of the rebuilt control rather than hardcoding them |
| `G5` | `m2-wrong-ref` | m1 byte copy; reply `ref` -> the review's real ref |
| `G5` | `m3-wrong-actor` | m1 byte copy; reply actor display AND actor_slot moved to the third agent together |
| `G5` | `m4-wrong-target` | m1 byte copy; reply target display AND target_slot moved to the third agent together |
| `G5` | `m5-routed-vs-routed-mismatch` | m1 byte copy; the ask's target_slot changed while the reply's actor_slot is left as produced — both sides routed, keys disagree |
| `G5` | `m6-mixed-routed-display` | m1 byte copy; all four routing members deleted from the reply only |
| `G6` | `stopped-plain` | 2-agent session with ordinary traffic, then the real `ae stop` |
| `G6` | `stopped-attention` | 2-agent session whose worker declared `blocked`, then the real `ae stop` |
| `G7` | `meta-unknown-keys` | G1/healthy byte copy; named addition: two unknown keys appended to meta |
| `G7` | `events-unknown-keys` | G1/healthy byte copy; named additions: an unknown member inserted into a state event and another appended to an ask event |
| `G7` | `events-unknown-action` | produced LIVE — three unknown action values emitted by the real `send` helper through its documented `_AE_EVENT_ACTION` override (no mutation) |
| `G8` | `no-trailing-newline` | G1/healthy byte copy; named mutation: the file's single trailing newline dropped |
| `G8` | `partial-trailing-record` | G1/healthy byte copy; named mutation: 40 trailing bytes truncated, leaving a partial unterminated final record |
| `G9` | `goals-distinct-ts` | four real `goal` invocations under the clock hook at 1755000000/+600/+1200/+1800 |
| `G10` | `same-display-diff-routing` | produced LIVE — the real `spawn` helper adds a second agent with the SAME display name, then both run the real `ask`, so one display name carries two genuine routing keys (main vs spawned.0) |
| `G10` | `display-only-legacy` | G1/healthy byte copy; named mutation: all four routing members deleted from BOTH sides of the ask/reply pair |
| `G11` | `escapes` | five escape classes (quote, backslash, newline, tab, CR) each fed to the real say/memo/send helpers; the producer INPUT bytes are stored beside the emitted bytes |
| `A1` | `510b-empty-vs-omitted` | the same real `send` helper with its documented event overrides set to a genuinely EMPTY STRING versus left UNSET, plus `state` with an empty vs absent reason and `memo` with an empty vs absent topic |
| `A1` | `510c-recover-ref` | recover-ref bytes from the REAL producer (the generated watchdog running `ae _recover-pending`). The agent binary is the controllable fake copied to `codex` so the frozen classifier reports tool_kind=codex; a codex-shaped session log is planted as external PRODUCER INPUT, hashed and reproduced in the provenance file, written after launch so its mtime beats launch_time and carrying the launch marker ae itself injected. No harness probe runs first — it would CLAIM the pending slot and consume the recovery |
| `A1` | `510e-dupkey-known` | a KNOWN key (`summary`) appearing twice with conflicting values on one event line; both values are producer-derived byte values from that same file |
| `A1` | `510e-dupkey-known-reversed` | the same pair with MEMBER ORDER REVERSED |
| `A1` | `510f-dupkey-unknown` | the identical construction with an UNKNOWN key (`zzz_unknown`) |
| `A1` | `510f-dupkey-unknown-reversed` | the same pair with MEMBER ORDER REVERSED |
| `A1` | `511a-omitted-routing` | a cohort produced only by helpers whose emitter never sets the routing members, so routing is genuinely OMITTED at the producer. The KNOWN-routing half of the pair is G5/m1-control |

Artifact paths (under `docs/migration/evidence/batch-c-artifacts/templates/`):

- `FINGERPRINTS.tsv` and `FINGERPRINTS.superseded-pre-locale-fix.tsv`
- `<group>/_meta/<member>.txt` — provenance: source sandbox and session, the exact helper
  invocations and their rc, clock-hook values where used, both fingerprints
- `<group>/_meta/<member>.modes.tsv` — the recursive manifest the fingerprint is taken over
- `<group>/_meta/<member>.mutation.txt` — the named byte diff for every derived member
- `<group>/_meta/shim-inactive-equivalence.*.txt` and `*date-shim-invocations.log`
- `<group>/fixture-bytes/<member>/` — `config`, `sessions/<s>/meta`,
  `sessions/<s>/events.jsonl`, `sessions/<s>/messages/*`, plus G11's producer INPUT files
  and A1/510c's planted producer input. The remaining files in a member are the
  frozen-`ae`-generated helper set, hashed in `_meta/<member>.modes.tsv` rather than duplicated here
- `date-shim/date`, `harness/` — the clock hook and every build script
- `SHA256SUMS.txt`

### Clock hook (date-shim contract)

Used by `G9/goals-distinct-ts`, `G2/unanswered` and `G2b/competing`, and nothing else.
PATH-first; **delegates every invocation to the real binary** except four exact now-forms
— `-u +%FT%TZ`, `-u +%Y-%m-%dT%H:%M:%SZ`, `-u +%Y%m%dT%H%M%SZ`, `+%s` — which it
substitutes from `AE_FAKE_NOW`. The substituted values are produced BY THE REAL BINARY
from that epoch (`date -u -r <epoch> +FMT`), so the shim never supplies parsing or
formatting behaviour. With `AE_FAKE_NOW` unset it reaches `exec $REAL "$@"` before any
case analysis. Real binary `/bin/date`, sha256
`0c7f77e19bc79013bc5bc4e67beea3b9d546d6d09a36799c2eed2693967af8c6`.

**Per-fixture inactive equivalence**: the shim's delegate-and-log records every `date`
argv each fixture's producers actually invoked; every DISTINCT argv is replayed through
the shim with `AE_FAKE_NOW` unset and through the real binary, comparing stdout, stderr
and rc byte for byte. Deterministic argv are compared once; now-forms, whose output
legitimately advances with the clock, are compared over 20 back-to-back paired trials and
the match count is reported. Results per group in `_meta/shim-inactive-equivalence.*.txt`.
Shim log files are harness artifacts and are excluded from product-state manifests.


### Composite and A3 members

`A2/composite` is a COMPOSITION, not a mutation: ten whole producer-built session dirs
copied byte-for-byte into one `AE_HOME`. `templates/A2/_meta/composite.txt` records, per
session, which group/member it came from and that member's own protected fingerprint, plus
the deliberately chosen `events.jsonl` mtime and why. The composite's own two fingerprints
are in the table above and in `FINGERPRINTS.tsv`, and `templates/A2/fixture-bytes/composite/`
carries the config plus every one of the ten sessions' `meta`, `events.jsonl` and
`messages/`. The arms verify their clone against the protected fingerprint and record the
comparison in the case ledger (`clone-VERIFIED`).

| group | member | construction |
|---|---|---|
| `A2` | `composite` | ten producer-built session dirs copied byte-for-byte into one AE_HOME (config from G1/healthy); `events.jsonl` mtimes chosen deliberately — `tg1` recent, the other nine pinned to `2026-01-01T12:00:00` — because the frozen reader takes session activity from that mtime rather than from event ts |
| `A3` | `017g-unanswered-vs-agent-owned` | 3-agent session under the clock hook: an AGENT-OWNED declaration (`blocked`, bravo) at T0 and a SESSION-LEVEL `ask` targeting a third agent at T1, never replied and aged past the 1800s default. Arrival order is descending in the frozen `_attn_rank` ladder, so a last-wins reader and a rank-wins reader are distinguishable |
| `A3` | `524a-future-ts-ordinary-mtime` | G1/healthy byte copy; named mutation: the last event's `ts` set to a FUTURE value, the file's mtime pinned ordinary |
| `A3` | `524b-ordinary-ts-future-mtime` | G1/healthy byte copy; event BYTES UNCHANGED (hash recorded against the base), only the file mtime set to a FUTURE value |

The 524 pair starts from the same base member and differs only in WHICH of the two
candidate activity sources is made anomalous, so the arm sees the incumbent's source
choice directly instead of a fixture where both sources happen to agree.

### A3b members — the adjacent-pair discrimination set

The rank ladder is read out of the frozen source (`_attn_rank`, ae@72c7293:3571-3581, and
the comment at :3586): dead 6 > stale 5 > waiting-user 4 > blocked 3 > throttled 2 >
unanswered 1. Each pair member puts the HIGHER-rank reason FIRST in arrival order and the
adjacent LOWER-rank reason LAST, so a last-wins reader and a rank-wins reader disagree,
and each reason is owned by a DIFFERENT agent so no reason-owner is the actor of any later
event.

| member | construction |
|---|---|
| `A3b/pair-dead-over-stale` | the fake child under `fake:high`'s pane is killed (real watchdog raises the dead alert), then `fake:low` is left static past the shortened stale window until the real watchdog nudges twice and alerts |
| `A3b/pair-stale-over-waitinguser` | `fake:high` is left static until the watchdog alerts; `fake:low` is kept demonstrably ACTIVE meanwhile (a line into its own pane every 15s) so it cannot accumulate a stale alert of its own, then declares waiting-user |
| `A3b/pair-waitinguser-over-blocked` | `fake:high` declares waiting-user, then `fake:low` declares blocked, both through their own real `state` helpers |
| `A3b/pair-blocked-over-throttled` | `fake:high` declares blocked, then `fake:low` prints the documented generic throttle phrase into its pane tail and the real watchdog emits `throttled` |
| `A3b/pair-throttled-over-unanswered` | `fake:high` prints the throttle phrase (real watchdog emits `throttled`), then `fake:asker` — an agent owning NO reason — asks `fake:low` under the clock hook, aged past the 1800s default and never replied |
| `A3b/competing-noclear` | dead (`fake:high`, real watchdog) then waiting-user (`fake:low`) then an aged unanswered ask issued by `fake:asker`, an agent owning no reason. This is the fix for the own-activity clear: in `G2b/competing` the asker was the dead agent itself, so its own later event cleared its own alert and the competition collapsed |

`G2b/competing` is kept exactly as captured — the collapse it shows is itself a recorded
observation, not something to erase — and `A3b/competing-noclear` is the additive
non-collapsing construction beside it.

### D members

| group | member | construction |
|---|---|---|
| `D` | `d02-pending-with-harvested-reply` | a real `ask`, then a real `reply` produced by the real reply helper from the RESPONDER's own pane so it is identity-valid in every routing member; the reply LINE is then lifted out of `events.jsonl` (named mutation, byte diff recorded) leaving a genuinely PENDING request, and the removed bytes are carried INSIDE the member as `_d02-controller-payload.reply.jsonl` so a clone brings its own payload |
| `D` | `d03-31-numbered-events` | 31 real `memo` invocations each carrying a unique numbered marker (`D03-SEED-EVENT-01..31`), so a follow window can be read off the pane BY NUMBER rather than by counting lines; three further real events are harvested and removed, and are carried inside the member as `_d03-payloads/` — one append sentinel and two distinct rotation sentinels |

## Arm group A1 — schema/document (bash lane)

### A1 — what the arm does

Rows: SC-509, SC-509b, SC-506, SC-510a, SC-510b, SC-510c, SC-510d, SC-510e, SC-510f,
SC-511a, SC-511b, SC-405k. (SC-511c is B0's and is not run here.)

Each document case runs TWICE from the same template member: once on a **protected**
clone (the design's read-only vehicle) and once on a **writable** clone whose modes are
restored to what the producer wrote. Both are published. The pair exists because a
protected clone can turn a write into a refusal rather than revealing it, while the
writable clone lets the manifest diff be the proof the design asks for; publishing only
one would hide which of the two happened.

Consumer families per document case: `ae list`, `ae list --json`, `ae list --all`,
`ae list --all --json`, `ae ls`, `ae ls --all`, `ae status <session>`, `ae next`, the
session's `requests all`, its `agents`, and its `events-tail`. `events-tail` is a
streaming consumer with no one-shot mode, so it is bounded by the harness and the stop is
recorded beside its bytes and in the ledger. Every consumer runs under `env -i` plus the
documented minimum (`HOME`, `AE_HOME`, `PATH`, `TZ=UTC`, `LANG=LC_ALL=en_US.UTF-8`,
`TERM`, `TMUX_TMPDIR`, `AE_TMUX_SERVER`+kind) — never inherited shell state.

The five A1-specific template members (`510b-empty-vs-omitted`, `510c-recover-ref`,
`510e-dupkey-known(-reversed)`, `510f-dupkey-unknown(-reversed)`, `511a-omitted-routing`)
are described in the template construction table above.

### SC-405k live sub-arm

`c20-405k-live` is not a template clone: a live 2-agent launch on its own tmux server with
two named topology manipulations, captured at three stages — `s0-baseline`,
`s1-extra-pane` (an EXTRA runtime pane stamped `@ae_agent=fake:ghost`, `@ae_slot=ghost.0`,
absent from meta) and `s2-extra-pane-and-missing-roster-pane` (the pane of roster slot
`worker.0` killed, its meta entry left in place). Each stage carries the full consumer
battery under `out/<stage>/`, a tmux snapshot, the roster lines from meta, a recursive
AE_HOME manifest, and a raw probe running the exact tmux query the frozen consumer makes
from the same scrubbed environment and socket.

`out/s0-baseline-clocale/` is the PAIRED RAW capture: the same consumers, the same
unchanged topology, the same argv, with the locale pinned to C instead of UTF-8. Both
halves are published raw; which, if either, is the product's intended behaviour is not
decided here. It exists because the frozen script has seven TAB-separated tmux format
sites — the two pane/alive walks at :3631 and :4207 and five pane-id/agent resolution
sites at :6488, :12151, :12170, :12297 and :12962.
### A1 case table

`checks<first consumer` names the ledger sequence numbers of the TAB round-trip
COMPLETE, the tmux-shim equivalence COMPLETE (`-` where the case starts no server),
and the first `consumer-START`. The ledger is append-only and written by the checks
themselves, so the ordering is established by the original durable content — not by
file mtimes and not by a hash list added afterwards. For a barrier case the first
consumer activity is `barrier-ARMED` (the hooked run has no `consumer-START` line).

A case whose design includes a CONTROLLER MUTATION necessarily shows a tmux delta;
what the controller did, when, and from where is in `controller-mutation.txt` and in
the ledger, and the before/at-barrier/after tmux snapshots bracket it.

| case | clone | rows | template | clone fp = template fp | manifest diff | tmux snapshot identical | consumers | checks<first consumer | ordered |
|---|---|---|---|---|---|---|---|---|---|
| `c01-healthy-ro` | ro | SC-509,SC-509b,SC-510a | `G1/healthy` | yes | 0 | yes | 11 | 6/-/8 | yes |
| `c01-healthy-rw` | rw | SC-509,SC-509b,SC-510a | `G1/healthy` | yes | 0 | yes | 11 | 6/-/8 | yes |
| `c02-meta-mode-000-ro` | ro | SC-506 | `G3/meta-mode-000` | yes | 0 | yes | 11 | 6/-/8 | yes |
| `c02-meta-mode-000-rw` | rw | SC-506 | `G3/meta-mode-000` | yes | 0 | yes | 11 | 6/-/8 | yes |
| `c03-malformed-line-ro` | ro | SC-506,SC-509b | `G3/malformed-complete-line` | yes | 0 | yes | 11 | 6/-/8 | yes |
| `c03-malformed-line-rw` | rw | SC-506,SC-509b | `G3/malformed-complete-line` | yes | 0 | yes | 11 | 6/-/8 | yes |
| `c04-empty-vs-omitted-ro` | ro | SC-510b | `A1/510b-empty-vs-omitted` | yes | 0 | yes | 11 | 6/-/8 | yes |
| `c04-empty-vs-omitted-rw` | rw | SC-510b | `A1/510b-empty-vs-omitted` | yes | 0 | yes | 11 | 6/-/8 | yes |
| `c05-recover-ref-ro` | ro | SC-510c | `A1/510c-recover-ref` | yes | 0 | yes | 11 | 6/-/8 | yes |
| `c05-recover-ref-rw` | rw | SC-510c | `A1/510c-recover-ref` | yes | 0 | yes | 11 | 6/-/8 | yes |
| `c06-escapes-ro` | ro | SC-510d | `G11/escapes` | yes | 0 | yes | 11 | 6/-/8 | yes |
| `c06-escapes-rw` | rw | SC-510d | `G11/escapes` | yes | 0 | yes | 11 | 6/-/8 | yes |
| `c07-dupkey-known-ro` | ro | SC-510e | `A1/510e-dupkey-known` | yes | 0 | yes | 11 | 6/-/8 | yes |
| `c07-dupkey-known-rw` | rw | SC-510e | `A1/510e-dupkey-known` | yes | 0 | yes | 11 | 6/-/8 | yes |
| `c08-dupkey-known-rev-ro` | ro | SC-510e | `A1/510e-dupkey-known-reversed` | yes | 0 | yes | 11 | 6/-/8 | yes |
| `c08-dupkey-known-rev-rw` | rw | SC-510e | `A1/510e-dupkey-known-reversed` | yes | 0 | yes | 11 | 6/-/8 | yes |
| `c09-dupkey-unknown-ro` | ro | SC-510f | `A1/510f-dupkey-unknown` | yes | 0 | yes | 11 | 6/-/8 | yes |
| `c09-dupkey-unknown-rw` | rw | SC-510f | `A1/510f-dupkey-unknown` | yes | 0 | yes | 11 | 6/-/8 | yes |
| `c10-dupkey-unknown-rev-ro` | ro | SC-510f | `A1/510f-dupkey-unknown-reversed` | yes | 0 | yes | 11 | 6/-/8 | yes |
| `c10-dupkey-unknown-rev-rw` | rw | SC-510f | `A1/510f-dupkey-unknown-reversed` | yes | 0 | yes | 11 | 6/-/8 | yes |
| `c11-routing-known-ro` | ro | SC-511a,SC-511b | `G5/m1-control` | yes | 0 | yes | 11 | 6/-/8 | yes |
| `c11-routing-known-rw` | rw | SC-511a,SC-511b | `G5/m1-control` | yes | 0 | yes | 11 | 6/-/8 | yes |
| `c12-routing-omitted-ro` | ro | SC-511a | `A1/511a-omitted-routing` | yes | 0 | yes | 11 | 6/-/8 | yes |
| `c12-routing-omitted-rw` | rw | SC-511a | `A1/511a-omitted-routing` | yes | 0 | yes | 11 | 6/-/8 | yes |
| `c13-same-display-routing-ro` | ro | SC-511b | `G10/same-display-diff-routing` | yes | 0 | yes | 11 | 6/-/8 | yes |
| `c13-same-display-routing-rw` | rw | SC-511b | `G10/same-display-diff-routing` | yes | 0 | yes | 11 | 6/-/8 | yes |
| `c14-display-only-legacy-ro` | ro | SC-511b | `G10/display-only-legacy` | yes | 0 | yes | 11 | 6/-/8 | yes |
| `c14-display-only-legacy-rw` | rw | SC-511b | `G10/display-only-legacy` | yes | 0 | yes | 11 | 6/-/8 | yes |
| `c15-meta-unknown-keys-ro` | ro | SC-509,SC-509b | `G7/meta-unknown-keys` | yes | 0 | yes | 11 | 6/-/8 | yes |
| `c15-meta-unknown-keys-rw` | rw | SC-509,SC-509b | `G7/meta-unknown-keys` | yes | 0 | yes | 11 | 6/-/8 | yes |
| `c16-events-unknown-keys-ro` | ro | SC-509b,SC-510a | `G7/events-unknown-keys` | yes | 0 | yes | 11 | 6/-/8 | yes |
| `c16-events-unknown-keys-rw` | rw | SC-509b,SC-510a | `G7/events-unknown-keys` | yes | 0 | yes | 11 | 6/-/8 | yes |
| `c17-unknown-action-ro` | ro | SC-509b,SC-510a | `G7/events-unknown-action` | yes | 0 | yes | 11 | 6/-/8 | yes |
| `c17-unknown-action-rw` | rw | SC-509b,SC-510a | `G7/events-unknown-action` | yes | 0 | yes | 11 | 6/-/8 | yes |
| `c18-no-trailing-newline-ro` | ro | SC-509b | `G8/no-trailing-newline` | yes | 0 | yes | 11 | 6/-/8 | yes |
| `c18-no-trailing-newline-rw` | rw | SC-509b | `G8/no-trailing-newline` | yes | 0 | yes | 11 | 6/-/8 | yes |
| `c19-partial-tail-ro` | ro | SC-509b | `G8/partial-trailing-record` | yes | 0 | yes | 11 | 6/-/8 | yes |
| `c19-partial-tail-rw` | rw | SC-509b | `G8/partial-trailing-record` | yes | 0 | yes | 11 | 6/-/8 | yes |
| `c20-405k-live` | live | SC-405k | `live/no-template (live 2-agent launch + two named topology manipulations)` | - | - | - | 26 | 5/7/9 | yes |

Artifact paths — `docs/migration/evidence/batch-c-artifacts/arms/A1/<case>/`:

- `admissibility-ledger.txt` — append-only, monotonic `seq` + UTC + epoch per event:
  case open, rows, clone verification (clone vs expected fingerprint), the TAB
  round-trip START/COMPLETE, the tmux-shim equivalence START/COMPLETE, the
  before/after manifests, and every consumer START/COMPLETE with its rc and its
  stdout / stderr / tmuxtrace sha256
- `env-tab-selfcheck.txt` — the TAB round-trip in this case's own scrubbed
  environment, plus the paired `LANG=LC_ALL=C` probe on the same throwaway server
- `tmux-shim-equivalence.txt` — live cases only: the delegate-and-log shim proven
  byte-identical to the real binary on this arm's own stable topology
- `case.txt`, `env.txt`, `consumers.tsv` (label, rc, stdout/stderr sha256 + bytes,
  tmuxtrace sha256 + line count, bounded flag, exact argv)
- `out/<label>.stdout`, `out/<label>.stderr` (present only when non-empty),
  `out/<label>.tmuxtrace` — per invocation: the effective `AE_TMUX_SERVER` and kind,
  the effective locale, and the DELEGATED tmux argv
- `manifest.before.tsv` / `manifest.after.tsv` / `manifest.diff.txt` — recursive:
  type, mode, content hash, symlink target, path across the cloned AE_HOME
- `tmux.before.txt` / `tmux.after.txt`
- `A1/ledger.tsv` (case -> row ids), `A1/harness/` (the exact scripts and the
  tmux shim), `SHA256SUMS.txt` (every file above)


## Arm group A2 — list filters (bash lane)

### A2 — what the arm does

Rows: SC-017a, SC-017b, SC-017c, SC-017d, SC-017e, SC-017f, SC-017i, SC-021 (the `ls`
alias) and SC-521a.

**Template `A2/composite`.** Ten whole producer-built session dirs copied byte-for-byte
into ONE `AE_HOME`, so the list filters have something to discriminate between; not one
byte inside any of them changes, and each member's source group/member and source
protected fingerprint are recorded in `templates/A2/_meta/composite.txt`:
`tg1` <- G1/healthy · `twda1` <- G2/dead · `twda2` <- G2/stale · `twda3` <- G2/throttled ·
`tg2wu` <- G2/waiting-user · `tg2bl` <- G2/blocked · `tg2un` <- G2/unanswered ·
`tg2b` <- G2b/competing · `tg6a` <- G6/stopped-plain · `tg6b` <- G6/stopped-attention.
The config is taken from G1/healthy.

**events.jsonl MTIME is load-bearing and is therefore chosen, not inherited.** The frozen
reader takes session activity from that file's mtime rather than from event timestamps
(ae@72c7293:3993-4009, 4220-4228). A plain copy stamps every session with "whenever the
copy ran", which would make all ten look equally fresh and quietly destroy the
`--active` / `--busy` discrimination. So the composite sets mtimes deliberately — `tg1`
recent, the other nine pinned to a fixed `2026-01-01T12:00:00` — recorded per session in
the composite's provenance and again per case in `fixture-mtimes.txt` as epoch and UTC
exactly as the consumer sees them.

**Harness change, recorded with its reason:** template cloning now preserves mtimes
(`cp -Rp`), so a clone carries the fixture's chosen activity clock instead of the clone's
own timestamp. Fingerprints are unaffected — mtime is deliberately not part of the
manifest the fingerprint is taken over.

**Live topology** (the design's "controlled panes, never live models" clause): a dedicated
socket per case; `tg1`, `twda1`, `tg2wu` and `tg2b` each get a tmux session with one pane
per roster entry from that session's own meta, running the fixture's controllable fake
binary, stamped `@ae_agent`/`@ae_slot` and carrying the session environment ae itself
writes at launch (`AE_SESSION`, `AE_ORIGIN`, `AE_DIR`, `AE_MODE`, `AE_HOME` —
ae@72c7293:17311-17318; without `AE_SESSION` the frozen enumerator does not treat a tmux
session as an ae session at all). `twda2`, `twda3`, `tg2bl`, `tg2un`, `tg6a` and `tg6b`
get no tmux session. Every roster agent gets a pane, so the harness introduces no
synthetic missing-pane `dead`: the only dead-family bytes in play are the ones the step-0
watchdog really produced.

**Consumers — one invocation per flag AND per documented alias**, plain and `--json`, on
both `list` and `ls`: `(default)`, `--running`, `--all`, `--stopped`, `--needs-attn`,
`--needs-me`, `--needs`, `--attn`, `--active`, `--busy` (10 x 2 x 2 = 40); plus the
intersection arms in BOTH orders — `--needs-attn --all`, `--all --needs-attn`,
`--active --all`, `--all --active`, `--needs-attn --stopped`, `--active --stopped`
(6 x 2 = 12); plus an unknown flag and `--help` (2). 54 invocations per clone mode, run on
both the protected and the writable clone.

**Activity window made non-vacuous (remediation).** At the wall clock the composite's one
recent session is already far outside the documented 300s default, so every `--active`
capture was empty and the active/stopped intersections proved nothing. The window is now
exercised from BOTH sides by freezing the consumer's clock with the PATH-first date shim,
using the fixture's OWN recorded mtime as the reference: `inside_window_now = mtime + 60s`
and `outside_window_now = mtime + 100000s`, both recorded per case in
`activity-window.txt` with the mtime they derive from. Nine invocations at each of the two
frozen nows: `--active`, `--active --json`, `--busy`, and the intersections
`--active --all`, `--all --active`, `--active --stopped`, `--stopped --active`,
`--needs-attn --active`, `--active --needs-attn` — every intersection in BOTH argument
orders. The shim's `date-shim.log` for the case records every `date` invocation the
consumers made.
### A2 case table

`checks<first consumer` names the ledger sequence numbers of the TAB round-trip
COMPLETE, the tmux-shim equivalence COMPLETE (`-` where the case starts no server),
and the first `consumer-START`. The ledger is append-only and written by the checks
themselves, so the ordering is established by the original durable content — not by
file mtimes and not by a hash list added afterwards. For a barrier case the first
consumer activity is `barrier-ARMED` (the hooked run has no `consumer-START` line).

A case whose design includes a CONTROLLER MUTATION necessarily shows a tmux delta;
what the controller did, when, and from where is in `controller-mutation.txt` and in
the ledger, and the before/at-barrier/after tmux snapshots bracket it.

| case | clone | rows | template | clone fp = template fp | manifest diff | tmux snapshot identical | consumers | checks<first consumer | ordered |
|---|---|---|---|---|---|---|---|---|---|
| `c01-filters-ro` | ro | SC-017a,SC-017b,SC-017c,SC-017d,SC-017e,SC-017f,SC-017i,SC-021,SC-521a | `A2/composite` | yes | 0 | yes | 72 | 8/10/12 | yes |
| `c01-filters-rw` | rw | SC-017a,SC-017b,SC-017c,SC-017d,SC-017e,SC-017f,SC-017i,SC-021,SC-521a | `A2/composite` | yes | 0 | yes | 72 | 8/10/12 | yes |

Artifact paths — `docs/migration/evidence/batch-c-artifacts/arms/A2/<case>/`:

- `admissibility-ledger.txt` — append-only, monotonic `seq` + UTC + epoch per event:
  case open, rows, clone verification (clone vs expected fingerprint), the TAB
  round-trip START/COMPLETE, the tmux-shim equivalence START/COMPLETE, the
  before/after manifests, and every consumer START/COMPLETE with its rc and its
  stdout / stderr / tmuxtrace sha256
- `env-tab-selfcheck.txt` — the TAB round-trip in this case's own scrubbed
  environment, plus the paired `LANG=LC_ALL=C` probe on the same throwaway server
- `tmux-shim-equivalence.txt` — live cases only: the delegate-and-log shim proven
  byte-identical to the real binary on this arm's own stable topology
- `case.txt`, `env.txt`, `consumers.tsv` (label, rc, stdout/stderr sha256 + bytes,
  tmuxtrace sha256 + line count, bounded flag, exact argv)
- `out/<label>.stdout`, `out/<label>.stderr` (present only when non-empty),
  `out/<label>.tmuxtrace` — per invocation: the effective `AE_TMUX_SERVER` and kind,
  the effective locale, and the DELEGATED tmux argv
- `manifest.before.tsv` / `manifest.after.tsv` / `manifest.diff.txt` — recursive:
  type, mode, content hash, symlink target, path across the cloned AE_HOME
- `tmux.before.txt` / `tmux.after.txt`
- `A2/ledger.tsv` (case -> row ids), `A2/harness/` (the exact scripts and the
  tmux shim), `SHA256SUMS.txt` (every file above)


## Arm group A3 — rollup / severity (bash lane)

### A3 — what the arm does

Rows: SC-017g, SC-017h, SC-524.

Every A3 case runs on a LIVE topology — the rollup reads panes as well as events, so a
document-only clone could not exercise it. One tmux session per session dir on a dedicated
socket, one pane per roster entry from that session's own meta, running the fixture's
controllable fake binary, stamped `@ae_agent`/`@ae_slot` and carrying the session
environment ae writes at launch. Never a live model.

Cases c01–c07 walk the six G2 members and G2b, one attention reason each plus the
competing set. Case c08 is the amended SC-017g cohort: a SESSION-LEVEL `ask` aged past the
1800s default and never replied, competing against an AGENT-OWNED `blocked` declaration
owned by a different agent, produced under the clock hook with the declaration arriving
FIRST and the aged ask LAST — descending in the frozen `_attn_rank` ladder, so a last-wins
reader and a rank-wins reader are distinguishable.

Cases c09 and c10 are the SC-524 source-discrimination pair: identical cloned inputs from
the same base member, differing only in WHICH candidate activity source is anomalous —
`524a` a FUTURE event `ts` with an ordinary file mtime, `524b` UNCHANGED event bytes (hash
recorded against the base) with a FUTURE file mtime.

Consumers per case: `ae list`, `list --json`, `list --all --json`, `list --needs-attn`
(plain and `--json`), `list --active` (plain and `--json`), `ae next`, `ae status`, and
the session's `requests all`.

Two extra per-case artifacts carry what these rows are about:
`activity-sources.txt` records BOTH candidate activity sources as the consumer sees them —
the `events.jsonl` mtime (epoch + UTC), the last event `ts`, the file hash/size/line count,
and the harness's own clock at capture time. `attention-fields.txt` lifts the session
`needs_attention` / `attention` / `attention_rank` and every per-agent `ref`/`alive`/
`state`/`reason` verbatim out of the captured `list --json` bytes, so the two fields the
row asks for are readable without re-parsing the JSON.
### A3 case table

`checks<first consumer` names the ledger sequence numbers of the TAB round-trip
COMPLETE, the tmux-shim equivalence COMPLETE (`-` where the case starts no server),
and the first `consumer-START`. The ledger is append-only and written by the checks
themselves, so the ordering is established by the original durable content — not by
file mtimes and not by a hash list added afterwards. For a barrier case the first
consumer activity is `barrier-ARMED` (the hooked run has no `consumer-START` line).

A case whose design includes a CONTROLLER MUTATION necessarily shows a tmux delta;
what the controller did, when, and from where is in `controller-mutation.txt` and in
the ledger, and the before/at-barrier/after tmux snapshots bracket it.

| case | clone | rows | template | clone fp = template fp | manifest diff | tmux snapshot identical | consumers | checks<first consumer | ordered |
|---|---|---|---|---|---|---|---|---|---|
| `c01-dead-ro` | ro | SC-017g,SC-017h | `G2/dead` | yes | 0 | yes | 10 | 8/10/12 | yes |
| `c01-dead-rw` | rw | SC-017g,SC-017h | `G2/dead` | yes | 0 | yes | 10 | 8/10/12 | yes |
| `c02-stale-ro` | ro | SC-017g,SC-017h | `G2/stale` | yes | 0 | yes | 10 | 8/10/12 | yes |
| `c02-stale-rw` | rw | SC-017g,SC-017h | `G2/stale` | yes | 0 | yes | 10 | 8/10/12 | yes |
| `c03-throttled-ro` | ro | SC-017g,SC-017h | `G2/throttled` | yes | 0 | yes | 10 | 8/10/12 | yes |
| `c03-throttled-rw` | rw | SC-017g,SC-017h | `G2/throttled` | yes | 0 | yes | 10 | 8/10/12 | yes |
| `c04-waiting-user-ro` | ro | SC-017g,SC-017h | `G2/waiting-user` | yes | 0 | yes | 10 | 8/10/12 | yes |
| `c04-waiting-user-rw` | rw | SC-017g,SC-017h | `G2/waiting-user` | yes | 0 | yes | 10 | 8/10/12 | yes |
| `c05-blocked-ro` | ro | SC-017g,SC-017h | `G2/blocked` | yes | 0 | yes | 10 | 8/10/12 | yes |
| `c05-blocked-rw` | rw | SC-017g,SC-017h | `G2/blocked` | yes | 0 | yes | 10 | 8/10/12 | yes |
| `c06-unanswered-ro` | ro | SC-017g,SC-017h | `G2/unanswered` | yes | 0 | yes | 10 | 8/10/12 | yes |
| `c06-unanswered-rw` | rw | SC-017g,SC-017h | `G2/unanswered` | yes | 0 | yes | 10 | 8/10/12 | yes |
| `c07-competing-ro` | ro | SC-017g,SC-017h | `G2b/competing` | yes | 0 | yes | 10 | 8/10/12 | yes |
| `c07-competing-rw` | rw | SC-017g,SC-017h | `G2b/competing` | yes | 0 | yes | 10 | 8/10/12 | yes |
| `c08-unanswered-vs-agent-owned-ro` | ro | SC-017g | `A3/017g-unanswered-vs-agent-owned` | yes | 0 | yes | 10 | 8/10/12 | yes |
| `c08-unanswered-vs-agent-owned-rw` | rw | SC-017g | `A3/017g-unanswered-vs-agent-owned` | yes | 0 | yes | 10 | 8/10/12 | yes |
| `c09-524a-future-ts-ordinary-mtime-ro` | ro | SC-524 | `A3/524a-future-ts-ordinary-mtime` | yes | 0 | yes | 10 | 8/10/12 | yes |
| `c09-524a-future-ts-ordinary-mtime-rw` | rw | SC-524 | `A3/524a-future-ts-ordinary-mtime` | yes | 0 | yes | 10 | 8/10/12 | yes |
| `c10-524b-ordinary-ts-future-mtime-ro` | ro | SC-524 | `A3/524b-ordinary-ts-future-mtime` | yes | 0 | yes | 10 | 8/10/12 | yes |
| `c10-524b-ordinary-ts-future-mtime-rw` | rw | SC-524 | `A3/524b-ordinary-ts-future-mtime` | yes | 0 | yes | 10 | 8/10/12 | yes |

Artifact paths — `docs/migration/evidence/batch-c-artifacts/arms/A3/<case>/`:

- `admissibility-ledger.txt` — append-only, monotonic `seq` + UTC + epoch per event:
  case open, rows, clone verification (clone vs expected fingerprint), the TAB
  round-trip START/COMPLETE, the tmux-shim equivalence START/COMPLETE, the
  before/after manifests, and every consumer START/COMPLETE with its rc and its
  stdout / stderr / tmuxtrace sha256
- `env-tab-selfcheck.txt` — the TAB round-trip in this case's own scrubbed
  environment, plus the paired `LANG=LC_ALL=C` probe on the same throwaway server
- `tmux-shim-equivalence.txt` — live cases only: the delegate-and-log shim proven
  byte-identical to the real binary on this arm's own stable topology
- `case.txt`, `env.txt`, `consumers.tsv` (label, rc, stdout/stderr sha256 + bytes,
  tmuxtrace sha256 + line count, bounded flag, exact argv)
- `out/<label>.stdout`, `out/<label>.stderr` (present only when non-empty),
  `out/<label>.tmuxtrace` — per invocation: the effective `AE_TMUX_SERVER` and kind,
  the effective locale, and the DELEGATED tmux argv
- `manifest.before.tsv` / `manifest.after.tsv` / `manifest.diff.txt` — recursive:
  type, mode, content hash, symlink target, path across the cloned AE_HOME
- `tmux.before.txt` / `tmux.after.txt`
- `A3/ledger.tsv` (case -> row ids), `A3/harness/` (the exact scripts and the
  tmux shim), `SHA256SUMS.txt` (every file above)


## Arm group A3b — SC-017g adjacent-pair discrimination (bash lane)

### A3b — what the arm does

Row: SC-017g. This arm exists because A3 alone proved that each reason CAN appear and that
waiting-user beats unanswered and blocked beats unanswered — but nothing in it
discriminated the five ADJACENT pairs of the rank ladder, and in `G2b/competing` the later
ask was issued by the dead agent itself, so its own activity cleared its own alert and the
competition collapsed to waiting-user.

Six cases, each run on both a protected and a writable clone, each on its own live
topology: the five adjacent pairs (dead>stale, stale>waiting-user, waiting-user>blocked,
blocked>throttled, throttled>unanswered) with the higher-rank reason arriving FIRST, plus
a competing set whose aged unanswered ask is issued by an agent owning no reason at all.

`attention-fields.txt` in every A3 and A3b case is re-derived from that case's captured
`out/list-json.stdout` bytes — the source file's sha256 is recorded in the derived file
and nothing is re-run — and now carries the session's `needs_attention` / `attention` /
`attention_rank` / `last_active_epoch` plus the COMPLETE per-agent object (ref, alias,
name, session_id, alive, state, reason). The earlier grep-based extraction dropped
`alive` and `state`, which mattered: a null per-agent `reason` sitting beside a non-null
session `attention` is visible now instead of being hidden by a lossy filter.
### A3b case table

`checks<first consumer` names the ledger sequence numbers of the TAB round-trip
COMPLETE, the tmux-shim equivalence COMPLETE (`-` where the case starts no server),
and the first `consumer-START`. The ledger is append-only and written by the checks
themselves, so the ordering is established by the original durable content — not by
file mtimes and not by a hash list added afterwards. For a barrier case the first
consumer activity is `barrier-ARMED` (the hooked run has no `consumer-START` line).

A case whose design includes a CONTROLLER MUTATION necessarily shows a tmux delta;
what the controller did, when, and from where is in `controller-mutation.txt` and in
the ledger, and the before/at-barrier/after tmux snapshots bracket it.

| case | clone | rows | template | clone fp = template fp | manifest diff | tmux snapshot identical | consumers | checks<first consumer | ordered |
|---|---|---|---|---|---|---|---|---|---|
| `c01-dead-over-stale-ro` | ro | SC-017g | `A3b/pair-dead-over-stale` | yes | 0 | yes | 10 | 8/10/12 | yes |
| `c01-dead-over-stale-rw` | rw | SC-017g | `A3b/pair-dead-over-stale` | yes | 0 | yes | 10 | 8/10/12 | yes |
| `c02-stale-over-waitinguser-ro` | ro | SC-017g | `A3b/pair-stale-over-waitinguser` | yes | 0 | yes | 10 | 8/10/12 | yes |
| `c02-stale-over-waitinguser-rw` | rw | SC-017g | `A3b/pair-stale-over-waitinguser` | yes | 0 | yes | 10 | 8/10/12 | yes |
| `c03-waitinguser-over-blocked-ro` | ro | SC-017g | `A3b/pair-waitinguser-over-blocked` | yes | 0 | yes | 10 | 8/10/12 | yes |
| `c03-waitinguser-over-blocked-rw` | rw | SC-017g | `A3b/pair-waitinguser-over-blocked` | yes | 0 | yes | 10 | 8/10/12 | yes |
| `c04-blocked-over-throttled-ro` | ro | SC-017g | `A3b/pair-blocked-over-throttled` | yes | 0 | yes | 10 | 8/10/12 | yes |
| `c04-blocked-over-throttled-rw` | rw | SC-017g | `A3b/pair-blocked-over-throttled` | yes | 0 | yes | 10 | 8/10/12 | yes |
| `c05-throttled-over-unanswered-ro` | ro | SC-017g | `A3b/pair-throttled-over-unanswered` | yes | 0 | yes | 10 | 8/10/12 | yes |
| `c05-throttled-over-unanswered-rw` | rw | SC-017g | `A3b/pair-throttled-over-unanswered` | yes | 0 | yes | 10 | 8/10/12 | yes |
| `c06-competing-noclear-ro` | ro | SC-017g | `A3b/competing-noclear` | yes | 0 | yes | 10 | 8/10/12 | yes |
| `c06-competing-noclear-rw` | rw | SC-017g | `A3b/competing-noclear` | yes | 0 | yes | 10 | 8/10/12 | yes |

Artifact paths — `docs/migration/evidence/batch-c-artifacts/arms/A3b/<case>/`:

- `admissibility-ledger.txt` — append-only, monotonic `seq` + UTC + epoch per event:
  case open, rows, clone verification (clone vs expected fingerprint), the TAB
  round-trip START/COMPLETE, the tmux-shim equivalence START/COMPLETE, the
  before/after manifests, and every consumer START/COMPLETE with its rc and its
  stdout / stderr / tmuxtrace sha256
- `env-tab-selfcheck.txt` — the TAB round-trip in this case's own scrubbed
  environment, plus the paired `LANG=LC_ALL=C` probe on the same throwaway server
- `tmux-shim-equivalence.txt` — live cases only: the delegate-and-log shim proven
  byte-identical to the real binary on this arm's own stable topology
- `case.txt`, `env.txt`, `consumers.tsv` (label, rc, stdout/stderr sha256 + bytes,
  tmuxtrace sha256 + line count, bounded flag, exact argv)
- `out/<label>.stdout`, `out/<label>.stderr` (present only when non-empty),
  `out/<label>.tmuxtrace` — per invocation: the effective `AE_TMUX_SERVER` and kind,
  the effective locale, and the DELEGATED tmux argv
- `manifest.before.tsv` / `manifest.after.tsv` / `manifest.diff.txt` — recursive:
  type, mode, content hash, symlink target, path across the cloned AE_HOME
- `tmux.before.txt` / `tmux.after.txt`
- `A3b/ledger.tsv` (case -> row ids), `A3b/harness/` (the exact scripts and the
  tmux shim), `SHA256SUMS.txt` (every file above)


## Arm group A4 — status / next (bash lane)

### A4 — what the arm does

Rows: SC-016a, SC-016b, SC-016c, SC-016d, SC-513a, SC-513b, SC-513c, SC-019, SC-020a,
SC-020b, SC-020c. Every case is live tmux on its own dedicated server; the never-attach
rows are proven by `list-clients` snapshots taken before and after each run
(`clients.before.txt` / `clients.after.txt`, both hashed into the ledger).

`c01-status-live` and `c02-status-016b` run on a REAL `ae` launch rather than a template
clone, because these rows are about what the live pane set renders.
**SC-016b's discriminator**: each of the two panes is filled with 150 UNIQUELY NUMBERED
lines through that pane's OWN control FIFO, so the two streams cannot be confused for one
another. `pane-fill-summary.txt` records, per pane, the captured line count, the unique
marker count and the first/last marker; `panefull.<pane>.txt` is the full pane scrollback;
`out/status.stdout` is what the consumer rendered, with its per-pane binary/pane-id labels.

`c03`–`c05` run on `A2/composite` with a deliberately SINGLE attention candidate:
only `tg2wu` is given a live tmux session, so the session `next` resolves to is known by
construction rather than by asking the product first. `c04` runs `next --attach` from
outside any client — the frozen outside-tmux verb is a BLOCKING `attach-session`, so that
invocation is harness-bounded and the bound is recorded beside its bytes.

### SC-020b — the named barrier, on D04b's approved hook

`c05-020b-barrier` consumes b0-design.md Design 6 (D04b): hook `H_NEXT_SELECTED`, placed
after best-candidate resolution and BEFORE the exact recheck. That design names both the
cut and the capture and self-declares that SC-020b's Batch C arm consumes it, which is
batch-c-design.md's reuse condition.

The sequence, all of it in the ledger: `barrier-ARMED` → the hooked `ae next` runs until
the hook blocks → `barrier-REACHED` → the CONTROLLER kills the exact session `next` had
already resolved to, from a separate connection, never from inside the process under test
(`controller-mutation.txt`, with tmux state immediately before and after) →
`barrier-RELEASED` → the run finishes and its stdout/stderr/rc and the hook's own log are
captured. `tmux.at-barrier-before.txt` brackets the mutation on one side and
`tmux.after.txt` on the other.

Before any hooked capture, `hook-inactive-equivalence.txt` proves the patch inactive on
THIS fixture: six invocations through the unmodified binary twice (control-A, control-B)
and through the hooked binary once with `AE_HOOK` unset, clock frozen by the date shim so
run-to-run volatility cannot masquerade as a binary difference. The control-control pass
measures the volatility floor and the control-hooked comparison is read against it. Both
counts are recorded in the ledger; for this fixture the floor was 0 and the
control-versus-hooked divergence was 0. The patch itself is published at
`hook-patch/hook.patch` with its generator and hashes, and copied into the case as
`hook.patch`.
### A4 case table

`checks<first consumer` names the ledger sequence numbers of the TAB round-trip
COMPLETE, the tmux-shim equivalence COMPLETE (`-` where the case starts no server),
and the first `consumer-START`. The ledger is append-only and written by the checks
themselves, so the ordering is established by the original durable content — not by
file mtimes and not by a hash list added afterwards. For a barrier case the first
consumer activity is `barrier-ARMED` (the hooked run has no `consumer-START` line).

A case whose design includes a CONTROLLER MUTATION necessarily shows a tmux delta;
what the controller did, when, and from where is in `controller-mutation.txt` and in
the ledger, and the before/at-barrier/after tmux snapshots bracket it.

| case | clone | rows | template | clone fp = template fp | manifest diff | tmux snapshot identical | consumers | checks<first consumer | ordered |
|---|---|---|---|---|---|---|---|---|---|
| `c01-status-live-live` | live | SC-016a,SC-016c,SC-016d,SC-019,SC-513a,SC-513b,SC-513c | `live/none (live 2-agent launch)` | - | 0 | yes | 7 | 5/7/10 | yes |
| `c02-status-016b-live` | live | SC-016b | `live/none (live 2-agent launch, 150 uniquely numbered lines per pane)` | - | 0 | yes | 7 | 5/7/13 | yes |
| `c03-next-noattach-ro` | ro | SC-019,SC-020a,SC-020c | `A2/composite` | yes | 0 | yes | 3 | 7/9/12 | yes |
| `c03-next-noattach-rw` | rw | SC-019,SC-020a,SC-020c | `A2/composite` | yes | 0 | yes | 3 | 7/9/12 | yes |
| `c04-next-attach-outside-ro` | ro | SC-020a,SC-020c | `A2/composite` | yes | 0 | yes | 1 | 7/9/12 | yes |
| `c04-next-attach-outside-rw` | rw | SC-020a,SC-020c | `A2/composite` | yes | 0 | yes | 1 | 7/9/12 | yes |
| `c05-020b-barrier-rw` | rw | SC-020b | `A2/composite` | yes | 0 | no | 1 | 7/9/15 | yes |

Artifact paths — `docs/migration/evidence/batch-c-artifacts/arms/A4/<case>/`:

- `admissibility-ledger.txt` — append-only, monotonic `seq` + UTC + epoch per event:
  case open, rows, clone verification (clone vs expected fingerprint), the TAB
  round-trip START/COMPLETE, the tmux-shim equivalence START/COMPLETE, the
  before/after manifests, and every consumer START/COMPLETE with its rc and its
  stdout / stderr / tmuxtrace sha256
- `env-tab-selfcheck.txt` — the TAB round-trip in this case's own scrubbed
  environment, plus the paired `LANG=LC_ALL=C` probe on the same throwaway server
- `tmux-shim-equivalence.txt` — live cases only: the delegate-and-log shim proven
  byte-identical to the real binary on this arm's own stable topology
- `case.txt`, `env.txt`, `consumers.tsv` (label, rc, stdout/stderr sha256 + bytes,
  tmuxtrace sha256 + line count, bounded flag, exact argv)
- `out/<label>.stdout`, `out/<label>.stderr` (present only when non-empty),
  `out/<label>.tmuxtrace` — per invocation: the effective `AE_TMUX_SERVER` and kind,
  the effective locale, and the DELEGATED tmux argv
- `manifest.before.tsv` / `manifest.after.tsv` / `manifest.diff.txt` — recursive:
  type, mode, content hash, symlink target, path across the cloned AE_HOME
- `tmux.before.txt` / `tmux.after.txt`
- `A4/ledger.tsv` (case -> row ids), `A4/harness/` (the exact scripts and the
  tmux shim), `SHA256SUMS.txt` (every file above)


## Arm group A5 — doctor exits (SC-514) (bash lane)

### A5 — what the arm does

Row: SC-514. The doctor is run under a CONTROLLED PATH: a directory of 1517 SYMLINKS to
every executable on the standard search path, with `PATH` set to that directory ALONE. A
planted arm removes exactly ONE symlink, and the arm PROVES it removed exactly one — each
case publishes `bin-listing.txt`, and the diff against the clean arm's listing is a single
line naming the removed tool. The interpreter is invoked by ABSOLUTE path, so no removal
can take it away; `a5-no-bash-on-path` removes `bash` from the controlled directory
deliberately to show that.

The fixture (`A5/doctor-fixture`) is a real 2-agent launch with ordinary traffic, then the
real `ae stop`, so it is a settled on-disk session rather than a live one, and its config
names the agent by absolute path so the doctor's `agent:` check resolves through the
controlled directory like every other checklist item.

Each case publishes the doctor's stdout verbatim, its rc, `checklist.txt` (every
`OK`/`WARN`/`FAIL` line and the Summary, re-derived from the captured stdout with the
source hash recorded), the bin listing, and before/after AE_HOME manifests.

`platform-note.txt` records why there is no `a5-no-timeout` arm: `timeout` is GNU coreutils
and this platform does not provide it, so there was nothing to remove. The harness tried,
could not plant the removal, and HARNESS-ABORTED rather than publishing a case that removed
nothing and looked like a planted failure; the aborted directory was deleted rather than
published. The observation it would have produced is already in the clean arm, whose
checklist carries the `timeout` WARN with the full bin directory in place.
### A5 case table

`checks<first consumer` names the ledger sequence numbers of the TAB round-trip
COMPLETE, the tmux-shim equivalence COMPLETE (`-` where the case starts no server),
and the first `consumer-START`. The ledger is append-only and written by the checks
themselves, so the ordering is established by the original durable content — not by
file mtimes and not by a hash list added afterwards. For a barrier case the first
consumer activity is `barrier-ARMED` (the hooked run has no `consumer-START` line).

A case whose design includes a CONTROLLER MUTATION necessarily shows a tmux delta;
what the controller did, when, and from where is in `controller-mutation.txt` and in
the ledger, and the before/at-barrier/after tmux snapshots bracket it.

| case | clone | rows | template | clone fp = template fp | manifest diff | tmux snapshot identical | consumers | checks<first consumer | ordered |
|---|---|---|---|---|---|---|---|---|---|
| `a5-clean-controlled-path` | controlled-path | SC-514 | `A5/doctor-fixture` | yes | 2 | - | 1 | 7/-/9 | yes |
| `a5-no-bash-on-path-controlled-path` | controlled-path | SC-514 | `A5/doctor-fixture` | yes | 2 | - | 1 | 7/-/9 | yes |
| `a5-no-config-controlled-path` | controlled-path | SC-514 | `A5/doctor-fixture` | yes | 4 | - | 1 | 8/-/10 | yes |
| `a5-no-flock-controlled-path` | controlled-path | SC-514 | `A5/doctor-fixture` | yes | 2 | - | 1 | 7/-/9 | yes |
| `a5-no-git-controlled-path` | controlled-path | SC-514 | `A5/doctor-fixture` | yes | 2 | - | 1 | 7/-/9 | yes |
| `a5-no-tail-controlled-path` | controlled-path | SC-514 | `A5/doctor-fixture` | yes | 2 | - | 1 | 7/-/9 | yes |
| `a5-no-tmux-controlled-path` | controlled-path | SC-514 | `A5/doctor-fixture` | yes | 2 | - | 1 | 7/-/9 | yes |

Artifact paths — `docs/migration/evidence/batch-c-artifacts/arms/A5/<case>/`:

- `admissibility-ledger.txt` — append-only, monotonic `seq` + UTC + epoch per event:
  case open, rows, clone verification (clone vs expected fingerprint), the TAB
  round-trip START/COMPLETE, the tmux-shim equivalence START/COMPLETE, the
  before/after manifests, and every consumer START/COMPLETE with its rc and its
  stdout / stderr / tmuxtrace sha256
- `env-tab-selfcheck.txt` — the TAB round-trip in this case's own scrubbed
  environment, plus the paired `LANG=LC_ALL=C` probe on the same throwaway server
- `tmux-shim-equivalence.txt` — live cases only: the delegate-and-log shim proven
  byte-identical to the real binary on this arm's own stable topology
- `case.txt`, `env.txt`, `consumers.tsv` (label, rc, stdout/stderr sha256 + bytes,
  tmuxtrace sha256 + line count, bounded flag, exact argv)
- `out/<label>.stdout`, `out/<label>.stderr` (present only when non-empty),
  `out/<label>.tmuxtrace` — per invocation: the effective `AE_TMUX_SERVER` and kind,
  the effective locale, and the DELEGATED tmux argv
- `manifest.before.tsv` / `manifest.after.tsv` / `manifest.diff.txt` — recursive:
  type, mode, content hash, symlink target, path across the cloned AE_HOME
- `tmux.before.txt` / `tmux.after.txt`
- `A5/ledger.tsv` (case -> row ids), `A5/harness/` (the exact scripts and the
  tmux shim), `SHA256SUMS.txt` (every file above)


## Arm group A6 — requests / pairing and the unanswered threshold (bash lane)

### A6 — what the arm does

Rows: SC-518, SC-522, SC-523a, SC-523b.

**SC-518** runs the request consumer on each of the six G5 request-pair members — the
harvested control and its five named mutations — on both a protected and a writable clone,
each on its own live topology. Every case copies the member's own
`_meta/<member>.mutation.txt` in beside the captures as `member.mutation.txt`, so the byte
diff that produced the fixture sits next to the consumer's answer to it. Consumers:
`requests all`, `requests mine`, `requests inbox`, and `ae list --json`.

**SC-522 / SC-523a-b** read ONE fixture — `G2/unanswered`, whose ask timestamp is a known
epoch (1755000000) and which was never replied to — at several frozen clocks. Equality and
strictly-past are separate inputs: age exactly 1800s, and 1799s / 1801s either side.

The arm does not assume it can discriminate. Before the boundary triple means anything the
sensor has to RESPOND to age at all, so two controls far either side of the threshold are
read on the same fixture, differing only in the frozen clock, and `responsive=` is recorded
from that comparison. If the two agreed, the record says so and the arm is INCONCLUSIVE for
these rows rather than evidence of either answer. A threshold fixture whose readings all
land the same way cannot tell `>=` from `>`.

The threshold is also read from the environment at a fixed age under the scrubbed set:
unset (the documented default), an explicit smaller value, and a malformed value.

`discrimination.txt` carries every reading with the sha256 of the captured
`list --json` bytes it was derived from. It is built by `derive-discrimination.py`, a
tested helper, after the first version used sed BRE alternation — a GNU extension that
matches NOTHING on BSD — and reported every reading as empty, which the record then
rendered as `responsive=no`. The fixture was always responsive; the instrument was blind.
That correction is appended to the case ledger rather than edited into it, because the
ledger is append-only: the superseded line stays, and the correction names it, gives the
reason, and carries the corrected artifact's hash.
### A6 case table

`checks<first consumer` names the ledger sequence numbers of the TAB round-trip
COMPLETE, the tmux-shim equivalence COMPLETE (`-` where the case starts no server),
and the first `consumer-START`. The ledger is append-only and written by the checks
themselves, so the ordering is established by the original durable content — not by
file mtimes and not by a hash list added afterwards. For a barrier case the first
consumer activity is `barrier-ARMED` (the hooked run has no `consumer-START` line).

A case whose design includes a CONTROLLER MUTATION necessarily shows a tmux delta;
what the controller did, when, and from where is in `controller-mutation.txt` and in
the ledger, and the before/at-barrier/after tmux snapshots bracket it.

| case | clone | rows | template | clone fp = template fp | manifest diff | tmux snapshot identical | consumers | checks<first consumer | ordered |
|---|---|---|---|---|---|---|---|---|---|
| `a6-c01-m1-control-ro` | ro | SC-518 | `G5/m1-control` | yes | 0 | - | 4 | 7/9/11 | yes |
| `a6-c01-m1-control-rw` | rw | SC-518 | `G5/m1-control` | yes | 0 | - | 4 | 7/9/11 | yes |
| `a6-c02-m2-wrong-ref-ro` | ro | SC-518 | `G5/m2-wrong-ref` | yes | 0 | - | 4 | 7/9/11 | yes |
| `a6-c02-m2-wrong-ref-rw` | rw | SC-518 | `G5/m2-wrong-ref` | yes | 0 | - | 4 | 7/9/11 | yes |
| `a6-c03-m3-wrong-actor-ro` | ro | SC-518 | `G5/m3-wrong-actor` | yes | 0 | - | 4 | 7/9/11 | yes |
| `a6-c03-m3-wrong-actor-rw` | rw | SC-518 | `G5/m3-wrong-actor` | yes | 0 | - | 4 | 7/9/11 | yes |
| `a6-c04-m4-wrong-target-ro` | ro | SC-518 | `G5/m4-wrong-target` | yes | 0 | - | 4 | 7/9/11 | yes |
| `a6-c04-m4-wrong-target-rw` | rw | SC-518 | `G5/m4-wrong-target` | yes | 0 | - | 4 | 7/9/11 | yes |
| `a6-c05-m5-routed-vs-routed-mismatch-ro` | ro | SC-518 | `G5/m5-routed-vs-routed-mismatch` | yes | 0 | - | 4 | 7/9/11 | yes |
| `a6-c05-m5-routed-vs-routed-mismatch-rw` | rw | SC-518 | `G5/m5-routed-vs-routed-mismatch` | yes | 0 | - | 4 | 7/9/11 | yes |
| `a6-c06-m6-mixed-routed-display-ro` | ro | SC-518 | `G5/m6-mixed-routed-display` | yes | 0 | - | 4 | 7/9/11 | yes |
| `a6-c06-m6-mixed-routed-display-rw` | rw | SC-518 | `G5/m6-mixed-routed-display` | yes | 0 | - | 4 | 7/9/11 | yes |
| `a6-threshold-fixed-clock` | fixed-clock | SC-522,SC-523a,SC-523b | `G2/unanswered` | yes | 0 | - | 18 | 7/9/12 | yes |

Artifact paths — `docs/migration/evidence/batch-c-artifacts/arms/A6/<case>/`:

- `admissibility-ledger.txt` — append-only, monotonic `seq` + UTC + epoch per event:
  case open, rows, clone verification (clone vs expected fingerprint), the TAB
  round-trip START/COMPLETE, the tmux-shim equivalence START/COMPLETE, the
  before/after manifests, and every consumer START/COMPLETE with its rc and its
  stdout / stderr / tmuxtrace sha256
- `env-tab-selfcheck.txt` — the TAB round-trip in this case's own scrubbed
  environment, plus the paired `LANG=LC_ALL=C` probe on the same throwaway server
- `tmux-shim-equivalence.txt` — live cases only: the delegate-and-log shim proven
  byte-identical to the real binary on this arm's own stable topology
- `case.txt`, `env.txt`, `consumers.tsv` (label, rc, stdout/stderr sha256 + bytes,
  tmuxtrace sha256 + line count, bounded flag, exact argv)
- `out/<label>.stdout`, `out/<label>.stderr` (present only when non-empty),
  `out/<label>.tmuxtrace` — per invocation: the effective `AE_TMUX_SERVER` and kind,
  the effective locale, and the DELEGATED tmux argv
- `manifest.before.tsv` / `manifest.after.tsv` / `manifest.diff.txt` — recursive:
  type, mode, content hash, symlink target, path across the cloned AE_HOME
- `tmux.before.txt` / `tmux.after.txt`
- `A6/ledger.tsv` (case -> row ids), `A6/harness/` (the exact scripts and the
  tmux shim), `SHA256SUMS.txt` (every file above)


## Arm group A7 — meta grammar (bash lane)

### A7 — what the arm does

Rows: SC-405a through SC-405g, and SC-405j. 36 case runs, each on both a protected and a
writable clone, with the fixture's own `meta` and `events.jsonl` bytes copied in beside the
captures (`meta.bytes.txt`, `events.bytes.jsonl`) so the grammar under test sits next to the
consumer's reading of it, and the member's own named byte diff copied in as `member.mutation.txt` where one exists.

**SC-405a** uses a meta value the producer itself wrote containing several `=` characters
(`goal=alpha=beta=gamma delta=epsilon`), so a first-equals split and an any-equals split
disagree on a real line. `a7-c02` adds a DUPLICATE meta key: the same key twice with
different values, so a first-wins reader and a last-wins reader disagree too.

**SC-405f is an ORDER claim, so it carries the controls an order claim needs.** The opposed
fixture appends the NEWER timestamp first and the OLDER one second, which makes the two
candidate answers different strings by construction — a last-record reader and a
max-timestamp reader cannot both be right. Two controls sit beside it: an AGREEING pair
where both candidates coincide (so the reader is known to respond to a second goal at all)
and a SINGLE goal baseline, plus G9's four goals with increasing timestamps.
`order-discrimination.txt` names both candidate answers, states that they differ, and
records what the consumer rendered, with each source's sha256. No conclusion is drawn there.

**SC-405g** gives the two candidate branch sources DELIBERATELY DIFFERENT values — the git
branch in the work dir, and tmux `@ae_branch_name` set to a value no git branch has — and
reads the same fixture twice: `a7-c10` on a live server with the option set, `a7-c11` with
no server at all, leaving only the git source. Source ownership is observable rather than a
coincidence of the two agreeing.

**SC-405j** runs seven cases that share ONE display name and differ only in the completeness
of the reply's routing members: full and fresh, all four present but naming a slot and
session this is not, slot-only, session-only, keyless, one member as the empty string, and
all four as the empty string. `identity-405j-record.txt` carries every reading with its
source hash and a `discriminating=` line computed from the distinct outcomes observed.

**These cases were rebuilt.** The first attempt put them on a LONE ASK and all seven
rendered identically — a property of the fixture, not the product: a routing member with
nothing to pair against cannot affect anything. The A6 SC-518 captures had already shown the
consumer responds sharply to pairing inputs, which is what made a flat result suspect. The
cases now sit on a real ask→reply pair, the superseded ones were removed rather than
published, and the record says why.
### A7 case table

`checks<first consumer` names the ledger sequence numbers of the TAB round-trip
COMPLETE, the tmux-shim equivalence COMPLETE (`-` where the case starts no server),
and the first `consumer-START`. The ledger is append-only and written by the checks
themselves, so the ordering is established by the original durable content — not by
file mtimes and not by a hash list added afterwards. For a barrier case the first
consumer activity is `barrier-ARMED` (the hooked run has no `consumer-START` line).

A case whose design includes a CONTROLLER MUTATION necessarily shows a tmux delta;
what the controller did, when, and from where is in `controller-mutation.txt` and in
the ledger, and the before/at-barrier/after tmux snapshots bracket it.

| case | clone | rows | template | clone fp = template fp | manifest diff | tmux snapshot identical | consumers | checks<first consumer | ordered |
|---|---|---|---|---|---|---|---|---|---|
| `a7-c01-meta-multi-equals-ro` | ro | SC-405a | `A7/meta-multi-equals` | yes | 0 | - | 7 | 8/10/12 | yes |
| `a7-c01-meta-multi-equals-rw` | rw | SC-405a | `A7/meta-multi-equals` | yes | 0 | - | 7 | 8/10/12 | yes |
| `a7-c02-meta-duplicate-key-ro` | ro | SC-405a,SC-405d | `A7/meta-duplicate-key` | yes | 0 | - | 7 | 8/10/12 | yes |
| `a7-c02-meta-duplicate-key-rw` | rw | SC-405a,SC-405d | `A7/meta-duplicate-key` | yes | 0 | - | 7 | 8/10/12 | yes |
| `a7-c03-meta-unknown-keys-ro` | ro | SC-405b,SC-405c | `G7/meta-unknown-keys` | yes | 0 | - | 7 | 8/10/12 | yes |
| `a7-c03-meta-unknown-keys-rw` | rw | SC-405b,SC-405c | `G7/meta-unknown-keys` | yes | 0 | - | 7 | 8/10/12 | yes |
| `a7-c04-healthy-baseline-ro` | ro | SC-405b,SC-405c | `G1/healthy` | yes | 0 | - | 7 | 8/10/12 | yes |
| `a7-c04-healthy-baseline-rw` | rw | SC-405b,SC-405c | `G1/healthy` | yes | 0 | - | 7 | 8/10/12 | yes |
| `a7-c05-meta-mode-000-ro` | ro | SC-405e | `G3/meta-mode-000` | yes | 0 | - | 7 | 8/10/12 | yes |
| `a7-c05-meta-mode-000-rw` | rw | SC-405e | `G3/meta-mode-000` | yes | 0 | - | 7 | 8/10/12 | yes |
| `a7-c06-goal-order-opposed-ro` | ro | SC-405f | `A7/goal-order-opposed` | yes | 0 | - | 7 | 8/10/12 | yes |
| `a7-c06-goal-order-opposed-rw` | rw | SC-405f | `A7/goal-order-opposed` | yes | 0 | - | 7 | 8/10/12 | yes |
| `a7-c07-goal-order-agreeing-ro` | ro | SC-405f | `A7/goal-order-agreeing` | yes | 0 | - | 7 | 8/10/12 | yes |
| `a7-c07-goal-order-agreeing-rw` | rw | SC-405f | `A7/goal-order-agreeing` | yes | 0 | - | 7 | 8/10/12 | yes |
| `a7-c08-goal-order-single-ro` | ro | SC-405f | `A7/goal-order-single` | yes | 0 | - | 7 | 8/10/12 | yes |
| `a7-c08-goal-order-single-rw` | rw | SC-405f | `A7/goal-order-single` | yes | 0 | - | 7 | 8/10/12 | yes |
| `a7-c09-goals-distinct-ts-ro` | ro | SC-405f | `G9/goals-distinct-ts` | yes | 0 | - | 7 | 8/10/12 | yes |
| `a7-c09-goals-distinct-ts-rw` | rw | SC-405f | `G9/goals-distinct-ts` | yes | 0 | - | 7 | 8/10/12 | yes |
| `a7-c10-branch-live-ro` | ro | SC-405g | `A7/branch-two-sources` | yes | 0 | - | 7 | 9/11/13 | yes |
| `a7-c10-branch-live-rw` | rw | SC-405g | `A7/branch-two-sources` | yes | 0 | - | 7 | 9/11/13 | yes |
| `a7-c11-branch-stopped-ro` | ro | SC-405g | `A7/branch-two-sources` | yes | 0 | - | 7 | 8/-/10 | yes |
| `a7-c11-branch-stopped-rw` | rw | SC-405g | `A7/branch-two-sources` | yes | 0 | - | 7 | 8/-/10 | yes |
| `a7-c12-405j-pair-full-fresh-ro` | ro | SC-405j | `A7/pair-405j-full-fresh` | yes | 0 | - | 7 | 8/10/12 | yes |
| `a7-c12-405j-pair-full-fresh-rw` | rw | SC-405j | `A7/pair-405j-full-fresh` | yes | 0 | - | 7 | 8/10/12 | yes |
| `a7-c13-405j-pair-stale-keys-ro` | ro | SC-405j | `A7/pair-405j-stale-keys` | yes | 0 | - | 7 | 8/10/12 | yes |
| `a7-c13-405j-pair-stale-keys-rw` | rw | SC-405j | `A7/pair-405j-stale-keys` | yes | 0 | - | 7 | 8/10/12 | yes |
| `a7-c14-405j-pair-slot-only-ro` | ro | SC-405j | `A7/pair-405j-slot-only` | yes | 0 | - | 7 | 8/10/12 | yes |
| `a7-c14-405j-pair-slot-only-rw` | rw | SC-405j | `A7/pair-405j-slot-only` | yes | 0 | - | 7 | 8/10/12 | yes |
| `a7-c15-405j-pair-session-only-ro` | ro | SC-405j | `A7/pair-405j-session-only` | yes | 0 | - | 7 | 8/10/12 | yes |
| `a7-c15-405j-pair-session-only-rw` | rw | SC-405j | `A7/pair-405j-session-only` | yes | 0 | - | 7 | 8/10/12 | yes |
| `a7-c16-405j-pair-keyless-ro` | ro | SC-405j | `A7/pair-405j-keyless` | yes | 0 | - | 7 | 8/10/12 | yes |
| `a7-c16-405j-pair-keyless-rw` | rw | SC-405j | `A7/pair-405j-keyless` | yes | 0 | - | 7 | 8/10/12 | yes |
| `a7-c17-405j-pair-one-empty-ro` | ro | SC-405j | `A7/pair-405j-one-empty` | yes | 0 | - | 7 | 8/10/12 | yes |
| `a7-c17-405j-pair-one-empty-rw` | rw | SC-405j | `A7/pair-405j-one-empty` | yes | 0 | - | 7 | 8/10/12 | yes |
| `a7-c18-405j-pair-all-empty-ro` | ro | SC-405j | `A7/pair-405j-all-empty` | yes | 0 | - | 7 | 8/10/12 | yes |
| `a7-c18-405j-pair-all-empty-rw` | rw | SC-405j | `A7/pair-405j-all-empty` | yes | 0 | - | 7 | 8/10/12 | yes |

Artifact paths — `docs/migration/evidence/batch-c-artifacts/arms/A7/<case>/`:

- `admissibility-ledger.txt` — append-only, monotonic `seq` + UTC + epoch per event:
  case open, rows, clone verification (clone vs expected fingerprint), the TAB
  round-trip START/COMPLETE, the tmux-shim equivalence START/COMPLETE, the
  before/after manifests, and every consumer START/COMPLETE with its rc and its
  stdout / stderr / tmuxtrace sha256
- `env-tab-selfcheck.txt` — the TAB round-trip in this case's own scrubbed
  environment, plus the paired `LANG=LC_ALL=C` probe on the same throwaway server
- `tmux-shim-equivalence.txt` — live cases only: the delegate-and-log shim proven
  byte-identical to the real binary on this arm's own stable topology
- `case.txt`, `env.txt`, `consumers.tsv` (label, rc, stdout/stderr sha256 + bytes,
  tmuxtrace sha256 + line count, bounded flag, exact argv)
- `out/<label>.stdout`, `out/<label>.stderr` (present only when non-empty),
  `out/<label>.tmuxtrace` — per invocation: the effective `AE_TMUX_SERVER` and kind,
  the effective locale, and the DELEGATED tmux argv
- `manifest.before.tsv` / `manifest.after.tsv` / `manifest.diff.txt` — recursive:
  type, mode, content hash, symlink target, path across the cloned AE_HOME
- `tmux.before.txt` / `tmux.after.txt`
- `A7/ledger.tsv` (case -> row ids), `A7/harness/` (the exact scripts and the
  tmux shim), `SHA256SUMS.txt` (every file above)

## D-record executions (bash lane)

### D-record executions

All five b0-design concurrency designs have run, each with its CONTROLLER-ONLY TWIN — the
same mutation performed alone, with no reader blocked in it, captured identically, so the
controller's own effect can be subtracted. SC-1306a–e ride these five per the mapping note
and get no separate arms: 1306a→D01, 1306b→D04a, 1306c→D04b, 1306d→D02, 1306e→D03.

Every barrier case carries `hook.patch`, the per-fixture `hook-inactive-equivalence.txt`
with its measured control-control volatility floor, a `flockspy` log for every invocation
(a PATH-first `flock` shim that is pure delegate-and-log and preserves argv and rc), and
snapshots bracketing the mutation on both sides.

**D01 — Design 2, list reader vs a live writer.** Hook `H_LIST_META_CAPTURED` at the
running-session site, immediately after `meta_blob` is read. At the barrier the controller
invokes the session's OWN real `goal` helper once — one logical writer operation that
rewrites `goal` in meta AND appends a goal event, both captured before and after.

**D02 — Design 3, request-scan reader vs a reply writer.** Hook
`H_REQUEST_SCAN_COMPLETE`, after the reversed scan and before row emission. The controller
appends ONE producer-harvested, identity-valid reply event. `_ar_request_states` is emitted
into the session's OWN generated `requests` helper by `declare -f`, so the patch adds
`_ae_hook` to the `_lib` emission list and the case regenerates the clone's helper set with
the HOOKED binary through the frozen refresh path;
`helper-refresh-equivalence.txt` proves the refreshed helper answers byte-for-byte as the
pre-refresh one did with the hook inactive, exercised IN PLACE because the helper sources
`_lib` from its own directory and a copy elsewhere cannot run at all.

**D03 — Design 4, events-tail follow semantics.** No hook: the REAL generated events helper
runs in a pane and the LAUNCH BARRIER is a positive pane observation (the final baseline
record `D03-SEED-EVENT-31` rendered), bounded, with expiry recorded INCONCLUSIVE. Every
controller write is confirmed by a file-size STAT BARRIER, never by a sleep. Four arms:
the initial window; one complete harvested append; line framing in two steps (a partial
producer-derived line with NO terminating newline, stat-confirmed and captured, then the
withheld remainder plus the newline, stat-confirmed and captured); and rotation — a
hardlink held to the original inode, an atomic replace (inode change recorded), then
DISTINCT harvested sentinels appended to the new path and through the hardlink to the old
inode, each stat-confirmed and each followed by a bounded pane poll. Arms 2–4 have twins;
arm 1 is read-only and exempt.

**D04a — Design 5, status pane-set cut.** A delegating tmux shim captures the REAL
`list-panes` result, signals `H_STATUS_PANESET` and BLOCKS before replay; the controller
kills one listed pane and creates one new pane, then releases, and the shim replays exactly
the captured bytes and exit status. Both mandatory topology arms — exact-name only, and a
prefix-sibling session whose name EXTENDS the target's. The shim's inactive equivalence is
proven on the same stable topology before its active barrier is used; its default mode is
pure delegate-and-log and the active barrier exists only when `AE_TMUX_BARRIER_DIR` is set.

**D04b — Design 6, next selection/recheck cut.** Both hooks: `H_NEXT_SELECTED` (after
best-candidate resolution, before the exact recheck) and `H_NEXT_RECHECKED` (after the
successful exact match, before the final focus call). Attach arms run `next --attach` from
a pane INSIDE an attached client: the harness attaches a scripted client on a REAL pty to a
CALLER session first — `script(1)` cannot be used here because it calls `tcgetattr` on its
own stdin and this harness has no controlling terminal, so `pty-attach.py` forks a pty
directly. Every controller kill is issued from a SEPARATE connection, never from inside the
client under test. `list-clients` mappings are captured before and after. Three arms plus a
no-kill twin: kill at `H_NEXT_SELECTED`; kill after `H_NEXT_RECHECKED` WITH a prefix
sibling present; and the companion arm with no sibling.
### D case table

`checks<first consumer` names the ledger sequence numbers of the TAB round-trip
COMPLETE, the tmux-shim equivalence COMPLETE (`-` where the case starts no server),
and the first `consumer-START`. The ledger is append-only and written by the checks
themselves, so the ordering is established by the original durable content — not by
file mtimes and not by a hash list added afterwards. For a barrier case the first
consumer activity is `barrier-ARMED` (the hooked run has no `consumer-START` line).

A case whose design includes a CONTROLLER MUTATION necessarily shows a tmux delta;
what the controller did, when, and from where is in `controller-mutation.txt` and in
the ledger, and the before/at-barrier/after tmux snapshots bracket it.

| case | clone | rows | template | clone fp = template fp | manifest diff | tmux snapshot identical | consumers | checks<first consumer | ordered |
|---|---|---|---|---|---|---|---|---|---|
| `d01-controller-only-twin-twin` | twin | D01,SC-1306a | `G1/healthy` | yes | 8 | - | 2 | 7/9/- | NO |
| `d01-list-vs-goal-writer-barrier` | barrier | D01,SC-1306a | `G1/healthy` | yes | 8 | - | 3 | 7/9/16 | yes |
| `d02-controller-only-twin-twin` | twin | D02,SC-1306d | `D/d02-pending-with-harvested-reply` | yes | 4 | - | 2 | 7/9/- | NO |
| `d02-requests-vs-reply-writer-barrier` | barrier | D02,SC-1306d | `D/d02-pending-with-harvested-reply` | yes | 4 | - | 3 | 7/9/19 | yes |
| `d03-a1-initial-window-follow` | follow | D03,SC-1306e | `D/d03-31-numbered-events` | yes | 0 | - | 0 | 6/-/- | NO |
| `d03-a2-complete-append-follow` | follow | D03,SC-1306e | `D/d03-31-numbered-events` | yes | 4 | - | 0 | 6/-/- | NO |
| `d03-a2-twin-twin` | twin | D03,SC-1306e | `D/d03-31-numbered-events` | yes | 4 | - | 0 | 6/-/- | NO |
| `d03-a3-line-framing-follow` | follow | D03,SC-1306e | `D/d03-31-numbered-events` | yes | 4 | - | 0 | 6/-/- | NO |
| `d03-a3-twin-twin` | twin | D03,SC-1306e | `D/d03-31-numbered-events` | yes | 4 | - | 0 | 6/-/- | NO |
| `d03-a4-rotation-follow` | follow | D03,SC-1306e | `D/d03-31-numbered-events` | yes | 6 | - | 0 | 6/-/- | NO |
| `d03-a4-twin-twin` | twin | D03,SC-1306e | `D/d03-31-numbered-events` | yes | 6 | - | 0 | 6/-/- | NO |
| `d04a-exact-barrier-barrier` | barrier | D04a,SC-1306b | `live/no-template (live launch + pane-set cut)` | - | 0 | - | 2 | 5/7/10 | yes |
| `d04a-exact-twin-twin` | twin | D04a,SC-1306b | `live/no-template (live launch + pane-set cut)` | - | 0 | - | 1 | 5/7/- | NO |
| `d04a-prefix-barrier-barrier` | barrier | D04a,SC-1306b | `live/no-template (live launch + pane-set cut)` | - | 0 | - | 2 | 6/8/11 | yes |
| `d04a-prefix-twin-twin` | twin | D04a,SC-1306b | `live/no-template (live launch + pane-set cut)` | - | 0 | - | 1 | 6/8/- | NO |
| `d04b-arm1-kill-at-selected-attach` | attach | D04b,SC-1306c | `A2/composite` | - | 0 | - | 1 | 8/10/16 | yes |
| `d04b-arm2-prefix-kill-after-recheck-attach` | attach | D04b,SC-1306c | `A2/composite` | - | 0 | - | 1 | 9/11/17 | yes |
| `d04b-arm3-nosibling-kill-after-recheck-attach` | attach | D04b,SC-1306c | `A2/composite` | - | 0 | - | 1 | 8/10/16 | yes |
| `d04b-twin-no-kill-attach` | attach | D04b,SC-1306c | `A2/composite` | - | 0 | - | 1 | 8/10/16 | yes |

Artifact paths — `docs/migration/evidence/batch-c-artifacts/arms/D/<case>/`:

- `admissibility-ledger.txt` — append-only, monotonic `seq` + UTC + epoch per event:
  case open, rows, clone verification (clone vs expected fingerprint), the TAB
  round-trip START/COMPLETE, the tmux-shim equivalence START/COMPLETE, the
  before/after manifests, and every consumer START/COMPLETE with its rc and its
  stdout / stderr / tmuxtrace sha256
- `env-tab-selfcheck.txt` — the TAB round-trip in this case's own scrubbed
  environment, plus the paired `LANG=LC_ALL=C` probe on the same throwaway server
- `tmux-shim-equivalence.txt` — live cases only: the delegate-and-log shim proven
  byte-identical to the real binary on this arm's own stable topology
- `case.txt`, `env.txt`, `consumers.tsv` (label, rc, stdout/stderr sha256 + bytes,
  tmuxtrace sha256 + line count, bounded flag, exact argv)
- `out/<label>.stdout`, `out/<label>.stderr` (present only when non-empty),
  `out/<label>.tmuxtrace` — per invocation: the effective `AE_TMUX_SERVER` and kind,
  the effective locale, and the DELEGATED tmux argv
- `manifest.before.tsv` / `manifest.after.tsv` / `manifest.diff.txt` — recursive:
  type, mode, content hash, symlink target, path across the cloned AE_HOME
- `tmux.before.txt` / `tmux.after.txt`
- `D/ledger.tsv` (case -> row ids), `D/harness/` (the exact scripts and the
  tmux shim), `SHA256SUMS.txt` (every file above)

