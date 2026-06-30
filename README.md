# ae - agentic engineering

[![Release](https://img.shields.io/badge/release-0.2.1-blue.svg)](https://github.com/clemens33/ae/releases)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Bash](https://img.shields.io/badge/bash-%3E%3D4.0-green.svg)](https://www.gnu.org/software/bash/)
[![tmux](https://img.shields.io/badge/requires-tmux-1BB91F.svg)](https://github.com/tmux/tmux)
[![Install](https://img.shields.io/badge/install-curl%20%7C%20bash-orange.svg)](#install)

**ae** runs AI coding agents side-by-side in tmux. They know about each other, communicate by name, and survive reboots. One bash script, zero dependencies.

Works with [Claude Code](https://docs.anthropic.com/en/docs/claude-code), [Codex](https://github.com/openai/codex), [Gemini CLI](https://github.com/google-gemini/gemini-cli), [OpenCode](https://github.com/opencode-ai/opencode), or any CLI tool.

## Why ae

- **One command** -- `ae` starts a session, `ae` reattaches. That's the whole workflow.
- **Agents talk to each other** -- each agent gets workspace context injected into its system prompt. They send messages by name, spawn new agents, and coordinate without manual wiring.
- **Everything survives reboots** -- sessions, spawned agents, conversation history. Pick up exactly where you left off.
- **Nothing touches your repo** -- session state lives in `~/.ae/sessions/`. Your working directory stays clean.
- **Single bash script** -- no frameworks, no runtimes, no abstractions. Just bash, tmux, and git.

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/clemens33/ae/main/install | bash
```

Or clone manually:

```bash
git clone https://github.com/clemens33/ae.git ~/.local/share/ae
~/.local/share/ae/install
```

Both methods symlink `ae` to `~/.local/bin/ae`. Make sure `~/.local/bin` is on your `PATH`.

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
ae                             # default session (named after directory)
```

**Ask your agent to collaborate:**

Just tell your agent what you need -- it knows how to spawn others and coordinate. For example: *"Get a second agent to review the changes in src/"* or *"Spin up a pair programmer to help refactor auth."* Agents pick descriptive names, show up in adjacent panes, and talk to each other directly.

**Come back after a reboot:**
```bash
ae my-feature                  # all agents resume with their conversation history
```

**Check on agents without attaching:**
```bash
ae status my-feature           # see recent output from all agents
ae list                        # running sessions, per-agent state + attn marker
ae list --needs-attn           # only sessions needing attention (waiting-user/blocked/dead/stale)
ae list --active               # only sessions with recent activity (in flight)
ae list --attn                 # short alias for --needs-attn
ae list --all                  # include stopped sessions
ae list --json                 # machine-readable digest (for scripts/agents)
ae next                        # name the top session needing attention (alias: ae jump)
ae next --attach               # …and jump straight to it (switch-client / attach)
watch -n 10 'ae list'          # live dashboard
```

**Finish up:**
```bash
ae stop my-feature             # pause, keep state — resume later with 'ae my-feature'
ae end my-feature              # commit + push to ae/my-feature, remove ae state (KEEPS the
                               # claude/codex conversation files; --purge-history to delete them)
ae rm my-experiment            # same as ae end
```

## Session helpers

Inside a session, agents and humans have access to helper scripts in `~/.ae/sessions/<name>/`:

```bash
send <agent> <message>         # send a message to another agent
ask <agent> <question>         # tracked request with request id and exact reply command
review <agent> <request>       # critical review request with findings-first reply contract
reply <request-id> <message>   # reply to a logged ask/review request
requests [mine|inbox|all]      # inspect pending and replied requests
state <value> [reason]         # declare work state: working|waiting-user|blocked|done
mark-done [message]            # shim over `state done` (emits the legacy event the watchdog reads)
say <text>                     # push a free-text line to the human's Telegram chat (reply routes back)
memo add [--topic t] <text>    # append durable shared session memory
memo read [--topic t]          # read shared session memory
memo tail [n]                  # show latest memo entries
peek <agent> [lines]           # view recent output from an agent's pane (inspection only)
peak <agent> [lines]           # alias for peek
agents                         # list all agents with pane IDs
agents --all                   # list agents across all ae sessions
focus <agent>                  # switch tmux focus to an agent's pane
interrupt <agent> [message]    # stop an agent's current work, optionally redirect
spawn <alias:name> [prompt]    # add a new agent to the workspace
retire <agent>                 # remove a spawned agent cleanly
```

Internal helpers (prefixed `_`, e.g. `_register-sid`) are launched by ae itself and not part of the agent-facing surface.

Agent names resolve flexibly: `codex:reviewer` (exact), `codex` (alias, if unique), `reviewer` (bare name), or `%42` (pane ID).

**Cross-session communication:** Prefix with `@session:` to reach agents in other ae sessions:

```bash
send @other-feature:claude:lead "check my API changes"
peek @other-feature:reviewer 50
agents --all                   # discover agents across sessions
```

Agents use these automatically when you ask them to collaborate. You can also call them directly from any pane.
When a message includes an exact `reply` or `send` command, use that command verbatim instead of inferring the recipient.
Use `memo` for durable findings, decisions, and handoffs that should survive agent restarts or late joins. Keep it concise; it is shared memory, not a second chat transcript.

## Config

`~/.ae/config` is auto-created on first run. Per-project overrides go in `.ae/config` in your project directory.

```toml
[agents]
claude = "claude --permission-mode bypassPermissions --model claude-opus-4-6"
codex = "codex --yolo -m gpt-5.5 -c model_reasoning_effort=high"
gemini = "gemini --yolo -m gemini-2.5-pro"
opencode = "opencode -m google/gemini-3-pro-preview"

[workspace]
main = claude:lead
layout = vertical

[prompt]
instructions = "Always write tests. Prefer TypeScript."
```

**`[agents]`** -- register any CLI tool as an agent alias. The value is the shell command to launch it.

**`[workspace]`** -- control the session layout:

| Key       | Description                                          | Default       |
|-----------|------------------------------------------------------|---------------|
| `main`    | `alias:name` for the primary agent                   | `claude:lead` |
| `workers` | Comma-separated agents launched at startup           | *(empty)*     |
| `layout`  | `vertical` (side-by-side) or `horizontal` (stacked)  | `vertical`    |
| `copy`    | Working directory mode (see below)                     | `local`       |

Names show in pane borders and are how agents address each other.

**`[prompt]`** -- custom instructions injected into every agent's system prompt alongside the ae workspace context. Per-project `.ae/config` overrides the global one.

**Copy modes** -- how agents access your code:

| Mode | Flag | What it does |
|------|------|------|
| `local` | *(default)* | Agents work directly in your project directory. Simple and fast. |
| `full` | `--copy` | Full copy of the project. Use for complex features where agents need an isolated workspace. |
| `worktree` | `--worktree` | Git worktree. Lightweight branch isolation backed by git. |

**Pre-launch multiple agents:**
```toml
workers = codex:reviewer, opencode:tester
```

## Commands

```
ae [name]              Start or reattach a session
ae [name] use <alias>  Start session with a specific agent as main
ae list [--all|--stopped|--needs-attn]
                       List sessions (running by default) with state + attn
ae status [name]       Show agent output without attaching
ae doctor              Check local environment and ae config
ae doctor --refresh [name|all]
                       Refresh helper scripts/workspace.md in existing sessions
ae watchdog <start|stop|status> [name]
                       Toggle the stale-agent watchdog (per-session, persists across resume).
                       `ae loop …` still works as a deprecated alias.
ae telegram <setup|start|stop|status>
                       Machine-global Telegram bridge: forwards filtered events from every
                       ae session to one Telegram chat. Requires jq + curl (optional feature
                       dep — core ae works without them).
ae hub [--attach|--init]
                       Ensure the detached meta-agent hub is running; --attach switches
                       to it. --init scaffolds its config + charter (see contrib/aehub).
                       Optional feature.
ae stop [name]         Pause session, keep ae + agent conversation state for resume
ae end|rm [-f] [--purge-history|--keep-history] [name]
                       End session: commit, push to ae/<name>, remove ae state. KEEPS the
                       per-session claude/codex conversation files by default (token history);
                       --purge-history deletes them ([workspace] purge_agent_history sets default).
```

When run inside an ae session, `stop`, `end`, `status`, and `watchdog` detect the current session automatically.

## How it works

Each agent gets a workspace context injected into its system prompt (Claude Code's `--append-system-prompt`, Codex's `developer_instructions`, Gemini's `-i`). That context tells it who the other agents are, how to reach them by name, and how to spawn or retire agents. The actual communication happens through shell helpers (`send`, `peek`, `spawn`, `retire`, etc.) that ae generates in `~/.ae/sessions/` -- agents call them like any other CLI tool.

No custom protocols, no frameworks. Just system prompts and bash scripts that agents already know how to use.

## Requirements

- [tmux](https://github.com/tmux/tmux)
- [git](https://git-scm.com/)
- At least one AI coding agent ([Claude Code](https://docs.anthropic.com/en/docs/claude-code), [Codex](https://github.com/openai/codex), [Gemini CLI](https://github.com/google-gemini/gemini-cli), [OpenCode](https://github.com/opencode-ai/opencode), or any CLI tool)

After install, run:

```bash
ae doctor
```

It checks the local bash/tmux/git environment, your ae config, and whether configured agent executables are actually available on `PATH`.

After upgrading `ae` itself, you can refresh already-created session helpers without recreating the sessions:

```bash
ae doctor --refresh        # refresh every session in ~/.ae/sessions
ae doctor --refresh my-fix # refresh one session only
```

## Compatibility

`ae` is intentionally thin. It supports a small, tested core and treats the rest as best-effort:

| Area | Status |
|------|--------|
| Bash | Required: `>= 4.0` |
| tmux | Required |
| git | Required |
| Ubuntu / WSL2 | Primary development target |
| Other Linux / macOS setups | Best-effort, especially where GNU utilities differ |
| Agent session resume/capture | Best-effort for tools without stable public session APIs |

Practical meaning:

- Starting sessions, helper generation, and tmux orchestration are first-class.
- Resume/session capture for external agent CLIs depends on upstream tool behavior and local storage formats, which can change outside ae's control.
- `ae doctor` should be your first step after installing ae or upgrading any agent CLI.

## Troubleshooting

**`tmux` not found**

- Install tmux and rerun `ae doctor`.
- `ae` now fails fast on startup if tmux is missing instead of dying later inside session creation.

**Agent CLI not found**

- Run `ae doctor` and inspect the `agent:<alias>` lines.
- Fix the command in `~/.ae/config` or add the CLI to your `PATH`.

**`ae` installed but command still not found**

- Make sure `~/.local/bin` is on `PATH`.
- `ae doctor` warns if `~/.local/bin` is missing from the current shell environment.

**Session will not resume cleanly**

- Retry with `ae stop <name>` followed by `ae <name>`.
- Resume/session capture for external agent CLIs is best-effort and may fall back to a fresh start after upstream CLI changes.

**Session feels stuck**

- Use the generated `interrupt <agent>` helper for a soft cancel.
- Use `ae end -f <name>` to tear the session down and clean up hard state.

**Using fish or zsh**

- That is fine. `ae` itself runs under bash; your interactive shell does not need to be bash as long as `ae` is launched correctly.

## Development

Requires [just](https://github.com/casey/just) as task runner.

```bash
just check            # lint (shellcheck) + format check (shfmt)
just test             # unit + integration tests
just format           # auto-format with shfmt
just release          # full release: check → test → CalVer bump → changelog → tag → gh release
```

Releases use CalVer in `YYYY.MM.BUILD` format.

Dev tooling:

- [shellcheck](https://github.com/koalaman/shellcheck) — bash linter
- [shfmt](https://github.com/mvdan/sh) — bash formatter (indent=4, case-indent)
- [git-cliff](https://github.com/orhun/git-cliff) — changelog from conventional commits
- [gh](https://cli.github.com/) — GitHub CLI (releases)

## License

MIT
