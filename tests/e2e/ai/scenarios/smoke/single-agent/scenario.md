---
name: single-agent
timeout: 120
requires: claude
config: inline
---

# Scenario: a single real agent launches in an isolated workspace

## Purpose
The smallest end-to-end proof that the harness works: a REAL `claude` agent
starts inside a fully isolated ae workspace (real `$HOME` for auth, all ae state
under `AE_HOME`, private tmux server), and ae captures its session id. This is the
canary — if it fails, nothing deeper is worth debugging.

It also demonstrates the per-scenario config: the `ini` block below is written
verbatim to this scenario's isolated `CONFIG_FILE`. Change it and you change the
ae setup under test, nothing else.

## ae config
```ini
[profiles]
claude = "claude --permission-mode bypassPermissions --model opus[1m]"

[roster]
main = claude

[workspace]
main = main
copy = local
layout = vertical
```

## Steps
See `steps.sh`.

## Expect
Deterministic (the gate):
- the session's tmux session is alive;
- its `meta` records `seat.main=main` and `harness_session.main=<uuid>` with a real captured session
  id (not `pending`) — i.e. a real Claude process launched and ae hooked it.

Semantic (advisory judge): the agent answered a trivial prompt sensibly.
