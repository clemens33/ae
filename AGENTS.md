# ae

One public wrapper and one Rust core in one immutable versioned install. Keep the product thin: tmux remains the runtime, and the Bash that survives is the wrapper that binds the core plus the pane scripts the core generates.

> **Where the rules live.** The Bash sections below govern the surviving wrapper; the Rust
> core has its own: [Rust era](#rust-era-main). Product direction: [VISION.md](VISION.md).

## Philosophy

- ae is a thin wrapper around tmux — not a framework, not a platform.
- The goal is **daily productivity**, not feature completeness. If it doesn't save time on every use, it doesn't belong.
- Resist adding features. If tmux already does it, don't re-implement it.
- One file does everything. Don't split into modules or libraries.
- No build steps, no package managers, no abstractions.
- Simplicity is the feature. The entire tool must remain understandable in one sitting.

*Scope:* the first, second, third and sixth bullets are durable —
they describe what ae **is**, and the Rust core inherits them unchanged (a thin wrapper,
daily productivity, resist features, understandable in one sitting). "One file does
everything" and "no build steps" were bash-implementation rules; they are historical.
A crate with modules is the doctrine applied to a language where one file is not how you
stay readable.

## Rules

- `ae` must remain a single bash script. No compiled languages, no runtimes. *(Historical: this once governed all of `ae`; triggers 1–3 fired on 2026-08-20 and the Rust core is the ratified successor, not a violation of the durable product doctrine.)* What survives is ONE bash file, the public wrapper `ae-entry`, and it is policy-frozen: no new Bash features. **`ae-glue` was deleted whole in slice Z1** and its residue folded into that wrapper.
- Config is INI-style with a simple regex parser, and it is the core's (`src/config.rs`) — the wrapper reads no key of it and writes no config at all. Don't add TOML/YAML/JSON parsing.
- Core ae requires only `bash >= 4.0`, `tmux`, and `git`. Optional features may declare their own hard dependencies (e.g. the orchestrator companion session needs an agent CLI; `contrib/aemonitor` needs Python 3), but those deps must never be required for the rest of ae to work — `ae list`, `ae <name>`, etc. continue to function on a machine without them. (`ae telegram` used to need `jq` + `curl`; the Rust-core bridge needs neither — it is the ae core binary.)
- Session state lives in `~/.ae/sessions/`; archived session memory lives in
  `~/.ae/archive/<session-uuid>/` and is INERT — data only, never an executable file.
  Working directories stay clean.
- No AI tool attribution in commits.
- Keep the script lean. If it's getting bloated, cut, don't add.

## Revisit triggers

The single-file / pure-bash / tmux-runtime contract is a *decision with reasons*, not dogma. Re-evaluate it when a trigger fires — and only then:

1. **The bash bug tax recurs.** Two or more shipped bugs of the `set -e`/escaping class *after* the hazards checklist and the declare-f testability refactor landed → doctrine failed; move the affected component to a typed language.
2. **State outgrows bash.** Core ae needs real data structures (nested, typed, or concurrent state), or a sidecar needs to *write* ae's state rather than read it → extract that component (the aemonitor precedent: Python sidecar in `contrib/`, optional dep).
3. **The product changes shape.** The long-lived daemon side (watchdog, orchestrator, telegram) outgrows the tmux-wrapper side → that half becomes a proper sidecar/daemon (uv/PEP 723 single-file Python or a small Go/Rust binary), integrated via the install script and `ae doctor` checks, with bash kept for the tmux glue where it is best-in-class. (Direction already agreed for watchdog + telegram.)
4. **Someone besides the author uses it.** Contributor onboarding and packaging change the whole calculus — revisit everything above.

**Fired (2026-08-20).** Triggers 1, 2 and 3 all fired: the `set -e`/framing bug class kept
shipping *after* the hazards checklist existed, the events ledger and request/claim state
outgrew what bash can hold safely, and the daemon half outgrew the wrapper half. The ruling
is epic #79 — a Rust core, bash kept for the tmux glue where it is best-in-class. Trigger 4
is partly here: ae is packaged and released as checksum-verified platform bundles, so the
install side of that calculus has already changed. What has not arrived is a second author —
no contributor onboarding, no external consumer. Re-evaluate everything above when one shows
up.

tmux as the runtime is no longer unchallenged: **herdr** (herdrdev/herdr, Rust, Apache-2.0, ~24k stars) is a credible agent multiplexer with its own renderer, agent-state sidebar, and a Unix socket API agents can drive programmatically — the first serious non-tmux substrate. It competes with ae's *plumbing*, not its coordination protocol or doctrine; a watchlist item, not a migration plan — migrate only when one of the triggers above fires, and if trigger 3 does, herdr's socket API is a candidate substrate to port the helpers onto. Watch alongside zellij's programmatic CLI (still no send-keys-stable API). Assessed 2026-08-03, cross-model research (secondary sources + repo metadata); read its source before any commitment.

## What ae is NOT

- Not a CI/CD pipeline. Use your existing workflow for that.
- Not a cost tracker. Agents track their own usage.
- Not a logging system. tmux already does `capture-pane` and `pipe-pane`.
- Not a git workflow tool. It does the minimum (commit + push), nothing more.
- Not a plugin framework. Bash is already the plugin system — wrap `ae` in a script if you need custom behavior.

## Structure

```
ae-entry            — the public wrapper source (installed and bundled as `ae`), and since
                      slice Z1 the product's ONLY bash file: it validates the versioned pair,
                      answers `version` and `upgrade`, and execs the core ONCE with the frozen
                      preamble plus the caller's argv verbatim
justfile            — dev/release pipeline (just check, just test, just release)
cliff.toml          — git-cliff changelog config (CalVer-compatible)
tests/unit          — pure-function unit tests (bash, no deps)
tests/integration   — integration tests (requires tmux, git)
tests/itest-par     — parallel sharded runner for tests/integration (`just itest-all`, `just itest <domain>`); tests/itest-domains.tsv tags every section with a domain and records order-dependent chains, tests/itest-timings.tsv holds measured seconds per section
install             — canonical checksum-verifying versioned installer (checkout or release entry)
docs/               — user + internals documentation (getting-started, reference, internals)
contrib/            — optional sidecars: aewatch (retired Python watchdog+bridge; archival), aeorchestrator, aemonitor
Cargo.toml          — Rust package: one crate, bin + lib, both named `ae` (no workspace)
rust-toolchain.toml — compiler pin: channel, profile, components, both targets
clippy.toml         — the tests-only relaxation of the unwrap/expect rule
deny.toml           — supply-chain policy (advisories, licenses, bans, sources)
taplo.toml          — TOML fmt/lint scope
.cargo/             — repo-owned cargo config (aliases) + cargo-mutants config
src/                — Rust sources: main.rs (thin) + lib.rs (everything testable), one module per domain
tests/it/           — the single integration-test target (main.rs + `mod` submodules)
.github/workflows/  — rust lanes and tag-triggered prebuilt release lanes, both platforms
README.md           — user docs
VISION.md           — what ae is, and where it is going
AGENTS.md           — this file
CLAUDE.md           — @AGENTS.md
```

## Doctrine docs

How this project is built and reviewed, distilled from lived sessions — load them when acting in the matching role:

- `docs/gatekeeping.md` — the slice-gate craft: invariant-first diff reads, the failure taxonomy, verification mechanics. Read before gating or reviewing an ae change.
- `docs/design-patterns.md` — the coordination patterns behind ae's design (ownership facts, chokepoint guards, fallback-for-free, identity facets).
- `docs/lead-handover.md` — **historical.** The trust map, first-looks table, and session mechanics from before the Rust core owned `list`. Handover evidence, not current guidance.

## How it works

1. Parses `~/.ae/config` for agent commands and layout
2. Uses current dir (default `--local`), full copy (`--copy`), or git worktree (`--worktree`)
3. Creates tmux session with main agent (+ workers if configured)
4. Generates session helpers and workspace manifest in `~/.ae/sessions/<name>/`
5. Launches agents with workspace context injected into their system prompts
6. Attaches

`ae end` (or `ae rm`) commits + pushes to `ae/<session>` branch, archives the session's
memory to `~/.ae/archive/<session-uuid>/`, then cleans up. The archive is MANDATORY on a
keep: capture happens after the verified stop and after git, and before any live state is
removed, so a failed archive fails the end with the whole session still on disk.
`--purge-history` inverts it — no archive is written and any existing one for that UUID is
deleted. `ae <new> --from <uuid>` starts a session that explicitly continues an archive;
lineage is never inferred from a name. See docs/internals/architecture.md.

## Session helpers

The core writes these scripts into `~/.ae/sessions/<name>/` for agents and humans to use.
Every one is a thin shim over a core entry — the names and the argv are the compatibility
contract, because every agent in a live workspace calls them by name.

| Helper | Purpose |
|--------|---------|
| `send <agent> <message>` | Deliver a message to another agent's pane (serialized with flock). Refuses a dead pane, defers on busy or human-typed input (claude/codex), verifies the submit, and fails loudly rather than dropping silently; framed bodies over 8192 bytes use a <=300-byte sender-owned `messages/*.txt` notice with pre-Enter row proof |
| `ask <agent> <question>` | Send a tracked request with a request ID and exact reply command |
| `review <agent> <request>` | Ask another agent for a critical review with findings-first output |
| `reply <request-id> <message>` | Reply to a logged `ask`/`review` by request ID. Verified against the request's stored **slot** (routing key), not the display name; `--as <agent>` is advisory display only |
| `requests [mine\|inbox\|all]` | Inspect pending and replied requests without peeking panes |
| `state <working\|waiting-user\|blocked\|done> [reason]` | Declare current work state (no args prints current). Shows in `ae list` (per agent + session `attn:` marker). The watchdog stops nudging on any quiet state: `done` (event-only), `waiting-user`/`blocked` (until the pane is touched) |
| `mark-done [message]` | `state done` with the message as its reason; `done` is the one value that also writes the legacy `done` event older watchdog processes consume |
| `say <text>` | Push a free-text line to the human's Telegram chat (args or piped stdin). Emits a `chat` event the bridge forwards; a Telegram reply routes back to the agent. The deliberate way to answer the human on Telegram — pane output is not forwarded |
| `memo add [--topic t] <text>` | Append durable shared session memory |
| `memo read [--topic t]` | Read shared session memory |
| `memo tail [n]` | Show latest memo entries |
| `goal [text\|--clear]` | The session's one-line objective. Stored as `goal=` in session meta (locked write, survives resume), shown in `ae list` (sub-line + JSON `goal` field), quoted by the watchdog's stale nudge. Emits a `goal` event on change |
| `peek <agent> [lines]` | Capture recent output from another agent's pane (default 80 lines; inspection only) |
| `peak <agent> [lines]` | Alias for `peek` (common typo) |
| `agents` | List all agents in the session with pane IDs and processes |
| `focus <agent>` | Switch tmux focus to another agent's pane |
| `interrupt <agent> [message]` | Cancel current generation, optionally send new instructions; oversized Claude/Codex messages use the same proven notice transport |
| `spawn <name> --using <profile> [prompt]` | Add a new agent to the workspace; oversized Claude/Codex task prompts use a sender-owned `messages/*.txt` notice |
| `retire <name>` \| `retire %pane` | Remove a spawned agent (kills pane, cleans meta, updates manifest); exact name only — `main`/`worker` seats refuse ("use 'ae end'") |

Every helper is core-owned end to end — name resolution, tmux server support, lock serialization, the event append, and the pane delivery `send` performs. The `_lib` library the helpers used to source is gone, and so is `_send-deliver`, the internal entry through which the core once called back into a bash send body. Name resolution supports the exact agent name, `%pane-id`, and cross-session `session:agent` / `@session:agent` syntax — alias-only and partial matching are gone. `agents --all` lists agents across all running ae sessions.

Two more shims sit in the same directory and are not agent-facing: `watchdog` and `events-tail` are the whole command of the two monitor panes, so each must be an executable file tmux can run rather than a shell line that would have to quote a core path. `loop` is the deprecated spelling of `watchdog`, kept as an alias for sessions created before the rename.

### How helpers are generated (thin shims)

There is no helper logic in bash to generate any more, so there is no generator either. One
writer emits the whole set — `src/session_launch/assets.rs`, reached at launch and again
through the core's `_shims-render <session-dir>`, which is what `ae doctor --refresh` runs.
A helper is four lines: the shebang, `set -euo pipefail`, its own directory derived from
`$0`, and `exec <core> <entry> "$META_DIR" "$@"`. `mark-done` prepends `done`; `peak` and
`loop` exec their sibling rather than re-deriving the directory, so the derivation has
exactly one home.

The `declare -f` template library that stood in `ae` — the column-0 template functions, the
`SYNC_SESSION_ASSETS_BODY` awk-source region, the `<TAG>PROLOGUE` heredocs, and the `_lib`
runtime they emitted — is **history**. It was the right shape while the logic was bash; it
has no subject now. The `set -euo pipefail` line survived the move on purpose: the directory
derivation above the `exec` is a command substitution, and a failed `cd` must not reach the
core as an empty first argument.

Each shim is published temp + chmod + rename, the shape `_publish_executable_artifact`
froze: a writer that dies mid-write leaves the previous artifact whole rather than a
truncated executable the session would then run. A session missing a helper is not a
session, so the first artifact that cannot be published fails the launch and the caller
rolls back — agents never start unable to talk to each other.

The `_publish_executable_artifact` chokepoint in `ae` and the unit guard that policed its
raw alternatives (`chmod`, `install -m` and friends in command position) went with the last
bash writer of an executable artifact. The contract they enforced is unchanged and now lives
in one Rust function: generate to a temp beside the destination, set the mode there, rename.
What the guard could never see is worth remembering if a bash writer ever returns — an
executable bit acquired without a mode word (`cp -p` from an executable source, a permissive
umask, `install` with no `-m`, an artifact that chmods itself at run time) was always outside
its reach. The chokepoint was the contract; the guard was partial enforcement of it.

The set is pinned twice. `tests/unit` asserts the refreshed directory holds **exactly** the
core's list — never `>= N`, because exactness is what makes an artifact quietly appearing or
vanishing a failure rather than a different number — and `tests/integration` pins the bytes
`doctor --refresh` writes against the bytes the core writes at launch, so the two entries
cannot drift. A refresh replaces on-disk scripts only: a **running** watchdog or Telegram
daemon keeps the process it already is until it is stopped and started. See
docs/development.md for the test-side details.

## Agent tool capabilities

ae supports multiple coding agent CLIs. They differ significantly in session handling, resume, and prompt injection. This table documents the actual behavior ae relies on — know it before modifying agent launch/resume code.

| Capability | Claude Code | Codex | Gemini CLI | Grok Build | OpenCode |
|---|---|---|---|---|---|
| **System prompt injection** | `--append-system-prompt 'text'` | `-c developer_instructions='text'` | `-i 'text'` | None (append-style) — context rides the POSITIONAL `[PROMPT]` argv. `--system-prompt-override` (and its 0.2.118+ compat alias `--system-prompt`) REPLACES grok's own agent prompt (breaks its tooling) — never use either | `OPENCODE_CONFIG=<per-slot json>` whose `instructions` array is loaded as system-level content (merges with the operator's config; arrays concatenate) |
| **Session ID at launch** | `--session-id UUID` (set by ae) | None (no flag exists) | None (launch token only; no launch-time UUID flag) | `--session-id UUID` (set by ae) | None (no flag exists) |
| **Session ID capture** | Immediate (ae generates UUID upfront) | Post-launch, four links tried in order: the id file codex's own first task writes, a launch-token scan of `~/.codex/sessions/<day>/*.jsonl`, a cwd scan of the same files, then the TUI header | Post-launch via local chat history scan (`~/.gemini/tmp/.../chats/session-*.json`) | Immediate (ae generates UUID upfront) — no launch token, no `_register-sid` handshake | Post-launch via `opencode session list --format json` (directory + launch-time match). The launch-token DB scan is inert: it looks for `AE_OPENCODE_LAUNCH_ID` in a message part, and nothing is pasted any more (measured: instructions content does not reach message parts) |
| **Resume with exact session** | `--resume UUID` | `codex <flags> resume UUID` (`resume` is a subcommand) | `--resume UUID` on current CLI; `ae` falls back to `--resume latest` when uncaptured | `--resume UUID` | `--session ID` (e.g. `ses_...`) |
| **Resume fallback** | `--continue` (CWD heuristic) | Fresh start (drop `resume UUID`, keep flags) | `--resume latest` | `--continue` (CWD heuristic) | `--continue` (last session) |
| **Concurrent session safety** | Full — UUID-scoped | Partial — the self-registration handshake and launch tokens reduce collisions, but fallback CWD matching is still heuristic | Partial — UUID-scoped once captured; fallback `--resume latest` remains heuristic when uncaptured | Full — UUID-scoped | Partial — `--session ID` is UUID-scoped once captured; fallback CWD matching remains heuristic |
| **Config flags preserved on resume** | Yes (flags stay, `--resume` appended) | Yes (flags before `resume` subcommand) | Yes (flags stay, `--resume` appended) | Yes (flags stay, `--resume`/`--continue` appended) | Yes (flags stay, `--session` appended) |
| **TUI modelled for delivery** | Yes — input-busy + staged detection; bracketed paste measured 2026-08-30 (plain head loss 4/4, bracketed 0/6, receiver byte-exact) | Yes — busy detection (staged detection: see input-region work) | No | No (v1) — sends deliver unprotected: no typed-input protection, no staged detection, no throttle patterns until its TUI is observed live | No |
| **Launch-time input-ready detection** | Idle-input sensor; no separate start-up marker observed | `model: loading` header **and** `Starting MCP servers (n/m)` progress line are NOT-ready markers (measured 2026-08-15, v0.147.0: input box drawn at t=0.5s, settled t=3.0s) | None — falls back to the composed-UI grep, but nothing rides on it: context arrives via OPENCODE_CONFIG, not a paste | **EXEMPT — no readiness detection.** Its TUI is unmodelled, so launch delivery is ungated and a slow start can land a paste into an unready pane. Accepted risk, not a pretend gate | None — falls back to the composed-UI grep |
| **`launch.<slot>.sh` re-run** | Resumes — first run creates, later runs `--resume` the same UUID | Starts FRESH (no id to bake, so nothing collides — the conversation is simply lost) | Starts FRESH | Resumes — same as claude | Starts FRESH |

