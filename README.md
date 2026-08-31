# ae - agentic engineering

[![Release](https://img.shields.io/badge/release-0.2.1-blue.svg)](https://github.com/clemens33/ae/releases)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Bash](https://img.shields.io/badge/bash-%3E%3D4.0-green.svg)](https://www.gnu.org/software/bash/)
[![tmux](https://img.shields.io/badge/requires-tmux-1BB91F.svg)](https://github.com/tmux/tmux)
[![Install](https://img.shields.io/badge/install-curl%20%7C%20bash-orange.svg)](#install)

**ae** runs AI coding agents side-by-side in tmux. They know about each other, communicate by name, and survive reboots. One bash script, zero dependencies.

Works with [Claude Code](https://docs.anthropic.com/en/docs/claude-code), [Codex](https://github.com/openai/codex), [Gemini CLI](https://github.com/google-gemini/gemini-cli), [Grok Build](https://github.com/xai-org/grok-build), [OpenCode](https://github.com/opencode-ai/opencode), or any CLI tool.

> 🦀 **Rewrite in progress** — on branch `rust-rewrite` ([epic #79](https://github.com/clemens33/ae/issues/79)), bash `ae` is **frozen at `72c7293`** and shrinks to tmux/pane glue, while the state core, lifecycle and daemons move into a single Rust binary. The install stays one file. Everything in this README describes shipped bash behavior and holds until each domain flips. Direction, reasons and phases: **[VISION.md](VISION.md)**.

> 📖 **[Full documentation](docs/index.md)** — guides, command and helper reference, and internals. This README is the quick tour.

## Why ae

- **One command** -- `ae` starts a session, `ae` reattaches. That's the whole workflow.
- **Agents talk to each other** -- each agent gets workspace context injected into its system prompt. They send messages by name, spawn new agents, and coordinate without manual wiring.
- **Everything survives reboots** -- sessions, spawned agents, conversation history. Pick up exactly where you left off.
- **Nothing touches your repo** -- session state lives in `~/.ae/sessions/`. Your working directory stays clean.
- **Tiered delegation** -- leads run the strongest model; bounded chores go to cheap spawned workers in their own tmux windows, reviewed and retired. Convention, not machinery ([docs](docs/reference/delegation.md)).
- **A status bar that answers "who needs me"** -- inside its sessions ae owns the tmux footer: session, git branch, and watchdog health on line one; the focused agent plus a whole-org roster showing the watchdog's verdict per agent (`lead● colead✔ builder◌ grok⚡`) on line two. Verdicts, never claims.
- **A chief of staff on your phone** -- the optional orchestrator session watches your whole fleet and reports over Telegram; tell it your objective and it helps you hold it.
- **Single bash script** -- no frameworks, no runtimes, no abstractions. Just bash, tmux, and git. Optional companions live in `contrib/` and are never required.

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/clemens33/ae/main/install | bash
```

Or clone manually:

```bash
git clone https://github.com/clemens33/ae.git ~/.local/share/ae
~/.local/share/ae/install
```

Both methods symlink `ae` to `~/.local/bin/ae`. Make sure `~/.local/bin` is on your `PATH`, then run `ae doctor` to check your environment.

### Install the prebuilt ae-next rewrite

The rewrite is distributed as a verified prebuilt pair beside the frozen `ae`:

```bash
curl -fsSL https://raw.githubusercontent.com/clemens33/ae/rust-rewrite/contrib/ae-next/install-remote | bash
```

The installer downloads the platform bundle and `SHA256SUMS` to temporary files,
verifies the bundle before extraction, then atomically publishes an immutable
versioned install under `~/.ae-next`. Set `AE_VERSION=2026.8.1` to pin a release.

| Platform | Bundle | Status |
|---|---|---|
| macOS Apple Silicon | `darwin-arm64` | supported |
| Linux x86_64 (including WSL2) | `linux-x86_64-musl` | supported |
| macOS Intel | `darwin-x86_64` | rejected |
| Linux ARM | `linux-arm64` | rejected |
| Windows / MSYS | — | rejected |

## Quick start

```bash
cd ~/projects/my-app
ae
```

First run creates `~/.ae/config` with sensible defaults and launches your main agent in tmux. Detach with `Ctrl+b d` -- agents keep running in the background.

## What you can do

**Start a session and let agents collaborate:**
```bash
ae my-feature                  # named session
ae                             # default session (named after the directory)
```

**Ask your agent to bring in help:**

Just tell it what you need -- it knows how to spawn others and coordinate. *"Get a second agent to review the changes in src/"* or *"Spin up a pair programmer to help refactor auth."* Agents pick descriptive names, show up in their own tmux windows, and talk to each other directly.

**Come back after a reboot:**
```bash
ae my-feature                  # every agent resumes with its conversation history
```

**Check on agents without attaching:**
```bash
ae status my-feature           # recent output from all agents
ae list                        # running sessions: goal, git branch, per-agent state + attn marker
ae list --needs-attn           # only sessions needing attention (alias: --attn)
ae list --all                  # include stopped sessions
ae list --json                 # machine-readable digest (for scripts/agents)
ae next                        # name the top session needing attention (--attach jumps to it)
watch -n 10 'ae list'          # live dashboard
```

**Finish up:**
```bash
ae stop my-feature             # pause, keep state — resume later with 'ae my-feature'
ae end my-feature              # commit + push to ae/my-feature, archive the session's
                               #   memory to ~/.ae/archive/<uuid>/, remove ae state
                               #   (keeps conversation files; --purge-history deletes them
                               #    and writes no archive)
ae transfer my-feature vm.host # move a stopped session + conversations to another machine
```

**Continue where a finished session left off:**
```bash
ae archive preview my-feature  # what an end would keep (read-only, writes nothing)
ae my-feature-2 --from <uuid>  # a NEW session that explicitly continues an archived one
ae compact my-feature          # archive it and start fresh under the SAME name, continuing
                               #   from that archive (local mode; --digest-only skips the ask)
```

Ending a session used to be the moment everything it knew stopped existing. An archive is
that memory kept — goal, memo, event log, request payloads — as an inert, immutable,
UUID-keyed snapshot with no executable file in it. `--from` is the only way to inherit
one: lineage is explicit, never inferred from a name that happens to match.

`ae compact` is the move you actually want when an agent runs out of context: it asks the
main agent for a handover, archives, ends, and relaunches the same name from that archive
— one command instead of three, the middle one of which is you transcribing a UUID after
the session is already gone.

Full command reference: **[docs/reference/commands.md](docs/reference/commands.md)**.

## Session helpers

Inside a session, agents and humans share generated helper scripts in `~/.ae/sessions/<name>/` -- the wiring agents use to collaborate:

```bash
send <agent> <message>         # message another agent
ask <agent> <question>         # tracked request with a request id + exact reply command
review <agent> <request>       # critical review, findings-first reply contract
state <value> [reason]         # declare work state: working|waiting-user|blocked|done
say <text>                     # push a line to the human's Telegram chat (reply routes back)
memo add [--topic t] <text>    # durable shared session memory (survives restarts)
spawn <alias:name> [prompt]    # add an agent · retire <agent> removes one
peek <agent> [lines]           # view an agent's recent output (inspection only)
```

Agent names resolve flexibly -- `codex:reviewer` (exact), `codex` (alias), `reviewer` (bare name), `%42` (pane id) -- and `@session:agent` reaches across sessions:

```bash
send @other-feature:claude:lead "check my API changes"
agents --all                   # discover agents across every ae session
```

Agents call these automatically when you ask them to collaborate. Full helper catalog: **[docs/reference/helpers.md](docs/reference/helpers.md)**.

## Config

`~/.ae/config` is auto-created on first run; per-project overrides go in `.ae/config`.

```toml
[agents]
claude = "claude --permission-mode bypassPermissions --model opus"
codex = "codex --yolo -m gpt-5.6-sol -c model_reasoning_effort=high"
grok = "grok --always-approve -m grok-4.6 --effort high"
agy = "agy --dangerously-skip-permissions"   # any CLI works — agy has no special ae integration

[workspace]
main = claude:lead
workers = codex:reviewer
layout = vertical

[prompt]
instructions = "Always write tests. Prefer TypeScript."
```

Register any CLI tool under `[agents]`; `[workspace]` sets the layout; `[prompt]` injects custom instructions into every agent's system prompt. One binary can serve several logins — e.g. `CLAUDE_CONFIG_DIR=$HOME/.claude-work claude ...` as its own alias gives a seat its own subscription, login, and history ([details and caveats](docs/getting-started/config.md#multiple-identities-of-one-cli)). The default ae writes on first run -- mirrored in the repo as [`config.sample`](config.sample) -- is a documented lead-pair setup. Choose how agents see your code with a working-directory mode:

| Mode | Flag | What it does |
|------|------|------|
| `local` | *(default)* | Agents work directly in your project directory. Simple and fast. |
| `full` | `--copy` | Full copy of the project — an isolated workspace for complex features. |
| `worktree` | `--worktree` | Git worktree — lightweight branch isolation backed by git. |

Full lineup, role guidance, and every key: **[docs/getting-started/config.md](docs/getting-started/config.md)**.

## How it works

Each agent gets workspace context injected into its system prompt (Claude Code's `--append-system-prompt`, Codex's `developer_instructions`, Gemini's `-i`). That context tells it **which agent it is** (`You are agent <alias>:<name> (slot <slot>)`), who the other agents are, how to reach them by name, and how to spawn or retire agents. The communication itself happens through shell helpers (`send`, `peek`, `spawn`, …) that ae generates in `~/.ae/sessions/` -- agents call them like any other CLI tool.

No custom protocols, no frameworks. Just system prompts and bash scripts agents already know how to use.

## Still one bash script

The core contract has not moved: `ae` is a single bash script whose only hard dependencies are **bash >= 4.0, tmux, and git**. Your repos stay untouched -- session state lives in `~/.ae/sessions/`, and `--copy`/`--worktree` give agents isolated workspaces when you want them.

Everything else is **optional**, never required for core commands:

| Feature | What | Needs |
|---|---|---|
| `ae telegram` | machine-global bridge: fleet events to your Telegram chat, replies route back | a configured ae core (no extra CLI deps) |
| `ae orchestrator` ([contrib/aeorchestrator](contrib/aeorchestrator)) | the fleet's chief of staff: monitors every session, relays and reports, guards an objective once you set one | an agent CLI |
| [contrib/aemonitor](contrib/aemonitor) | deterministic sweep helper the orchestrator uses | Python 3 stdlib |

The long-running daemon half (watchdog, bridge) is where bash hurts most, so it is being carved into the typed Rust core -- per the revisit triggers in [AGENTS.md](AGENTS.md). The Telegram bridge is the first daemon fully in the core: it *is* the ae core binary, run in a background tmux session, with no `jq`/`curl` dependency. (An earlier stdlib-only Python sidecar, [contrib/aewatch](contrib/aewatch), prototyped this carve-out under byte-exact parity testing; it is retired now that the core owns the bridge.) The tmux-glue half, where bash is best-in-class, stays bash. Companions start automatically once you opt in; `AE_NO_AUTOSTART=1` skips.

Trying the rewrite is not supposed to cost you the ae you work with, so it installs as a
**second command**: `just next-install` puts `ae-next` beside `ae`, with its own state home
(`~/.ae-next`), its own tmux server and its own immutable copy of the core. Nothing under
`~/.ae` or at `~/.local/bin/ae` changes, and rollback is deleting the three things it made.
For a clean machine, use the [prebuilt one-line installer](#install-the-prebuilt-ae-next-rewrite);
`just next-install` remains the checkout-local development install. See
[docs/migration/coexistence.md](docs/migration/coexistence.md); it retires at the entry flip.

That carve-out is now the whole plan: this bash script is **frozen at `72c7293`** and the typed core takes over domain by domain on `rust-rewrite`. What carries over unchanged is the part you depend on — one file to install, no runtime, no repo pollution, `~/.ae/` as the only state. What goes away is bash as the implementation. See **[VISION.md](VISION.md)**.

## Requirements

- **bash >= 4.0** (macOS ships 3.2 — `brew install bash` and put brew's bin dir ahead of `/bin` on `PATH`)
- [tmux](https://github.com/tmux/tmux) and [git](https://git-scm.com/)
- At least one AI coding agent CLI (see *Works with*, above)

Linux (GNU userland) and macOS (BSD userland) are both first-class: ae routes
every divergent coreutil through a portability shim, so no GNU *coreutils*
package is required. Two extra binaries are worth installing on macOS
(`brew install flock coreutils`):

- **`flock`** — `ae list`, `ae <name>` and friends work without it, but the
  generated session helpers (`send`, `ask`, `state`, `goal`, the event log) lock
  unguarded, so a missing `flock` degrades them: `state`/`goal` writes fail, and
  a `send` can deliver its message and *then* report failure, inviting a
  duplicate retry. Install it if you use multi-agent messaging.
- **`timeout`** — bounds the watchdog's git probes; without it a wedged git
  stalls a status refresh.

`ae doctor` reports both, plus which userland it detected.

`ae` runs under bash, but your interactive shell can be anything (fish, zsh, …). Run **`ae doctor`** after installing ae or upgrading any agent CLI -- it checks bash/tmux/git, your config, and whether configured agents are on `PATH`. Resume/session capture for external CLIs is best-effort and depends on each tool's upstream behavior; `ae doctor` is your first stop when something looks off. More: **[docs/troubleshooting.md](docs/troubleshooting.md)**.

## Development

Requires [just](https://github.com/casey/just):

```bash
just check            # lint (shellcheck) + format check (shfmt)
just test             # unit + integration tests
just release          # check → test → CalVer bump → changelog → tag → gh release
```

The Rust lanes live beside them on `rust-rewrite`, prefixed `rust-` and touching nothing the bash recipes own. Prerequisites are honest: [rustup](https://rustup.rs) and just, nothing else.

```bash
just rust-setup       # pinned toolchain + dev tools (idempotent; installs nothing on a second run)
just rust-check       # fmt check + clippy (-D warnings) + nextest + doctests
just rust-deny        # supply chain: advisories, licenses, bans, sources
just rust-mutants     # do the tests discriminate, or do they merely pass?
```

Conventions, test layout, and the release pipeline: **[docs/development.md](docs/development.md)**. The Rust toolchain contract — exact pins, lint policy, the no-`unwrap` rule — is in **[AGENTS.md](AGENTS.md#rust-era-rust-rewrite-branch)**.

## License

MIT
