# tests/e2e/ai

AI-driven end-to-end tests: real agent CLIs run as *subjects* inside real ae
sessions; a plain-bash driver asserts on ae's observable state (`events.jsonl`,
session liveness, meta). Read **[AGENTS.md](AGENTS.md)** for the philosophy,
when to use this over `tests/integration`, and the scenario format.

## Run

```bash
AE_E2E_AI=1 just test-ai                                       # all scenarios
AE_E2E_AI=1 tests/e2e/ai/run_scenario.sh scenarios/smoke/single-agent/
```

Opt-in: without `AE_E2E_AI=1` it **skips** (these run real agents against your real
subscription — real tokens, your live rate budget). Missing a scenario's
`requires:` tool also skips. Not part of `just check`.

## Prerequisites

- the agent CLI a scenario needs (`claude`, maybe `codex`), already logged in —
  the harness keeps your real `$HOME` so existing auth is used as-is;
- `tmux`, `git`, `timeout` (coreutils).

## Layout

```
run_scenario.sh   runner: parse scenario.md, isolate, run steps.sh under timeout
lib.sh            helpers: isolated env, ae_e2e wrapper, event assertions, judge
scenarios/<group>/<name>/
  scenario.md     metadata + purpose + literal ae-config + expectations
  steps.sh        the driver (a real, lintable bash file)
```

Artifacts for each run: `.local/e2e-ai/<run-id>/` (gitignored).
