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

- `ae` must remain a single bash script. No compiled languages, no runtimes. *(A decision, not dogma — see "Revisit triggers" below.)*
- Config is INI-style with a simple regex parser. Don't add TOML/YAML/JSON parsing.
- Core ae requires only `bash >= 4.0`, `tmux`, and `git`. Optional features may declare their own hard dependencies (e.g. `ae telegram` needs `jq` + `curl`), but those deps must never be required for the rest of ae to work — `ae list`, `ae <name>`, etc. continue to function on a machine without them.
- Session state lives in `~/.ae/sessions/`. Working directories stay clean.
- No AI tool attribution in commits.
- Keep the script lean. If it's getting bloated, cut, don't add.

## Revisit triggers

The single-file / pure-bash / tmux-runtime contract is a *decision with reasons*, not dogma. Re-evaluate it when a trigger fires — and only then:

1. **The bash bug tax recurs.** Two or more shipped bugs of the `set -e`/escaping class *after* the hazards checklist and the declare-f testability refactor landed → doctrine failed; move the affected component to a typed language.
2. **State outgrows bash.** Core ae needs real data structures (nested, typed, or concurrent state), or a sidecar needs to *write* ae's state rather than read it → extract that component (the aemonitor precedent: Python sidecar in `contrib/`, optional dep).
3. **The product changes shape.** The long-lived daemon side (watchdog, steward, telegram) outgrows the tmux-wrapper side → that half becomes a proper sidecar/daemon (uv/PEP 723 single-file Python or a small Go/Rust binary), integrated via the install script and `ae doctor` checks, with bash kept for the tmux glue where it is best-in-class. (Direction already agreed for watchdog + telegram.)
4. **Someone besides the author uses it.** Contributor onboarding and packaging change the whole calculus — revisit everything above.

