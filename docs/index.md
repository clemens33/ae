# ae — agentic engineering

**Run AI coding agents side-by-side in tmux. They know about each other, communicate by name, and survive reboots.**

`ae` is one public wrapper over an immutable versioned Rust core and the Bash pane glue. It turns tmux into a multi-agent workspace: pair Claude Code with Codex for cross-model review, spin up a reviewer on demand, and wake to a complete event log after a long task.

[![Release: 2026.8.2](https://img.shields.io/badge/release-2026.8.2-blue.svg)](https://github.com/clemens33/ae/releases)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/clemens33/ae/blob/main/LICENSE)
[![Bash](https://img.shields.io/badge/bash-%3E%3D4.0-green.svg)](https://www.gnu.org/software/bash/)

## Why ae

- **One command.** `ae` starts a session, `ae` reattaches. That's the whole workflow.
- **Agents talk to each other.** Each agent gets workspace context injected into its system prompt — starting with which agent it is. They send messages by name, spawn new agents, and coordinate without manual wiring.
- **Everything survives reboots.** Sessions, spawned agents, conversation history. Pick up exactly where you left off.
- **One window to your whole fleet.** The optional [`ae orchestrator`](reference/commands.md#ae-orchestrator) meta-agent — your fleet's chief of staff — watches every session and relays to them — talk to *it* from your phone over [Telegram](reference/telegram.md#orchestrator-centric-routing-talk-to-the-meta-agent-not-ten-sessions) instead of juggling ten panes.
- **Nothing touches your repo.** Session state lives in `~/.ae/sessions/`. Your working directory stays clean.
- **Small versioned runtime.** The public wrapper validates a matched Rust core and Bash pane glue from one immutable version directory.

> Install with the one-liner in [getting started](getting-started/install.md); it downloads the
> current release, verifies its checksum before extracting, and publishes one immutable
> version directory.

## Works with

- [Claude Code](https://docs.anthropic.com/en/docs/claude-code)
- [Codex](https://github.com/openai/codex)
- [Gemini CLI](https://github.com/google-gemini/gemini-cli)
- [Grok Build](https://github.com/xai-org/grok-build)
- [OpenCode](https://github.com/opencode-ai/opencode)
- Any CLI agent — point ae at the binary and it works

## What it looks like

```text
┌─ claude:lead ────────────────┬─ codex:coworker ──────────────┐
│ > /ultrareview               │ ✓ ran tests                   │
│                              │ 17 passed                     │
│ launching multi-agent...     │                               │
│                              │ /home/ckriech/.ae/sessions/   │
└──────────────────────────────┴──ae-mysession/send "claude...─┘
```

Two agents in own tmux windows, talking to each other through generated shell helpers. A hidden monitor window streams the event log and runs the optional watchdog that keeps everything healthy.

## Next

- [Install ae](getting-started/install.md)
- [Quick start](getting-started/quick-start.md)
- [Configure agents and layouts](getting-started/config.md)
- [Browse the command reference](reference/commands.md)
- [Telegram bridge + orchestrator-centric routing](reference/telegram.md)

### Doctrine

How this project is built, reviewed, and handed off:

- [Gatekeeping](gatekeeping.md) — how changes are reviewed and gated before they land
- [Design patterns](design-patterns.md) — the recurring patterns behind ae's design (including the agent-identity model)
- [Lead handover](lead-handover.md) — historical: how a lead agent carried a session across context, from before the Rust core owned `list`
