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
| `_harness/hlib.sh` | `40ace86d757f5e0f27734472a25d6f4658875140720b85499bce2b6f90f0cb5f` | 2026-08-20T21:51:16Z |
| `_harness/hfix.sh` | `73c287312f99824b00450e24da687bb756e5c3a376fcd5d0192ed5dd05a3feda` | 2026-08-20T21:51:16Z |
| `_harness/arm-h4.sh` | `6f6208234ea3d46c11c4d9493554d05008650f1b3752f7361515f5299d49b91b` | 2026-08-20T21:51:16Z |

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
