# ae

One Rust core in one immutable versioned install, published read-only. tmux is the runtime.
The only Bash in the product is `install`, the 83-line bootstrap that publishes the core.

This file is the CURRENT contract: what to do, what never to do, who owns each rule. The
reasoning, the retired rules and every measurement narrative are in
[docs/history.md](docs/history.md). Product direction: [VISION.md](VISION.md).

## What ae is

- A thin wrapper around tmux. Not a framework, not a platform.
- Built for daily productivity, not completeness. If it does not save time on every use, cut it.
- Resistant to features. If tmux already does it, do not re-implement it.
- Understandable in one sitting. Keep it that way.
- One crate with modules, because that is how a Rust program stays readable.

## What ae is NOT

- Not a CI/CD pipeline. Use your existing workflow for that.
- Not a billing system. `ae usage` prices the agents' own transcripts at list price, offline;
  it never meters, caps or bills, and never substitutes a profile; the lead steps down.
- Not a logging system. tmux already does `capture-pane` and `pipe-pane`.
- Not a git workflow tool. It does the minimum (commit + push), nothing more.
- Not a plugin framework. Wrap `ae` in a script if you need custom behavior.

## Structure

```
src/                — Rust sources. main.rs thin (argv in, exit code out); lib.rs and one
                      module per domain hold everything testable. theme.rs is PURE: the
                      palettes, the seven marks and every tmux format ae draws a session with
tests/it/           — the one integration-test target. The behaviours of the retired bash
                      suites are pinned here as Rust tests.
                      doors.rs = capability boundary; gate.rs = justfile/install guards;
                      parity.rs = the one child-process door. Unit tests sit beside the code
tests/fixtures/     — frozen inputs the suites read (session shapes, list goldens)
install             — the bootstrap: download a bundle, prove it against the release
                      manifest, extract, `ae-core _install --from <tmp>`. 83 lines; the only
                      bash file
justfile            — dev/release pipeline; holds every dev-tool version pin
docs/               — user + internals docs; history.md holds the retired contract
fuzz/               — the cargo-fuzz crate: one thin target per hostile parser, tracked
                      seeds in seeds/<target>/. Its own lock and its own nightly, outside
                      the product graph. Human-run, never a CI gate
contrib/            — optional sidecars: aeorchestrator (templates only, no code)
.github/workflows/  — rust lanes (both platforms) + dispatch-only release-proof lanes
.github/ISSUE_TEMPLATE/ — the bug report form; docs/troubleshooting.md §Filing a bug is the fact list
Cargo.toml          — one crate, bin + lib, both named `ae`. No workspace
rust-toolchain.toml — compiler pin, profile, components, both targets
clippy.toml         — tests-only unwrap/expect relaxation + the two capability denies.
                      deny.toml — supply chain. taplo.toml — TOML scope. .cargo/ — aliases,
                      musl linker, cargo-mutants config
.config/nextest.toml — test scheduling: the tmux-client menu tests run in one small
                      group, so parallel load cannot starve a client they wait on
README.md  VISION.md  AGENTS.md  CLAUDE.md (@AGENTS.md)
```

## How to work here

The loop, in order:

1. `just test` — THE GATE. Exactly `just rust-check` (fmt-check + lint + test), ~25s warm.
   There is no fast/slow split; run the whole thing.
2. `just check` — only when you touched `install` (shellcheck + shfmt).
3. Cross-model review BEFORE the commit, for anything significant. Route it through ae
   (`<session>/ask`, `<session>/review`, `<session>/spawn <alias>:reviewer`) — never shell
   out to another CLI, never a harness-internal subagent.
4. Commit on green, after the review findings are answered. Plain message, no trailers,
   no AI attribution of any kind.
5. `just release` — local, one machine: gates, CalVer bump, both bundles, tag, push,
   assets. Nothing waits on a runner.

Prefer small vertical slices and fast iterations. Commit, release, push and roll out
standalone intermediate state or sub-features rather than waiting for a feature to finish.
A slice ships when it is green, reviewed and leaves the product coherent — not when its feature
is finished. This changes when we ship, not what we skip: `just test`, cross-model review, the
freeze protocol and every hard rule remain mandatory.

An upgrade is not a binary swap: it migrates, repoints and relinks every placeable session
before it moves the command link, reports and skips stopped unplaceable sessions, then prunes
unreferenced versions. A seat's pane line names the command link when installed and stays runnable across the upgrade.
A publish is `$HOME`-pinned, so a checkout run whose state root differs
REFUSES `ae upgrade` before downloading anything.
Live upgrade probes therefore go in a sandboxed `$HOME`, never `ae-dev`. Why:
[docs/upgrade.md](docs/upgrade.md).

Other rules of the loop:

- **Persistent development goes in the `ae-dev` namespace** — `~/.local/bin/ae-dev`: own
  `~/.ae-dev` home and config, own tmux server (`-L ae-dev`), checkout binary. NEVER the
  default server or `~/.ae`. A one-shot launch probe gets a fresh private `TMUX_TMPDIR`, clears
  `TMUX` / `TMUX_PANE`, and uses scratch state; never let checkout `-L ae` reach the real socket:
  `probe=$(mktemp -d)` then
  `TMUX_TMPDIR="$probe" AE_HOME="$probe/state" CONFIG_FILE="$probe/config" env -u TMUX -u TMUX_PANE target/debug/ae …`.
  `TMUX_TMPDIR` must EXIST — tmux silently falls back to the real socket dir otherwise (3.7b).
- **A session's LOOK is session-scoped, and it has three writers with one job each.**
  `src/theme.rs` owns the palettes, the seven marks and every format. A LAUNCH writes the
  layout, the look facts and the attention SEED, and stamps each WINDOW (tmux keeps pane
  borders, window entries and menu styles in the window table, where `set -t <session>`
  reaches only the current one). A RENAME rewrites the layout and the facts and leaves
  every verdict alone. The WATCHDOG owns the verdicts, refreshing them every cycle, and
  rewrites or UNSETS the layout only when `@ae_look_stamp` says the look has changed under
  it. A FOURTH writer is the ADOPTER: a running watchdog also writes
  `theme::FLEET_STRIP_OPTION`, and nothing else, into every same-server ae session that has
  no live watchdog of its own — drawn in the TARGET's look, with the TARGET as the current
  row. Any change to a status format or layout option MUST bump `theme::FORMAT_VERSION`,
  because that stamp change is the only thing that makes the watchdog repaint a running
  session's layout. Never write a global (`-g`) option, never `#()` in a format, and
  prove a look change by rendering it in `ae-dev` before it touches a live session.
  The launch also stamps, by
  main-pane ID, a session-scoped focus hook guarded by its captured session ID and a lead-pair
  window-scoped resize hook, never by name or globally; both apply with `theme = off` and are
  not part of the look. Launch and
  upgrade reassert the canonical server-global input map and remove stock right-click menus on
  ae-owned servers only; the user's tmux is untouched. On tmux 3.5+,
  `MouseDown1Status` sends session/window ranges through tmux navigation, `ae` / `ae-more` to
  the fleet picker and `ae-settings` to settings; `MouseDown3Status` sends those menu ranges to
  their matching menus and session ranges to the guarded Flip menu, and stale `MouseUp*Status`
  bindings are removed. On tmux 3.4, Down keeps navigation but does nothing on menu ranges, while
  Up opens the matching keyboard-driven menu or Flip menu after release. `<prefix> a` (default
  `C-b a`) opens the fleet picker without colliding with
  stock tmux's pane keys. The picker carries the invoking client explicitly, resolves that client's
  current session for its transient open marker, and a row switches to the captured session id
  before selecting its still-member lead pane. A bind cannot be session-scoped.
- **Workers get their own git worktree.** `git worktree add <path> -b <branch> main`.
- **Land a slice with `git merge --ff-only`** after the worker rebases onto the main tip. Never `--no-ff`: `just release` runs `git pull --rebase` and flattens merge commits.
- **One writer per file.** Other agents edit this tree concurrently; coordinate before
  reverting or overwriting anything you did not change.
