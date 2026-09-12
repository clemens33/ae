# ae - agentic engineering

[![Release: 2026.9.60](https://img.shields.io/badge/release-2026.9.60-blue.svg)](https://github.com/clemens33/ae/releases)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Bash](https://img.shields.io/badge/bash-%3E%3D4.0-green.svg)](https://www.gnu.org/software/bash/)
[![tmux](https://img.shields.io/badge/requires-tmux-1BB91F.svg)](https://github.com/tmux/tmux)
[![Install: curl | bash](https://img.shields.io/badge/install-curl%20%7C%20bash-orange.svg)](#install)

**ae** runs AI coding agents side-by-side in tmux. They know about each other, communicate by name, and survive reboots. One public command — a symlink straight to a versioned Rust core, nothing between them.

Works with any CLI-based agentic harness.

## Why ae

- **One command** -- `ae <name>` starts or reattaches a session; bare `ae` attaches to the
  fleet server's most recently used session.
- **Agents talk to each other** -- each agent gets workspace context injected into its system prompt. They send messages by name, spawn new agents, and coordinate without manual wiring.
- **Everything survives reboots** -- sessions, spawned agents, conversation history. Pick up exactly where you left off.
- **Nothing touches your repo** -- session state lives in `~/.ae/sessions/`. Your working directory stays clean.
- **Tiered delegation** -- leads run the strongest model; bounded chores go to cheap spawned workers in their own tmux windows, reviewed and retired. Convention, not machinery ([docs](docs/reference/delegation.md)).
- **A status bar that answers "who needs me"** -- inside its sessions ae draws the tmux footer: this session, its windows and mark-first agents (`●lead ✓colead ◌builder ⚠grok`), its branch and goal on line one; every ae session on the server, most actionable first and each one clickable, on line two. Verdicts, never claims, and `[workspace] theme = off` gives your own status line back.
- **A fleet overview in one pane** -- the watchdog pastes a changed `NEEDS YOU` / `WORKING` / `QUIET` overview into the optional orchestrator seat; explicit instructions relay as if you typed them in the target pane, and named sessions can be started or created with explicit `--local`, `--copy`, or `--worktree` modes, then stopped or ended with history kept by default under its charter.
- **Session creation follows explicit facts** -- when the human names the session and a directory that exists at the spelled path (including `~` or relative paths expanded), the orchestrator runs immediately in default local mode; confirm only when a fact is inferred or missing (directory missing or nonexistent, resolution lands elsewhere such as a symlink, or mode unclear), then propose `name=<n> dir=<canonical path> mode=local|copy|worktree` and wait for `yes` or an edit before using exactly one mode command.
- **Small public surface** -- one command, and an optional orchestrator seat in `contrib/` that is never required.

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/clemens33/ae/main/install | bash
```

The installer downloads the platform bundle (`ae-core`, `install`, `SHA256SUMS`) and
verifies it against the manifest before extraction, then publishes it read-only under
`~/.ae/versions/<V>/` and points `~/.local/bin/ae` straight at that version's `ae-core` —
one symlink, no separate wrapper or pointer file to keep in sync. Switching versions later
is one atomic rename of that symlink. Make sure `~/.local/bin` is on your `PATH`, then run
`ae init` to discover installed agent CLIs and write the starting config. Set
`AE_VERSION=2026.8.2` to pin a release.

Installed ae checks for strictly newer releases during ordinary use and applies
them quietly in a detached process. Set global `[workspace] auto_upgrade = off`
to disable this machine-wide; absence means `on`. Project-local config cannot
override software-update policy. `ae version` and `ae doctor` show the policy
and last result without starting a check.

| Platform | Bundle | Status |
|---|---|---|
| macOS Apple Silicon | `darwin-arm64` | supported |
| Linux x86_64 (including WSL2) | `linux-x86_64-musl` | supported |
| macOS Intel | `darwin-x86_64` | rejected |
| Linux ARM | `linux-arm64` | rejected |
| Windows / MSYS | — | rejected |

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

`ae init` discovers supported harnesses on `PATH` without running them, then proposes the lead,
colead and orchestrator profiles. Use `ae init --yes` in a non-interactive shell. A session launch
still creates the broad default config when init has not been run.
Session names are explicit; a directory never silently decides one. Detach with `Ctrl+b d` --
agents keep running in the background.

## What you can do

**Start a session and let agents collaborate:**
```bash
ae my-feature                  # start or reattach a named session
ae my-feature --solo           # lead only, no colead
ae                             # attach to the fleet server's most recent session
```

Inside the fleet's tmux server, bare `ae` shows `ae list` instead of switching. From another
tmux server it prints this line (or the declared `tmux -S <path> attach` form):

```text
ae: the ae fleet is on another tmux server. Attach with: tmux -L ae attach
```

**Ask your agent to bring in help:**

Just tell it what you need -- it knows how to spawn others and coordinate. *"Get a second agent to review the changes in src/"* or *"Spin up a pair programmer to help refactor auth."* Agents pick descriptive names, show up in their own tmux windows, and talk to each other directly.

**Come back after a reboot:**
```bash
ae my-feature                  # every agent resumes with its conversation history
```

**Check on agents without attaching:**
```bash
ae list                        # running sessions: goal, git branch, per-agent state + attn marker
ae list --needs-attn           # only sessions needing attention (alias: --attn)
ae list --all                  # include stopped sessions
ae list --json                 # machine-readable digest (for scripts/agents)
ae next                        # name the top session needing attention (--attach jumps to it)
ae brief [name] [--all]        # why a session needs you: goal, latest note per memo topic,
                               # each agent's declared state, and every unanswered ask
ae orchestrator                # start or reattach the local orchestrator seat
ae orchestrator --popup --client <name>
                               # status button / prefix a bindings supply the client name
watch -n 10 'ae list'          # live dashboard
```

The bare `ae orchestrator` command starts one overview seat using
`~/.ae/orchestrator.config`, independent of the current directory's
`.ae/config`. Bind its profile globally with `orchestrator = <profile>` under
`[roster]`; ae seeds the dedicated config on first run and refuses before
writing when the row is missing. Use `--no-attach` to build the seat without
attaching. Existing dedicated configs that still carry `[profiles]`/`[roster]`
remain compatible, with `[profiles]`/`[roster]` ignored for identity;
`[workspace]`/`[prompt]` still overlay. The seeded 120-second minimum spacing
delivers a changed `NEEDS YOU` / `WORKING` / `QUIET` view to that pane. An idle
seat only declares `done` and stays done between changes; a human instruction
already in progress finishes before the overview is acknowledged.
Its privileged `relay <session[:agent]> <text…>` helper delivers only explicit
human instructions as bare text and audits them in the orchestrator session.
It never stops or ends its own session (the seat named `orchestrator`) and never all sessions: on such a request it runs nothing and answers that the seat cannot stop or end itself; the human does that from a terminal.

On an ae-owned server, `prefix a` opens the picker; `switch-client -l`
(`prefix L`) is the way back. ae needs tmux 3.4+.

`ae status` was retired. `ae list` answers the same question from one implementation, and
inside a session the `peek` helper shows one agent's recent output.

**Finish up:**
```bash
ae stop my-feature             # pause, keep state — resume later with 'ae my-feature'
ae end my-feature              # commit + push to ae/my-feature, archive the session's
                               #   memory to ~/.ae/archive/<uuid>/, remove ae state
                               #   (keeps conversation files; --purge-history deletes them
                               #    and writes no archive)
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

Inside a session, agents and humans share a set of helpers in `~/.ae/sessions/<name>/` -- the wiring agents use to collaborate. Each is a symlink to the ae core binary, so call them by full path (they are deliberately not on `PATH`, and invoked by bare name they refuse):

```bash
send <agent> <message>         # message another agent
ask <agent> <question>         # tracked request with a request id + exact reply command
review <agent> <request>       # critical review, findings-first reply contract
state <value> [reason]         # declare work state: working|waiting-user|blocked|done
say <text>                     # push a line to the human's Telegram chat (reply routes back)
memo add [--topic t] <text>    # durable shared session memory (survives restarts)
spawn <name> --using <profile> [prompt]   # add an agent · retire <agent> removes one
peek <agent> [lines]           # view an agent's recent output (inspection only)
```

Agent names resolve exactly -- `reviewer` (the name), `%42` (pane id) -- and `session:agent` or `@session:agent` reaches across sessions:

```bash
send @other-feature:lead "check my API changes"
agents --all                   # discover agents across every ae session
```

Agents call these automatically when you ask them to collaborate. Full helper catalog: **[docs/reference/helpers.md](docs/reference/helpers.md)**.

## Config

`~/.ae/config` is auto-created on first run; per-project overrides go in `.ae/config`.

```toml
[clients]
claude = claude
codex = codex

[profiles]
claude = "claude --permission-mode bypassPermissions --model opus"
codex = "codex --yolo -m gpt-5.6-sol -c model_reasoning_effort=high"
gpt56luna = "codex -m gpt-5.6-luna -c model_reasoning_effort=xhigh -a never"
grok = "grok --always-approve -m grok-4.6 --effort high"
agy = "agy --dangerously-skip-permissions"   # any CLI works — agy has no special ae integration

[roster]
lead = claude
reviewer = codex
orchestrator = gpt56luna

[workspace]
main = lead
workers = reviewer
layout = vertical

[prompt]
instructions = "Always write tests. Prefer TypeScript."
```

The NAME in `[roster]` is the agent's identity -- it's what you address in `send`/`spawn` and what shows in pane titles and `ae list`; the profile is just metadata (`ae list` shows it alongside the name). `[clients]` names executable/config-home instances, so `cc-mic = claude config_home=$HOME/.claude-mic` gives profiles a second login and conversation store ([recipe and Claude default-state caveat](docs/getting-started/config.md#multiple-identities-of-one-cli)); `[profiles]` adds launch flags, `[roster]` binds names to profiles, `[workspace]` sets the layout, and `[prompt]` injects custom instructions. The default ae writes on first run -- mirrored in the repo as [`config.sample`](config.sample) -- is a documented lead-pair setup. Choose how agents see your code with a working-directory mode:

| Mode | Flag | What it does |
|------|------|------|
| `local` | *(default)* | Agents work directly in your project directory. Simple and fast. |
| `full` | `--copy` | Full copy of the project — an isolated workspace for complex features. |
| `worktree` | `--worktree` | Git worktree — lightweight branch isolation backed by git. |

Full lineup, role guidance, and every key: **[docs/getting-started/config.md](docs/getting-started/config.md)**.

## How it works

Each agent gets workspace context injected into its system prompt (Claude Code's `--append-system-prompt`, Codex's `developer_instructions`, Gemini's `-i`). That context tells it **which agent it is** (`You are agent <name> (slot <slot>)`), who the other agents are, how to reach them by name, and how to spawn or retire agents. The communication itself happens through helpers (`send`, `peek`, `spawn`, …) that ae publishes in `~/.ae/sessions/<name>/` -- each one a link to the ae binary, dispatched by the name it is called by -- and agents call them like any other CLI tool.

No custom protocols, no frameworks. Just system prompts and commands agents already know how to run.

## One public command, typed core

`ae` IS the versioned Rust core: `~/.local/bin/ae` is a symlink straight to
`~/.ae/versions/<V>/ae-core`, published read-only. There is no wrapper between them —
the core tells INSTALLED from CHECKOUT by where its own binary resolves to, not by a
second file agreeing with it. `--copy` and `--worktree` give agents isolated workspaces
when you want them.

Everything else is **optional**, never required for core commands:

| Feature | What | Needs |
|---|---|---|
| `ae telegram` | machine-global bridge: fleet events to your Telegram chat, replies route back | a configured ae core (no extra CLI deps) |
| the orchestrator seat ([contrib/aeorchestrator](contrib/aeorchestrator)) | a dedicated single-seat local session named `orchestrator`: receives watchdog-rendered changed fleet overviews and relays only explicit human instructions through an audited bare-text helper | an agent CLI; ae seeds its config |

Both daemons are Rust, start to finish: the watchdog pane runs core `_watchdog-run`, the bridge runs core `_telegram-run`, and `ae watchdog`/`ae telegram` are core operations. Neither needs `jq` or `curl`. The watchdog renders the orchestrator's overview from core fleet facts, persists its last delivered hash, and wakes the seat only for changed text; the seat never scans the fleet on a timer. It was a Python sidecar (`contrib/aemonitor`) until the core took the job, and that was the product's last Python. Autostart controls are per component: set `watchdog = false` in workspace config to disable the workspace watchdog; set `enabled = false` in Telegram config to disable Telegram; set global `auto_upgrade = off` to disable automatic releases; set `AE_NO_AUTOSTART=1` to suppress all three.

There is no coreless mode to fall back to: the public `ae` command is the core binary
itself, so there is nothing separate to bind. See **[VISION.md](VISION.md)**.

### Upgrade

```bash
ae upgrade
```

`ae upgrade` downloads the latest release (or an `AE_VERSION` pin), verifies the checksum
before extraction, then hands publication to the downloaded core. It publishes the new
version read-only under `~/.ae/versions/<V>/`; migrates and repoints every placeable
session's core record and helpers; reports and skips stopped unplaceable sessions; updates
running watchdogs and Telegram bridges; and only then atomically moves `~/.local/bin/ae` to
the new core. Existing agent harnesses are never restarted.
Partial migration/relink failures are diagnosed with the same journal and recovery path
for manual and automatic upgrades. See [docs/upgrade.md](docs/upgrade.md).

## Requirements

- **bash >= 4.0** (only to run `install`; macOS ships 3.2 — `brew install bash` and put brew's bin dir ahead of `/bin` on `PATH`)
- [tmux](https://github.com/tmux/tmux) and [git](https://git-scm.com/) for the core runtime
- [just](https://github.com/casey/just) only to build from source (`just install`)
- At least one AI coding agent CLI (see *Works with*, above)

Linux (GNU userland) and macOS (BSD userland) are both first-class, and no GNU
*coreutils* package is required. The divergent coreutils are simply not called
any more: the readers that needed them are Rust, and the core locks and times
out with its own code rather than shelling out to `flock` or `timeout`.

`ae doctor` reports the userland it detected along with the rest of the
environment.

Your interactive shell can be anything (fish, zsh, …). Run **`ae doctor`** after installing ae or upgrading any agent CLI -- it checks tmux/git, your config, and whether configured agents are on `PATH`. Resume/session capture for external CLIs is best-effort and depends on each tool's upstream behavior; `ae doctor` is your first stop when something looks off. More: **[docs/troubleshooting.md](docs/troubleshooting.md)**.

## Development

Requires [just](https://github.com/casey/just):

```bash
just check            # lint (shellcheck) + format check (shfmt) for the installer
just test             # Rust format, lint, and tests
just release          # check → test → CalVer bump → changelog → tag → gh release
```

The Rust test gate is the whole test surface. Prerequisites are honest: [rustup](https://rustup.rs) and just, nothing else.

```bash
just rust-setup       # pinned toolchain + dev tools (idempotent; installs nothing on a second run)
just rust-check       # fmt check + clippy (-D warnings) + nextest + doctests
just rust-deny        # supply chain: advisories, licenses, bans, sources
just rust-mutants     # do the tests discriminate, or do they merely pass?
```

Conventions, test layout, and the release pipeline: **[docs/development.md](docs/development.md)**. The Rust toolchain contract — exact pins, lint policy, the no-`unwrap` rule — is in **[AGENTS.md](AGENTS.md#toolchain-pins)**.

## License

MIT