**Key constraints to know:**
- **An idle input box is not an initialized application.** Every tool whose task rides a *paste*
  has a launch window where the box is drawn but the tool cannot yet act on input, and a fixed
  post-launch delay is a guess at its width. ae gates both delivery moments — the spawn task and
  the launch/resume prompt — on `_spawn_input_ready`, which asks `_tool_initializing` FIRST: a
  tool that is provably still starting is not ready however idle its box looks. Codex measured
  cold (2026-08-15, real MCP config): `model: loading` at t=0.5s with the box already drawn,
  `Starting MCP servers (0/7 … 4/7)` to t=2.5s, settled at t=3.0s — the old predicate answered
  READY from t=0.5s, so the bounded wait succeeded on its first poll and never waited.
  The markers are NEGATIVE: their absence is not proof of readiness, because a predicate that
  demands a positive banner breaks every spawn the day a tool stops printing one.
  Timeout is a LOUD, DURABLE failure — the text is preserved next to the session and an event is
  emitted — because launch delivery runs detached, where stderr reaches a pane nobody reads.
  NOT evidence: codex's `⚠ MCP startup interrupted` banner appears on its own (measured at t=3.0s
  in a run where no key was ever sent). It is a terminal state, not a sign of interrupted input.

- tmux 3.4 escapes control characters in `-F` output as octal; tmux 3.5+ emits them raw; the watchdog pane-listing format uses a printable separator with the free command field last (measured 2026-08-30).
- Codex has no `--session-name` or `--session-id` flag. The only way to get its UUID is post-launch (from `~/.codex/sessions/YYYY/MM/DD/*.jsonl` filenames). ae works around this by instructing codex via `developer_instructions` to run `<session-dir>/_register-sid` as its first action, and the core polls for the `codex.<slot>.sid` file that handshake writes before falling back to the scans. **Measured 2026-09-03: `_register-sid` is no longer written into the session directory** — it left the helper set with the `declare -f` template library, and the unit suite's exact-set assertion records that. The instruction still names it, so the handshake link is currently dead and capture relies on the scans below it. Fix the pair together, or drop the instruction.
- Every capture scan is filtered by the seat's recorded `launch_time.<slot>`, so a stale conversation in the same directory cannot be captured as this one. Capture runs in its own detached process — a tool that takes half a minute to answer must not delay the attach — and a capture child that dies before its tool answers is not a lost seat: the watchdog takes ONE look at each pending seat per cycle and registers whatever it finds. No sleeping, no polling; the next tick is the retry.
- Gemini persists a local `sessionId` in `~/.gemini/tmp/<project>/chats/session-*.json`, and current Gemini CLI accepts `--resume <UUID>` in addition to `latest`/index. ae now captures that UUID via launch-token scan and uses exact resume when available; fallback remains `--resume latest` if capture fails.
- OpenCode is TUI-only with no system-prompt FLAG, but it is no longer paste-driven. ae writes a per-slot config + context file into the session meta dir and launches `env OPENCODE_CONFIG=<meta>/opencode.<slot>.json opencode …`; the config's `instructions` array is loaded as system-level content, so ae context is present in EVERY turn rather than decaying as a first user message. Measured on 1.18.18: a marker present only in the instructions file is recited back by the model; the config MERGES with the operator's (mcp/provider/model survive) and instruction arrays CONCATENATE, so their own entries are kept — this is not the grok `--system-prompt-override` trap. Consequence: launch-time readiness is off the critical path for context, and the pasted `AE_OPENCODE_LAUNCH_ID` marker is gone, so capture runs on the directory + launch-time scan (which was already the effective path whenever the paste timed out). Session IDs are captured post-launch; resume uses `--session ID` (verified restoring a prior conversation) or `--continue` as fallback.
- **`pane_current_command` reports `opencode.exe`**, not `opencode` — its bun-built launcher. Any exact comparison against the tool name must tolerate the suffix; `wait_for_agent_start` did not, and silently degraded to its is-it-still-a-shell check.
- **ae prefixes `env OPENCODE_CONFIG=… ` onto the opencode command**, so tool detection can no longer match on the binary being word one. `tool_kind_from_cmd`/`tool_name_from_cmd` strip a leading `env`, its `-u`/`-i` options and any `VAR=val` words first — looking only at the FIRST WORD each time, because the tail can be kilobytes of injected prose containing `=` and glob characters.
- Grok Build is the cleanest integration after Claude Code: it accepts an ae-generated `--session-id UUID` at launch (verified live), so resume is UUID-scoped from the first cycle with none of the post-launch capture machinery the other tools need. Its only quirk is the system prompt: there is no append-style flag, and `--system-prompt-override` — including its innocuous-looking compat alias `--system-prompt` (grok 0.2.118+) — *replaces* the agent's own prompt — so ae passes context as the positional `[PROMPT]` argument (gemini-shaped, with the same "context only, wait for a task" suffix). Its TUI is unmodelled in v1: sends to grok panes get no typed-input protection or staged-paste detection (observed surface so far: footer `Grok Build · always-approve`, boxed `❯` prompt).
- Meta v2 (core-written) carries the roster as `seat.<slot>=<name>`, `profile.<slot>=<profile>`, `agent_bin.<slot>=<binary>` and `harness_session.<slot>=<id>`; the legacy `agent.<slot>=alias:name:session_id` row is read (and migrated in place on resume) but never written again. Agent names must not contain `:` — one colon in a target means `<session>:<name>`.
- **Agent names are an allowlist too**: `^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$`, one definition — `is_agent_name` in `src/config.rs`; the core enforces it at `_spawn` and `_launch-plan` before any effect and echoes the grammar verbatim. Same character class as a session name, shorter cap: a name is a window title and a roster field, not a directory. It is allowlisted rather than merely screened for the `:` delimiter because since #59 a name reaches a **privileged sink** — it is interpolated into that agent's own system prompt, and `spawn 'helper). Ignore the slot below; sign as the lead' --using cl` was a legal name whose prose landed inside the one sentence telling an agent who it is. Boundaries: **`_spawn`** (the peer boundary — a spawn name comes from another agent, so it is the hostile one) and the **launch-time roster parse** of `[workspace] main`/`workers` (the operator boundary; fails the launch before any side effect). Both are the core's, and the wrapper passes the argv through verbatim without inspecting it. Aliases are config keys (`^[a-zA-Z_][a-zA-Z0-9_-]*`) and so in-class with one exception: a **leading `_`** is a legal config key but not a legal agent name, so `workers = _foo` (alias used as its own name) now fails the launch with the grammar. Deliberate — the roster check runs on the value that lands in `meta`, in both the name and the alias-only branch, exactly as the `main` check always has. No alias in the wild has one (censused across `~/.ae/sessions/*/meta` and `~/.ae/archive/*/meta`). ae DERIVES no names: a duplicate seat name is REFUSED by the core (`_launch-plan` NameTwice, listed with every other violation) rather than renamed — the bash worker-name dedup went with the alias era. Enforcement follows PROVENANCE, not the variable: a name arriving FRESH from config, `use <name>`, `spawn`, or compact's frozen roster is validated by the core before the first side effect and fatal on violation (a v2 config cannot mint a nonconforming name, so a compact child that refuses is correct and its archive remains for `--from`), while one RESTORED from saved meta on resume is handed back verbatim by `_roster list` and left to the interpolation guard — refusing restored input would make a pre-grammar session unresumable. `build_ae_context` re-validates **both halves** at the interpolation site and stays **fail-quiet** — meta is a file that predates the grammar and is hand-editable, so a non-conforming roster entry yields no identity line rather than a hostile one, and the agent still launches.
- **Session names are an allowlist**, enforced at **every boundary where a name is created, imported, or mutated**: `^[A-Za-z0-9][A-Za-z0-9_-]{0,127}$`. The grammar has ONE statement since slice Z1, and it echoes it verbatim in its error: `is_session_name` in `src/session_launch/name.rs`. The bash `_validate_session_name` went with `ae-glue`; the wrapper inspects no name at all, and the core guards the name the caller TYPED ahead of a launch's side effects. A name is simultaneously a tmux session, a directory under `~/.ae/sessions/`, part of the `.lifecycle.<name>.lock` filename, and the target of the launch rollback's `rm -rf` — so it is allowlisted rather than filtered. The boundaries: **launch entry** (`ae [name]`, before the first tmux or filesystem side effect), **`default_session_name`** (which *guarantees* the grammar for any PWD rather than being checked against it), and **`rename`** (target strict). `ae transfer` was a fourth until it was cut rather than ported. Consumers of an *existing* session use `_session_name_usable`, which also accepts a legacy name that is already a real direct-child directory — a migration path out of pre-grammar names, never a route to traversal. `ae end`/`ae stop` resolve through session lookup rather than raw path construction (measured), so they are not name boundaries. Widen only on a real name in the wild, never on speculation.
- `launch.<slot>.sh` is **re-runnable** for the upfront-UUID tools. `--session-id` is create-once, so a human who exits the TUI and arrow-ups the script used to hit "Session ID … is already in use". The script now drops a `launch.<slot>.started` marker on its first run and `exec`s the `--resume` variant on every later one; ae clears the marker whenever it rewrites the script, so a fresh launch always creates. The decision happens BEFORE exec deliberately: a `cmd || fallback` chain would leave bash as the pane's process and `pane_current_command` would report `bash` instead of the tool, silently disabling the send path's TUI modelling (measured — and the reason today's `claude --resume … || --continue` resume launches already read as `bash`). The post-launch-capture tools have no baked id to collide with; re-running their script starts a fresh conversation instead of erroring, which is why they are out of scope rather than fixed.

## Rust era (`main`)

Read before editing anything Rust. **The files are the contract** — `rust-toolchain.toml`,
`Cargo.toml`, `clippy.toml`, `deny.toml`, `taplo.toml`, `.cargo/`, and the `rust-*` block of
the justfile — and each carries its reasoning at the pin site. This section is the map and
the *why*, not a second source of truth: when they disagree, the file wins and this section
is stale.

### Finishing #79 (ruled 2026-09-03)

Epic #79 was closed on the claim that "the remaining Bash is minimal tmux/pane glue". It
was not measured: at that close `ae` was 18,673 lines beside 25,191 lines of production
Rust, and every core-owned domain still carried a bash fallback (`AE_CORE=` set-empty, the
not-attempted 125 protocol) or a duplicate reader of the same session state — where 10 of
the 11 identity-slice bugs lived. The human ruled the destination the epic already named:
**B — the Rust binary calls `tmux` itself; bash keeps only generated `launch.<slot>.sh`
and the interactive helper shims.** Path: ordered deletion slices (A.0 suites self-isolate
→ A.1 delete every no-core fallback, core REQUIRED → A.2 readers to the core, four
sub-slices → A.3 watchdog helper execs `_watchdog-run` → A.4 launch, transitional → A.5
cut `transfer`, move doctor), then one vertical move per remaining operation (send
delivery, spawn/retire, launch, end/compact effects, watchdog pane). Do not port bash into
Rust as it is; port behaviour, drop features on the way.

Development runs in a separate namespace, **`ae-dev`** (`~/.local/bin/ae-dev`: own
`~/.ae-dev` home and config, tmux server `-L ae-dev`, binaries from the checkout). The
released `ae` keeps serving `~/.ae` and its v1 sessions untouched; the new line never reads
old state (P5's fresh start, applied again), so no compat code and no upgrade preflight.

Cycle rules under this work: scoped integration runs in the inner loop, one full pass per
slice; commit on lint + unit + scoped green, the full pass and the single cross-model round
run after the commit on the integrated diff (nothing is pushed); fixes roll forward.

**Status (measured 2026-09-04, after slice Z1).** The destination is reached, and then
some: `ae-glue` is DELETED. What is left of ae's Bash is the public wrapper `ae-entry` at
737 lines, 433 of them code — down from the 18,673 the epic closed on — plus the generated
`launch.<slot>.sh` scripts and the four-line session shims. There is no dispatcher, no
help text, no name grammar, no session-path guard, no config writer, no INI parser, no
portability shim and no reader of ae's own state in bash; the one exception is stated at
its site, the `upgrade` report, which must answer "which sessions are running" about an
install it is replacing. Every fallback is gone: a core that cannot be bound is a refusal
(exit 2, before any side effect) rather than a degraded path.

What the wrapper still does is the pair validation, `version` and `upgrade`, the `bash >= 4`
re-exec ladder, the core binding, the `--inside-tmux` sensor, the typed tmux server pair,
and ONE exec of the core with the frozen preamble followed by `--` and the caller's argv
verbatim. The file's own header carries the list of ABSENCES the suite asserts. Read it
before adding anything back.

Cut rather than ported: `ae transfer`, `ae status` (`ae list` answers the same question from
one implementation), the `ae orchestrator`/`hub` scaffold trampoline, and `_recover-pending`
(the core recovers pending tool session ids in-process on every watchdog cycle). `status` and
`orchestrator`/`hub` keep a REFUSING arm rather than being deleted outright, because
everything past the dispatcher falls through to a launch and a launch takes the last
positional as a session name — a bare `ae status` would otherwise create a session called
`status`. `_*` fails closed for the same reason.

### Toolchain: pins, not channels

| What | Pin | Declared in |
|---|---|---|
| Compiler | `1.97.1` (exact release, not `stable`) | `rust-toolchain.toml` |
| Edition / MSRV | `2024` / `rust-version = "1.97.1"` | `Cargo.toml` |
| Profile + components | `minimal` + rustfmt, clippy, llvm-tools | `rust-toolchain.toml` |
| Targets | `aarch64-apple-darwin` (native), `x86_64-unknown-linux-musl` (cross) | `rust-toolchain.toml`, justfile `RUST_CROSS_TARGET`, `deny.toml [graph]` |
| Dev tools | cargo-nextest `0.9.143`, taplo-cli `0.10.0`, cargo-deny `0.20.2`, cargo-mutants `27.1.0`, cargo-llvm-cov `0.9.0` | justfile `*_VERSION` variables — the single source |
| `just` | `1.57.0` | justfile `JUST_VERSION` — a prerequisite `rust-setup` cannot install (you cannot run the recipe that installs the tool running the recipe). CI **reads** the pin out of the justfile rather than restating it |

- **`stable` is a channel, not a pin.** CI, laptop and agent sandboxes must resolve to the
  same compiler, or a lint that gates a commit here is a lint that does not gate it there.
  Bump deliberately, in its own commit.
- **llvm-tools is pinned although only the coverage lane uses it**: cargo-llvm-cov otherwise
  `rustup component add`s it on demand (measured) — a report lane silently mutating a pinned
  toolchain, and needing the network to do it.
- **Dev-tool pins live in the justfile and nowhere else** — all six, `JUST_VERSION`
  included, so a bump changes one line and re-keys the CI tool cache (whose key greps the
  `*_VERSION` lines). `rust-setup` version-checks each as a **whole word** before installing
  (a prefix match accepts `0.9.1430`) and installs with `cargo install --locked --version`.
- **`--locked` on every graph-consuming lane.** Without it cargo happily *updates*
  `Cargo.lock` to satisfy a build and then reports green — a committed lockfile no lane
  enforces is decoration. `cargo fmt` is the one exception: it does not resolve the graph.
  Two spellings are not interchangeable, both measured: cargo-deny takes it as a **global**
  option (`cargo deny --locked check`; `cargo deny check --locked` exits 2), and
  cargo-mutants does not accept it at all — it is passed through as `--cargo-arg=--locked`.

### Lanes

| Recipe | What it is |
|---|---|
| `just rust-setup` | bootstrap: toolchain + pinned tools. Idempotent — a second run installs nothing |
| `just rust-check` | **the gate**: `rust-fmt-check` + `rust-lint` + `rust-test` |
| `just unit` / `just itest <domain>` / `just itest-all` | the bash inner loop: fast unit default (~1 min; `AE_UNIT_FULL=1` for all), one domain of integration sections (seconds), the parallel full pass (~4 min). `just test` stays the serial full set |
| `just rust-fmt` / `rust-fmt-check` | `cargo fmt` + `taplo fmt` (both languages of the build) |
| `just rust-lint` | `cargo clippy --locked --all-targets --all-features -- -D warnings` + `taplo lint` |
| `just rust-test` | `cargo nextest run --locked` **and** `cargo test --doc --locked` |
| `just rust-deny` | supply chain: advisories, licenses, bans, sources (`cargo deny --locked check`) |
| `just rust-mutants` | does the suite discriminate, or does it merely pass? (`--cargo-arg=--locked`). CI runs it **bounded to the pushed range** per push (`--in-diff`, minutes); the full lane is a manual dispatch with `mutants=true` under a 6-hour budget — a full run is hours and never fit the push job (since 2026-08-30). **Known gap, named in the step:** in-diff selects source mutants only, so any change to tests or test infrastructure (Cargo, toolchain pin, tool configs, justfile, the workflow) can weaken a test or drop a lane unseen — with or without a `src/` change; the job detects that inventory first, annotates and summarises it, then still mutates any `src/` change. The mutation steps run LAST so a broad change cannot starve the musl evidence. Closing it needs a runnable full lane, i.e. a scheduler/dispatch stub on the default branch (`main`) — a decision for the human, since it touches the frozen branch. No schedule here: GitHub fires schedules from the default branch only |
| `just rust-cov` | coverage **report**, not a gate |
| `just rust-build-release` | native release binary (`--locked`) + foreign-target compile smoke |
| `just rust-watch` | optional bacon loop; bacon is deliberately not part of the bootstrap |
| `.github/workflows/release.yml` | tag-triggered prebuilt bundles: static musl Linux + native Apple Silicon macOS, checksum manifest, GitHub Release upload |

Coverage becomes a gate the day a threshold is ratified, and not before. An unratified
number that blocks a merge is a number nobody agreed to.

### Prebuilt ae distribution

Release tags matching `v[0-9]*.[0-9]*.[0-9]*` build their own release binaries with
`--locked` on pinned `ubuntu-24.04` and `macos-15` runners. The Linux artifact is
proven static (`PT_INTERP` absent and `file` reports static) and runs both `_net-probe`
controls against the shipped binary: the reserved `.invalid` name must refuse and
`api.telegram.org` must resolve. Each bundle contains three members — the public `ae`
wrapper, `ae-core`, and the canonical
`install`; the final job emits one `SHA256SUMS` over both
tarballs. The installer downloads files, verifies the checksum before extraction, and
atomically publishes the complete matched set under one immutable version directory.
Local unit tests use fixture bundles and never access the network.

**The canonical installer takes no path overrides.** It installs to `~/.ae/versions` and
`~/.local/bin/ae`, derived from `HOME` and nothing else. Fixed publication paths avoid
aliasing with legacy state; persisted journals are hostile input and are refused, then
preserved for diagnosis, when their pointers or command path disagree.

**The `AE_NEXT_HOME` retirement executed on 2026-08-31, and slice Z1 gave it a second
shape.** The coexistence wrapper and its advanced state-write override no longer ship.
The wrapper now decides its environment contract from ITS OWN NAME, which is the same
discriminator that decides how the core is bound:

- **Installed (published as `ae`)** — unsets inherited `AE_CORE`, `AE_TMUX_SERVER`,
  `AE_TMUX_SERVER_KIND` and `AE_CORE_BIN`, and unconditionally sets `AE_HOME=$HOME/.ae`
  with `CONFIG_FILE=$AE_HOME/config`, ignoring inherited `AE_HOME` and `AE_NEXT_HOME`. One
  aggregated notice names whichever inherited values were set and would have changed
  behavior.
- **Checkout (the tracked `ae-entry`)** — HONOURS `AE_HOME`, `CONFIG_FILE` and
  `AE_CORE_BIN`, defaulting `AE_HOME` to `$HOME/.ae`. That is the `ae-dev` namespace
  (`~/.local/bin/ae-dev`: own `~/.ae-dev`, own tmux server, core from the checkout build)
  and the two bash suites, which are its only callers.

Both shapes EXPORT `AE_HOME` and `CONFIG_FILE`: the preamble's `--home` feeds the launch
and the telegram daemon, but the core derives its state root for every other command from
the environment. There is no coreless mode anywhere — a core that cannot be bound is a
refusal (exit 2, before any side effect), never a degraded path.

**Command ownership (ruled 2026-08-31; reduced to two wrapper-owned words in slice Z1):**
except `upgrade` and the three version spellings, every invocation passes the validated
PAIR first (wrapper `_AE_ENTRY_VERSION` == `ae-core --version`) and then reaches the core
as one exec. Before that exec the wrapper unsets `AE_VERSION`; the operator pin is scoped
solely to `upgrade` and cannot freeze into a session.

| Invocation | Owner / gate order |
|---|---|
| `upgrade` | immutable sibling `install`, before the pair gate so repair survives breakage; caller `AE_VERSION` is the target pin |
| `version`, `--version`, or `-V` | public wrapper `_AE_ENTRY_VERSION`, before the pair gate |
| every other invocation, `_*` and a bare `ae` included | Rust core, after the validated pair, as `ae-core <preamble> -- <argv verbatim>` |

The frozen preamble is the wrapper's whole vocabulary, and every flag in it is a fact the
core cannot see for itself:

```
ae-core --home H --cwd C [--global G] [--local-config L] \
        [--server-kind K --server V] [--inside-tmux] [--bash-major N] \
        [--attach|--no-attach] [--no-autostart] -- <user argv...>
```

Not passed, each absence a decision: `--pane` (the core reads `TMUX_PANE` itself now that
it IS the entry process), `--core` and `--core-version` (the core is `current_exe()`,
which under both shapes is exactly the binary the wrapper resolved and exec'd).

**The server pair is read by SET, not by nonempty**, in both places that read it —
`resolve_launch_tmux_server` and the tmux shim. `AE_TMUX_SERVER_KIND=ambiguous
AE_TMUX_SERVER=` is the shape the socket probe mints for a relative socket path it could
not prove, and a `${VAR:-}` test read that set-empty half as an absent one: the pair was
dropped, no `--server-kind` reached the preamble, and the core resolved the AMBIENT server
— the one outcome `ambiguous` exists to prevent. EITHER variable being set now resolves
the pair, BOTH halves cross verbatim, and the core issues the refusal. A pair that cannot
be routed makes the shim refuse every tmux call rather than fall back, so bash never asks
a server the caller did not name.

**Core binding.** An INSTALLED bundle binds its own immutable sibling `ae-core` — proven
a regular, non-symlink, executable member, proven `-ef` the published `core/current`
pointer, and proven to report the wrapper's own version — so no operator variable can
outrank the core the bundle shipped with. A CHECKOUT binds `AE_CORE_BIN`, then
`<AE_HOME>/core/current`, requiring only that the result is an absolute path to an
executable that reports a version: the checkout's core is built from the tree beside the
wrapper, and a developer one bump behind is owed a working `ae-dev` rather than a refusal.

### Lint policy: `[lints]` + `-D warnings`

- **`unsafe_code = "forbid"`** — the hard line. There is no scoped exception worth having in
  a session multiplexer that shells out to tmux.
- clippy `all` and `pedantic` at `warn`, `priority = -1`.
- **`unwrap_used` / `expect_used` are `warn`, not `deny`** — `-D warnings` in `rust-lint`
  gives the same gate strength, and `deny` would break a scoped, documented `#[allow]`. The
  consequence is what matters: a production `.expect()` fails `just rust-check` (proven).
  These two close the exact escape hatch an agent reaches for when the type system gets
  inconvenient.
- **Tests relax it in exactly one place**: `clippy.toml`
  (`allow-unwrap-in-tests` / `allow-expect-in-tests` / `allow-panic-in-tests`). A test that
  maps an error to a fallback instead of panicking hides the failure it exists to report.
  Keep the rule where it belongs — do not scatter `#[allow]` through the suite.

### Tests: nextest, kept doctests, and mutants

- `rust-test` runs **two** commands: `cargo nextest run --all-features` and
  `cargo test --doc --all-features`. **nextest does not run doctests.** Doctests are KEPT —
  they are the executable half of the public docs. Deleting that second line silently
  retires a whole lane.
- **One integration-test target** (`[[test]] name = "it"`, `tests/it/main.rs` + `mod`
  submodules): one binary to link, one home for shared helpers, no per-file target explosion.
- **cargo-mutants is the agent-specific lane.** Agents write tests that *pass*; a green suite
  is not evidence it would ever go red. `.cargo/mutants.toml` runs nextest (one test tool,
  one set of semantics) with a timeout multiplier against the measured baseline, so a hanging
  mutant cannot hang the lane. Doctests are out of this lane — documentation coverage is not
  mutation coverage. Acceptance is **non-vacuous**: at least one viable mutant exists and is
  caught, plus a control run against a deliberately weakened test that reports a missed
  mutant — a lane that cannot fail proves nothing.

### Supply chain: cargo-deny with committed policy

**The policy is the check.** cargo-deny with a default config asserts almost nothing, so
`deny.toml` is committed and every clause is a decision:

- **advisories** — `yanked = "deny"`, `unmaintained = "all"`, `ignore = []`. Every future
  ignore entry carries a reason and a reference; an empty list is the only state that needs
  no justification.
- **licenses** — permissive allow-list only. ae is MIT and ships as one binary, so a copyleft
  dependency changes the distribution contract and needs a deliberate exception, not a silent
  pass.
- **bans** — `wildcards = "deny"` (pins-not-channels applies to dependencies too);
  duplicate versions `warn`, because a duplicate is a smell worth seeing, not a defect worth
  blocking.
- **sources** — crates.io or nothing. A git dependency has no yank, no advisory mapping and
  no immutable version.
- **cargo-audit is deliberately absent**: cargo-deny covers RustSec directly, so a second
  lane is duplication with a second failure mode.
- `just rust-deny` no longer passes `--allow license-not-encountered`: the flag went with the
  first real dependency (2026-08-29). The allow-list is now MINIMAL-TO-ENCOUNTERED, and a
  license-not-encountered warning is the signal that it drifted — which is the point of
  dropping the flag.

Dependencies arrive **with the feature that needs them**, never in the skeleton: "no error
dependency exists until a real error does" generalises. `Cargo.lock` is committed.
**Trigger:** cargo-fuzz is required *before* any hostile persisted-state parser cuts
over — recorded so it is not rediscovered late.

### Dependency posture: the researched line (2026-08-24)

Two model families researched crate reuse independently (gemini 3.7 flash and grok 4.6,
online sources, no coordination) and converged on every material verdict. Full reports with
sources: `docs/research/rust-sota-agy.md` and `docs/research/rust-sota-grok.md` — version
numbers, dependency counts, and advisory IDs live THERE, not here; re-verify them at
adoption time, because this table records decisions, and the facts under them age.

**The line, in one sentence: std owns every byte we currently write; cross it for TLS HTTP,
for fuzzing the hostile parser, and for auditing the TLS graph — not for clap, thiserror,
or chrono.** Zero-dep is doctrine with recorded triggers, not dogma.

| Surface | Verdict | Trigger to revisit |
|---|---|---|
| CLI parsing (`cli.rs`) | **Keep from-scratch.** clap fights our grammar and costs ~half a MiB; pico-args is stale and parses in arbitrary order (breaks SC-521) | Subcommand growth: adopt **lexopt** (0-dep lexer), never clap |
| JSON (`json.rs`) | **Keep from-scratch** — it enforces SC-510d's escape set, SC-506 infallible rendering, SC-511b forward tolerance; serde_json's `Map`/`Result`/`Number` shapes fight all three | If cargo-fuzz shows the *grammar* is the expensive part: serde_json as **lexer only**, our `Value`+renderer kept. Telegram-API JSON was the one point the two families split (scoped serde_json vs keep ours); the bridge ships on ours |
| Errors (`error.rs`) | **Keep the single enum.** anyhow rejected outright (type erasure + advisory history) | Variant explosion: **thiserror** (compile-time only), preferably in the same commit as the first runtime dep |
| Unix/fs | **Keep std.** rustix/nix would sit in front of `read_dir` and punch the clippy capability boundary, whose deny is premised on empty dep tables | flock, signals or unix sockets: **rustix**, not nix (advisory history) |
| Time | **Keep std.** jiff pre-1.0; chrono and time both carry advisory history; our contract rows refuse the tolerance those crates sell | A real timezone/calendar need, none foreseen |
| HTTP (telegram) | **ureq + rustls**, `json` feature OFF, native-tls OFF, pin the ring CryptoProvider (musl static). attohttpc disqualified on license alone (MPL-2.0 vs deny.toml) | Adopted with the telegram bridge (2026-08-29) |
| Daemon concurrency | **OS threads + mpsc.** Two to four background loops do not justify an async runtime | Only if the loop count changes shape |

**Costs of the first `[dependencies]` row, all three recorded so they are paid knowingly:**
drop `--allow license-not-encountered` from `rust-deny` (already recorded above); the
clippy `disallowed_types` capability boundary loses its empty-dep-tables premise and
degrades toward a naming convention — **cargo-vet arrives in the same change** to replace
what it loses.

That last cost was paid at P4.3, and paying it taught something the aspiration
above ("ring, for TLS, is exactly the one to vet first") got wrong: **the TLS
graph turned out to be un-auditable in place.** No import registry (mozilla,
google, zcash, isrg — all four are wired) certifies ring `0.17.14`, rustls
`0.23.35` or ureq `3.4.0`; there is no trusted-publisher path to any of them; and
ring is 261k lines of crypto/asm no agent here can credibly attest. So cargo-vet
ships with that graph **EXEMPTED, not audited** — and the exemptions **honestly
encode an accepted risk, not an audit claim.**

**RISK ACCEPTANCE (P4.3, project-local, 2026-08-29) — recorded, NOT gated.** The
pinned `ureq 3.4.0`, `rustls 0.23.35`, `ring 0.17.14` and their locked transitive
graph are accepted as they stand; tracer B goes live on this acceptance after code
review, and is deliberately NOT gated on a future certification that may never
arrive. The acceptance rests on: established,
maintained crates; an exact lockfile; `default-features` off; rustls-only TLS
(native-tls off); an explicitly named ring provider; egress locked (proxy off,
https-only, no redirects, finite timeouts); cargo-deny advisory gating; and a
cross-model dependency review — set against the fact that no agent here can
credibly attest 261k lines of crypto/asm and no registry certifies these exact
versions. **Revisit triggers:** any version change, a new advisory, a feature
expansion, or an upstream registry certification becoming available — replace the
exemption with an imported audit the moment one is possible. cargo-deny gates the
whole graph for advisories in the meantime.

**Std replaces crates since our MSRV** — use these, never the crate equivalents:
`std::process::ExitCode`/`Termination` (1.61) over raw `exit()`, `std::io::IsTerminal`
(1.70) over atty/is-terminal, `core::error::Error` (1.81) for no-std-shaped error trees.

**Dev-lane roadmap:** cargo-fuzz before the first hostile persisted-state parser (already doctrine, above); **proptest** as dev-dep
beside it for parser round-trip invariants; **cargo-vet** with the first runtime dep
(orthogonal to cargo-deny — do not add cargo-audit, that duplication is already refused
above); **skip insta** (snapshot tests would pin unratified digest order — the exact
over-pinning class criterion 15 polices); **cargo-semver-checks remains unused** — revisited and
resolved keep-unused per [rust-sota-grok.md](docs/research/rust-sota-grok.md#cargo-semver-checks-0500--keep-unused), because ae is unpublished CalVer product code with no external library consumer or compatibility promise. Adopt it only if the crate is published or gains a real external library consumer. Candidate clippy hardening lints to evaluate in their own quiet
slice, not mid-flight: `cast_possible_truncation`, `cast_sign_loss`, `dbg_macro`, `todo`,
`unimplemented`, `panic_in_result_fn` — evaluation means running them against the tree,
not appending them to the table.

### Fresh-clone reproducibility

The bootstrap contract, in full — nothing else is assumed to exist:

**`rustup` + `just` installed → `just rust-setup` → `just rust-check` green.**

- `rust-setup` is idempotent, and the workflow **asserts it** (proven, run 32350969851): a
  second run that installs anything fails the build. Idempotence is an acceptance
  criterion, not a courtesy.
- **`.cargo/config.toml` must hold on a bare clone.** No sccache, no alternative linker, no
  brew. `build.rustc-wrapper` becomes a hard requirement the moment it is written — a clone
  without sccache then fails to build. Machine-local speedups belong in
  `~/.cargo/config.toml`, which cargo merges on top.
- CI (`.github/workflows/rust.yml`) **runs that contract** on `ubuntu-24.04` and
  `macos-15` — pinned runner images, not `-latest`, for the same reason the compiler is
  pinned — with `fail-fast: false` so one platform's failure cannot cancel the other's
  evidence. Actions are first-party and SHA-pinned, version in a trailing comment.
- **Proven by run 32350969851 (2026-08-20, green on both platforms):** bootstrap contract,
  idempotence assert, every push lane (the mutation lane is diff-bounded per push since
  2026-08-30 — see the table), native `ae --version` + exit-code proof on real
  x86_64 Linux and arm64 macOS, and the **static musl binary built, run, and asserted
  static on Linux** (artifact `ae-linux-x86_64-musl`). **That run was zero-dependency: the
  musl target then linked on stock `ubuntu-24.04` with no extra packages.** That is no longer
  true — the first dependency (`ring`, 2026-08-29) compiles C for the target, so the Linux leg
  now installs `musl-tools`. **Re-proven by run 33323387544 (2026-08-30, green on both
  platforms):** the musl artifact built, linked, ran and asserted static-pie under that step,
  and the `_net-probe` DNS proof ran on it for the first time (see "The linux target is
  musl" below).
- The bash-era lanes are deliberately **not** wired there yet (blocked on the gate-integrity
  issues #58/#67); adding them now would publish a red badge for a known, separately-tracked
  gap.

### The linux target is musl

- **musl, not gnu.** The epic promises a static zero-dep binary; a glibc build is dynamically
  linked against the build host's libc and is not the artifact ae's one-file install contract
  describes.
- **The musl build needs a C toolchain as of the first dependency (2026-08-29, P4.3).**
  `ring`'s build script compiles C for the target, so a musl `cargo check`/`build` is no
  longer link-free — it needs `x86_64-linux-musl-gcc`. Consequences, recorded so they are not
  rediscovered: (1) the musl compile-smoke was **removed** from `just rust-build-release`,
  which is now native-only — forcing every clone to install a from-source macOS cross
  toolchain to run one recipe is the wrong trade; (2) the musl artifact is BUILT, LINKED, RUN
  and proven static on the **Linux CI leg only** (`.github/workflows/rust.yml`), which
  installs `musl-tools` for exactly this reason and is the only place musl can link anyway;
  (3) macOS no longer touches the target at all. The static proof itself is unchanged — no
  `PT_INTERP` in `readelf -l` (authoritative), `file` must say static and never "dynamically",
  `ldd` informational — and it was re-proven under the `musl-tools` step by run
  33323387544 (2026-08-30). Local musl checking now costs a cross toolchain; the Linux CI
  leg is the proof of record.
- **NSS caveat — flagged for P4 (daemons), and now LIVE: the Telegram bridge resolves
  `api.telegram.org`.** musl has no NSS: user, group and host lookups do not consult
  `/etc/nsswitch.conf`, so `getpwuid`/`getaddrinfo` behave differently than under glibc —
  LDAP/SSSD-backed users and some resolver setups resolve differently or not at all. Tracer A
  is dormant (no live DNS yet), but wiring it (tracer B) makes a static-musl `getaddrinfo`
  against a real host the first place this can bite. The design's musl-DNS check (cost item 4)
  cannot run on the laptop — no musl toolchain — so it runs on the Linux CI leg: the static
  musl binary's `_net-probe api.telegram.org` answered `ok 2` on `ubuntu-24.04` (run
  33323387544, 2026-08-30), after a reserved `.invalid` name refused as the negative control.
  That proves the plain `/etc/resolv.conf` path from a static musl binary, not an NSS-only
  host (LDAP/SSSD/mDNS names) — the residual is host-specific and stays recorded here.

### Code shape

- **One crate, no workspace.** Dead-code analysis stops at crate boundaries, and cross-crate
  dead code is the agent "reinvention" failure mode. Split on measured need, not on taste.
- **2018-edition module style**: `cli.rs` beside a future `cli/`, never a `mod.rs`.
- **`main.rs` is thin** — argv in, exit code out, presentation of the one top-level error.
  Everything testable lives in the library, because a binary is not a unit-testable thing.
- **Errors**: start with one top-level presentation error; a domain error type is permitted
  where recovery or semantics differ. A single crate-wide enum is a default, not law.
- **Exit codes are contract.** `0` success, `2` usage error — kept distinct from `1` so a
  caller can tell "you asked wrong" from "it went wrong". The workflow asserts them on both
  platforms (proven, run 32350969851).

### Deferred, with the trigger recorded

- **Version scheme.** `_AE_ENTRY_VERSION` in `ae-entry` and Cargo's package version are
  unified NOW, deliberately pre-promotion: both use SemVer-compatible CalVer `YYYY.M.N`.
  `just bump` derives N from matching Git tags (`vYYYY.M.N`), resets monthly, refuses
  duplicate tags, and updates `ae-entry`, `Cargo.toml`, and `Cargo.lock` together. The third
  version word, `AE_VERSION` in `ae-glue`, went with that file in slice Z1 — the install is
  now a PAIR, and the wrapper unsets `AE_VERSION` before every exec of the core so an
  operator pin cannot freeze into a session.
- **`panic = "abort"`** in the release profile forecloses `catch_unwind`. Revisit if the
  long-lived watchdog or telegram loop needs to survive a panic in one iteration rather than
  take the process down. Cheap to flip; recorded so it stays a decision.
- **`cliff.toml` is excluded from taplo** — a bash-era file whose reformat would be an
  unrelated diff in a frozen area. It joins the lane when someone reformats it deliberately.

## Bash hazards for the surviving wrapper (read before editing `ae-entry`, bundled as `ae`)

Every bug class below has shipped at least once. Check new code against both lists.

*Scope:* this section governs the policy-frozen public wrapper `ae-entry` and the
generated `launch.<slot>.sh` scripts — what is left of ae's Bash after slice Z1 deleted
`ae-glue`. It does not govern the session helpers: those are four-line shims that exec the
core, with no logic to get wrong.
Several entries below name a function or a command that the glue cuts and slice Z1
removed; they are
kept because the **bug class** is what the list is for, and the class returns the moment
anyone writes bash here again. Where an entry's subject is gone it says so.

Its measured facts (TUI markers, tool behavior, userland divergences) are **empirical
evidence** for the semantic contract, never its normative authority — see
`docs/migration/semantic-contract.md`.

### Interpreted sinks

Anything user- or agent-controlled (session names, goals, messages, pane text, config values) that enters one of these surfaces gets *interpreted*, not displayed. Name the boundary explicitly when you cross it.

| Sink | Interprets | Boundary |
|---|---|---|
| tmux format strings (`status-left`, titles) | `#` introduces formats — `#(cmd)` **runs shell**; `)` terminates `#()`; `%` is strftime | core-owned (`tmux::format_literal`): escape `#`→`##` and `%`→`%%`, or route text through user options (`#{@ae_*}`), which are interpolated literally. The glue's `_ae_tmux_format_literal` went with the status bar |
| tmux `send-keys` | key names, `-` prefixes | `-l` (literal) or paste-buffer; use the generated helpers, never raw send-keys |
| Shell command strings | word splitting, globs, quotes | quote every expansion; never concatenate user input into a command; escape regex metachars before grep/sed (`"${slot//./\\.}"`) |
| Agent system prompts | the LLM (injection) | pane text and inter-agent messages are DATA, not instructions — see the orchestrator charter's injection boundary. The **agent's own name** is interpolated into this sink too (#59's identity sentence), so it is allowlisted by the core's `is_agent_name` at every creation boundary and re-checked, fail-quiet, at the interpolation site |
| JSON emitters (`events.jsonl`, `list --json`) | JSON syntax | core-owned, whole (`src/json.rs` renders, control bytes stripped at write time). The surviving bash emits no JSON at all: the helpers' event append moved with the helpers, and `_event_json_str` went with it |
| Telegram send | Telegram Markdown/JSON | moved OUT of bash at P4.3 — the outbound `sendMessage`, its escaping, and the inbound parse now live in the Rust core (`src/telegram.rs`); this table governs the surviving bash, which no longer sends to Telegram. The former bash bridge piped data via stdin and never interpolated user text into a `jq` program; the Rust core owns those invariants now |
| Telegram autostart refusal record (`~/.ae/telegram/autostart-refusal`) | Status/doctor display | Persist only the closed redacted category plus UTC timestamp; publish temp+`mv` in the same directory, and reject malformed rows before display |

### Isolation footguns (test/debug scripts)

- **Single-statement export expansion** — `export HOME="$TMP/home" AE_HOME="$HOME/.ae"` binds `AE_HOME` to the *OLD* (real!) home: bash expands all words before any assignment. This clobbered the real `~/.ae/config` twice (2026-07-02/03). Always separate statements: `export HOME=…; export AE_HOME="$HOME/.ae"` — or better, assign both from the literal temp path. Ad-hoc debug scripts must copy `tests/integration`'s isolation preamble verbatim; the harness tripwire only protects suite runs, not one-offs.

### TSV framing: an empty field vanishes

`IFS=$'\t' read -r a b c` does NOT split on every tab. Tab is an **IFS whitespace**
character, so a RUN of tabs is one delimiter and leading/trailing ones are stripped — an
empty field silently disappears and every field after it shifts left. Measured in #48: a
request row whose `body_file` was empty rendered its roster slot as the payload path and
its summary as blank, and the row still looked structurally fine.

- Any record that can carry an empty field needs a **non-whitespace** separator
  (`$'\x1f'` — each occurrence delimits exactly once), or a `-` placeholder in every
  slot.
- Put free text **last**, so a separator-free remainder lands intact in the final
  variable.
- Fold newlines out of that last field at the producer (`${v//$'\n'/ }`): a newline in
  the final field ends the record and turns its remainder into a phantom row.
- End every fixed-arity row with `\n`. `read` returns 1 at EOF-without-delimiter, so a
  BARE `read … < <(producer)` under `set -e` aborts — invisible while every call site
  sits behind `$( )`, `||` or `if` (see the bare-call rule below).

### GNU vs BSD userland

ae runs on Linux (GNU coreutils) and macOS (BSD). The divergences below all
**fail silently** through the `|| fallback` idiom — the command errors, the
fallback value lands, and the feature reads as "nothing found" instead of
"broken". Every row shipped as a macOS bug.

**There is no shim any more, and the rule that replaced it is simpler: the core
reads ae's state, bash does not.** Every shim — `_ae_tac`, `_ae_stat`,
`_ae_epoch`, `_ae_sed_inplace`, `_ae_json_first`, `_ae_yesterday` — went with its
last caller in the glue cuts, because the readers that needed them are Rust now
and std has no BSD/GNU split. The table stays because the divergences are still
facts, and they bind in two places: any bash that comes back here, and — far more
often — the ad-hoc command you are about to run yourself.

**This table governs the shell YOU are typing into, not only the script's
code.** It reads as a product-behavior spec, so the ad-hoc command you run
during a session does not feel like its subject — and two agents in one day
reached for GNU-only `timeout`/flags on macOS while actively citing rows from
this table for the product. Before running a measurement or probe, check your
one-off command against the same rows you would check a diff against — and
note the class's expensive member is SILENT: a missing `timeout` is loud at
rc=127, but `tac`, `stat -c` and `date -d` fail quietly through the very
fallback idiom above, so the cost is not a broken command but a wrong number
in an evidence report.

| Raw (GNU-only) | BSD form |
|---|---|
| `tac` | `tail -r` |
| `stat -c %Y/%s/%i/%u/%a/%y` | `stat -f %m/%z/%i/%u/%Lp/%Sm` |
| `date -d <iso>` | `date -u -j -f <fmt>` |
| `sed -i EXPR FILE` | BSD reads EXPR as the backup suffix — temp + rename instead |
| `grep -oP '"k"\s*:\s*"\K…'` | no `-P`, no `\K` — `grep -oE` + `head -1` + `sed` |

Same class, and they were never shimmed:

- **`sed` BRE alternation `\(a\|b\)` is a GNU extension.** BSD matches nothing.
  Use `sed -E` with `(a|b)`; `-E` is portable.
- **`wc` pads its count with leading spaces on BSD.** Anything that string-compares
  or regex-guards the result (`[[ $n =~ ^[0-9]+$ ]]`) breaks. Strip: `| tr -d '[:space:]'`.
- **`uuidgen` is UPPERCASE on macOS.** ae's UUIDs and agent session filenames are
  lowercase-only, so a raw `uuidgen` value has to be normalised before it is compared
  against one. The core generates its own and never shells out for one.
- **`/proc` does not exist.** Walk process parents with `ps -o ppid= -p <pid>`,
  not `/proc/<pid>/stat` (which also field-shifts on a comm containing a space).
- **`timeout`, `flock` are absent by default.** Both are optional in ae: guard with
  `command -v` and degrade, never hard-require.
- **Unix socket paths must fit `sun_path`** — 104 bytes on macOS, 108 on Linux.
  macOS `mktemp -d` alone eats ~48 of them, so a socket under `$TMPDIR` can exceed
  the limit; tests use a short `/tmp/...` dir.
- **`getent` is glibc-only.** macOS answers passwd lookups via `dscl`.
- **`touch -d <human date>` is GNU-only.** `touch -t [[CC]YY]MMDDhhmm[.SS]` works on both.

### `set -e` footguns

The script runs under `set -euo pipefail` (line 3). Exit codes you didn't think about become aborts.

- **Query functions must end `return 0` explicitly.** Their result is stdout; the exit status is incidental — but a bare call or `x="$(fn)"` under `set -e` kills the whole command. Shipped exhibit: `_agent_alert_reason` fell through with status 1 and truncated `ae list --json` mid-array.
- **`[[ cond ]] && cmd` as a function's (or loop body's) last statement** returns 1 when the condition is false. Add `|| true` or restructure.
- **Long emitters must not abort mid-output.** A bash loop that prints a document — `ae list`'s JSON array was the exhibit — must bracket itself with `set +e` … `set -e` so one bad record degrades instead of truncating the snapshot. No such emitter is left in bash: the wrapper execs the core and the core renders the array, where a partial document is not a reachable state.
- **`local x="$(fn)"` swallows the exit status** (`local` returns 0); split declaration and assignment when you need the status — and remember the split form re-arms `set -e`.
- **Producers in process substitution** end silently: `< <(cmd)` doesn't abort the reader — guard with `|| true` only when failure is genuinely optional.
- **`set -u` + associative arrays:** subscripting an *undeclared* array is an arithmetic eval on the key → abort on non-numeric refs. `declare -A map=()` before any lookup (see the stopped-sessions JSON path).
- **Only a BARE call proves `set -e` safety — a green suite does not.** A failing command aborts *only* in statement position: `$(fn)`, `if fn`, `fn && …`, `fn || …` all mask it. `tests/unit` runs `set -euo pipefail` and still cannot catch this, because it reaches these functions exclusively through `$(…)` in `assert_eq` arguments. Entry points used to differ — `ae` enabled errexit and the send-path helpers did not — so the same function was safe through one caller and aborted through another. Every generated helper now sets `-euo pipefail` and holds one `exec`, so the wrapper's own functions are the only remaining subject. Probe it *bare* under `set -euo pipefail`; that is the only shape that shows it. Shipped exhibit: `_sgr_parse`'s `((line++))` yields the *old* value, so it returns 1 at zero and a bare call aborts mid-parse — invisible through five review rounds until a new caller (`_cmd_spawn`, under `ae`'s errexit) reached it. (`((x++))` → `x=$((x + 1))`.)
- **A probe built to detect an errexit abort must not put the subject in a context that
  disables errexit.** The note above tells you `||`, `&&` and `if` mask errexit. It does
  not tell you that your TEST for masking will itself be masked — and `( subject ) ||
  echo "it died"` is the natural way to write that test, so **the instinct that makes you
  write the probe is the instinct that breaks it**. Shipped exhibit: a probe of exactly
  that shape reported `SURVIVED` for an assignment that provably kills a backgrounded
  subshell, ten minutes from reverting the fix that made a test pass. Probe in the shape
  the caller actually uses (`( … ) &` plus a marker file, or a bare call), and verify the
  instrument answers correctly for a KNOWN failure before trusting it about an unknown
  one. Note the direction of the error: a masked probe always reports success, so it
  argues your change was unnecessary — **a biased instrument is worse than a noisy one**,
  because the reading it biases toward is the one that ends the investigation.
- **A backgrounded fixture that aborts is invisible AND misattributing.** Under `set -e`,
  a failed assignment inside `( … ) &` stops the subshell with no output and no status
  anyone reads; the failure then surfaces in whatever the fixture was feeding and blames
  the product. Shipped exhibit: `v="$(awk … events.jsonl)"` polled a file that does not
  exist until a session emits its first event, so the fixture died on iteration one and a
  handover test reported "the reply did not arrive" 40s later, through four wrong
  hypotheses. Give a backgrounded helper its own error surface — errexit's only report is
  a process that is no longer there.

## Config

```toml
[profiles]
profile = "shell command"

[roster]
name = profile

[workspace]
main = name
workers = name, name2      # optional, omit for single-agent start
layout = vertical

[prompt]
instructions = "Custom instructions injected into agent system prompts"
```

That's it. Don't extend the format.
