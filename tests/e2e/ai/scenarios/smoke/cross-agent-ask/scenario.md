---
name: cross-agent-ask
timeout: 300
requires: claude
config: inline
---

# Scenario: real agents talk via the ae helpers (tracked request → reply)

## Purpose
Proves real helper-compliance that the dummy-`bash` integration tests cannot: two
REAL agents in one session, where a tracked `ask` to one produces a real `reply`
event back. This exercises the workspace-context injection + the request/reply
protocol with an actual model in the loop.

Config-wise it shows a DIFFERENT per-scenario setup from `single-agent`: a `main`
plus a `workers` agent, both written verbatim from the block below into this
scenario's isolated `CONFIG_FILE`. (Two `claude` agents are used so the scenario
needs only the `claude` CLI; swap the worker to `codex:coworker` to test cross-CLI.)

## ae config
```ini
[agents]
claude = "claude --permission-mode bypassPermissions --model opus[1m]"

[workspace]
main = claude:lead
workers = claude:reviewer
layout = vertical
```

## Steps
See `steps.sh`. The driver (simulating you) sends a tracked `ask` from outside to
the `reviewer` agent and waits for the protocol to round-trip.

## Expect
Deterministic (the gate):
- both `lead` and `reviewer` agents are present in the session meta;
- after the `ask`, a `reply` event addressed back to the asker appears in the
  session's events.jsonl within the timeout.

Semantic (advisory judge): the reply actually answers the question asked.
