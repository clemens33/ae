# aemonitor — meta-agent state/dedup helper

**Optional contrib tooling. NOT part of core `ae`, NOT auto-installed.** Core ae
stays a single jq-free bash script; this is a separate consumer of
`ae list --json` for the [steward meta-agent](../../docs/) (Layer 3).

`ae` itself never depends on this. `ae list`, `ae next`, session start, the watchdog,
and the telegram bridge all work whether or not `aemonitor` exists.

## What it does

The steward agent is good at *phrasing* and *orchestrating* but bad at durable
bookkeeping — left to manage its own state file it drifts (freeform notes,
hand-written wrong timestamps). `aemonitor` owns the **deterministic** part:

- reads `ae list --json --running` (the structured fleet snapshot),
- diffs it against a flock'd state file,
- computes what **changed** since the last sweep — attention (blocked /
  waiting-user / dead / stale), fleet changes (a session started/ended), and
  session-level "quiet (non-done agents)" — and **dedups** it,
- maintains a quiet-sweep **liveness** counter,
- prints exact, ready-to-send report lines (empty output = nothing to report).

The steward just runs it each sweep and lets it deliver; it does **not** re-phrase
the output (that would reintroduce drift).

## Delivery-aware dedup (the important guarantee)

A change is marked **notified** only after delivery actually succeeds. Pass the
steward's `say` helper as `--notify-cmd`:

```bash
aemonitor sweep --notify-cmd /home/you/.ae/sessions/steward/say
```

`aemonitor` runs `say "<report>"` and advances `notified` **only if it exits 0**.
`last_seen` advances every sweep regardless — so a seen-but-undelivered change is
**re-reported next sweep until it lands**. A failed or forgotten send can never
permanently swallow an alert.

## Requirements

- **Python 3** (stdlib only — no jq, no third-party packages).
- `ae` on `PATH` (only when reading live data, i.e. without `--input`).

## Usage

```text
aemonitor sweep [--state PATH] [--input PATH|-] [--now EPOCH]
                [--notify-cmd CMD] [--init] [--dry-run]
                [--format text|json] [--quiet-secs N] [--liveness-sweeps N]
```

| flag | meaning |
|------|---------|
| `--state PATH` | state file. Default: derived from the current tmux session → `~/.ae/sessions/<session>/meta-agent-state.json`. Always pass it explicitly in tests. |
| `--input PATH\|-` | `ae list --json` input (file or stdin). Omit to run `ae list --json --running`. Files/stdin make it deterministically testable. |
| `--now EPOCH` | "now" in epoch seconds (default: real time). For deterministic tests. |
| `--notify-cmd PATH` | path to a single executable (e.g. the steward's `say` helper) — invoked as `PATH "<report>"`, not via a shell (no arg-splitting/injection). Commits `notified` only on exit 0. |
| `--init` | seed the state file to the current snapshot **silently** (no first-install spam), then exit. |
| `--dry-run` | preview report lines without mutating state. |
| `--format text\|json` | `text` (default, one line each) or `json` (`{report:[…], delivered:bool}`) for tests/inspection. |
| `--quiet-secs N` | a non-done session idle longer than this → "quiet … may need you" (default 1200 = 20m). |
| `--liveness-sweeps N` | silent sweeps before a "still alive" ping (default 36 ≈ 3h at the 5-min cadence). |

First run (empty state) reports current **attention** once but **suppresses the
fleet inventory** so a fresh install doesn't announce every existing session; use
`--init` to seed silently instead.

## Caveats

- **"quiet" is session-level**, derived from `ae list --json`'s per-session
  `last_active_epoch` — not per-agent activity (ae does not yet expose that). It
  is phrased as a possibility ("may need you"), never a definite "waiting".
- Fleet narration is intentionally conservative (started / ended only; no
  agent-count or active/idle churn) to keep noise down.
- The attention-reason vocabulary (`dead`/`stale`/`waiting-user`/`blocked`/
  `throttled`) is **duplicated** here from core ae's `_attn_rank`. If ae ever
  adds a new attention reason, `aemonitor`'s `RANK` map must be updated too —
  otherwise the helper silently ignores the new reason. (A `ae list --json`
  `schema_version` bump would also force a deliberate update, since unknown
  schema fails closed.)

## State file schema (v1)

Attention is keyed **per agent** (`"<session><agent-ref>"`) so a same-session
handoff (one blocked agent → another) and agent-change are not silently deduped.

```json
{
  "schema_version": 1,
  "last_sweep_at": 1750000000,
  "quiet_sweeps": 0,
  "attention": {"<session><ref>": {"reason": "blocked", "rank": 2, "first_seen": 0, "last_seen": 0, "notified": true, "cleared": false}},
  "quiet":     {"<session>": {"first_seen": 0, "last_seen": 0, "notified": true}},
  "sessions":  {"<session>": {"agents": 2}},
  "last_report_hash": "…"
}
```

`first_seen`/`last_seen` are epoch seconds; `last_seen` advances every sweep,
`notified` only after a successful delivery.

Written atomically (temp + `os.replace`), mode `0600`, under a `.lock` (flock).
An unknown/corrupt/future-schema state file is treated as empty (start clean).
