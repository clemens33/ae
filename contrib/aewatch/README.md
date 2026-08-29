# aewatch (optional contrib sidecar)

> **RETIRED (P4.3, 2026-08-29):** ae no longer launches this sidecar.
> `AE_WATCHDOG_IMPL=uv` no longer selects it, and the Telegram bridge it
> prototyped is now the Rust core (the ae core binary). Kept as archival source
> and the bash-vs-Python parity oracle; not wired into ae.
>
> Everything below is the historical description of what this sidecar was built
> to be. Read it as design history, not as a description of a live component.

`aewatch` is an **optional** uv / [PEP 723](https://peps.python.org/pep-0723/)
single-file Python sidecar that will take over ae's long-lived daemon work — the
per-session watchdog sweep and the Telegram bridge — the part where bash hurts
most (state machines, JSON, signal handling). It speaks ae's **existing** file
contracts (session `meta`, `events.jsonl`, tmux user options) so core `ae` stays
the entry point and the tmux glue.

> **Status: phase 1 — skeleton only.** Runnable CLI + `--version`. No watchdog,
> no Telegram, no tmux writes, no session-meta mutation yet. Design and the
> phase-1 slice breakdown live in the internal phase plan. Later slices add
> contract validation (a committed `CONTRACTS.md` fixture matrix), session
> discovery, and a read-only `daemon --once` tick whose only writes land under
> `$AE_HOME/aewatch/`.

## Not part of core ae

Core `ae` remains a single, dependency-light bash script and keeps working with
**no** `uv` and **no** Python — the bash watchdog stays the fallback until this
sidecar reaches behavioral parity. `aewatch` is a contrib consumer, isolated in
`contrib/aewatch/`.

## Requirements

- Python **>= 3.11** (stdlib only — the PEP 723 dependency block is empty).
- Optionally `uv` for `uv run` / `uvx` execution; plain `python3` also works.

## Usage

```bash
uv run contrib/aewatch/aewatch --version     # or: python3 contrib/aewatch/aewatch --version
```

## Tests

```bash
just test-aewatch-fast   # FAST commit inner loop — skips the bash-oracle dual-runs (seconds)
just test-aewatch        # FULL phase gate — bash-vs-python dual-run oracle (minutes)
```

The runner skips cleanly when `python3` is absent or older than 3.11. Tests build
isolated temp `AE_HOME` roots and never read or write the real `~/.ae`.

The watchdog port is validated by a **dual-run oracle**: each fixture drives the
REAL generated bash watchdog (via `ae doctor --refresh` under fakebin tmux/ae/date/
sleep shims) AND the Python `run_watchdog_cycle`, then diffs the ordered effect
streams. Those subprocess-backed runs are the slow part, so `AEWATCH_FAST=1`
(`just test-aewatch-fast`) skips them at one choke point — use it for the commit
inner loop. It is **not** the phase gate.

**Phase gate** (all must pass, and the dual-run must actually run):

```bash
just test-aewatch                                    # full dual-run oracle
python3 contrib/aewatch/aewatch contracts validate   # fixture matrix
just check                                            # shellcheck + shfmt
git diff --exit-code -- ae                            # zero-ae-edit invariant
```

A guard suite (`tests/aewatch_tests/test_22_phase_gate.py`) enforces the fast-lane
skip, that every bash-side effect kind is in `EFFECT_KINDS`, and that `ae` is
untouched.