- **No lifecycle commands against a live session you do not own** — no `ae end`, `ae stop`,
  `ae rm`, no `retire` of someone else's agent. An unforced `ae stop` or `ae end` from
  inside a session delegates confirmation to the attached human's tmux client; agents must
  not answer that prompt or pass `-y` / `-f` unless the human explicitly authorized it.
- **Never `export HOME=… AE_HOME="$HOME/.ae"` in ONE statement.** The shell expands every
  word before any assignment, so `AE_HOME` binds to the REAL home. This clobbered `~/.ae`
  twice. Separate the statements, or assign both from the literal temp path.
- **CI pins too**: runner images are exact (`ubuntu-24.04`, `macos-15`, never `-latest`), and
  actions are first-party and SHA-pinned with the version in a trailing comment.
- **Port behaviour, not code.** Drop features on the way rather than transliterating.
- **A bug report carries the facts in docs/troubleshooting.md §Filing a bug** (platform, tmux, terminal, locale, harness versions, `ae version`/`ae doctor`) — an issue or a triage without them asks for them first.

## Hard rules — never do these

- `unsafe_code` is **forbid**. There is no exception worth having.
- No `unwrap()` / `expect()` in production code. `-D warnings` makes it fail the gate.
- No `std::process::Command` outside the enumerated doors. The PRODUCT has five sites over
  four files — `src/transport.rs`, `src/run.rs`, `src/upgrade.rs` (TWICE: `tar`, and the
  downloaded core's own `_install`), `src/install.rs` — and the suite has fourteen, across
  `tests/it/`'s `cli.rs`, `install.rs`, `migrate.rs`, `shape.rs`, `parity.rs` and `doors.rs`.
  `tests/it/doors.rs` pins the exact per-file counts, so a new door is a review, not a diff.
  Same for the world-reading methods in `clippy.toml`'s `disallowed-methods`: each lives at a
  named door carrying its reason.
- No clap, serde, anyhow, thiserror, chrono or nix. Adding any runtime dependency is a
  ruling, not a commit. See docs/history.md §11 for the researched line and its triggers.
- **No new Bash.** `install` is policy-frozen. There is no other bash file and none may
  be added.
- **No Python anywhere in the product.**
- **Never write through a helper path.** `>`, `>>`, `chmod`, `cp`, `sed -i` FOLLOW the
  symlink and corrupt the shared core binary. `rm -f` first. Fixtures are where this bites.
- **Pass a helper message as ONE shell argument, single-quoted.** Double quotes do NOT
  word-split, but they DO interpolate `$`, backticks and `$(…)`; unquoted also splits and
  globs. Write a literal single quote inside the body with the `'\''` idiom.
- **Never `tmux send-keys` by hand.** Deliver through the helpers: they own the literal
  send, the busy/dead-pane refusal, and the submit verification.
- No `--force`, `--no-verify` or `-f` to get past a check. Fix the cause.
- No secrets in tracked files. `$ENV_VAR` placeholders only.
- **A parser of hostile persisted state gets cargo-fuzz BEFORE it cuts over.** Session
  meta, journals, archives and anything hand-editable are hostile input. The lane is
  `just rust-fuzz`; its targets, seeds and pending parsers are in `fuzz/README.md`.

## Toolchain pins

Pins, not channels. CI, laptop and agent sandbox must resolve to the same compiler.

| What | Pin | Declared in |
|---|---|---|
| Compiler | `1.97.1` (exact release) | `rust-toolchain.toml` |
| Edition / MSRV | `2024` / `rust-version = "1.97.1"` | `Cargo.toml` |
| Profile + components | `minimal` + rustfmt, clippy, llvm-tools | `rust-toolchain.toml` |
| Targets | `aarch64-apple-darwin`, `x86_64-unknown-linux-musl` | `rust-toolchain.toml`, justfile, `deny.toml` |
| Dev tools | nextest `0.9.143`, taplo `0.10.0`, deny `0.20.2`, mutants `27.1.0`, llvm-cov `0.9.0`, vet `0.10.2`, git-cliff `2.13.1`; cargo-fuzz `0.13.2` on request, not by `rust-setup` | justfile `*_VERSION` — the single source |
| Fuzz compiler | `nightly-2026-08-20` (exact). The product compiler does not move; there is no `fuzz/rust-toolchain.toml`, the lane passes `cargo +<pin>` | justfile `FUZZ_TOOLCHAIN` — the only source |
| `just` | `1.57.0` | justfile `JUST_VERSION`; CI reads the pin from there |
| tmux floor | `3.4` — a launch and the picker REFUSE below it; `list`/`version`/`doctor`/`upgrade` do not. Ubuntu 24.04 and Homebrew both package a tmux that clears it, so CI installs the package | `src/tmux_floor.rs`; CI step in `.github/workflows/rust.yml` |

`--locked` on every graph-consuming lane. Spellings differ: `cargo deny --locked check`
(global option), `cargo mutants --cargo-arg=--locked` (no native flag), `cargo fmt` exempt.

## Lanes

| Recipe | What it is |
|---|---|
| `just rust-setup` | bootstrap: toolchain + pinned tools. Idempotent, and CI asserts it |
| `just test` / `just rust-check` | **the gate**: `rust-fmt-check` + `rust-lint` + `rust-test` |
| `just check` | the bash lane: shellcheck + shfmt over `install` |
| `just rust-fmt` / `rust-fmt-check` | `cargo fmt` + `taplo fmt` |
| `just rust-lint` | `cargo clippy --locked --all-targets --all-features -- -D warnings` + `taplo lint` |
| `just rust-test` | `cargo nextest run --locked` **and** `cargo test --doc --locked`. Both. nextest does not run doctests |
| `just rust-deny` / `rust-vet` | supply chain. The TLS graph is EXEMPTED, not audited (docs/history.md §11) |
| `just rust-mutants` | does the suite discriminate? CI runs it diff-bounded per push. `rust-cov` reports, never gates |
| `just rust-fuzz target=<name> secs=60` / `rust-fuzz-all secs=60` | human-run cargo-fuzz over the hostile parsers, on an exact nightly. Refuses on an unpinned tool or a stale `fuzz/Cargo.lock`, ends on the cutover evidence line, and REPORTS — CI carries no nightly and never runs it (`fuzz/README.md`) |
| `just rust-build-release` / `bundles` | native release binary (native only, a bare clone must build) / both platform bundles + `SHA256SUMS` into `dist/` (needs the musl cross toolchain) |
| `just release` | the whole release, locally. Pre-flight refuses before any state is written |

## Session helpers

The core LINKS 25 names into `~/.ae/sessions/<name>/`. Every one is a **symlink to the core
binary**; the core dispatches on `argv[0]`'s basename and derives the session from its
dirname. Names and argv are the compatibility contract.

| Helper | Purpose |
|---|---|
| `send [--cross-session] <agent> <msg>` | Deliver to the same session; another session needs `--cross-session`. Refuses a dead pane, defers on busy/human input, verifies the submit |
| `relay <session[:agent]> <text…>` | Orchestrator-only, audited bare-text delivery with human authority; linked everywhere, refused unless the caller session records `meta_agent=true` |
| `ask [--cross-session] <agent> <question>` | Tracked request in the same session; another session needs `--cross-session` |
| `review [--cross-session] <agent> <request>` | Critical review request in the same session; another session needs `--cross-session` |
| `reply <request-id> <msg>` | Reply to a logged ask/review. Verified against the request's stored slot |
| `requests [mine\|inbox\|all]` | Inspect request state (`pending`, `replied`, `cancelled` or `retired`) without peeking panes; `retired` means either party's seat closed it without a reply |
| `state <working\|waiting-user\|waiting-agent\|blocked\|done> [reason]` | Declare work state; shows in `ae list`. `waiting-agent` means waiting on ANOTHER ae agent (name them in the reason; no counterparty validation or redirect — that is a separate ruling). A `spawned.<n>` seat cannot declare `waiting-user` (exit 2): its spawner owns the human question, so it declares `waiting-agent` and lets the spawner escalate. Only `done`, `waiting-user`, `waiting-agent` and `blocked` quiet the watchdog — `working` does not, but OUTSTANDING OWN WORK defers its nudge: a request you sent that the ledger has not closed, or an agent you spawned that still holds a seat, buys quiet until the deferral ceiling, and `ae list` says so on your line. Fresh `waiting-agent` claims no human; past `idle_nudge_secs * OWN_WORK_AGE_CAP` nudge periods (the same multiplier as the own-work deferral — but at `idle_nudge_secs = 0` the deferral is vacuous while the attention ceiling scales from the documented default 300 s, so 1200 s, because zero keeps the marker and only suppresses the nudge) it escalates to exactly `blocked`: attention marker, and nudging too only when the idle-nudge cadence is enabled (`idle_nudge_secs = 0` keeps the marker and suppresses the nudge). `blocked` means a concrete EXTERNAL blocker only — a dependency, a service, a human decision elsewhere, a broken host — never a wait on another agent. The read surfaces escalate on the daemon's currency rule (no later relevant event mentioning the agent) through the ONE classifier, never by recomputing the ceiling; ONE pane-only residual remains and is named, not faked: a directory read cannot see pane activity, so a declaration the daemon has already yielded by two-cycle pane churn can still read as current on the human-marker surfaces, while the pane border follows the daemon. THREE records close a request: a `reply` closes it by REQUEST ID — for an external slotless asker (`tracked::is_external`: the chat bridges and ae's own `ae:` verbs), by the target display; a `cancel` closes it by REQUEST ID but no agent helper emits one — the only production emitters are `ae reboot --digest-only` for its own handover and `ae compact` for its own stale checkpoints, each refusing any request it did not open; and a `retire` closes it by SEAT at EITHER end — the slot it was sent to or the slot that sent it, compared by routing key and never by display name, recorded in the SAME session, that session not renamed since. Retiring a worker clears the questions it asked from its target's inbox — nobody is left to read a reply. Every other request goes on deferring, including a cross-session one read in the CALLER's log, one recorded before a rename, and one whose party vanished with no `retire` at all. `mark-done [msg]` = `state done` plus the legacy `done` event |
| `say <text>` | Push a line to the human's Telegram chat. Pane output is NOT forwarded |
| `memo add [--topic t] <text>` / `memo read` / `memo tail [n]` | Durable shared session memory. Topics are STABLE and reused (`goal`, `decision`, `parking`, `<feature>`) and each record is a CHECKPOINT that supersedes the last one on its topic — `ae brief` shows only the latest per topic |
| `goal [text\|--clear]` | The session's one-line objective. Survives resume; shown in `ae list` |
| `peek <agent> [lines]` / `peak` | Capture recent pane output. Inspection only, never a reply channel |
| `agents [--all]` | List agents with pane IDs and processes. `focus <agent>` switches tmux focus |
| `quota` | Show each configured client scope's locally cached quota windows, freshness and EFFECTIVE headroom (declared `manual_resets` + reported credits); Codex rollout owners are read across the local fleet |
| `usage` | Show this live session's offline API-equivalent token usage and reference-price spend |
| `interrupt [--cross-session] <agent> [msg]` | Cancel in the same session; another session needs `--cross-session` |
| `spawn <name> --using <profile> [prompt]` | Add an agent to the workspace |
| `retire <name>` \| `retire %pane` | Remove a spawned agent. Exact name only; `main`/`worker` refuse |
| `relaunch <agent>` | Bring a PROVABLY DEAD seat of the caller's own session back — same slot, same pane, same name, same conversation when the tool still has it. Under the lifecycle lock it proves the pane sits at an IDLE shell with the recorded tool gone (the ONE liveness owner), clears the line, pastes the seat's `_run` line with its recorded `work_dir` restored, then requires the SEAT'S OWN TOOL to be observed running — `_run` in the foreground is not a start. Exit 0 only then, and only when the launch turn is not `undelivered`. Every other case REFUSES by name with the next step: running, liveness unknown (saying which gap), pane not found, pane dead, pane busy, no recorded tool, unusable `work_dir`, lock timeout, tool changed. Nothing is rolled back; a second relaunch refuses `running` if the seat came up after all. The lock is DROPPED before the launch turn, because a gated turn blocks up to 45 s and no session may be held out of its own lifecycle for that — so a relaunch overlapping another's turn delivery sees the seat running and refuses. No cross-session target |
| `_register-sid` | codex's own session-id handshake. The one helper no human types |
| `watchdog`, `events-tail`, `loop` | The two monitor panes' whole command (`loop` = deprecated alias) |

