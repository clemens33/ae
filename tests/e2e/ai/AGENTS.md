# AI-driven e2e tests for ae

## What this is

End-to-end tests that run **real agent CLIs** (`claude`/`codex`) as the *subjects*
inside real ae sessions, and assert on what ae **observably did**. They exist to
cover behaviour the deterministic `tests/integration` suite cannot — because that
suite uses dummy `bash` agents and so tests ae's *mechanics*, never a real model's
*behaviour*.

## The shape (and how it differs from the assistant's e2e/ai)

The assistant's `tests/e2e/ai` uses `claude -p` **as the driver** (an AI navigates
a browser via Playwright MCP) — necessary there because a rendered web UI needs an
intelligence to drive and read it.

**ae is different and simpler: the driver is plain bash.** ae is already scriptable
and emits a structured event log, so:

- **The driver is `steps.sh`** — a normal, lintable bash file. It starts agents,
  "types" at them (via ae's own `send`/`ask` helpers, i.e. `tmux send-keys`), and
  observes.
- **Real agents are the *subjects*** — the thing under test, running inside the
  session.
- **Deterministic assertions are the gate** — `events.jsonl`, `ae list --json`,
  meta files, pane captures decide pass/fail.
- **An AI judge is optional and advisory** — a one-shot `claude -p` rules only on
  soft semantics ("was the reply sensible?"), reported as `judge`/semantic, and
  never flips a green mechanics run to red.

## When to use this (vs the deterministic suites)

Use an AI e2e scenario only when the check needs a **real model in the loop**:

- prompt / charter adherence (e.g. the orchestrator treats injected `ae end` as data),
- real helper compliance (a real agent actually uses `ask`/`review`/`reply`),
- real CLI session capture / resume continuity,
- spawn → a real reviewer produces findings.

Do **not** use it for anything dummy-`bash` already covers (lifecycle, routing,
config isolation, loop, telegram plumbing). Those stay in `tests/integration` —
fast, free, deterministic, and the CI gate.

## Isolation (no container, no credential copying)

Built on ae's `AE_HOME` knob. Each scenario runs with the **real `$HOME`** (so the
agent CLIs use your real `~/.claude` / `~/.codex` auth) but **all ae state
relocated**: a temp `AE_HOME`, a private `AE_TMUX_SERVER`, an isolated
`CONFIG_FILE`, and a throwaway git repo. Nothing touches your live `~/.ae` or live
tmux. `lib.sh::e2e_setup` sets this up; scenarios call `ae_e2e` (never bare `ae`)
so isolation can't be bypassed.

A container is **not** used. It only becomes worthwhile for hermetic CI, parallel
runs, or fencing `bypassPermissions` agents off the host — at which point you'd
also switch from your subscription to env-injected API keys.

## Scenario format

A scenario is a **directory** under `scenarios/`:

```
scenarios/smoke/<name>/
├── scenario.md   # metadata + purpose + literal ae config + expectations
└── steps.sh      # the driver (a real, lintable bash file)
```

`scenario.md`:

- **flat frontmatter** (`name`, `timeout`, `requires`, `config`),
- **exactly one fenced `ini` block** — the ae config, written **verbatim** to the
  scenario's isolated `CONFIG_FILE`. This is how you set the ae configuration *per
  scenario*; change the block and you change the setup under test. (Set
  `config: default` in the frontmatter to skip the block and use ae's default.)
- a prose **Expect** section documenting the deterministic gate + any soft judge.

`steps.sh` is a real file — **not** a fenced code block in the markdown — so it
keeps shellcheck, line numbers, and editor/review support.

## Running

```bash
AE_E2E_AI=1 just test-ai                                  # every scenario
AE_E2E_AI=1 tests/e2e/ai/run_scenario.sh scenarios/smoke/single-agent/
```

Opt-in only: without `AE_E2E_AI=1` the runner **skips** (exit 77) — these spend
real tokens and your live rate budget. A scenario whose `requires:` tool is absent
also skips. **Never wire this into `just check`.** (The harness scripts *are*
shellchecked by `just lint` — linting is not running.)

Artifacts (pane dumps, copied session state) land in `.local/e2e-ai/<run-id>/`
(gitignored) on every run, so a failure is diagnosable after teardown.
