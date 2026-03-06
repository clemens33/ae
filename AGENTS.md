# ae

Single bash script. No dependencies beyond bash and tmux. Keep it that way.

## Philosophy

- ae is a thin wrapper around tmux — not a framework, not a platform.
- The goal is **daily productivity**, not feature completeness. If it doesn't save time on every use, it doesn't belong.
- Resist adding features. If tmux already does it, don't re-implement it.
- One file does everything. Don't split into modules or libraries.
- No build steps, no package managers, no abstractions.
- Simplicity is the feature. The entire tool must remain understandable in one sitting.

## Rules

- `ae` must remain a single bash script. No compiled languages, no runtimes.
- Config is INI-style with a simple regex parser. Don't add TOML/YAML/JSON parsing.
- No dependencies beyond bash >= 4.0, tmux, and git.
- Session state lives in `~/.ae/sessions/`. Working directories stay clean.
- No AI tool attribution in commits.
- Keep the script lean. If it's getting bloated, cut, don't add.

## What ae is NOT

- Not a CI/CD pipeline. Use your existing workflow for that.
- Not a cost tracker. Agents track their own usage.
- Not a logging system. tmux already does `capture-pane` and `pipe-pane`.
- Not a git workflow tool. It does the minimum (commit + push), nothing more.
- Not a plugin framework. Bash is already the plugin system — wrap `ae` in a script if you need custom behavior.

## Structure

```
ae                  — the script
justfile            — dev/release pipeline (just check, just test, just release)
cliff.toml          — git-cliff changelog config (SemVer)
tests/unit          — pure-function unit tests (bash, no deps)
tests/integration   — integration tests (requires tmux, git)
install             — symlink or curl|bash installer
README.md           — user docs
AGENTS.md           — this file
CLAUDE.md           — @AGENTS.md
```

## How it works

1. Parses `~/.ae/config` for agent commands and layout
2. Uses current dir (default `--local`), full copy (`--copy`), or git worktree (`--worktree`)
3. Creates tmux session with main agent (+ workers if configured)
4. Generates session helpers and workspace manifest in `~/.ae/sessions/<name>/`
5. Launches agents with workspace context injected into their system prompts
6. Attaches

`ae end` (or `ae rm`) commits + pushes to `ae/<session>` branch, then cleans up.

## Session helpers

ae generates these scripts in `~/.ae/sessions/<name>/` for agents and humans to use:

| Helper | Purpose |
|--------|---------|
| `send <agent> <message>` | Send a message to another agent's pane (serialized with flock) |
| `ask <agent> <question>` | Like send, but embeds caller identity and reply-to command in the message |
| `peek <agent> [lines]` | Capture recent output from another agent's pane (default 80 lines) |
| `agents` | List all agents in the session with pane IDs and processes |
| `focus <agent>` | Switch tmux focus to another agent's pane |
| `interrupt <agent> [message]` | Cancel current generation, optionally send new instructions |
| `spawn <alias:name> [prompt]` | Add a new agent to the workspace |
| `retire <agent>` | Remove a spawned agent (kills pane, cleans meta, updates manifest) |
| `heartbeat` | One-shot status check for all panes; `heartbeat --watch` runs as background daemon |
| `register-sid [slot]` | Codex-specific: self-register session ID post-launch |

All helpers share a `_lib` library that provides name resolution, tmux server support, and flock serialization. Name resolution supports exact `alias:name`, bare name (e.g. `lead`), `%pane-id`, and cross-session `@session:agent` syntax. `agents --all` lists agents across all running ae sessions.

## Agent tool capabilities

ae supports multiple coding agent CLIs. They differ significantly in session handling, resume, and prompt injection. This table documents the actual behavior ae relies on — know it before modifying agent launch/resume code.

| Capability | Claude Code | Codex | Gemini CLI | OpenCode |
|---|---|---|---|---|
| **System prompt injection** | `--append-system-prompt 'text'` | `-c developer_instructions='text'` | `-i 'text'` | None — paste as first message via tmux buffer |
| **Session ID at launch** | `--session-id UUID` (set by ae) | None (no flag exists) | None (index-based only) | None (no flag exists) |
| **Session ID capture** | Immediate (ae generates UUID upfront) | Post-launch via `register-sid` helper (codex self-registers) | N/A — no UUID concept | Post-launch via `opencode session list --format json` (CWD matching) |
| **Resume with exact session** | `--resume UUID` | `codex <flags> resume UUID` (`resume` is a subcommand) | `--resume latest` or `--resume <index>` — no UUID | `--session ID` (e.g. `ses_...`) |
| **Resume fallback** | `--continue` (CWD heuristic) | Fresh start (drop `resume UUID`, keep flags) | `--resume latest` only | `--continue` (last session) |
| **Concurrent session safety** | Full — UUID-scoped | Partial — `register-sid` picks latest session file globally, slot-scoped `.sid` files prevent cross-slot collisions | Weak — `--resume latest` picks globally | Partial — `--session ID` is UUID-scoped once captured; capture picks newest session matching CWD, so multiple opencode agents in the same directory may collide |
| **Config flags preserved on resume** | Yes (flags stay, `--resume` appended) | Yes (flags before `resume` subcommand) | Yes (flags stay, `--resume` appended) | Yes (flags stay, `--session` appended) |

**Key constraints to know:**
- Codex has no `--session-name` or `--session-id` flag. The only way to get its UUID is post-launch (from `~/.codex/sessions/YYYY/MM/DD/*.jsonl` filenames). ae works around this by instructing codex via `developer_instructions` to run a `register-sid` helper script.
- Gemini has the weakest resume: index-based, no UUID scoping. With multiple concurrent gemini agents, `--resume latest` may resume the wrong session. Nothing ae can do about this.
- OpenCode is TUI-only with no system prompt flag. Context is injected by pasting text into the TUI as the first user message. Session IDs are captured post-launch via `opencode session list --format json` filtered by directory (CWD) matching. Resume uses `--session ID` for exact match or `--continue` as fallback.
- Agent names in meta use `:` as delimiter (`alias:name:session_id`). Agent names must not contain `:`.

## Config

```toml
[agents]
alias = "shell command"

[workspace]
main = alias
workers = alias, alias2    # optional, omit for single-agent start
layout = vertical

[prompt]
instructions = "Custom instructions injected into agent system prompts"
```

That's it. Don't extend the format.