**Bodies between agents are caveman-terse.** Rule 10 of the context document: what you pass
through `send`/`ask`/`review`/`reply`/`memo`, a spawn brief or an `interrupt` is read by another
agent and costs its context, so drop filler and keep file:line, commands, errors, ids and
verdict words exact. Nothing else is covered: replies to the human, `say`, commits, code and
docs follow whatever your own instructions define.

`relay` is the ONE unenveloped sender: its bare text has human authority. The link is published
in every session but works only when the caller pane belongs to the orchestrator seat
(`meta_agent=true`), and every attempt is audited in that caller session. It is for verbatim,
explicit human instructions only, never inferred or judgment work.

**Call a helper by its FULL PATH.** No `/` in `argv[0]` means no session to derive, and the
core exits 2 rather than guessing. That is why they are not on `PATH`.
Name resolution takes the exact name, `%pane-id`, or `session:agent` / `@session:agent`.

## Agent tool capabilities

| | Claude Code | Codex | Gemini CLI | Antigravity (`agy`) | Grok Build | Muse Code | OpenCode |
|---|---|---|---|---|---|---|---|
| **Prompt injection** | `--append-system-prompt` | `-c developer_instructions=` | `-i`, brief folds into the turn | none — rides `-i` as a user turn, brief folds in | none — rides positional `[PROMPT]`, brief folds in; never `--system-prompt-override` | none — rides positional `[PROMPT]`, brief folds in | `OPENCODE_CONFIG` json `instructions` |
| **Session id at launch** | `--session-id UUID` | none | none | none | `--session-id UUID` | none | none |
| **Id capture** | immediate | post-launch: sid file verified by launch-token rollout born at/after the pre-exec capture floor, then token scan. A token miss stays `pending`; cwd/TUI are legacy no-token fallbacks and may never replace an id | post-launch chat-history scan | post-launch: `<id>.db` bytes with a token, else `cli-*.log`. A token miss stays `pending` | immediate | post-launch: launch-token scan of the dated `session.jsonl` store; the token-proven directory basename is the id. A token miss stays `pending` | post-launch `session list --format json` |
| **Exact resume** | `--resume UUID` | `resume UUID` (subcommand) | `--resume UUID` | `--conversation UUID` | `--resume UUID` | `resume UUID` (subcommand; no positional context turn) | `--session ID` |
| **Resume fallback** | `--continue` | fresh start | `--resume latest` | `--continue` | `--continue` | fresh start | `--continue` |
| **TUI modelled for delivery** | yes | yes | no | no | no | yes | no |
| **`_run` re-run** | exact resume when the recorded id passes the tool's store probe (or the tool has no probe); a gone conversation takes the fallback above; the installed pane line names the command link, so the re-run survives `ae upgrade`. The `relaunch` helper is that re-run performed FOR a proven-dead seat, under the lifecycle lock | same | same | same | same | same | same |

- A drawn input box is not an initialized tool. Paste-driven delivery is gated by
  `src/deliver.rs::input_ready` / `wait_input_ready`; a timeout is a loud, durable failure.
