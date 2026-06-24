# ae — agentic engineering

**Run AI coding agents side-by-side in tmux. They know about each other, communicate by name, and survive reboots.**

`ae` is a single bash script that turns tmux into a multi-agent workspace. Pair Claude Code with Codex for cross-model review. Spin up a reviewer agent on demand. Sleep through a long task and wake up to a complete event log. Zero dependencies beyond bash, tmux, and git.

[![Release](https://img.shields.io/badge/release-0.2.1-blue.svg)](https://github.com/clemens33/ae/releases)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/clemens33/ae/blob/main/LICENSE)
[![Bash](https://img.shields.io/badge/bash-%3E%3D4.0-green.svg)](https://www.gnu.org/software/bash/)

## Why ae

- **One command.** `ae` starts a session, `ae` reattaches. That's the whole workflow.
- **Agents talk to each other.** Each agent gets workspace context injected into its system prompt. They send messages by name, spawn new agents, and coordinate without manual wiring.
- **Everything survives reboots.** Sessions, spawned agents, conversation history. Pick up exactly where you left off.
- **One window to your whole fleet.** The optional [`ae hub`](reference/commands.md#ae-hub) meta-agent watches every session and relays to them — talk to *it* from your phone over [Telegram](reference/telegram.md#hub-centric-routing-talk-to-the-meta-agent-not-ten-sessions) instead of juggling ten panes.
- **Nothing touches your repo.** Session state lives in `~/.ae/sessions/`. Your working directory stays clean.
- **Single bash script.** No frameworks, no runtimes, no abstractions. Just bash, tmux, and git.

## Works with

- [Claude Code](https://docs.anthropic.com/en/docs/claude-code)
- [Codex](https://github.com/openai/codex)
- [Gemini CLI](https://github.com/google-gemini/gemini-cli)
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

Two agents in adjacent panes, talking to each other through generated shell helpers. A hidden monitor window streams the event log and runs the optional watchdog that keeps everything healthy.

## Next

- [Install ae](getting-started/install.md)
- [Quick start](getting-started/quick-start.md)
- [Configure agents and layouts](getting-started/config.md)
- [Browse the command reference](reference/commands.md)
- [Telegram bridge + hub-centric routing](reference/telegram.md)
