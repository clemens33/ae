# ae — agentic engineering

[![Release: 2026.11.127](https://img.shields.io/badge/release-2026.11.127-blue.svg)](https://github.com/clemens33/ae/releases)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![tmux](https://img.shields.io/badge/requires-tmux%20%3E%3D%203.4-1BB91F.svg)](https://github.com/tmux/tmux)
[![Platforms](https://img.shields.io/badge/platform-macOS%20arm64%20%7C%20Linux%20x86__64-lightgrey.svg)](#install)
[![Install: curl | bash](https://img.shields.io/badge/install-curl%20%7C%20bash-orange.svg)](#install)

**A fleet of AI coding agents in tmux that work as a team and tell you when they need you.**

**North star:** spend your attention on building something valuable and organising the work, not on picking models, watching quota, managing subscriptions or handling git, CI and deployment.
ae is meant to work with any harness and model provider, and to keep the work context across a switch; you keep intent, steering, priorities, product choices and review.
Further out: work beyond software, topics shared across people and machines, remote compute. A direction, not a promise ([vision](VISION.md#north-star)).

## What ae is

A thin coordination layer over [tmux](https://github.com/tmux/tmux) for several agent CLIs side by side: each agent knows its name and who else is there, reaches the others by name and says when it needs you. It is one Rust binary from a checksum-verified bundle; session state lives in `~/.ae/`, not in your repo.

## What it does

- **Many harnesses.** Claude Code, Codex, OpenCode, Gemini CLI, Antigravity (`agy`), Grok Build and Muse Code have adapters of [differing depth](docs/reference/contract.md#agent-tool-capabilities). Other CLIs run without most integrations.
- **Your choice of client, harness, model and effort.** A profile is a launch command with its flags; a client is a CLI install, or a second Claude Code or Codex login. `ae init` sets up the lead pair, the `spawn` helper adds agents (`spawn <name> --using <profile>`) and [`ae reseat`](docs/reference/commands.md#ae-reseat-session-agent---using-profileclient---stop-unknown) moves one to another profile.
- **Effort.** ae does not estimate a task's effort: leads are told to pick the cheapest [profile that fits](docs/reference/delegation.md). Opt-in [`auto_effort`](docs/getting-started/config.md#workspace) can restart an idle Claude Code or Codex agent (lead pair or spawned) one effort level lower when its quota runs high. After recovery it restores the profile's effort, unless you changed it. It needs a matching lower-effort profile and by default steps only from `max` or `xhigh`.
- **Quota.** [`ae quota`](docs/reference/commands.md#ae-quota) shows each configured account's locally cached quota windows, with `EFFECTIVE` headroom and `PACE`; coverage differs per harness. Leads are told to check it before delegating. Opt-in [`auto_reseat`](docs/internals/reseat.md) moves eligible idle agents off a usage limit, or before they reach one.
- **A team.** Every agent gets [helpers](docs/reference/helpers.md): `send`, tracked `ask` and `review` with an exact reply command, `spawn` and shared `memo` notes. Leads are told to get significant work reviewed by another model provider; ae does not enforce it.
- **Who needs you.** Agents declare their state. The status bar, `ae list --needs-attn`, `ae next` and `ae brief` show who waits for a human, and a watchdog nudges idle agents.
- **Terminal UI.** `ae app` (or plain `ae` in your terminal) opens [the app](docs/app.md): your sessions in a sidebar, plus the selected one's agents and lead chat. [`ae chat`](docs/chat.md) gives that chat as text.
- **Chief of staff.** [`ae orchestrator`](docs/reference/commands.md#the-orchestrator-seat) is one agent over the fleet. It relays your instructions, keeps a board of what waits for you, and creates, starts, stops, renames or ends sessions when you ask, [within limits](contrib/aeorchestrator/README.md#role).
- **Telegram.** An opt-in [bridge](docs/reference/telegram.md) brings the orchestrator's chat to your phone and pins its board. With `scope = all`, or with no proven orchestrator, it forwards every running session's chat. Users you allow can write back.
- **Web UI, coming.** Early today: [`ae web`](docs/web.md) serves the orchestrator's chat to a browser on the same machine, where you send messages, answer its decisions and follow topic tabs.
- **Memory.** Resume sessions after `ae stop` or a reboot (conversations too, where the harness supports it). By default, `ae end` [archives](docs/reference/commands.md#session-archives) goal, memos and events, and `ae <new-name> --from <uuid>` starts from that archive; `ae reboot` does both in one step.
- **And more.** `ae usage` (list-price cost of live sessions, offline), `ae board` (user turns from the fleet's transcripts, with known ae and harness turns filtered out) and [`ae upgrade`](docs/upgrade.md) (agents keep running). In progress, not active yet: dictation in the app.

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/clemens33/ae/main/install | bash
```

For macOS on Apple Silicon and Linux x86_64 (WSL2 too). You need tmux 3.4 or newer, git and an AI agent CLI you are logged in to; the installer also needs `curl`, `tar` and `sha256sum` or `shasum`. Put `~/.local/bin` on your `PATH`. [Install guide](docs/getting-started/install.md): pin a version, build from source, uninstall.

## First three commands

```bash
ae init                  # 1. find your agent CLIs and set up the lead and colead
ae my-app                # 2. in your project directory: start or reattach the session
ae list --needs-attn     # 3. from another shell: which running sessions need you
```

Agents keep running when you leave the app or detach from tmux. Ask your lead in plain words, for example *"Get a second agent to review the changes in src/"*. Run `ae doctor` if something looks off.

## Learn more

- Start: [quick start](docs/getting-started/quick-start.md) · [configuration](docs/getting-started/config.md) ([`config.sample`](config.sample)) · [troubleshooting](docs/troubleshooting.md)
- Reference: [commands](docs/reference/commands.md) · [session helpers](docs/reference/helpers.md) · [how it works](docs/internals/architecture.md)
- Contribute: [development](docs/development.md) (`just rust-setup`, then `just test`) · [AGENTS.md](AGENTS.md)

ae is [not](AGENTS.md#what-ae-is-not) a CI/CD pipeline, a billing or logging system, a git workflow tool or a plugin framework.

## License

MIT
