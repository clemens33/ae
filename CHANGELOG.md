# Changelog

All notable changes to this project will be documented in this file.
## [v0.2.1] - 2026-03-06

### Bug Fixes

- Fix integration assertions for agent identity

### Other

- Add opencode resume support
## [v0.2.0] - 2026-03-02

### Bug Fixes

- Fix release: make gh release creation best-effort

Git tag push via SSH works regardless of gh auth. The GitHub release
creation is now optional — logs a warning instead of failing the pipeline.
- Fix agent launch: add delay between paste and C-m submit

The default agent launch path (Claude Code) used paste-buffer
followed immediately by send-keys C-m. On large commands (long
--append-system-prompt payloads), the paste hasn't finished
rendering before C-m fires, causing the agent to never start.
Add 0.3s delay matching the send helper pattern.
- Fix heartbeat: select-pane before paste for codex TUI compat

Codex TUI requires pane focus to process Enter after paste-buffer.
Add select-pane with focus restore to hb_send, matching the send
helper pattern.
- Fix send reliability across codex and claude

### Other

- Improve agent prompt: add concurrent collaboration awareness

Agents now know other agents are editing files simultaneously.
Unexpected modifications trigger verification (send) before
reverting, not blind acceptance. Clarify peek is for inspecting
work state, not polling for replies.
- Add heartbeat: background daemon detects stale/dead agents

Polls panes every 60s, checks alive via pane_current_command and
output freshness via capture-pane checksum. Dead agents trigger
tmux alerts; stale workers get nudged (max 2), then human alert.
Background-safe send (no focus switch), self-terminates when
session disappears. Configurable via AE_HEARTBEAT_INTERVAL_SEC
and AE_HEARTBEAT_STALE_MIN env vars.

### Refactoring

- Atomic tmux paste-and-submit, eliminate race condition
## [v0.1.1] - 2026-02-25

### Bug Fixes

- Fix agent send-keys instructions in workspace manifest

Use Enter instead of C-m and add explicit wrong/right examples
so agents keep the Enter key outside the quoted message string.
- Fix send helper: use literal text (-l) and C-m for reliable submit

The previous helper sent text with `Enter` key name which is unreliable
in TUI apps. Now uses `-l` flag for literal text injection and a separate
`C-m` (carriage return) for submit. Also fixes argument handling to
capture full multi-word messages and updates manifest to direct agents
to always use the helper instead of raw tmux send-keys.
- Fix codex resume: don't pass prompt as argument

codex resume --last doesn't accept inline prompts — the prompt was being
interpreted as a session ID. Now launches codex resume first, then sends
the prompt as user input after a delay. Guards against resume failure by
checking codex is still running before sending.
- Fix review findings: quoting bug, stale docs, test robustness

