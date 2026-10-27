# ae — agentic engineering

[![Release: 2026.10.141](https://img.shields.io/badge/release-2026.10.141-blue.svg)](https://github.com/clemens33/ae/releases)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![tmux](https://img.shields.io/badge/requires-tmux%20%3E%3D%203.4-1BB91F.svg)](https://github.com/tmux/tmux)
[![Platforms](https://img.shields.io/badge/platform-macOS%20arm64%20%7C%20Linux%20x86__64-lightgrey.svg)](#install)
[![Install: curl | bash](https://img.shields.io/badge/install-curl%20%7C%20bash-orange.svg)](#install)

**A fleet of AI coding agents in tmux. They message each other by name, flag when they need you, and survive reboots.**

**ae** is a thin coordination layer over [tmux](https://github.com/tmux/tmux) for multi-agent work. Run Claude Code, Codex, OpenCode and other agent CLIs side by side, and ae makes them a fleet instead of a pile of tabs: each agent knows who it is and who else is in the workspace, agents reach each other by name, and the session shows you who is waiting for a human.

Works with Claude Code, Codex, OpenCode and other agent CLIs ([supported list](#requirements)). Any CLI agent runs; resume is best-effort for the ones ae has not modelled.

## Why ae

- **A team, not a pile of tabs.** Every agent is told its name and the workspace roster. Agents `send` messages, `ask` tracked questions (request id plus an exact reply command) and `review` each other's work, across sessions too.
- **You see who needs you.** Agents declare `working`, `waiting-user`, `waiting-agent`, `blocked` or `done`. The tmux status bar, `ae list --needs-attn`, `ae next` and the `ae app` terminal UI show who is waiting for a human; Telegram reaches you away from the desk.
- **Pick up where you left off.** Sessions, spawned agents, conversation history and shared `memo` notes survive stops and reboots. `ae reboot` hands a context-heavy session to a fresh one from an archive.
- **Mix vendors.** Pair one model's work with another's review; lead on the strongest model and send bounded chores to cheaper spawned workers ([delegation](docs/reference/delegation.md)).
- **Small and out of the way.** tmux keeps panes, scrollback and attach. Session state lives in `~/.ae/`, never in your repo. One checksum-verified Rust binary; no runtime or service to keep alive.

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/clemens33/ae/main/install | bash
```

The installer verifies the platform bundle (`ae-core`, `install`, `SHA256SUMS`) against the release manifest before extraction, publishes it read-only under `~/.ae/versions/<V>/` and points `~/.local/bin/ae` straight at that version's `ae-core` — one symlink, no wrapper. Make sure `~/.local/bin` is on your `PATH`, then run `ae init`. Set `AE_VERSION=2026.8.2` to pin a release. Installed ae checks for newer releases quietly; global `[workspace] auto_upgrade = off` disables that ([upgrade](docs/upgrade.md)).

Supported: macOS Apple Silicon (`darwin-arm64`) and Linux x86_64 including WSL2 (`linux-x86_64-musl`). Intel macOS, Linux ARM and Windows/MSYS are rejected.

### Build from source

```bash
git clone https://github.com/clemens33/ae.git ~/.local/share/ae
cd ~/.local/share/ae
just install
```

Prerequisites: [rustup](https://rustup.rs/) and [just](https://github.com/casey/just) installed; then run `just rust-setup` once to provision the pinned toolchain. This runs the same canonical installer and compiles a native binary for this machine.

## Quick start

```bash
cd ~/projects/my-app
ae init
ae my-app
```

`ae init` discovers supported harnesses on `PATH` without running them, then proposes the lead, colead and orchestrator profiles (`ae init --yes` in a non-interactive shell). Session names are explicit; a directory never silently decides one. Detach with `Ctrl+b d` -- agents keep running in the background.

## What you can do

**Start, reattach, come back after a reboot:**
```bash
ae my-feature                  # start or reattach a named session; after a reboot every agent resumes with its history
ae my-feature --solo           # lead only, no colead
ae                             # open the app on every recorded session; ae --tmux attaches tmux instead
```

Outside tmux, bare `ae` first resumes the sessions a crashed tmux server held, then opens the app on a terminal, else attaches ([details](docs/reference/commands.md#tmux-server)).

**Ask your agent to bring in help:** just tell it what you need -- it knows how to spawn others and coordinate. *"Get a second agent to review the changes in src/"* or *"Spin up a pair programmer to help refactor auth."* Agents pick descriptive names, show up in their own tmux windows, and talk to each other directly.

**Check on agents without attaching:**
```bash
ae app [session]               # the fleet as a terminal UI beside the selected chat
ae chat [session] [--follow]
                               # the human lane: lead-pair turns, bridge asks, say lines, cards,
                               # every seat that needs you; /open <seat> shows its pane
ae list                        # running sessions: goal, git branch, per-agent state + attn marker
                               # (--needs-attn, --all, --json; `watch -n 10 'ae list'` for a live board)
ae next                        # name the top session needing attention (--attach jumps to it)
ae brief [name] [--all]        # why a session needs you: goal, latest note per memo topic,
                               # each agent's declared state, and every open ask
ae orchestrator                # start or reattach the local orchestrator seat
```

The orchestrator is one optional overview seat for the whole fleet: the watchdog pastes a changed `NEEDS YOU` / `WORKING` / `QUIET` view into it, and it relays only explicit human instructions ([details](docs/reference/commands.md#ae-orchestrator)). On an ae-owned server, `prefix a` opens the fleet picker.

**Finish up, and continue where a finished session left off:**
```bash
ae stop my-feature             # pause, keep state — resume later with 'ae my-feature'
ae end my-feature              # commit + push to ae/my-feature, archive the session's memory to
                               #   ~/.ae/archive/<uuid>/, remove ae state (keeps conversation
                               #   files; --purge-history deletes them and writes no archive)
ae archive preview my-feature  # what an end would keep (read-only, writes nothing)
ae my-feature-2 --from <uuid>  # a NEW session that explicitly continues an archived one
ae reboot my-feature           # archive it and start fresh under the SAME name, continuing
                               #   from that archive (local mode; --digest-only skips the ask)
```

An archive is a session's memory kept — goal, memo, event log, request payloads — as an inert, immutable, UUID-keyed snapshot. `--from` is the only way to inherit one: lineage is explicit, never inferred from a name. `ae reboot` is the move for an agent that runs out of context: handover, archive, end and relaunch under the same name in one command. (`ae compact` is the in-place seat compaction.)

Full command reference: **[docs/reference/commands.md](docs/reference/commands.md)**.

## Session helpers

Inside a session, agents and humans share helpers in `~/.ae/sessions/<name>/` -- the wiring agents use to collaborate. Each is a symlink to the ae core binary. Call one by its full path or by the short form `ae @<session> <helper> …`; they are deliberately not on `PATH`, and a bare name refuses ([the two spellings](docs/reference/helpers.md#two-spellings-one-helper)):

```bash
send <agent> <message>         # message another agent
ask <agent> <question>         # tracked request with a request id + exact reply command
review <agent> <request>       # critical review, findings-first reply contract
state <value> [reason]         # declare work state: working|waiting-user|waiting-agent|blocked|done
say <text>                     # a lead-pair seat's line to the human's Telegram chat (reply routes back)
memo add [--topic t] <text>    # durable shared session memory (survives restarts)
spawn <name> --using <profile> [prompt]   # add an agent · retire <agent> removes one
peek <agent> [lines]           # view an agent's recent output (inspection only)
```

Agent names resolve exactly -- `reviewer` (the name), `%42` (pane id) -- and `session:agent` or `@session:agent` reaches across sessions (`send @other-feature:lead "check my API changes"`; `agents --all` discovers agents across every ae session). Agents call these automatically when you ask them to collaborate. Full helper catalog: **[docs/reference/helpers.md](docs/reference/helpers.md)**.

## Config

`~/.ae/config` is auto-created on first run; per-project overrides go in `.ae/config`.

```toml
[clients]
claude = claude
codex = codex

[profiles]
claude = "claude --permission-mode bypassPermissions --model opus"
codex = "codex --yolo -m gpt-5.6-sol -c model_reasoning_effort=high"
agy = "agy --dangerously-skip-permissions"   # Antigravity CLI; any other CLI can be a profile too

[roster]
lead = claude
reviewer = codex
orchestrator = codex

[workspace]
main = lead
workers = reviewer
layout = vertical
fleet_order = aedev, thinking, infra

[prompt]
instructions = "Always write tests. Prefer TypeScript."
```

The NAME in `[roster]` is the agent's identity -- it's what you address in `send`/`spawn` and what shows in pane titles and `ae list`; the profile is just metadata. `[clients]` names executable/config-home instances, so `cc-mic = claude config_home=$HOME/.claude-mic` gives profiles a second login and conversation store ([recipe](docs/getting-started/config.md#multiple-identities-of-one-cli)); `[profiles]` adds launch flags, `[roster]` binds names to profiles, `[workspace]` sets the layout, and `[prompt]` injects custom instructions. The default ae writes on first run is mirrored as [`config.sample`](config.sample). Choose how agents see your code with a working-directory mode: `local` (default, agents work directly in your project directory), `--copy` (a full copy, an isolated workspace for complex features) or `--worktree` (a git worktree, lightweight branch isolation).

Full lineup, role guidance, and every key: **[docs/getting-started/config.md](docs/getting-started/config.md)**.

## How it works

Each agent gets workspace context through its harness's supported launch channel (Claude Code's `--append-system-prompt`, Codex's `developer_instructions`, Gemini's `-i`, and the positional prompt for Grok Build and Muse Code). That context tells it **which agent it is** (`You are agent <name> (slot <slot>)`), who the other agents are, how to reach them by name, and how to spawn or retire agents. The communication itself happens through helpers (`send`, `peek`, `spawn`, …) that ae publishes in `~/.ae/sessions/<name>/` -- each one a link to the ae binary, dispatched by the name it is called by -- and agents call them like any other CLI tool.

No custom protocols, no frameworks. Just harness-native context and commands agents already know how to run. `ae` IS the versioned Rust core: `~/.local/bin/ae` is a symlink straight to `~/.ae/versions/<V>/ae-core`, with no wrapper between them. See **[VISION.md](VISION.md)**.

Everything else is **optional**, never required for core commands. `ae telegram` is a machine-global bridge from each running session's shared chat to your Telegram chat, labelled by session; a phone message is asked in the selected session's shared chat (`/use <session>`, `@session`, or a reply), and it needs no extra CLI deps. The orchestrator seat ([contrib/aeorchestrator](contrib/aeorchestrator)) is a dedicated single-seat local session named `orchestrator` that receives watchdog-rendered changed fleet overviews and relays only explicit human instructions through an audited bare-text helper; it needs an agent CLI, and ae seeds its config.

Autostart is per component: `watchdog = false` in workspace config disables the workspace watchdog, `enabled = false` in Telegram config disables Telegram, global `auto_upgrade = off` disables automatic releases, and `AE_NO_AUTOSTART=1` suppresses all three.

### Upgrade

`ae upgrade` downloads the latest release (or an `AE_VERSION` pin), verifies the checksum before extraction, then hands publication to the downloaded core. It publishes the new version read-only under `~/.ae/versions/<V>/`; migrates and repoints every placeable session's core record and helpers; reports and skips stopped unplaceable sessions; updates running watchdogs and Telegram bridges; and only then atomically moves `~/.local/bin/ae` to the new core. Existing agent harnesses are never restarted. See [docs/upgrade.md](docs/upgrade.md).

## Requirements

- [tmux](https://github.com/tmux/tmux) **3.4 or newer** and [git](https://git-scm.com/). Ubuntu 24.04 and Homebrew both package a tmux that clears the floor.
- `curl`, `tar` and `sha256sum` or `shasum`, for the installer. Any bash runs it, including macOS's stock 3.2.
- At least one AI coding agent CLI on `PATH`. Supported first-class: Claude Code, Codex, OpenCode, Gemini CLI, Antigravity (`agy`), Grok Build and Muse Code. Any other CLI runs as a profile too.
- [just](https://github.com/casey/just) and [rustup](https://rustup.rs) only to build from source (`just install`).

Linux and macOS are both first-class, and no GNU *coreutils* package is required ([why](docs/troubleshooting.md)). Your interactive shell can be anything (fish, zsh, …). Run **`ae doctor`** after installing ae or upgrading any agent CLI -- it checks tmux/git, your config, the userland it detected, and whether configured agents are on `PATH`. Resume/session capture for external CLIs is best-effort and depends on each tool's upstream behavior; `ae doctor` is your first stop when something looks off. More: **[docs/troubleshooting.md](docs/troubleshooting.md)**.

## Development

Requires [just](https://github.com/casey/just) and [rustup](https://rustup.rs), nothing else:

```bash
just rust-setup       # pinned toolchain + dev tools (idempotent; installs nothing on a second run)
just test             # the gate: format, lint, compile every fuzz target, tests (= just rust-check)
just check            # lint (shellcheck) + format check (shfmt) for the installer
just release          # check → test → rust-deny → CalVer bump → changelog → tag → gh release
```

`just rust-deny` audits supply chain on every release; `just rust-mutants` asks whether the tests discriminate. Lanes and conventions: **[docs/development.md](docs/development.md)**. The Rust toolchain contract — exact pins, lint policy, the no-`unwrap` rule — is in **[AGENTS.md](AGENTS.md#toolchain-pins)**.

## License

MIT
