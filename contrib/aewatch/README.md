# aewatch (optional contrib sidecar)

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
just test-aewatch      # bash tests/aewatch → stdlib unittest suite (tests/aewatch_tests/)
```

The runner skips cleanly when `python3` is absent or older than 3.11. Tests build
isolated temp `AE_HOME` roots and never read or write the real `~/.ae`.