- send_agent_cmd: use buffer-paste with escaped single quotes to
  prevent prompt quoting breakage (Codex review IMPORTANT #1)
- AGENTS.md: fix stale "worktree default" → local is the default
- README: local sessions now survive reboots, clarify agent resume
- test: remove head-200 brittleness, match "func() {" to skip
  heredoc copies, add sanitize_branch_name and default_session_name
  tests (43 total)
- Move regenerate_manifest above dispatcher so spawn works
- Fix claude nesting detection: use env -u instead of bash-only unset

unset is bash syntax — breaks in fish shell tmux panes. env -u is
POSIX and shell-independent.
- Fix send/spawn Enter delivery: use C-m, increase paste delays

Enter key name can be remapped by tmux; C-m is the raw carriage
return that always works. Increased pre-submit delay to 0.3s for
TUI ingestion, added post-submit delay in send to keep focus while
target processes input.
- Fix spawn: wait for new pane shell init before sending launch command

split-window returns immediately but the shell in the new pane may
not be ready to accept input yet, causing paste-buffer to miss the
target pane.
- Fix send: serialize concurrent sends with flock

Concurrent sends to the same target pane could interleave paste and
C-m steps, causing messages to appear pasted but not submitted.
Add per-target flock serialization keyed by pane ID. Replace EXIT
trap with explicit focus restore to avoid racing with C-m delivery.
- Fix helpers: honor AE_TMUX_SERVER, filter non-agent panes

All session helpers (send, peek, agents, focus) now read tmux_server
from meta and wrap tmux with -L flag when set. Spawn exports
AE_TMUX_SERVER for the child ae process. Agents helper uses pipe
delimiter to correctly skip panes without @ae_agent set.
- Fix retire: validate pane-id belongs to session, prevent cross-session kills

Pane-ID targets now resolve through session pane list instead of
direct tmux access, preventing accidental kills of panes from other
sessions. Also use grep -Fv for fixed-string meta removal and update
manifest docs to show pane-id support.
- Fix integration tests: use ae end -f to skip confirmation prompt

All ae end calls in integration tests now pass -f flag to bypass
the interactive confirmation prompt that was causing 4 test failures.
22/22 integration tests passing.
- Fix resume: restore config, mode, and CWD from session meta

Claude Code's --resume is CWD-scoped — sessions are stored under
~/.claude/projects/<encoded-CWD>/. When ae resumed from a different
directory, both --resume UUID and --continue failed silently, starting
agents fresh instead of resuming conversations.

- Restore CONFIG_FILE and AE_LOCAL_CONFIG from meta before agent
  alias resolution (prevents "agent not defined" on cross-dir resume)
- Restore COPY_MODE from meta when no CLI flag override (prevents
  mode drift between original start and resume)
- Restore WORK_DIR from meta in local mode so tmux panes get the
  correct CWD (the primary fix for Claude Code session lookup)
- Restore ORIGIN_DIR from meta in all modes (worktree cleanup and
  env vars depend on it)
- Fix env -u CLAUDECODE prefix bug in resume fallback chain: was
  using $cmd instead of $launch_cmd, losing the nesting guard
- Fix lint and format: shfmt auto-format, shellcheck clean, add Developer section to README

Apply shfmt canonical formatting (redirect spacing, arithmetic, case alignment).
Fix all shellcheck warnings: suppress false positives (SC2015, SC2001, SC2034),
remove dead code (unused kind/MAIN_TOOL_KIND vars), fix real issues (SC2059 printf
format, SC2004 array index). Add Developer section to README with dev tooling info.

### Documentation

- Add session helpers to README and AGENTS.md

### Other

- Initial commit
- Initial release

tmux-based multi-agent workspace launcher with shared awareness.
Agents discover each other via .ae/workspace.md manifest and
communicate through tmux send-keys/capture-pane.
- Add project docs and improve installer

- AGENTS.md with structure, design decisions, and rules
- CLAUDE.md referencing AGENTS.md
- install script handles ./install, curl|bash, and missing parent dirs
- README install section updated with curl one-liner
- Add badges to README
- Add named sessions, session tagging, and ae list improvements

- ae <name> creates/reattaches named sessions (not just auto-generated)
- Tag sessions with AE_SESSION/AE_DIR env vars for reliable listing
- ae list shows directory column
- ae kill all uses env var tags instead of prefix matching
- Rewrite AGENTS.md to emphasize simplicity philosophy
- Guard against hijacking non-ae tmux sessions

- ae <name> refuses to attach if existing tmux session lacks AE_SESSION tag
- add .gitignore for .ae/ and .local/
- Revise README title and description

Updated project title and description in README.md.
- Isolate workspace manifests per session

Write to .ae/<session>/workspace.md instead of .ae/workspace.md so
multiple sessions from the same directory don't overwrite each other.
Single-quote the initial prompt to prevent shell expansion of session names.
- Add hardlink worktree isolation for all sessions

Every ae session now works on a hardlink copy at ~/.ae/worktrees/<session>/.
Agents work on the copy, push to remote, merge from there.

- Worktrees stored in ~/.ae/worktrees/ (invisible to user)
- Config validated before creating worktree (no orphaned dirs)
- Stale worktrees auto-cleaned on session start
- ae kill removes worktree on cleanup
- ae list shows origin directory
- Set tmux window name to session name
- Add send helper script to fix agents not pressing Enter
- Use absolute path for send helper in manifest
- Add session resume across reboots

Worktrees persist on disk at ~/.ae/worktrees/. Running ae <name> again
after reboot detects the existing worktree and resumes agents with their
previous conversation context (claude --continue, codex resume --last).

- ae list shows running and stopped (resumable) sessions
- ae kill handles stopped sessions (worktree-only cleanup)
- ae kill all cleans both running sessions and stopped worktrees
- Sanitize kill target to prevent path traversal
- Replace hardlink copy with git worktree default and full copy opt-in

cp -al shared inodes so agent edits could corrupt originals. Replace
with git worktree (detached HEAD) as default and cp -a as opt-in via
--full flag. Add session metadata for mode-aware cleanup, MODE column
in ae list, copy mode validation, and improved send helper that
focuses pane before paste+Enter for reliable TUI input.
- Add local mode and rename flags to --worktree/--copy/--local

New --local flag runs agents directly in the current directory
without any copy or worktree. Rename --git to --worktree and
--full to --copy for consistency across three modes. Store
AE_MODE in tmux env so ae list/kill work for local sessions
which have no on-disk worktree directory.
- Single-agent default with on-demand spawn helper

Start with just the main agent, spawn more on demand via
.ae/<session>/spawn <alias> [prompt]. Workers config still
works for fixed layouts but is no longer in the default config.

- rename default aliases to full names (claude/codex/opencode)
- add spawn helper with safe meta parsing and buffer-paste prompt
- regenerate workspace.md from live tmux panes (ae: prefix filter)
- extend meta file with session/work_dir/layout/config/main_pane
- always refresh dynamic meta fields on resume (pane IDs change)
- include spawn instructions in workspace.md and initial prompt
- Replace kill with end (commit+push+cleanup) and discard

ae end: auto-commits dirty state, pushes to ae/<session> branch,
then removes the tmux session and worktree/copy. Preserves session
on commit or push failure. Local mode just kills tmux.

ae discard: destroys session without saving (old kill behavior).
ae kill: deprecated alias to discard with warning.
- Move session state to ~/.ae/sessions/, keep working dirs clean

Session metadata, helpers (send/spawn), and workspace.md now live
in ~/.ae/sessions/<session>/ instead of <workdir>/.ae/<session>/.
Working directories stay clean — no .ae/ pollution, no gitignore
needed. Agents use fully-expanded absolute paths from the manifest.

Backward-compat: read_session_meta falls back to old worktree-nested
path for existing sessions. cleanup_session removes legacy paths.
- Switch default mode from git worktree to local
- Session-scoped agent resume across reboots

Thread a unique UUID per agent pane through the full session lifecycle:
generate on first start, persist in meta, inject into agent CLI flags,
and restore on resume. Claude Code uses --session-id/--resume, Codex
gets post-launch capture with flock-serialized meta writes, unknown
agents fall back to fresh start.

Also: local mode now detects existing sessions for resume, flag
stripping uses whole-token matching, and gen_uuid has no python
fallback (bash/tmux/git only per project rules).
- Add test suite for pure functions

34 assertions covering strip_session_flags, resume_cmd_from_cmd,
inject_session_id, tool_kind_from_cmd, tool_name_from_cmd, and
gen_uuid. Pure bash, no test framework dependency. Extracts functions
from ae via awk and tests them in isolation.
- Harden ae: health check, spawn self-invoke, spawn resume, integration tests

1. ae list shows agent health (alive/total, ! for crashed agents)
2. spawn refactored from declare-f heredoc to ae _spawn self-invocation,
   eliminating function inlining drift risk
3. spawned agents persist in meta and survive reboot with session-scoped
   IDs, flock-serialized writes, and codex capture support
4. 18 integration tests using isolated tmux server (AE_TMUX_SERVER),
   covering lifecycle, resume, health check, spawn persistence, and
   end-session workflows

Also moved resolve_agent_session_id and capture_codex_session_id to
top-level function block for availability across all code paths.
- Sharpen docs: emphasize simplicity, fix stale spawn info

AGENTS.md: add "What ae is NOT" section, line count cap (~1500),
strengthen philosophy ("simplicity is the feature"). README: rewrite
opening to lead with the value prop (one command, everything resumes),
fix stale note about spawned agents being ephemeral (they now persist).
- Add status/end-without-name/project-config, system prompt injection

- ae status [name]: show recent agent output without attaching
- ae end/discard/status auto-detect current session from $TMUX
- per-project config: .ae/config in project dir shadows global
- inject ae workspace context into system prompt (Claude Code
  via --append-system-prompt, Codex via -c developer_instructions)
  so agents retain ae awareness through context compaction
- slim initial prompt (system prompt carries all instructions now)
- move tests to tests/unit and tests/integration
- Add ae stop: pause session for later resume

Kills tmux session but preserves all meta — next ae <name>
resumes with all agents (main, workers, spawned) restored.
- Drop initial prompt on fresh start, system prompt is sufficient

Agents with system prompt injection (claude, codex) start
interactive — no busywork reading workspace.md on first turn.
Resume still sends a short nudge about changed pane IDs.
- Drop resume initial prompt too, system prompt is sufficient
- Name agent panes ae:<alias>:<name> for clearer identification

Main pane: ae:claude:main, workers: ae:codex:worker-0, spawned:
ae:claude:reviewer (user-named) or ae:claude:spawned-0 (auto).
Spawn syntax: spawn <alias>[:<name>] [prompt]. Manifest, meta,
and resume all parse the new format with backward compat for old
sessions. Spawn index scan + auto-naming moved inside flock to
prevent races.
- Agents address each other by name instead of raw pane IDs

Send helper resolves agent names (claude:main, codex:worker-0) to
pane IDs by scanning titles. System prompt and workspace.md tell
agents to use names. Pane border strips ae: prefix for cleaner
tmux display. ae status shows clean names too.
- Config-driven agent names, @ae_agent pane option, encourage creative naming

Config supports alias:name (e.g. main=claude:lead, workers=codex:reviewer).
Default name is the alias itself. Duplicate names auto-deduplicated.

Agent identity stored in tmux pane option @ae_agent — immune to title
overrides by tools like Claude Code. All scanning (manifest, health,
status, send) uses @ae_agent. Border display uses it too.

System prompt and workspace.md encourage descriptive names when spawning
(codex:reviewer, claude:pair-programmer). Auto-fallback: helper-N.
Role labels: lead/agent instead of main/worker/spawned.
- Rewrite README: streamlined, focused on real workflow

Drop verbose sections (modes table, session management list, workspace.md
internals). Lead with why-ae bullet points, show 4 real use cases, keep
config and commands compact. Reflects current state: named agents,
system prompt injection, reboot persistence, clean repos.
- Natural language for collaboration, document copy modes
- Explain how inter-agent communication works under the hood
- Configurable [prompt] instructions injected into agent system prompts
- Opencode support: inject workspace context as emphasized initial message
- Gemini cli support: context injection via -i, resume via --resume latest

Gemini gets workspace context through -i (prompt-interactive) flag.
Resume uses --resume latest (index-based, no UUID scoping).
Gemini-specific strip_gemini_prompt_flags() avoids breaking -i on
non-gemini commands.
- Ae list: show TARGET column for copy/worktree working directories
- Unified agent meta format, per-agent ae list, resilient resume, codex self-registration

- unified meta: agent.SLOT=alias:name:session_id replaces separate spawned.N + agent_session.N entries
- ae list: per-agent rows with truncated session IDs and idle markers, columnar layout with indented target
- resilient resume: claude --resume UUID || --continue fallback, codex resume || fresh start
- codex self-registration: register-sid helper script with slot-scoped sid files (prevents race conditions)
- preserve config flags (e.g. --yolo) through codex resume path
- colon validation: reject agent names containing ':' in main, worker, and spawn paths
- collapse discard/kill into end (ae end|rm is the only exit command)
- AGENTS.md: agent tool capabilities table documenting session/resume/prompt differences
- fix: shell-quote injection in codex developer_instructions when meta_dir contains single quotes
- fix: send_agent_cmd defined before dispatcher so _cmd_spawn can call it
- Ae <name> use <alias> CLI override, drop discard command, update docs

- ae <name> use <alias>: override main agent from CLI without editing config
- remove discard/kill commands — ae end|rm is the only exit path
- update README: document use syntax, replace discard references, fix ae list format
- integration test for use override (pane title + meta assertion)
- Config parser: allow hyphens, fix resume/codex/gemini, ae end safeguard

- parse_config: allow hyphens in section names and keys (gemini-flash etc)
- resume: read agent.main from session meta to preserve 'use' override
- codex: send initial "Go" prompt to trigger developer_instructions
- gemini: add "wait for task" instruction to -i context injection
- ae end: interactive y/N confirmation (single keypress), -f to bypass
- Reply-back communication pattern, fix claude nesting detection

- build_ae_context: teach agents to reply via send instead of polling capture-pane
- workspace.md: document reply-back pattern as primary communication flow
- spawn: resolve caller agent name, include reply-back instruction in spawn prompt
- send_agent_cmd: unset CLAUDECODE env var so claude launches from inside ae sessions
- Resolve bare agent names (e.g. send "lead" instead of "claude:lead")
- Add peek helper, fix local-outside-function bug in send

Add peek session helper: thin wrapper around tmux capture-pane with
agent name resolution. Supports bare names, numeric line count with
clamping (default 80, max 2000). Documented in workspace manifest.

Fix send helper: remove `local` keyword used outside a function in
the name resolution loop (caused errors in bash strict mode).
- Add agents and focus session helpers

agents: list all agents in session with pane ID and process name.
focus: switch to another agent's pane by name, with same name
resolution as send/peek (exact alias:name + bare name fallback).
Both documented in workspace manifest.
- Add retire helper: clean removal of spawned agents

retire kills the pane, removes the agent.spawned entry from meta
(flock-protected), rebalances layout, and regenerates the manifest.
Guards against retiring main or worker agents. Implemented as
ae _retire internal command with thin helper script, matching the
spawn pattern.
- Add interrupt helper: cancel agent generation with optional redirect

Single Escape to interrupt (safe across all TUIs — double-Escape
triggers edit/rewind on Claude, Codex, Gemini). Shares per-target
flock with send to prevent interleaving. Optional message delivered
inline after 0.5s delay. Documented in manifest, README, AGENTS.md.
- Add ask helper, expand agent system prompt with all helpers

The injected system prompt (build_ae_context) only mentioned send and
spawn. Agents didn't know about peek, agents, focus, interrupt, or
retire — limiting their ability to collaborate effectively.

- Expand build_ae_context to list all 8 session helpers with brief
  descriptions
- Add `ask` helper: thin wrapper around send that auto-detects
  caller identity via @ae_agent and embeds reply-to metadata in the
  message, making request-response between agents reliable
- Fall back to plain send if caller identity can't be detected
- Use alias:name (not bare name) in reply-to for unambiguous routing
- Update OpenCode initial prompt to reference helpers generically
- Document ask in AGENTS.md helper table
- Add justfile pipeline, version support, ask helper, expanded agent prompt

- Add justfile with check/lint/test/release pipeline (SemVer, git-cliff
  changelog, shellcheck, shfmt, GitHub releases via gh)
- Add AE_VERSION constant and ae version/--version command
- Add cliff.toml for git-cliff with SemVer tag pattern
- Add ask helper: structured send with reply-to metadata so agents
  reliably respond back to the asking agent
- Expand build_ae_context to list all 8 session helpers (was only
  send + spawn, agents didn't know about peek/agents/focus/interrupt/retire)
- Add version badge to README, document ask helper
- Update AGENTS.md structure section

### Refactoring

- Extract subcommands into named functions
- Refactor helpers into shared _lib, add cross-session communication

Extract duplicated resolver/lock logic from all helpers into _lib shared
library. Add @session:agent syntax for cross-session send/peek/focus/interrupt.
Add agents --all for cross-session discovery. Lock files now use shared
~/.ae/sessions/.locks/ dir for correct cross-session serialization.

Net -42 lines despite new features.
