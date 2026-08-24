# ae

Single bash script. No dependencies beyond bash and tmux. Keep it that way.

> **Branch `rust-rewrite` (epic #79).** Bash `ae` is **frozen at `72c7293`** and shrinks to
> tmux/pane glue; the state core, lifecycle and daemons become one Rust binary. This file is
> **additive** for the duration: every bash section below still governs the frozen glue and
> stays authoritative until its domain flips. The single-file / pure-bash rules retire at
> **P5**, not now. Rust-era rules: [Rust era](#rust-era-rust-rewrite-branch). Direction and
> phases: [VISION.md](VISION.md).

## Philosophy

- ae is a thin wrapper around tmux — not a framework, not a platform.
- The goal is **daily productivity**, not feature completeness. If it doesn't save time on every use, it doesn't belong.
- Resist adding features. If tmux already does it, don't re-implement it.
- One file does everything. Don't split into modules or libraries.
- No build steps, no package managers, no abstractions.
- Simplicity is the feature. The entire tool must remain understandable in one sitting.

*Scope on `rust-rewrite`:* the first, second, third and sixth bullets are durable — they
describe what ae **is**, and the Rust core inherits them unchanged (a thin wrapper, daily
productivity, resist features, understandable in one sitting). "One file does everything"
and "no build steps" describe the bash **implementation**: they govern the frozen glue and
retire with it at P5. A crate with modules is not a violation of the doctrine; it is the
doctrine applied to a language where one file is not how you stay readable.

## Rules

- `ae` must remain a single bash script. No compiled languages, no runtimes. *(A decision, not dogma — see "Revisit triggers" below.)* **Scope on `rust-rewrite`: this now governs the frozen bash glue — the tracked `ae` script and its generated helpers — until the P5 entry flip. The Rust binary is the ratified consequence of triggers 1–3 firing, not a violation of this rule.**
- Config is INI-style with a simple regex parser. Don't add TOML/YAML/JSON parsing.
- Core ae requires only `bash >= 4.0`, `tmux`, and `git`. Optional features may declare their own hard dependencies (e.g. `ae telegram` needs `jq` + `curl`), but those deps must never be required for the rest of ae to work — `ae list`, `ae <name>`, etc. continue to function on a machine without them.
- Session state lives in `~/.ae/sessions/`; archived session memory lives in
  `~/.ae/archive/<session-uuid>/` and is INERT — data only, never an executable file.
  Working directories stay clean.
- No AI tool attribution in commits.
- Keep the script lean. If it's getting bloated, cut, don't add.

## Revisit triggers

The single-file / pure-bash / tmux-runtime contract is a *decision with reasons*, not dogma. Re-evaluate it when a trigger fires — and only then:

1. **The bash bug tax recurs.** Two or more shipped bugs of the `set -e`/escaping class *after* the hazards checklist and the declare-f testability refactor landed → doctrine failed; move the affected component to a typed language.
2. **State outgrows bash.** Core ae needs real data structures (nested, typed, or concurrent state), or a sidecar needs to *write* ae's state rather than read it → extract that component (the aemonitor precedent: Python sidecar in `contrib/`, optional dep).
3. **The product changes shape.** The long-lived daemon side (watchdog, steward, telegram) outgrows the tmux-wrapper side → that half becomes a proper sidecar/daemon (uv/PEP 723 single-file Python or a small Go/Rust binary), integrated via the install script and `ae doctor` checks, with bash kept for the tmux glue where it is best-in-class. (Direction already agreed for watchdog + telegram.)
4. **Someone besides the author uses it.** Contributor onboarding and packaging change the whole calculus — revisit everything above.

**Fired (2026-08-20).** Triggers 1, 2 and 3 all fired: the `set -e`/framing bug class kept
shipping *after* the hazards checklist existed, the events ledger and request/claim state
outgrew what bash can hold safely, and the daemon half outgrew the wrapper half. The ruling
is epic #79 — a Rust core, bash kept for the tmux glue where it is best-in-class. Trigger 4
has not fired; nothing here is packaged for contributors yet.

tmux as the runtime is no longer unchallenged: **herdr** (herdrdev/herdr, Rust, Apache-2.0, ~24k stars) is a credible agent multiplexer with its own renderer, agent-state sidebar, and a Unix socket API agents can drive programmatically — the first serious non-tmux substrate. It competes with ae's *plumbing*, not its coordination protocol or doctrine; a watchlist item, not a migration plan — migrate only when one of the triggers above fires, and if trigger 3 does, herdr's socket API is a candidate substrate to port the helpers onto. Watch alongside zellij's programmatic CLI (still no send-keys-stable API). Assessed 2026-08-03, cross-model research (secondary sources + repo metadata); read its source before any commitment.

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
Cargo.toml          — Rust package: one crate, bin + lib, both named `ae` (no workspace)
rust-toolchain.toml — compiler pin: channel, profile, components, both targets
clippy.toml         — the tests-only relaxation of the unwrap/expect rule
deny.toml           — supply-chain policy (advisories, licenses, bans, sources)
taplo.toml          — TOML fmt/lint scope
.cargo/             — repo-owned cargo config (aliases) + cargo-mutants config
src/                — Rust sources: main.rs (thin) + lib.rs (everything testable)
tests/it/           — the single integration-test target (main.rs + `mod` submodules)
.github/workflows/  — rust lanes, both platforms (bash lanes deliberately not wired yet)
README.md           — user docs
VISION.md           — what ae is, and where it is going
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

`ae end` (or `ae rm`) commits + pushes to `ae/<session>` branch, archives the session's
memory to `~/.ae/archive/<session-uuid>/`, then cleans up. The archive is MANDATORY on a
keep: capture happens after the verified stop and after git, and before any live state is
removed, so a failed archive fails the end with the whole session still on disk.
`--purge-history` inverts it — no archive is written and any existing one for that UUID is
deleted. `ae <new> --from <uuid>` starts a session that explicitly continues an archive;
lineage is never inferred from a name. See docs/internals/architecture.md.

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

Every generated **executable** artifact outside a session's helper set is published through the single chokepoint `_publish_executable_artifact <dest> <mode> <generator...>` — it generates to a temp, sets the mode there, and renames, so a generator that dies mid-write leaves the previous artifact whole. The generator is passed as a *command*, never piped in: a pipeline's producer can fail after emitting a prefix, and a downstream `cat` would publish that prefix. A unit guard forbids the raw alternatives it can recognise — `chmod`/`command chmod`/`/bin/chmod`/`chmod --`/`install [-m]` with a mode word, in command position — so those spellings cannot reappear. It does **not** see an executable bit acquired without a mode word (`cp -p` from an executable source, a permissive umask, `install` with no `-m`, a wrapper that chmods for its caller, an artifact that chmods itself at run time): the chokepoint is the contract, the guard is partial enforcement of it. Session helpers are exempt **by shape** (`chmod` on `${AE_META}/<name>.tmp.$$` whose *very next line* mv's it back under `${AE_META}`) because they already publish temp+chmod+mv and have their own guard family.

This makes helper logic unit-testable, shellcheck/shfmt-covered, and greppable. Unit guards enforce the invariants (emission-list completeness, one definition per emitted name, template-vs-emitted parity, and the whole section must source silently under `set -u`); a `doctor --refresh` integration canary runs regenerated artifacts end-to-end. A **running** watchdog keeps its loaded body until `watchdog stop`/`start` — refresh only replaces the on-disk script. See docs/development.md for the test-side details.

## Agent tool capabilities

ae supports multiple coding agent CLIs. They differ significantly in session handling, resume, and prompt injection. This table documents the actual behavior ae relies on — know it before modifying agent launch/resume code.

| Capability | Claude Code | Codex | Gemini CLI | Grok Build | OpenCode |
|---|---|---|---|---|---|
| **System prompt injection** | `--append-system-prompt 'text'` | `-c developer_instructions='text'` | `-i 'text'` | None (append-style) — context rides the POSITIONAL `[PROMPT]` argv. `--system-prompt-override` (and its 0.2.118+ compat alias `--system-prompt`) REPLACES grok's own agent prompt (breaks its tooling) — never use either | `OPENCODE_CONFIG=<per-slot json>` whose `instructions` array is loaded as system-level content (merges with the operator's config; arrays concatenate) |
| **Session ID at launch** | `--session-id UUID` (set by ae) | None (no flag exists) | None (launch token only; no launch-time UUID flag) | `--session-id UUID` (set by ae) | None (no flag exists) |
| **Session ID capture** | Immediate (ae generates UUID upfront) | Post-launch via `_register-sid` internal helper plus launch-token/file scan | Post-launch via local chat history scan (`~/.gemini/tmp/.../chats/session-*.json`) | Immediate (ae generates UUID upfront) — no launch token, no `_register-sid` handshake | Post-launch via `opencode session list --format json` (directory + launch-time match). The launch-token DB scan is inert: it looks for `AE_OPENCODE_LAUNCH_ID` in a message part, and nothing is pasted any more (measured: instructions content does not reach message parts) |
| **Resume with exact session** | `--resume UUID` | `codex <flags> resume UUID` (`resume` is a subcommand) | `--resume UUID` on current CLI; `ae` falls back to `--resume latest` when uncaptured | `--resume UUID` | `--session ID` (e.g. `ses_...`) |
| **Resume fallback** | `--continue` (CWD heuristic) | Fresh start (drop `resume UUID`, keep flags) | `--resume latest` | `--continue` (CWD heuristic) | `--continue` (last session) |
| **Concurrent session safety** | Full — UUID-scoped | Partial — `_register-sid` + launch tokens reduce collisions, but fallback CWD matching is still heuristic | Partial — UUID-scoped once captured; fallback `--resume latest` remains heuristic when uncaptured | Full — UUID-scoped | Partial — `--session ID` is UUID-scoped once captured; fallback CWD matching remains heuristic |
| **Config flags preserved on resume** | Yes (flags stay, `--resume` appended) | Yes (flags before `resume` subcommand) | Yes (flags stay, `--resume` appended) | Yes (flags stay, `--resume`/`--continue` appended) | Yes (flags stay, `--session` appended) |
| **TUI modelled for delivery** | Yes — input-busy + staged detection | Yes — busy detection (staged detection: see input-region work) | No | No (v1) — sends deliver unprotected: no typed-input protection, no staged detection, no throttle patterns until its TUI is observed live | No |
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

- Codex has no `--session-name` or `--session-id` flag. The only way to get its UUID is post-launch (from `~/.codex/sessions/YYYY/MM/DD/*.jsonl` filenames). ae works around this by instructing codex via `developer_instructions` to run the internal `_register-sid` helper script as its first action.
- Gemini persists a local `sessionId` in `~/.gemini/tmp/<project>/chats/session-*.json`, and current Gemini CLI accepts `--resume <UUID>` in addition to `latest`/index. ae now captures that UUID via launch-token scan and uses exact resume when available; fallback remains `--resume latest` if capture fails.
- OpenCode is TUI-only with no system-prompt FLAG, but it is no longer paste-driven. ae writes a per-slot config + context file into the session meta dir and launches `env OPENCODE_CONFIG=<meta>/opencode.<slot>.json opencode …`; the config's `instructions` array is loaded as system-level content, so ae context is present in EVERY turn rather than decaying as a first user message. Measured on 1.18.18: a marker present only in the instructions file is recited back by the model; the config MERGES with the operator's (mcp/provider/model survive) and instruction arrays CONCATENATE, so their own entries are kept — this is not the grok `--system-prompt-override` trap. Consequence: launch-time readiness is off the critical path for context, and the pasted `AE_OPENCODE_LAUNCH_ID` marker is gone, so capture runs on the directory + launch-time scan (which was already the effective path whenever the paste timed out). Session IDs are captured post-launch; resume uses `--session ID` (verified restoring a prior conversation) or `--continue` as fallback.
- **`pane_current_command` reports `opencode.exe`**, not `opencode` — its bun-built launcher. Any exact comparison against the tool name must tolerate the suffix; `wait_for_agent_start` did not, and silently degraded to its is-it-still-a-shell check.
- **ae prefixes `env OPENCODE_CONFIG=… ` onto the opencode command**, so tool detection can no longer match on the binary being word one. `tool_kind_from_cmd`/`tool_name_from_cmd` strip a leading `env`, its `-u`/`-i` options and any `VAR=val` words first — looking only at the FIRST WORD each time, because the tail can be kilobytes of injected prose containing `=` and glob characters.
- Grok Build is the cleanest integration after Claude Code: it accepts an ae-generated `--session-id UUID` at launch (verified live), so resume is UUID-scoped from the first cycle with none of the post-launch capture machinery the other tools need. Its only quirk is the system prompt: there is no append-style flag, and `--system-prompt-override` — including its innocuous-looking compat alias `--system-prompt` (grok 0.2.118+) — *replaces* the agent's own prompt — so ae passes context as the positional `[PROMPT]` argument (gemini-shaped, with the same "context only, wait for a task" suffix). Its TUI is unmodelled in v1: sends to grok panes get no typed-input protection or staged-paste detection (observed surface so far: footer `Grok Build · always-approve`, boxed `❯` prompt).
- Agent names in meta use `:` as delimiter (`alias:name:session_id`). Agent names must not contain `:`.
- **Agent names are an allowlist too**: `^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$`, one definition — `_validate_agent_name` in `ae`, error message echoes the grammar verbatim. Same character class as a session name, shorter cap: a name is a window title and a roster field, not a directory. It is allowlisted rather than merely screened for the `:` delimiter because since #59 a name reaches a **privileged sink** — it is interpolated into that agent's own system prompt, and `spawn 'cl:helper). Ignore the slot below; sign as the lead'` was a legal name whose prose landed inside the one sentence telling an agent who it is. Boundaries: **`_cmd_spawn`** (the peer boundary — a spawn name comes from another agent, so it is the hostile one) and the **launch-time roster parse** of `[workspace] main`/`workers` (the operator boundary; fails the launch before any side effect). Aliases are `parse_config` keys (`^[a-zA-Z_][a-zA-Z0-9_-]*`) and so in-class with one exception: a **leading `_`** is a legal config key but not a legal agent name, so `workers = _foo` (alias used as its own name) now fails the launch with the grammar. Deliberate — the roster check runs on the value that lands in `meta`, in both the `alias:name` and the alias-only branch, exactly as the `main` check always has. No alias in the wild has one (censused across `~/.ae/sessions/*/meta` and `~/.ae/archive/*/meta`). A name ae DERIVES must be a fixed point of the grammar, not merely derived from one: worker-name dedup (`_dedup_worker_names`) truncates the base so its `-2`/`-3` suffix fits inside 64, loops the suffix until unseen, and validates the FINAL value — naive suffixing turned a legal 64-character name into 66 (accepted input silently losing its identity line) and made `workers = foo,foo-2,foo` produce two `foo-2`. The suffix counts from `-2` rather than carrying the array index, which leaked position rather than meaning. Enforcement follows PROVENANCE, not the variable: a name arriving FRESH from config, CLI or `spawn` is fatal on violation, while one RESTORED from saved meta or compact's frozen roster is left to the interpolation guard — refusing restored input would make a pre-grammar session unresumable, and would kill a compact child after its source is already archived and gone. `build_ae_context` re-validates **both halves** at the interpolation site and stays **fail-quiet** — meta is a file that predates the grammar, survives `ae transfer`, and is hand-editable, so a non-conforming roster entry yields no identity line rather than a hostile one, and the agent still launches.
- **Session names are an allowlist**, enforced at **every boundary where a name is created, imported, or mutated**: `^[A-Za-z0-9][A-Za-z0-9_-]{0,127}$`. The grammar and the reasoning live in one place — `_validate_session_name` in `ae`; the error message echoes it verbatim. A name is simultaneously a tmux session, a directory under `~/.ae/sessions/`, part of the `.lifecycle.<name>.lock` filename, an rsync destination on both ends of `ae transfer`, and the target of the launch rollback's `rm -rf` — so it is allowlisted rather than filtered. The boundaries: **launch entry** (`ae [name]`, before the first tmux or filesystem side effect), **`default_session_name`** (which *guarantees* the grammar for any PWD rather than being checked against it), **`ae transfer`** (both directions, before any path, SSH probe, `mkdir`, or `rsync`), and **`ae rename`** (target strict). Consumers of an *existing* session use `_session_name_usable`, which also accepts a legacy name that is already a real direct-child directory — a migration path out of pre-grammar names, never a route to traversal. `ae end`/`ae stop` resolve through session lookup rather than raw path construction (measured), so they are not name boundaries. Widen only on a real name in the wild, never on speculation.
- `launch.<slot>.sh` is **re-runnable** for the upfront-UUID tools. `--session-id` is create-once, so a human who exits the TUI and arrow-ups the script used to hit "Session ID … is already in use". The script now drops a `launch.<slot>.started` marker on its first run and `exec`s the `--resume` variant on every later one; ae clears the marker whenever it rewrites the script, so a fresh launch always creates. The decision happens BEFORE exec deliberately: a `cmd || fallback` chain would leave bash as the pane's process and `pane_current_command` would report `bash` instead of the tool, silently disabling the send path's TUI modelling (measured — and the reason today's `claude --resume … || --continue` resume launches already read as `bash`). The post-launch-capture tools have no baked id to collide with; re-running their script starts a fresh conversation instead of erroring, which is why they are out of scope rather than fixed.

## Rust era (`rust-rewrite` branch)

Read before editing anything Rust. **The files are the contract** — `rust-toolchain.toml`,
`Cargo.toml`, `clippy.toml`, `deny.toml`, `taplo.toml`, `.cargo/`, and the `rust-*` block of
the justfile — and each carries its reasoning at the pin site. This section is the map and
the *why*, not a second source of truth: when they disagree, the file wins and this section
is stale.

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
| `just rust-fmt` / `rust-fmt-check` | `cargo fmt` + `taplo fmt` (both languages of the build) |
| `just rust-lint` | `cargo clippy --locked --all-targets --all-features -- -D warnings` + `taplo lint` |
| `just rust-test` | `cargo nextest run --locked` **and** `cargo test --doc --locked` |
| `just rust-deny` | supply chain: advisories, licenses, bans, sources (`cargo deny --locked check`) |
| `just rust-mutants` | does the suite discriminate, or does it merely pass? (`--cargo-arg=--locked`) |
| `just rust-cov` | coverage **report**, not a gate |
| `just rust-build-release` | native release binary (`--locked`) + foreign-target compile smoke |
| `just rust-watch` | optional bacon loop; bacon is deliberately not part of the bootstrap |

Coverage becomes a gate the day a threshold is ratified, and not before. An unratified
number that blocks a merge is a number nobody agreed to.

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
- `just rust-deny` passes `--allow license-not-encountered` **only because the crate has zero
  dependencies**. Remove it at the first real dependency — from then on the warning is the
  signal that the allow-list drifted.

Dependencies arrive **with the feature that needs them**, never in the skeleton: "no error
dependency exists until a real error does" generalises. `Cargo.lock` is committed.
**Trigger:** cargo-fuzz is required *before* any hostile persisted-state parser cuts over
(a P2/P3 entry condition, recorded so it is not rediscovered late).

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
| CLI parsing (`cli.rs`) | **Keep from-scratch.** clap fights our grammar and costs ~half a MiB; pico-args is stale and parses in arbitrary order (breaks SC-521) | Subcommand growth (P5): adopt **lexopt** (0-dep lexer), never clap |
| JSON (`json.rs`) | **Keep from-scratch** — it enforces SC-510d's escape set, SC-506 infallible rendering, SC-511b forward tolerance; serde_json's `Map`/`Result`/`Number` shapes fight all three | If cargo-fuzz shows the *grammar* is the expensive part: serde_json as **lexer only**, our `Value`+renderer kept. Telegram-API JSON at P4 is an **open choice** (the one point the two families split: scoped serde_json vs keep ours) — decide it then |
| Errors (`error.rs`) | **Keep the single enum.** anyhow rejected outright (type erasure + advisory history) | Variant explosion: **thiserror** (compile-time only), preferably in the same commit as the first runtime dep |
| Unix/fs | **Keep std.** rustix/nix would sit in front of `read_dir` and punch the clippy capability boundary, whose deny is premised on empty dep tables | flock / signals / unix sockets (P2/P4): **rustix**, not nix (advisory history) |
| Time | **Keep std.** jiff pre-1.0; chrono and time both carry advisory history; our contract rows refuse the tolerance those crates sell | A real timezone/calendar need, none foreseen |
| HTTP (P4 telegram) | **ureq + rustls**, `json` feature OFF, native-tls OFF, pin the ring CryptoProvider (musl static). attohttpc disqualified on license alone (MPL-2.0 vs deny.toml) | Arrives WITH the telegram daemon, not before |
| Daemon concurrency (P4) | **OS threads + mpsc.** Two to four background loops do not justify an async runtime | Only if the loop count changes shape |

**Costs of the first `[dependencies]` row, all three recorded so they are paid knowingly:**
drop `--allow license-not-encountered` from `rust-deny` (already recorded above); the
clippy `disallowed_types` capability boundary loses its empty-dep-tables premise and
degrades toward a naming convention — **cargo-vet arrives in the same change** to replace
what it loses; and the first dep graph (ring, for TLS) is exactly the one to vet first.

**Std replaces crates since our MSRV** — use these, never the crate equivalents:
`std::process::ExitCode`/`Termination` (1.61) over raw `exit()`, `std::io::IsTerminal`
(1.70) over atty/is-terminal, `core::error::Error` (1.81) for no-std-shaped error trees.

**Dev-lane roadmap:** cargo-fuzz at P2 (already doctrine, above); **proptest** as dev-dep
beside it for parser round-trip invariants; **cargo-vet** with the first runtime dep
(orthogonal to cargo-deny — do not add cargo-audit, that duplication is already refused
above); **skip insta** (snapshot tests would pin unratified digest order — the exact
over-pinning class criterion 15 polices); **cargo-semver-checks at P5** (publish=false,
version 0.0.0 until then). Candidate clippy hardening lints to evaluate in their own quiet
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
  idempotence assert, all lanes, native `ae --version` + exit-code proof on real
  x86_64 Linux and arm64 macOS, and the **static musl binary built, run, and asserted
  static on Linux** (artifact `ae-linux-x86_64-musl`) — the musl target links on stock
  `ubuntu-24.04` with no extra packages.
- The bash-era lanes are deliberately **not** wired there yet (blocked on the gate-integrity
  issues #58/#67); adding them now would publish a red badge for a known, separately-tracked
  gap.

### The linux target is musl

- **musl, not gnu.** The epic promises a static zero-dep binary; a glibc build is dynamically
  linked against the build host's libc and is not the artifact ae's one-file install contract
  describes.
- **On a laptop the cross target is a compile smoke only** — `cargo check` never links, so
  nothing produced here can be mistaken for a runnable artifact. Target-native proof belongs
  on that target's own machine. The CI ubuntu leg is **configured** to upgrade it — build,
  run, and assert static: no `PT_INTERP` segment in `readelf -l` (authoritative), `file` must
  say static and must never say "dynamically", `ldd` informational only. A missing `readelf`
  fails loudly rather than skipping the assertion, because a skipped assertion is a vacuous
  gate. Configured, not yet observed — see the pending-first-run note above.
- **NSS caveat — flagged for P4 (daemons).** musl has no NSS: user, group and host lookups do
  not consult `/etc/nsswitch.conf`, so `getpwuid`/`getaddrinfo` behave differently than under
  glibc — LDAP/SSSD-backed users and some resolver setups resolve differently or not at all.
  Nothing in today's surface depends on it; the daemons that land in P4 might.

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

- **Version scheme.** Bash CalVer (`AE_VERSION`) and Cargo semver (`0.0.0`) are *not*
  reconciled, and `ae --version` prints the Cargo one. Unify at the **P5 entry flip** — the
  moment the two can no longer disagree without a user seeing it.
- **`panic = "abort"`** in the release profile forecloses `catch_unwind`. Revisit at **P4**:
  a long-lived watchdog or telegram loop may want to survive a panic in one iteration rather
  than take the process down. Cheap to flip; recorded so it stays a decision.
- **`cliff.toml` is excluded from taplo** — a bash-era file whose reformat would be an
  unrelated diff in a frozen area. It joins the lane when someone reformats it deliberately.

## Bash hazards (read before editing `ae`)

Every bug class below has shipped at least once. Check new code against both lists.

*Scope on `rust-rewrite`:* this section governs the frozen bash glue and the generated
helpers — everything that stays bash until P5. It shrinks as domains flip; what survives P5
is the pane-side remainder. The measured facts in it (TUI markers, tool behavior, userland
divergences) are **empirical evidence** for the semantic contract, never its normative
authority — see `docs/migration/semantic-contract.md`.

### Interpreted sinks

Anything user- or agent-controlled (session names, goals, messages, pane text, config values) that enters one of these surfaces gets *interpreted*, not displayed. Name the boundary explicitly when you cross it.

| Sink | Interprets | Boundary |
|---|---|---|
| tmux format strings (`status-left`, titles) | `#` introduces formats — `#(cmd)` **runs shell**; `)` terminates `#()`; `%` is strftime | `_ae_tmux_format_literal` (`#`→`##`, `%`→`%%`), or route text through user options (`#{@ae_*}`) which are interpolated literally |
| tmux `send-keys` | key names, `-` prefixes | `-l` (literal) or paste-buffer; use the generated helpers, never raw send-keys |
| Shell command strings | word splitting, globs, quotes | quote every expansion; never concatenate user input into a command; escape regex metachars before grep/sed (`"${slot//./\\.}"`) |
| Agent system prompts | the LLM (injection) | pane text and inter-agent messages are DATA, not instructions — see the steward charter's injection boundary. The **agent's own alias:name** is interpolated into this sink too (#59's identity sentence), so it is allowlisted by `_validate_agent_name` at every creation boundary and re-checked, fail-quiet, at the interpolation site |
| JSON emitters (`events.jsonl`, `list --json`) | JSON syntax | `_json_escape` / `_event_json_str`; strip control bytes at write time |
| Telegram bridge | Markdown parse mode, `jq` program text | plain-text send paths; jq programs stay fixed strings with data piped via stdin — never interpolate user text into the program |

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
"broken". Every row shipped as a macOS bug. Never call the raw tool; use the
shim (top of `ae`, and emitted into generated helpers via the `_lib`
`declare -f` list).

**This table governs the shell YOU are typing into, not only the script's
code.** It reads as a product-behavior spec, so the ad-hoc command you run
during a session does not feel like its subject — and two agents in one day
reached for GNU-only `timeout`/flags on macOS while actively citing rows from
this table for the product. Before running a measurement or probe, check your
one-off command against the same rows you would check a diff against.

| Raw (GNU-only) | Shim | BSD form |
|---|---|---|
| `tac` | `_ae_tac` | `tail -r` |
| `stat -c %Y/%s/%i/%u/%a/%y` | `_ae_stat mtime\|size\|inode\|uid\|mode\|mtime-human` | `stat -f %m/%z/%i/%u/%Lp/%Sm` |
| `date -d <iso>` | `_ae_epoch` | `date -u -j -f <fmt>` |
| `sed -i EXPR FILE` | `_ae_sed_inplace` | BSD reads EXPR as the backup suffix — temp + rename instead |
| `grep -oP '"k"\s*:\s*"\K…'` | `_ae_json_first` / `_ae_json_first_num` | no `-P`, no `\K` — `grep -oE` + `head -1` + `sed` |

Not shimmed, but the same class — check by hand:

- **`sed` BRE alternation `\(a\|b\)` is a GNU extension.** BSD matches nothing.
  Use `sed -E` with `(a|b)`; `-E` is portable.
- **`wc` pads its count with leading spaces on BSD.** Anything that string-compares
  or regex-guards the result (`[[ $n =~ ^[0-9]+$ ]]`) breaks. Strip: `| tr -d '[:space:]'`.
- **`uuidgen` is UPPERCASE on macOS.** `_transfer_validate_uuid` and agent session
  filenames are lowercase-only. `gen_uuid` normalises — don't call `uuidgen` directly.
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
- **Long emitters must not abort mid-output.** A loop that prints a document (e.g. `cmd_list`'s JSON array) brackets itself with `set +e` … `set -e` so one bad session degrades instead of truncating the snapshot. That region is guarded by a structural unit test — don't remove it.
- **`local x="$(fn)"` swallows the exit status** (`local` returns 0); split declaration and assignment when you need the status — and remember the split form re-arms `set -e`.
- **Producers in process substitution** end silently: `< <(cmd)` doesn't abort the reader — guard with `|| true` only when failure is genuinely optional.
- **`set -u` + associative arrays:** subscripting an *undeclared* array is an arithmetic eval on the key → abort on non-numeric refs. `declare -A map=()` before any lookup (see the stopped-sessions JSON path).
- **Only a BARE call proves `set -e` safety — a green suite does not.** A failing command aborts *only* in statement position: `$(fn)`, `if fn`, `fn && …`, `fn || …` all mask it. `tests/unit` runs `set -euo pipefail` and still cannot catch this, because it reaches these functions exclusively through `$(…)` in `assert_eq` arguments. Entry points differ too — `ae` and the generated `spawn`/`retire` helpers enable errexit, the send-path helpers (`_lib`, `send`, `ask`, …) do not — so the same function is safe through one caller and aborts through another. Probe it *bare* under `set -euo pipefail`; that is the only shape that shows it. Shipped exhibit: `_sgr_parse`'s `((line++))` yields the *old* value, so it returns 1 at zero and a bare call aborts mid-parse — invisible through five review rounds until a new caller (`_cmd_spawn`, under `ae`'s errexit) reached it. (`((x++))` → `x=$((x + 1))`.)
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