- **codex's rollout does not exist until the first USER turn.** Measured on codex-cli
  0.153.2 (2026-09-04): ae's exact argv with no positional prompt writes nothing under
  `~/.codex/sessions/<day>/` for 30s, and the header carries no session id before that
  turn either — so both the token scan and the header scrape have nothing to find. The
  turn stays, and it is PASSIVE: wording and reason at `src/launch.rs::initial_prompt_for`.
- **agy's trust modal blocks the pane** until a human answers, and its trust list is
  exact-path. ae's context survives it (argv), and a spawn brief rides that same turn — no paste to lose.
- Meta v2 roster: `seat.<slot>` / `profile.<slot>` / `agent_bin.<slot>` / `harness_session.<slot>`,
  plus `harness_session_prior.<slot>` — up to four abandoned predecessor conversations, oldest
  first, with the current row cleared to `pending` when a resume falls back. Each element is
  `<tool>:<uuid>`: a seat can be moved to another CLI, so an id says which STORE it lives in
  rather than being looked up in the tool the slot names now. The tag is a basename, the id a
  lowercase UUID. A WRITER settles the whole row or nothing: one unusable element leaves the
  row alone, or restarts it from the new id, because a migration may not destroy what it
  cannot judge. A READER judges per element — a good one reads in the store its tag names, a
  bad one becomes a coverage row or a purge note, and the others are unaffected — so a hand
  edit costs at most the element it damaged and never makes a session unresumable. A bare
  uuid is the LEGACY spelling and means the tool of the slot at read time; the `meta_version`
  2->3 step writes the tag in. A predecessor whose tool is not the slot's current one is
  REPORTED, never guessed at: only the current `config_home` is recorded, so neither the
  purge nor the board can name that store.
  A legacy `agent.<slot>` row is refused and recorded `degraded: true`, never migrated.

## Environment doors

`src/doors.rs` is the ENTRY/SHAPE surface — the facts the deleted wrapper used to hand over.
Other ambient reads are named doors at their own use site, each carrying its reason and each
inventoried by the clippy `disallowed-methods` boundary. Never read the world ad hoc.

| Door | Decides | Read in |
|---|---|---|
| `HOME` | where ae state lives | both shapes |
| `PWD` | the caller's working directory; public launch `--dir` overrides it for that launch | both |
| `AE_HOME` | relocates ALL ae state | CHECKOUT only |
| `CONFIG_FILE` | which global config is read | CHECKOUT only |
| `AE_TMUX_SERVER` + `AE_TMUX_SERVER_KIND` | which tmux server a launch lands on; absent → named server `ae` | CHECKOUT only |
| `AE_NO_AUTOSTART` | start no companion: neither the watchdog nor the Telegram bridge | both |
| `AE_TEST_BOOT_TIME` | the host boot time the absence proof compares against | CHECKOUT only |
| `AE_TEST_QUOTA_TRACE` | when set to a path, each quota pass appends one attestation line there (`skipped`, or `observed max=<pct> booked=<n>`) — a due cadence pass and the limit-release recovery pass alike; test seam, unset in production | CHECKOUT only |
| `AE_TEST_RENAME_CRASH_AT` | when set to exactly one of `after-intent`, `after-work-move`, `after-state-move`, `after-meta`, `after-assets`, `after-result`, a stopped rename attests `rename-crash-boundary: <value>` on stderr past the named facts, then parks at most 60s and exits `1` with `rename-crash-timeout: <value>`; test seam, unset in production | CHECKOUT only |
| `TMPDIR` | where a captured child's scratch file lands: the opencode export capture (`transport::run_opencode`, whose child abandons a pipe at exit), and upgrade's older scratch directory (`upgrade::Scratch`) | both |
| `TMUX` / `TMUX_PANE` | which pane this shell is, for `stop` and `watchdog` | both |

`doors::boot_time` is the one door with two spellings and no variable of its own: Linux's
`/proc/stat` `btime` when that file exists, otherwise `transport::run_sysctl` — a fixed-program
leg of the EXISTING process door (`sysctl -n kern.boottime`, no argument a caller chooses), so
the `std::process::Command` inventory is unchanged. `tests/it/doors.rs` pins its one product
caller beside `run_git`'s and `run_ps`'s.

In the core `AE_VERSION` is scoped to `upgrade` alone; `install` reads it too, as the CalVer
target. `AE_CORE_BIN` and `AE_NEXT_HOME` are dead in both shapes.

## GNU vs BSD — check the command YOU are about to type

macOS is BSD, Linux is GNU. Every row below fails SILENTLY through a `|| fallback`, so the
cost is a wrong number in an evidence report, not a broken command.

| GNU-only | BSD form |
|---|---|
| `tac` | `tail -r` |
| `stat -c %Y/%s/%i/%u/%a` | `stat -f %m/%z/%i/%u/%Lp` |
| `date -d <iso>` | `date -u -j -f <fmt>` |
| `sed -i EXPR FILE` | BSD reads EXPR as a backup suffix — temp + rename |
| `grep -oP … \K` | no `-P`, no `\K` — `grep -oE` + `head -1` + `sed` |
| `touch -d <human date>` | `touch -t [[CC]YY]MMDDhhmm[.SS]` |

BRE alternation `\(a\|b\)` is a GNU extension: use `sed -E` with the ERE form `(a|b)`.
`wc` pads its count on BSD (`| tr -d '[:space:]'`); `uuidgen` is UPPERCASE; there is no
`/proc` (`ps -o ppid= -p <pid>`) and no `getent` (`dscl`); `timeout` and `flock` are absent.

## Key invariants

Each is one rule with one owner. Change the owner, not a copy.

