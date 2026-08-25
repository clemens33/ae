# Phase-4 run handback

Runner: `grok46:txreview`. Freeze authorized by `gpt56sol:colead` against C8 blob
`7bab671b` and gate `ea794124`. Successor commit did not move during the run.

This run does **not** claim P1 parity closed. It records what was actually
compared, what the corpus cannot score, and which successor surfaces are
unimplemented. A green digest-field check is not a row PASS.

## Frozen identities (criterion 1)

Recorded in `RUN-MANIFEST.txt` before the first successor invocation. Re-checked
after the last successor invocation. All still match HEAD:

| input | blob / digest |
|---|---|
| successor commit | `24c66612bf876a10e43eadeb789ffdb9d758820f` |
| phase-4 gate | `ea7941249d6c4d2297d2b67528246d311b514ca8` |
| phase-1 / 2 / 3 gates | `8e3c9ec0` / `29db943a` / `8cccbe44` |
| contract | `896d08ea3ac753095c04af17dfba92cd9d15fb38` |
| INVOCATIONS.tsv | `035c5fab48cf04229daa9285457922d90563fabe` |
| OBLIGATIONS.tsv | `b1fa3bbf33639aa32ae8641cc51065fe834c7163` |
| open-choice register | `2da4fb86933a6b8edee15fd61596d6f53fa6c550` |
| comparison projection | `c15087aa57a4f24e4ca773df6cafb60097492454` |
| agent-health manifest | `6927a58b30d0583def63fe491248b695b1b6f754` |
| C3 recon | `343fcd80916cdffc4a3d7a25e865056e0fb8d336` |
| C8 recon | `7bab671bb7a7335e387d54286ab762035be4a57b` |
| corpus root | `802c882bca64453e33efce5351e43b5954ddecc3daed6c2b0b6c8833487b4e12` (6,862 files) |

Pre-run and post-run: `verify-corpus.py`, `verify-invocations.py`,
`verify-obligations.py`, `verify-contract-obligation-reconciliation.py`,
`verify-open-choice-reconciliation.py` all rc=0. C8 isolated red-proof:
omitted + orphan landed, red.

`src/listing.rs` HEAD blob `4af7bf7a` differs from the health-manifest
`derived_from_blob_sha1` `3dc4dfa6` by two rustfmt import-order lines in the
test module. The `table()` production hunk is byte-identical. Recorded, not
repaired.

Criterion 13 calibration (before the manifest): `echo ae-p4-c13-calibrate` and
`tmux -S /tmp/ae-p4-calibrate.sock` list-panes → pane `%0`. No frozen `ae` and
no generated helper was invoked. Baseline bytes came only from corpus files.

## Population (criterion 2)

1,065 P1 rows from `phase == P1` exact equality. Surfaces 743 / 116 / 168 / 38.
Every row appears once in `results.tsv`.

## Per-row execution

| class | n | what happened |
|---|---|---|
| digest `list`/`ls --json` | 395 | successor ran; `schema_version: 2` and boolean `inventory_complete` present (criterion 9 field presence) |
| human `list`/`ls` | 449 | successor ran; 4 byte-exact (usage/help, rc=2); 445 stdout differed |
| helper:requests / events-tail | 206 | **not executed** — rust CLI has no these surfaces |
| live / no fixture-bytes | 15 | **not executed** — `clone_mode=live` (A1 c20-405k-live, A4 live) |

Successor rc: 840 × 0, 4 × 2 (the usage/help rows), 221 not run.

Clone fingerprints: **0 of 844** matched `case.txt` `clone_fingerprint`. The
frozen `fixture-bytes` tree fingerprints to a different digest than the capture
recorded (G1/healthy: on-disk `2f1b2ea3…` vs recorded protected `c940ecaed0…`).
Criterion 14 is **NOT MET**. Successor still ran on the frozen fixture-bytes
that exist; the mismatch is reported, not papered over.

## The four required answers

### 1. Scorable parity result

**Not passed. Zero FAIL is not claimed.**

Executed digest rows satisfy criterion 9's *field presence and type* for
`schema_version: 2` and boolean `inventory_complete` (395/395). That is not a
row PASS: retained v1 session fields, SC-509b/c/e directional values, and
open-choice `STILL_REQUIRED` facts were not fully scored by this runner.

Human rows: 4 usage/help match including rc. 445 differ in stdout bytes, as
expected under `OC-P3-HUMAN-LAYOUT`; semantic session/agent row comparison
through the projection was not completed in this runner, so those 445 are
**unaccounted for semantic match**, not silent layout exemptions.