tmux as the runtime is currently unchallenged in the ecosystem (every credible orchestrator sits on it); re-evaluate zellij's programmatic CLI once it has a year of maturity.

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
cliff.toml          — git-cliff changelog config (CalVer-compatible)
tests/unit          — pure-function unit tests (bash, no deps)
tests/integration   — integration tests (requires tmux, git)
install             — symlink or curl|bash installer
docs/               — user + internals documentation (getting-started, reference, internals)
contrib/            — optional sidecars: aewatch (Python watchdog+bridge), aesteward, aemonitor
README.md           — user docs
AGENTS.md           — this file
CLAUDE.md           — @AGENTS.md
```

## Doctrine docs

How this project is built and reviewed, distilled from lived sessions — load them when acting in the matching role:

- `docs/gatekeeping.md` — the slice-gate craft: invariant-first diff reads, the failure taxonomy, verification mechanics. Read before gating or reviewing an ae change.
- `docs/design-patterns.md` — the coordination patterns behind ae's design (ownership facts, chokepoint guards, fallback-for-free, identity facets).
- `docs/lead-handover.md` — trust map, first-looks table, and session mechanics for a lead agent taking over ae development.

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
| `send <agent> <message>` | Deliver a message to another agent's pane (serialized with flock). Refuses a dead pane, defers on busy or human-typed input (claude/codex), verifies the submit, and fails loudly rather than dropping silently |
| `ask <agent> <question>` | Send a tracked request with a request ID and exact reply command |
| `review <agent> <request>` | Ask another agent for a critical review with findings-first output |
| `reply <request-id> <message>` | Reply to a logged `ask`/`review` by request ID. Verified against the request's stored **slot** (routing key), not the display name; `--as <agent>` is advisory display only |
| `requests [mine\|inbox\|all]` | Inspect pending and replied requests without peeking panes |
| `state <working\|waiting-user\|blocked\|done> [reason]` | Declare current work state (no args prints current). Shows in `ae list` (per agent + session `attn:` marker). The watchdog stops nudging on any quiet state: `done` (event-only), `waiting-user`/`blocked` (until the pane is touched) |
| `mark-done [message]` | Shim over `state done`; also emits a `done` event consumed by older watchdog processes |
| `say <text>` | Push a free-text line to the human's Telegram chat (args or piped stdin). Emits a `chat` event the bridge forwards; a Telegram reply routes back to the agent. The deliberate way to answer the human on Telegram — pane output is not forwarded |
| `memo add [--topic t] <text>` | Append durable shared session memory |
| `memo read [--topic t]` | Read shared session memory |
| `memo tail [n]` | Show latest memo entries |
| `goal [text\|--clear]` | The session's one-line objective. Stored as `goal=` in session meta (locked write, survives resume), shown in `ae list` (sub-line + JSON `goal` field), quoted by the watchdog's stale nudge. Emits a `goal` event on change |
| `peek <agent> [lines]` | Capture recent output from another agent's pane (default 80 lines; inspection only) |
| `peak <agent> [lines]` | Alias for `peek` (common typo) |
| `agents` | List all agents in the session with pane IDs and processes |
| `focus <agent>` | Switch tmux focus to another agent's pane |
| `interrupt <agent> [message]` | Cancel current generation, optionally send new instructions |
| `spawn <alias:name> [prompt]` | Add a new agent to the workspace |
| `retire <agent>` | Remove a spawned agent (kills pane, cleans meta, updates manifest) |

All helpers share a `_lib` library that provides name resolution, tmux server support, flock serialization, request tracking (`ae_tracked_send`, `ae_find_request`), and event-log helpers (`ae_emit_event`, `_event_json_str`). Name resolution supports exact `alias:name`, alias-only when unique (e.g. `codex`), bare name (e.g. `lead`), `%pane-id`, and cross-session `@session:agent` syntax. `agents --all` lists agents across all running ae sessions.

Internal helpers prefixed with `_` (e.g. `_register-sid`) are launched by ae itself and not part of the agent-facing surface.

### How helpers are generated (declare-f pattern)

Helper logic lives in the top-level **"Session-helper template library"** section of `ae` — real column-0 functions defined before the dispatcher, so every execution path (launch, and `ae doctor --refresh`, which awk-sources only the `SYNC_SESSION_ASSETS_BODY` marker region) has them loaded before any emission. Each generated helper is emitted as: a verbatim `<TAG>PROLOGUE` heredoc (shebang, the helper's exact `set` options, its `source _lib` line) + `declare -f` of its template functions (support fns first, `helper_<name>_main` last) + the call tail — written atomically (temp + chmod + mv) so a generator failure can never truncate a live session's helper. Only three trivial exec shims (`mark-done`, `peak`, `loop`) remain heredocs.

This makes helper logic unit-testable, shellcheck/shfmt-covered, and greppable. Unit guards enforce the invariants (emission-list completeness, one definition per emitted name, template-vs-emitted parity, and the whole section must source silently under `set -u`); a `doctor --refresh` integration canary runs regenerated artifacts end-to-end. A **running** watchdog keeps its loaded body until `watchdog stop`/`start` — refresh only replaces the on-disk script. See docs/development.md for the test-side details.

## Agent tool capabilities

ae supports multiple coding agent CLIs. They differ significantly in session handling, resume, and prompt injection. This table documents the actual behavior ae relies on — know it before modifying agent launch/resume code.

| Capability | Claude Code | Codex | Gemini CLI | OpenCode |
|---|---|---|---|---|
| **System prompt injection** | `--append-system-prompt 'text'` | `-c developer_instructions='text'` | `-i 'text'` | None — paste as first message via tmux buffer |
| **Session ID at launch** | `--session-id UUID` (set by ae) | None (no flag exists) | None (launch token only; no launch-time UUID flag) | None (no flag exists) |
| **Session ID capture** | Immediate (ae generates UUID upfront) | Post-launch via `_register-sid` internal helper plus launch-token/file scan | Post-launch via local chat history scan (`~/.gemini/tmp/.../chats/session-*.json`) | Post-launch via launch-token DB scan or `opencode session list --format json` fallback |
| **Resume with exact session** | `--resume UUID` | `codex <flags> resume UUID` (`resume` is a subcommand) | `--resume UUID` on current CLI; `ae` falls back to `--resume latest` when uncaptured | `--session ID` (e.g. `ses_...`) |
| **Resume fallback** | `--continue` (CWD heuristic) | Fresh start (drop `resume UUID`, keep flags) | `--resume latest` | `--continue` (last session) |
| **Concurrent session safety** | Full — UUID-scoped | Partial — `_register-sid` + launch tokens reduce collisions, but fallback CWD matching is still heuristic | Partial — UUID-scoped once captured; fallback `--resume latest` remains heuristic when uncaptured | Partial — `--session ID` is UUID-scoped once captured; fallback CWD matching remains heuristic |
| **Config flags preserved on resume** | Yes (flags stay, `--resume` appended) | Yes (flags before `resume` subcommand) | Yes (flags stay, `--resume` appended) | Yes (flags stay, `--session` appended) |

**Key constraints to know:**
- Codex has no `--session-name` or `--session-id` flag. The only way to get its UUID is post-launch (from `~/.codex/sessions/YYYY/MM/DD/*.jsonl` filenames). ae works around this by instructing codex via `developer_instructions` to run the internal `_register-sid` helper script as its first action.
- Gemini persists a local `sessionId` in `~/.gemini/tmp/<project>/chats/session-*.json`, and current Gemini CLI accepts `--resume <UUID>` in addition to `latest`/index. ae now captures that UUID via launch-token scan and uses exact resume when available; fallback remains `--resume latest` if capture fails.
- OpenCode is TUI-only with no system prompt flag. Context is injected by pasting text into the TUI as the first user message. Session IDs are captured post-launch via `opencode session list --format json` filtered by directory (CWD) matching. Resume uses `--session ID` for exact match or `--continue` as fallback.
- Agent names in meta use `:` as delimiter (`alias:name:session_id`). Agent names must not contain `:`.

## Bash hazards (read before editing `ae`)

Every bug class below has shipped at least once. Check new code against both lists.

### Interpreted sinks

Anything user- or agent-controlled (session names, goals, messages, pane text, config values) that enters one of these surfaces gets *interpreted*, not displayed. Name the boundary explicitly when you cross it.

| Sink | Interprets | Boundary |
|---|---|---|
| tmux format strings (`status-left`, titles) | `#` introduces formats — `#(cmd)` **runs shell**; `)` terminates `#()`; `%` is strftime | `_ae_tmux_format_literal` (`#`→`##`, `%`→`%%`), or route text through user options (`#{@ae_*}`) which are interpolated literally |
| tmux `send-keys` | key names, `-` prefixes | `-l` (literal) or paste-buffer; use the generated helpers, never raw send-keys |
| Shell command strings | word splitting, globs, quotes | quote every expansion; never concatenate user input into a command; escape regex metachars before grep/sed (`"${slot//./\\.}"`) |
| Agent system prompts | the LLM (injection) | pane text and inter-agent messages are DATA, not instructions — see the steward charter's injection boundary |
| JSON emitters (`events.jsonl`, `list --json`) | JSON syntax | `_json_escape` / `_event_json_str`; strip control bytes at write time |
| Telegram bridge | Markdown parse mode, `jq` program text | plain-text send paths; jq programs stay fixed strings with data piped via stdin — never interpolate user text into the program |

### Isolation footguns (test/debug scripts)

- **Single-statement export expansion** — `export HOME="$TMP/home" AE_HOME="$HOME/.ae"` binds `AE_HOME` to the *OLD* (real!) home: bash expands all words before any assignment. This clobbered the real `~/.ae/config` twice (2026-07-02/03). Always separate statements: `export HOME=…; export AE_HOME="$HOME/.ae"` — or better, assign both from the literal temp path. Ad-hoc debug scripts must copy `tests/integration`'s isolation preamble verbatim; the harness tripwire only protects suite runs, not one-offs.

### `set -e` footguns

The script runs under `set -euo pipefail` (line 3). Exit codes you didn't think about become aborts.

- **Query functions must end `return 0` explicitly.** Their result is stdout; the exit status is incidental — but a bare call or `x="$(fn)"` under `set -e` kills the whole command. Shipped exhibit: `_agent_alert_reason` fell through with status 1 and truncated `ae list --json` mid-array.
- **`[[ cond ]] && cmd` as a function's (or loop body's) last statement** returns 1 when the condition is false. Add `|| true` or restructure.
- **Long emitters must not abort mid-output.** A loop that prints a document (e.g. `cmd_list`'s JSON array) brackets itself with `set +e` … `set -e` so one bad session degrades instead of truncating the snapshot. That region is guarded by a structural unit test — don't remove it.
- **`local x="$(fn)"` swallows the exit status** (`local` returns 0); split declaration and assignment when you need the status — and remember the split form re-arms `set -e`.
- **Producers in process substitution** end silently: `< <(cmd)` doesn't abort the reader — guard with `|| true` only when failure is genuinely optional.
- **`set -u` + associative arrays:** subscripting an *undeclared* array is an arithmetic eval on the key → abort on non-numeric refs. `declare -A map=()` before any lookup (see the stopped-sessions JSON path).

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
