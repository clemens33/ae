# Batch H — run manifest

PRE-REGISTRATION. Every arm script is committed and hash-registered HERE BEFORE its first
run. Any amendment to a script is durable and REOPENS the arms that ran under the previous
hash. Adaptive capture — choosing what to record after seeing a reading — is therefore a
diff rather than a promise.

## Exposure record (required by seat ruling)

The author of the design and census (opus5:cexec) saw answer-labelled source outcomes in
design drafts v1-v3 before they were moved to the seat annex: `interrupt`'s silent
non-refusal path, `goal`'s arity guard, the `%*` branch's unconditional return,
`_register-sid`'s selection order, and `say`'s exact stdout. Recorded so a seat reading
these captures knows what the author knew. The arms are built from
`batch-h-input-list.md`, which is generated from the census by a columnar drop and gated
by a checker with per-arm neutral/mutated calibration.

## Scope

D14b is HELD and EXCLUDED pending the ownership-record split; no probe of it runs.
SC-1301 runs last, under the hook and barrier machinery its own section describes.

## Pre-registered scripts

| script | sha256 | registered (UTC) |
|---|---|---|
| `_harness/hlib.sh` | `b2cf84dd1990c01fdbfc9512c00834d10543d4745ed6a2dc2ad6c134ea27455d` | 2026-08-20T21:51:16Z |
| `_harness/hfix.sh` | `1227077c8691e481c0f5074b847e113acf4a963787ec875e238c7fe7dd2e61a0` | 2026-08-20T21:51:16Z |
| `_harness/arm-h5.sh` | `cc6cfbd8abe690024bae907a7c87f28ce12c97a61212119d000cd461786735fb` | 2026-08-21 (registered BEFORE its first run) |
| `_harness/arm-h4.sh` | `5944b88c92acd4752addffabe9705ab087ff6c430bed0ea1d0bf6d8040693009` | 2026-08-20T21:51:16Z |

### A3 — `_harness/derive-h4-record.py` registered after A-H4's run

- new sha256: `c7c96f2a2da3c67703b11a4fe6f8dd101b70146d2f7a7556fdb996b11f15f15f`
- what it is: a REPORTING script that reads A-H4's committed captures and emits
  `resolution-record.txt`. It records no new observation and cannot change one.
- why it is registered anyway: pre-registration is about the capture program, and a
  reporting script that runs after the captures exist is outside it — but it is committed
  and hashed here so a seat reading the record can regenerate it and get the same bytes.

### A5 — `_harness/finalize-h-arm.sh` registered after A-H4's run

- new sha256: `3e6bdba0ccc314ebeb2700900d1c17b956623b8e07db5f9032a5a573ae6898de`
- what it is: a post-capture indexer. It writes the content-bound case index (case dir ->
  ledger sha256, ledger lines, file count) and records no observation.

### A6 — constructed inputs in A-H5, declared before the run

`_register-sid` reads Codex session `.jsonl` files and the `launch_id`/`launch_time` meta
keys. There is no offline producer for either — this batch runs with no live models and no
network, and the fake agent is not a codex-kind tool, so ae never writes those keys.

The candidate files and meta lines are therefore written by the CONTROLLER, and each case
records the exact bytes it planted in `planted-inputs.txt` with their hashes. They are
INPUT DATA the surface reads, not helper bytes: every helper byte still comes from a real
frozen launch. Declared here rather than discovered later, because "producer-derived" is a
claim this arm cannot make about its candidates and should not appear to.

### A7 — `_harness/derive-h5-record.py` registered after A-H5's run

- new sha256: `f71e06aea76925ad10e036ad1830afb4a618324d10486e28be1e44602a7e09d4`
- post-capture reporting only; reads the committed captures and records no observation.

## Frozen source

`72c7293`, verified by hash at every case open and recorded in each case's ledger.

## Amendments

### A1 — `_harness/hlib.sh`, before any arm completed

- previous sha256: `a655b4394a65bf7f429cd003fd5b6c4316bad876f42b62d4c2bd5ec1bbe4d320`
- new sha256: `40ace86d757f5e0f27734472a25d6f4658875140720b85499bce2b6f90f0cb5f`
- what changed: `canary()`'s three locals were declared in ONE `local` statement whose
  third value interpolated the first two. Bash expands every word of such a statement
  before any assignment takes effect, so `when` was still unset when `label` was built and
  the run aborted under `set -u`. Split into separate statements.
- why it is not adaptive capture: the arm never produced a reading. A-H4's first run died
  inside the PRE canary of its first case — the instrument refusing to start, which is the
  canary doing its job — and its partial output was deleted rather than published.
- arms reopened: A-H4 (it had not completed; nothing ran under the previous hash).
- note: this is the exact class AGENTS.md documents for `export HOME=... AE_HOME=...`. A
  documented hazard does not prevent itself.

### A2 — `_harness/hfix.sh` and `_harness/arm-h4.sh`, after a run whose fixture was wrong

- `hfix.sh`: `73c287312f99824b00450e24da687bb756e5c3a376fcd5d0192ed5dd05a3feda` -> `1227077c8691e481c0f5074b847e113acf4a963787ec875e238c7fe7dd2e61a0`
- `arm-h4.sh`: `6f6208234ea3d46c11c4d9493554d05008650f1b3752f7361515f5299d49b91b` -> `5944b88c92acd4752addffabe9705ab087ff6c430bed0ea1d0bf6d8040693009`
- what changed: the H5 fixture did not carry the collisions three of its cases name, and
  the run was discarded rather than published. Specifically:
  1. `workers = cx:lead` was RENAMED to `cx:lead-2` by the frozen launcher's own worker
     dedup, so the bare-name-AMBIGUOUS case had no collision — it measured a unique name.
     The collision is now created through the real `spawn` helper, the product path that
     can make one.
  2. the second session reused the first's config and came up with an identical roster, so
     the cross-session case that names "session exists, agent present" resolved nothing and
     measured the same thing as the "agent absent" case beside it. The second session now
     gets its own roster via `h_reconfig`.
  3. the dead-pane manipulation killed `zz:only` — which is also the fixture for
     "alias-only unique". One case's manipulation destroyed another case's precondition. A
     fourth alias (`qq:spare`) exists solely to be killed.
- why it is not adaptive capture: nothing from the first run is published. The defect is in
  the FIXTURE's ability to present the input class, not in what any reading said, and it was
  found by reading the roster the arm itself captured — which is why the arm now writes
  `fixture-validity.txt` naming what the roster and the server actually carry, before any
  case runs.
- arms reopened: A-H4, entirely.

### A4 — `_harness/hlib.sh`, event name made batch-distinct

- `40ace86d757f5e0f27734472a25d6f4658875140720b85499bce2b6f90f0cb5f` -> `b2cf84dd1990c01fdbfc9512c00834d10543d4745ed6a2dc2ad6c134ea27455d`
- what changed: `surface_state()` emitted `source-state-captured`, the same event batch C
  uses to declare it captured `meta-state.txt`. The shared name made batch C's schema rule
  apply to batch H's cases, which capture `surface-state.txt` instead — so the gate
  demanded an artifact this batch does not produce. Renamed to `surface-state-captured`,
  and the schema kinds now fork by batch layout rather than one batch's rule being
  weakened to fit the other's captures.
- why it is not adaptive capture: the event name is a LEDGER LABEL, not a reading. No
  capture changes. A-H4 is re-run anyway so its committed ledgers match the script that is
  registered against them — a ledger naming an event its script no longer emits is exactly
  the drift the hash registration exists to prevent.
- arms reopened: A-H4.