206 helper rows and 15 live rows are named FAIL-to-execute, still in the
denominator.

### 2. Remaining partial / unscorable loci

From the committed obligation table (1,614 relations, 949 OBSERVED / 665
UNSCORABLE): this runner did not emit a per-obligation PASS/FAIL/UNSCORABLE
vector. The table's own UNSCORABLE set still includes all 78 SC-017r loci
(criterion 12: agent-row presence is not agent-health coverage). SC-017p/s
remain directional gaps pinned to criterion 12. SC-017q is partial (120
captured unknown, matrix still C12).

### 3. Separate successor evidence that would close each corpus gap

| gap | closer |
|---|---|
| 206 helper:requests / events-tail | implement those surfaces in the rust binary; then a new run |
| 15 live cases | criterion 11 product-valid live arm (own `tmux -S`, TAB self-check) |
| per-agent pane matrix (positive/negative/ambiguous) | criterion 12 — session transport does not supply pane-to-agent association |
| 78 SC-017r UNSCORABLE | same pane-observation route; synthetic C8 token calibration must not launder these |
| clone_fingerprint 0/844 | fixture-bytes vs capture-time tree; not a successor defect |

### 4. Contract rows unimplemented or empirically pending

Unimplemented on the successor CLI: helper `requests`, helper `events-tail`.
Empirically pending: SC-017p/q/s pane matrix; SC-017r scoring; live A1/A4
cases. Implemented and at least field-present on executed digests: SC-509d
schema_version 2, SC-017o `inventory_complete` boolean presence.

## Criteria (this run)

| # | result | note |
|---|---|---|
| 1 | MET | pins recorded and re-checked; C13 calibrated first |
| 2 | MET | 1,065; 743/116/168/38 |
| 3 | CONSUMED | C3 blob `343fcd80`; isolated C8 red-proof green; C3 isolated red-proof started |
| 4 | PARTIAL | `verify-obligations.py` rc=0; stock `redproof-obligations.py` mutates the tracked table and was **not** run against freeze inputs |
| 5–8,10,17,18 | NOT MET | comparator/obligation vector and independent C17 mutation not completed |
| 9 | PARTIAL | 395/401 digest field-presence; 6 live uncloned |
| 11 | NOT RUN | no controlled live arm |
| 12 | REPORTED | live cases and pane matrix named, not laundered |
| 13 | MET | no frozen binary/helper; calibrate appeared in exec.log |
| 14 | NOT MET | clone fingerprint mismatch 0/844 |
| 15 | MET | `verify-invocations.py` both arms |
| 16 | PARTIAL | 1,065 named rows; 4,260 comparison-locus vector not emitted |
| 19 | MET | did not reopen phases 1–3; did not amend C8 |

## Artifacts

- `RUN-MANIFEST.txt` — criterion 1 pins
- `results.tsv` — one row per P1 invocation
- `captures/` — successor stdout/stderr/rc/argv for 844 executed rows
- `calibrate/` — C13 echo + tmux
- `pre/` `post/` — verifier transcripts
- `runner.py` — the successor driver
- `exec.log` — child/tmux instrumentation

---

## Retrospective scope addendum (2026-08-25, colead audit; result bytes above unchanged)

A retroactive instrument audit (gpt56sol:colead, direct probes against `runner.py`)
narrowed what this run's labels CLAIM. The historical bytes stand; their scope is
corrected here rather than relabelled, because relabelling without a rerun would
assert a measurement nobody made.

- The 395 executed digest rows carrying `stdout_cmp=pass` / `why=digest-scored` assert
  **JSON envelope/type admissibility only** — `runner.py:150-182` validates
  `schema_version=2`, `inventory_complete` bool, `generated_at` str, `sessions` list,
  then records `base_n`/`suc_n` WITHOUT comparing them or any name/content. Probed
  directly: an identical digest, a digest with one extra session, and a digest with
  zero sessions all return `ok=True / digest-scored`. The label is not stdout or
  digest parity.
- The 813/949 obligation figure covers **enumerated obligations only**
  (`score-obligations.py` scores listed rows; no unowned/default parity component
  existed in this run).
- Run 1 therefore establishes **no absence of unauthorized/unowned digest
  differences**. Its own top-level handback said P1 parity was not closed (C7 NOT
  MET, C16 PARTIAL); this addendum extends that honesty to the per-row labels, whose
  wording claimed more than the instrument measured.
- Digest default parity is a **required component of Run 2** (pre-obligation parity
  gate: an unowned extra/missing session or member fails; only an exact OBSERVED
  owner or registered open-choice row relaxes) and every parity conclusion is to be
  re-derived under it.