| Invariant | Owner |
|---|---|
| Quota-aware leadership profile selection is guidance only: query once per batch or creation for every selected `--lead`/`--colead`/`--seat`, init, or `spawn --using` profile; do not knowingly pick a known-exhausted applicable window; unclear correlation is unknown; `quota = off` injects none; ae never substitutes | `src/render.rs::QUOTA_GUIDANCE` |
| Session name `^[A-Za-z0-9][A-Za-z0-9_-]{0,127}$`, checked before any side effect. A word the entry ROUTES is refused on top of that grammar, at the two places a name is CREATED — an explicit launch and a rename — so the class closes rather than the word a slice happened to add; the picker's own resume is exempt because it addresses a session that already exists, and the canonical orchestrator is exempt because that seat IS the session of its word | `src/session_launch/name.rs::is_session_name` is the grammar; `src/entry.rs::ROUTED_VERBS` is the set and `::name_is_routed_verb` the one predicate both sites ask |
| Agent name `^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$`; it reaches a system prompt, so it is an allowlist | `src/config.rs::is_agent_name` |
| That name is re-validated at the interpolation site, fail-quiet | `src/render.rs::context_document` |
| Dispatch is on `argv[0]`'s basename; no `/` means exit 2, never a guess | `src/shim.rs` |
| `current_exe()` has exactly ONE caller | `src/shape.rs::resolved_exe` |
| `launch.<slot>.started` decides create-vs-resume, before the exec | `src/run.rs` |
| A seat's config home is resolved once by `launch_cmd::config_home`; first start records explicit-variable mode as `config_home.<slot>=<canonical path>` or implicit-default mode as `config_home.<slot>=implicit:<canonical path>` plus `config_home_base.<slot>=<canonical effective HOME>`. Both implicit rows are one identity and are published atomically; every meta rebuild carries them, and the recorded store, mode and base win for a retained conversation | `src/run.rs`, `src/meta.rs`, `src/session_launch.rs`, `src/session_launch/capture.rs`, `src/lifecycle/end.rs` |
| `capture_floor.<slot>` is published before a capture tool starts. A retained exact conversation keeps its original floor; a still-pending resume gets a fresh one, and a resume fallback republishes a fresh one in the same guarded replacement that records the predecessor and clears the row — transition-only, capture tools only. Every session-store scan uses this floor, while `launch_time.<slot>` remains a separate post-exec lifecycle stamp. A token-proven Codex lookup covers UTC partitions from the floor through today, or the last 30 days when the floor is unknown; tokenless cwd lookup remains today/yesterday only | `src/session_launch.rs`, `src/spawn.rs`, `src/session_launch/capture.rs`, `src/meta.rs::record_abandoned_session` |
| The install gate is STRUCTURAL and hashes nothing. Every command and helper passes it EXCEPT `version` and `upgrade`, which diagnose and repair a broken install | `src/shape.rs`, ordered in `src/lib.rs::run` |
| The one hashing site: both members re-digested against `SHA256SUMS` before publication | `src/install.rs` |
| Published dir 0555, members 0555/0444; `~/.local/bin/ae` is the current pointer | `src/install.rs` |
| A seat's pane launch line stays runnable across upgrades: installed names the command link, checkout the absolute core | `src/run.rs::pane_head` |
| Every session meta carries `meta_version=<N>`; the chain steps N->N+1 and runs wherever the core touches a session. A meta with no row but `schema=2` IS version 2 — the pre-chain shape — and is stamped in place, never refused; only a meta with NEITHER key is refused, at resume, and merely REPORTED at stop and end, which must never be blocked. A publish migrates and repoints every placeable session before it moves the command link; stopped unplaceable sessions are reported and skipped untouched. It then deletes every `versions/<V>` no meta records | `src/migrate.rs` (`placed` owns the rule for the chain and for `ae list` alike), called from `src/install.rs::publish_steps`, `src/session_launch.rs`, `src/lifecycle.rs` and `src/lifecycle/end.rs` |
| A harness session id is a NAME: the purge proves it against the archive UUID grammar before it builds a path | `src/lifecycle/end.rs::purge_conversation_files`; grammar in `src/archive.rs::canonical_uuid` |
| A monitor sweep may act only on the CALLER'S own session (`$TMUX_PANE`) — the ONE named exception is the watchdog's publication of a single display fact, `theme::FLEET_STRIP_OPTION`, into a same-server ae session with no live watchdog; sweeps stay own-session | `src/monitor.rs`; the exception is `src/watchdog_daemon.rs::adoption_writes` |
| Every tmux format uses a printable pipe separator, never a control byte — tmux 3.4 octal-escapes those. Each format literal is written out; `SLOTS_FORMAT` deliberately uses an unspaced pipe | `src/tmux.rs` (`FIELD_SEPARATOR` is the parser delimiter) + the control-char-free test over every format constant |
| A session name handed to tmux as a target is `=name` — exact, never prefix/fnmatch | `src/tmux.rs::session_target`; pinned by `tests/it/entry.rs::a_launch_target_is_an_exact_session_name_not_a_prefix` |
| The tmux floor is REFUSED at exactly two places — a launch (before its first write) and the picker — and REPORTED at three: `ae version`, `ae doctor`, a publish. The parse and the verdict have one owner | `src/tmux_floor.rs`; the sites are pinned by `tests/it/floor.rs` |
| The percentage ae JUDGES a quota window by is derived, never raw: a reached spend cap outranks unlimited credits, which outrank the operator's declared `manual_resets` (`0`-`9`, which spread one window's usage over `1 + n`), and an absent declaration leaves the raw window. ONE function derives it and every consumer reads that one value — `ae quota`'s `EFFECTIVE` column, the watchdog threshold and the settings quota dialog's per-window projection. The dialog names the derivation (`xN`, `unlimited` or `spend-cap`) beside the judged percentage so an account constraint never reads like one a window reset frees. Delegation guidance points at the column. Two labels on one config home are one account: the SMALLEST explicit count wins, because claimed headroom nobody declared would suppress a real advisory. An unusable declaration is ignored with one visible note rather than refusing a config, and ae never CONSUMES a count — a used reset is the operator's to re-declare | `src/quota.rs::derived` is the one derivation (`::effective` its rule, `::Derived::settings_cell` the compact spelling, `::credits_label` its credit spelling, `::merge_declaration` the pessimistic reconciliation); `src/config.rs::parse_manual_resets` owns the declaration grammar |
| Account facts (credits, spend control) are SCOPE facts with FIELD-LEVEL provenance, never a window's: each carries the stamp of the record that last USABLY reported it, and a record moves only the fields it asserts — and only when it names its own bucket and stamps itself unskewed. An absent, null or malformed field neither overwrites a held value nor refreshes its age, so a proven spend cap lifts ONLY on an explicit `false`. Every ambiguity resolves toward LESS apparent headroom: a cap never ages out, while an unlimited-credit claim relieves a window only when it was reported no earlier than that window's own observation, and a claim ae will not use is still shown with a note saying why. A balance ae cannot state exactly is reported as available WITHOUT an amount, never clipped to a different number. ONE merge enforces all of that, for a record inside one read, for another rollout of the same scope, and for a later cycle alike — no caller replaces an account wholesale or pairs one scope's declaration with another's facts. A held cap is not durable beyond the evidence: a bounded tail that no longer carries the record, or a daemon restart with no held state, both begin again from what the client reports now | `src/quota.rs::Account::absorb` is that one merge and `::Policy` the declaration-plus-account value every consumer reads; `src/quota/codex.rs::merge_account` decides what a record usably reports (`::reported_credits` / `::reported_spend_control`; `CREDIT_BALANCE_MAX` bounds an exact literal); `src/quota.rs::Account::credits_relieve` owns the permissive-fact rule |
| A quota window belongs to the CLIENT SCOPE, not to the conversation that observed it: the watchdog's tracked state is keyed by canonical source, bucket, qualifier and window and NEVER by rollout, so N rollout owners under one config home produce ONE advisory per transition. The scope is also who the CHECKPOINT ASK follows: ENTERING `Low` or worse — first sight included — asks every roster seat on that scope once, fixed and spawned alike, matched by the ONE recorded-identity join the throttle line already makes and skipping a seat with no live non-shell pane; staying in the band or moving deeper asks nothing, and only `classify`'s own hysteresis lets it be entered again. It is advisory — no request, no reply, no state — and rides the advisory's own delivery, marker and single retry under the action `quota-checkpoint`, while the ADVISORY's recipients stay the lead pair | `src/watchdog_daemon.rs::QuotaKey` + `::quota_samples_at` (newest observation per key); `::entered_low` is the edge, `::quota_ask_candidates` the fan-out, `crate::quota::recorded_identity` the one join, `crate::quota::Advisory::checkpoint_ask` the one text |
| A quota classification answers to TWO clocks, kept apart. The RAW observation is accepted or refused by TIMESTAMP ALONE, whatever the policy says, so an older sample never enters a classification; the accepted row is HELD. A POLICY change — the declaration or the reported account — carries no vendor clock, so it re-derives that HELD row, and the notice it books and renders carries the held provenance too. The policy path is never a way in for a sample the freshness rule refused. Direction-symmetric: declaring a reset clears a critical, withdrawing it re-arms one. The observation that decided a level also SUPPLIES it: the percentage, the derivation and the age in every notice and in the throttle line a seat reads come from the held reading, never from the sample that was refused. A level exists only inside a classified reading, nothing can set one, no renderer takes a state beside one, and a policy is assembled only where a scope is read. OUTSIDE the `quota` module tree that is the compiler's answer and each bypass measured on the way here is a compile error; INSIDE it the same rule is a CONVENTION, because Rust privacy admits a descendant and cannot say "visible to the parent but not to a sibling", so the two parser modules keep their parent's access and a calibrated source guard is the whole of the enforcement there | `src/quota.rs::Reading` binds the row, the policy and their provenance (`::Reading::adopt` is the two-clock rule, `::Adopted` its answer); `::Classified` owns the level and is the only carrier of one (`::classify` the hysteresis, private; `::Classified::first` / `::adopt` the only producers; `::Observation::transition` / `::state_line_at` the only renderers); `src/watchdog_daemon.rs::QuotaCarry::reconcile_with_candidates` holds one per key on `::QuotaTracked::classified`, and `::throttle_quota_line` renders it through `::QuotaReadout`. `tests/it/quota.rs::the_quota_surface_cannot_pair_a_level_with_an_observation_it_did_not_judge` pins the shape of that surface |
| A session's look is SESSION- and WINDOW-scoped, never global. Four writers, one job each: a launch writes the layout, the facts and the attention SEED; a rename rewrites the layout and the facts only; the watchdog owns the verdicts every cycle and rewrites the layout ONLY on a look-stamp change; and an ADOPTER — any running watchdog — writes `theme::FLEET_STRIP_OPTION` and NOTHING else into every same-server ae session with no live watchdog of its own, because only a session's own watchdog wrote that strip and a session nobody measures was left with an empty second line. The adopted strip is STATIC, drawn in the TARGET's look and with the TARGET as its current row, in the shared `FleetOrder`; N adopters therefore write byte-identical text and converge with no leader election. A target is admitted only through the durable inventory and the ONE liveness classifier, on the same ownership proof `seed_unwatched` makes (an `AE_SESSION` marker plus an `AE_HOME` naming this state root) and only while its own watchdog is not live — unknown liveness is never adopted. Enumeration is once per verdict cycle on the cycle's own process table; the 2 s tick spawns nothing when there are no targets. The adopter takes the strip back by simply ceasing to write, so the returning owner publishes over it, and a stopping adopter never retracts a peer's strip: a cross-session retraction is a race, and a stale strip is a map one cycle out of date rather than no map at all. A watchdog that WITHDRAWS from a session still running — `ae watchdog stop`, or a daemon exiting because the record stopped naming one server — returns those three verdict options to the launch SEED instead of unsetting them, under the ownership proof the UUID backfill uses, because every fleet reader drops a session that publishes no rank; it says so on that session's own bar through `@ae_watchdog_status`, which a launch that starts no watchdog writes the same way, from the one owner `theme::watchdog_off_segment`. Launch and upgrade reassert one capability-canonical server-global input map and remove stock right-click menus on an ae-owned server because tmux cannot scope a key binding to one session; the user's tmux is untouched. The `ae` menu range draws only the `≡`/`=` glyph as the status line's first cell, before the fleet strip, and `ae-more` draws the plain `+N`; both inherit the line's dim style while quiet. The orchestrator button sits immediately after the menu glyph, before the fleet strip, as a `session|{id}` range exactly three cells — one blank, bare `◆`/`o`, then one blank — with a stable glyph and the verdict in the foreground colour; when no orchestrator is published the line is byte-identical to the menu-plus-fleet line. Every foreground the look draws on `base`, `selected` or `panel` clears its WCAG 2.1 contrast bar — 3.0:1 for a mark, 4.5:1 for text — pinned by `src/theme.rs::every_drawn_pair_clears_its_wcag_contrast_bar`, whose darcula exemptions name that IDE-frozen palette's own sub-bar pairs. The final `ae-settings` range is exactly three cells: one blank, bare `⚙`/`*`, then one blank. Both blanks are deliberate clickable padding, and selected styling covers the whole range; it carries no version fact or U+FE0F. The settings menu reads its exact invoking client's session-scoped `@ae_version`: only an exact `ae <CalVer>` becomes `ae <CalVer> settings`; missing or malformed falls back to `ae settings`. The picker resolves its invoking client's current session from one client snapshot, sets or refreshes transient `@ae_menu_open` there, and opens bottom-left above the status line with `-x 0 -y S`; settings uses that exact-client route, sets or refreshes `@ae_settings_open`, computes numeric `-x client_width.saturating_sub(menu_columns)` from the same snapshot and final menu budget, and opens above the status line with `-y S`. Each menu clears the other's marker before setting its own, so selection colours light exactly one button. A selected row or failed draw clears its own marker; Escape relies on half-interval watchdog expiry, or the next menu/action when the watchdog is disabled. On tmux 3.5+, Left-Down sends `session`/`window` through navigation and each menu range to its matching menu, Right-Down sends each menu range to its matching menu and `session` to the guarded Flip menu, and both stale Up bindings are removed. On tmux 3.4, Down keeps navigation but is a no-op on menu ranges; Up opens the matching menu or Flip menu, so each status click opens exactly once after release. The session range's right-click delegates the ROOT draw to the read-only `_session-menu show`: the binding keeps its seven captured facts and never captures a user option, `show` reads every source first, then takes ONE final clicker proof, and refuses neither enrichment nor fit (full → facts-dropped → rows-dropped → status-only → today's exact floor against the live dimensions that proof returned; below the floor tmux trims as always); only a tmux-refused DRAW reports and exits nonzero, and the floor is the byte-identical Flip action word plus the seven-fact Stop confirm argv with no `--uuid`. Its root draws a facts block (mode/dir/source/branch, `unrecorded` never a guess) above declared states only — newest per roster actor, at most three, truthful age, reason clipped by the one `event_text::display_cell` — then keyed `Activity…`/`Memos…` rows opening read-only dialogs (newest 10 `state`/`done`/`goal`/`spawn`/`retire`/`relaunch`/`ask`/`review`/`reply`/`watchdog-start`/`watchdog-stop` records, or the latest memo per topic exactly as `ae brief` computes it, a short client dropping the oldest rows until title, rows, separator and `Close` fit), all under the `@ae_session_uuid` = meta `session_id` correlation; a gap is named, never filled, and no other incarnation's records can render. Its menus are keyboard-driven and rows pick by key. `<prefix> a` (default `C-b a`) opens the picker for its invoking client. Every menu gets release protection (`display-menu -O`); tmux 3.5+ also gets mouse handling (`-M`). The watchdog atomically replaces the bounded versioned `@ae_agents` roster fact each cycle, carrying its own cadence, and unsets it on stop; the picker strictly parses/fuzzes it, bounds past age and future skew from that cadence, draws frozen keyless agent rows under stable-key session rows, and degrades by client height and terminal-cell width before tmux can refuse the menu. That fact is written in the `v2` grammar — the four `v1` fields plus `client`, `model`, `effort` and a bare `!` drift mark, all four DISPLAY-only and none of them ever reaching the meta — and read in BOTH: an entry's observed cells are accepted only all-or-nothing, a model ae cannot spell exactly empties that ENTRY's trio rather than being escaped, and an identity is taken from a capture only when the tool's own composer is drawn in that SAME capture, held in daemon memory for at most 30 cycles after. A roster that will not fit degrades in fleet-wide RUNGS — `v2` full, `v2` without effort and drift, `v2` with only the client kept, then `v1` byte-identical to what ae wrote before these cells existed — because the model cell is a COLUMN and must never be the reason a roster vanishes; within `v2` the client is the cheapest cell and the last to go. An older core reading `v2` fails its version check and takes the existing `agents: unavailable` degrade, which is why no compatibility shim exists. A picker row switches to its captured session id, then selects its captured lead or agent pane only when a build-time membership read and an execution-time session-id guard prove it belongs there. STOPPED rows are a SECOND source, and the picker invents no fourth notion of stopped: the durable inventory is read once and fed through the one `liveness::classify` with a backend that answers only recorded servers `SocketPaths` proves equivalent to the caller, so a row draws exactly where `ae list` would read `stopped` — a marker-on-rank live sighting is `running`, a bare live name without ownership is `unknown`, and other servers, missing/ambiguous selectors and damaged records stay unlisted (the named gap: `ae list --all` shows some of them). Stopped rows sort after running ones by `inventory::last_live` descending then name, draw Dead's glyph with the picker word `stopped`, share the columns, the height/width/omission budget and the key alphabet, and NEVER expand a roster. Choosing one re-execs the launcher as `orchestrator --picker-resume <name>` with the captured client, its pid, the server identity pair and a bounded deadline; the continuation re-proves that identity through the one `ExpectedLaunch` and calls the ordinary launch owner with `ExpectedState::StoppedSession` (recorded-server resume, no extra session proof beyond the launch's own under the lifecycle lock), then switches the captured client to the resumed session. No second stop path exists: stopping stays the session menu's guarded confirm. Installed menu bindings call `~/.local/bin/ae`; checkout bindings bake their state, config and server namespace plus their absolute core | `src/theme.rs` splits the sets (`layout_options` / `fact_options` / `seed_options`); `src/session_launch.rs::apply_status_bar` / `redress_status_bar` / `stamp_window` write them; `src/watchdog_daemon.rs::reconcile_look` is the only one that rewrites them, and `::publish` the only writer of the values while a daemon runs — `::seed_unwatched` writes the three attention options back to `seed_options` when one withdraws, and `src/session_launch.rs::start_watchdog_pane` writes the off mark for a launch that starts none; `src/session_tmux.rs::status_bindings_argv` owns the bindings' server boundary |
| The session-UUID fact `@ae_session_uuid` is write-once per tmux session incarnation: seeded vacant-only immediately after a SUCCESSFUL meta publication from that same in-memory document, backfilled for a legacy running session at the membership-proven upgrade site that already stamps `@ae_main_pane`, never by the watchdog, and encoded so a failed observation (`OptionReading::Unknown`) is never treated as vacant. The root menu renders record-derived state only when the option's canonical UUID equals the `meta session_id` read from the captured-name directory; a same-name recreation (new tmux id) or a replaced state directory floors with a named gap | `src/session_launch.rs::publish_meta_and_seed_uuid` / `::seed_session_uuid` is the ONE writer; `src/session_menu.rs::run_show` is the ONE reader and the one root draw |
| The server pair is read by SET, not by nonempty; an untypeable pair is refused | `src/doors.rs` |
| A resume never rewrites the pair under a running session; a re-pair is written only by the build's publication | `src/session_launch.rs` |
| Control bytes never reach JSON raw: they are written as JSON escapes | `src/json.rs` |
| Exit codes: `0` success, `2` usage error, `1` everything else | `src/cli.rs` + `src/lib.rs::run`; `src/main.rs` only maps the byte |
| A session is `Absent` only on POSITIVE proof. `stop` and `reboot` cross the STRICT proof (`tmux::interpret_stopped`): the server said so, and a missing socket is `Unknown` because a live server whose socket was unlinked answers the same ENOENT. A RESUME, the fleet LISTING, an `END` and a `RENAME` may also cross the boot-time proof (`tmux::classify_absence`): on ENOENT alone, a session whose own last sign of life predates the host's boot is `Absent`, because no process survives a reboot. For an END this is reachable ONLY after the human passed `--assume-stopped` for THAT single target AND the target's POSITIVE record names the unreachable server — the flag and the independent proof, both or refuse; for a RENAME (which deletes nothing) the proof alone suffices. ONE composition, `src/lifecycle/end.rs::boot_proved_stopped`, serves both verbs: no other site may compose the proof, and the Positive arm's refusal text is byte-identical when either half is missing. That sign of life is `inventory::last_live` — the `.launch-attempt` stamp (written under the lifecycle lock BEFORE every tmux create, by launch, resume and spawn, for every tool, and CHECKED: a launch that cannot write it creates no session), plus `started`, `launch_time.<slot>`, `capture_floor.<slot>` and the watchdog pidfile's mtime. NEVER the meta's mtime (migration, refresh and rename rewrite it) and NEVER `events.jsonl` (`memo add`, `goal` and audit records are appended from outside a live session). A moment is a STRICTLY POSITIVE epoch and the grammar is per source: a row that names a launch moment and spells none is damage, and `capture_floor.<slot>=0` is the ONE documented non-positive claim (the no-origin sentinel), silent for liveness and borrowable by nothing else. Every gap fails closed to `Unknown` and says which gap | `src/tmux.rs` (`classify_absence` owns the rule, `interpret_stopped` the strict one), `src/inventory.rs::last_live`, `src/store.rs::LAUNCH_ATTEMPT`, `src/doors.rs::boot_time`, `src/lifecycle/end.rs::boot_proved_stopped`; the callers and the one-occurrence-per-primitive count in `end.rs` are pinned by `tests/it/doors.rs::the_boot_time_proof_is_reachable_from_exactly_its_named_operations` |
| Archive on `ae end` is MANDATORY: a failed archive fails the end with state intact | `src/lifecycle/end.rs::archive_step`, ordered before cleanup; `src/archive/publish.rs` only publishes |
| An archive under `~/.ae/archive/<uuid>/` is INERT — data only, never an executable file | `src/archive/store.rs` (`write_file_0600`, `mkdir_0700`) |
| Every turn ae injects into an agent's input carries a first-line marker from ONE owner — `msg` (peer), `ctx` (ae's own binding setup), `brief` (task contract), `interrupt` (control action); absence of one means the human, `relay` stays bare by design, `ae compact`'s dispatch paste is bare by the one ruled exception beside it (a slash command must start with `/`), and no emission site spells a marker by hand | `src/provenance.rs` owns the spellings and the one first-line renderer; the verbs `RULES` describes equal `VERBS`, pinned by `src/render.rs::the_authority_rule_names_exactly_the_verbs_the_owner_emits` |
| `ae compact [name]` compacts the fixed seats in place, in roster order, one nonblocking run lock for the whole run and no persisted run state: each admitted seat first proves a fresh durable checkpoint (a reply AND a memo carrying this request's id, both with a complete caller triple), then takes the guarded dispatch whose R10 call is matched into exactly `dispatched` (attempted, never proof of submission), `skipped (<reason>)` or `not dispatched (<reason>)`; spawned seats are skipped untouched, every outcome advances, and the run audits each seat plus its own start/end | `src/seatcompact_run.rs::run` is the verb, `src/seatcompact.rs` the vocabulary/gates/report, `src/deliver.rs::deliver_guarded` the one guarded operation |
| A seat sitting on a prompt only the HUMAN may answer is NAMED within 2 watchdog cycles and ae NEVER answers it. The detector is PURE and cannot reach a pane — it takes a captured frame and a binary name, nothing else — and it is gated on the exact binary `agy`, so a renamed binary reads as no prompt and degrades SAFE. The shape must hold INSIDE a fixed window measured up from the last non-blank row, never a trailing run of non-blank rows (agy's modal ends on a status line, so a run degenerates to that one line): a question row, a SELECTED option row with at least one sibling on EITHER side of it — the selection travels with the human's arrow keys — a key-hint row below them, and NO composer, that last read by the composer's own owner (`deliver::region::composed_ui`) rather than a second copy of its fence, scoped to the same window. Every question row is tried top-down and the first COMPLETE shape wins, because a single-shot match pairs a transcript question with the real hint row, encloses no option, and loses the live modal beneath it. STABILITY is the latch: the streak IS the flag, reaching the bound exactly once, so an episode is named once however long it lasts — `throttle_streak`'s own rule — and the clear needs a SUCCESSFUL capture, because a failed read is an absence of evidence that would otherwise flap the latch against an untouched modal. It ranks BELOW dead, the sweep, a declaration and the two vendor verdicts, and ABOVE the harness frames, because that is where `Stale` is decided and a seat waiting on a modal is silent BY NATURE — below them the feature would never draw. It reuses `Mark::NeedsYou` and the EXISTING `Reason::Blocked`, so the frozen seven-reason contract stands and no drawn format changes. A brief retry refuses while it waits, through TWO channels because daemon memory does not cross into the helper process: the daemon skips the exec from the verdict it already has, and the leg asks the pane itself, fail-closed | `src/watchdog.rs::human_prompt_class` is the ONE detector and `HUMAN_PROMPT_WINDOW` its anchor; `src/watchdog_daemon.rs::book_human_prompt` owns the latch and branch 8.5, its 5c the clear; `src/events.rs::alert_meaning` maps both actions; `src/brief_retry.rs::decide` holds the refusing arm |
| A BRIEF `spawn` could not deliver is delivered LATER by the session's own watchdog, byte-identical to what `spawn` would have pasted (first line = `provenance::brief(<original spawner>)`), AT MOST ONCE submitted, only into the SAME seat incarnation, or it is given up LOUDLY. The record is written by ONE writer, `spawn` on its undelivered path, and read by NAMING a roster slot's own file — never a glob — so a legacy `undelivered.*.txt` stays inert forever. `attempts` is the number of times ae ENTERED `deliver`: readiness, busy and human-typing are seen BEFORE anything is published and skip the cycle untouched, so only the narrow race between the readiness proof and the target lock burns one; `created` carries the wall bound instead, and past 30 minutes a record is given up whatever its attempts say. The crash window is closed by ORDER, not by a guess: the bumped attempt and `phase=pasting` are made durable BEFORE the paste, so a record found mid-flight is never pasted again, only given up. A re-arm is therefore reachable only from the four refusals `deliver` decides BEFORE its first key reaches the pane. The marker is NOT a stored field: `deliver`'s `frame` stamps it from the record's `actor` under `Shape::Launch`, which is what puts it out of a caller's reach and why a marker written into the body would be pasted twice. TRUST: the leg takes the text AND the actor from the record and never from argv, the environment or the caller, so forging the trigger can at most re-fire a brief the spawner already authorized; the record file itself is trusted exactly as far as the meta store is. Every mutation is compare-and-swapped against the bytes the flight read, because a `retire` plus a re-spawn mid-flight leaves a SUCCESSOR at the same slot. Scheduling is a ROTATION over oldest-created-first, total-ordered by slot, resuming after the slot tried last — one delivery a cycle still, but a stuck record cannot starve the briefs behind it — while damage classification stays OUTSIDE that budget. The `brief-delivered` EVENT is the truth: a kept `undelivered.<name>.txt` after one is residue, never a reason to hand-send. A brief that rides the launch turn instead of a paste arms no record at all | `src/brief_retry.rs` owns the record, its damage classes and the ONE gate `::decide`; `src/brief_retry/leg.rs` owns the delivery, `::fly` the ordering proof; `src/watchdog_daemon.rs::retry_briefs` owns the rotation and the one-per-cycle budget; `src/spawn.rs::record_for_retry` is the one writer |
| `ae reseat <session> <agent> --using <profile>` moves ONE seat to another profile IN PLACE — same slot, same pane, same name, same records — and KILLS NOTHING: the seat's tool must already be gone, proven by the ONE dead proof `relaunch` makes, asked with this verb's word so both verbs refuse for the same reasons in the same order and no second liveness rule exists. Everything DURABLE is answered from the records before tmux is read at all — the argv, the session, the caller (a pane ae STAMPED may reseat only its own session; an unstamped shell may reseat any), the ROSTER seat, the profile differing from the recorded one, and the profile resolving exactly as `run::read_seat` resolves a seat with no client override (no tool-class judgement: a reseat accepts the profiles a LAUNCH accepts) — so a stopped session diagnoses a typo exactly as a running one does; the pane's own stamp is then checked against the roster slot, because a disagreement would publish for one seat and paste into another's pane. Under `.lifecycle.<session>.lock`, in this order: prove dead; BUILD the seed, before anything is removed, because it reads the recorded first message the next step deletes; publish `seed.<agent>.md` 0600 and KEEP it, since it is what a human re-sends by hand; `run::clear_slot`; ONE guarded meta replacement; re-read the seat; paste and observe. The lock is DROPPED before any readiness wait, because a gated turn blocks up to 45 s. Past it the tool's OWN launch turn goes first where its adapter has one — codex's rollout does not exist until a user turn and that handshake must not sit under a 24 KB seed — then the seed as `⟦ae:ctx⟧`, then the capture, then a LAST identity reading, because exit 0 means the seat is up NOW. The move WRITES the profile, the tool, a fresh conversation (a UUID where the tool takes one at launch, `pending` otherwise), a launch token and a capture floor, and appends the old conversation to the predecessor list TAGGED with the tool that OWNS it, since the successor's tool reads a different store; it REMOVES the config home and its implicit base (they belong to the tool that is leaving), the launch time (it would date a launch that has not happened), the observed model and its pin (they would read as drift the moment the new tool answers) and the launch's `client` override — removed, never emptied, because an empty override is not the same fact as no override. The guard is the SEAT and not the slot: a `retire` plus a re-`spawn` can land a successor there mid-flight and it must not inherit the move. NEITHER crash window is atomic and neither needs to be: before the meta write the seat still records its old profile with no start marker, after it the pane sits at a shell, and `relaunch` finishes both. The seed is the pack `ae brief --seat` renders, from ONE builder, so what a human reads before a move and what the successor is handed are the same document — and because that document is PASTED, every field the pack takes out of a record routes through the ONE neutraliser, control bytes included: an escape sequence, a bell or a bracketed-paste terminator in a memo, a reason or a quoted turn would be KEYSTROKES in the successor's pane. ONE field is PROVEN instead of cleaned — a request id, because the pack also prints it into the `reply` command the successor is told to run and that command quotes an id without escaping it: ae mints every id it owns, so a row failing that grammar is dropped WHOLE and COUNTED, never rendered and never repaired, since a cleaned id would name a request that does not exist | `src/reseat.rs` is the verb, its ladder and its order; `src/seat_relaunch.rs::{prove_dead,start,observe_identity,turn_verdict}` is the borrowed operation and `::Verb` the one word that differs between the two callers; `src/meta.rs::reseated` is the pure move, `::seat_move_for` its guard and `::publish_seat_move` the only writer; `src/lib.rs::seat_pack` is the ONE pack builder `ae brief --seat` and the seed both read; `src/seatpack.rs::neutralise` is the ONE neutraliser, pinned by `::nothing_a_terminal_would_act_on_survives_into_a_pack_that_is_pasted`, and `::minted_requests` the one grammar gate, pinned by `::a_request_id_ae_never_minted_is_dropped_whole_and_counted` |

Role doctrine: [docs/gatekeeping.md](docs/gatekeeping.md) before gating or reviewing;
[docs/design-patterns.md](docs/design-patterns.md) for the coordination patterns.

## Config

INI-style, one regex parser, `src/config.rs`. No TOML/YAML/JSON parsing. Do not extend it — the ONE
exception is the `"""` block for `[prompt] instructions` shown above: raw multi-line text for that
one key, and nothing else.

```toml
[clients]
claude = claude
cc-mic = claude config_home=$HOME/.claude-mic
codex = codex manual_resets=1   # manual window resets in hand; 0-9, order-free

[profiles]
fable = "claude --model fable"
fable-mic = "cc-mic --model fable"

[roster]
name = profile
orchestrator = profile  # the seat ae orchestrator runs

[workspace]
main = name
workers = name, name2      # optional, omit for single-agent start
layout = vertical
auto_upgrade = on        # installed ae checks quietly; global config only
fleet_order = a, b, c      # fleet-strip order, named first; global config only
palette = darcula          # darcula (default), a = neutral dark, b = warmer
icons = on                 # off swaps the glyph set for its ASCII fallback
theme = on                 # off keeps YOUR status line; ae still fills @ae_*
motion = on                # off freezes the spinner
quota = on                 # off stops ae acting on vendor quota (advisories and the
                           # Low-entry checkpoint ask alike); ae quota stays
quota_every_secs = 300     # watchdog quota advisory cadence; 0 disables
idle_nudge_secs = 300      # positive empty-input reminder cadence; 0 disables

[prompt]
instructions = "Custom instructions injected into agent system prompts"
# A long value goes in a block: the opener line is exactly instructions = """
# and a line that is exactly """ closes it. Between them every line is raw
# text — a [section] header, a key = value line and a # comment are NOT parsed.
# A raw line that is exactly """ cannot appear inside the block, because that
# line closes it; no escape exists.
# instructions = """
# SPEND POLICY: keep replies short.
# Cite the file for every claim.
# """
```
