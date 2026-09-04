# ae

One Rust core in one immutable versioned install, published read-only. tmux is the runtime.
The only Bash in the product is `install`, the 79-line bootstrap that publishes the core.

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
- Not a cost tracker. Agents track their own usage.
- Not a logging system. tmux already does `capture-pane` and `pipe-pane`.
- Not a git workflow tool. It does the minimum (commit + push), nothing more.
- Not a plugin framework. Wrap `ae` in a script if you need custom behavior.

## Structure

```
src/                — Rust sources. main.rs thin (argv in, exit code out); lib.rs and one
                      module per domain hold everything testable
tests/it/           — the one integration-test target. The behaviours of the retired bash
                      suites are pinned here as Rust tests.
                      doors.rs = capability boundary; gate.rs = justfile/install guards;
                      parity.rs = the one child-process door. Unit tests sit beside the code
tests/fixtures/     — frozen inputs the suites read (session shapes, list goldens)
install             — the bootstrap: download a bundle, prove it against the release
                      manifest, extract, `ae-core _install --from <tmp>`. The only bash file
justfile            — dev/release pipeline; holds every dev-tool version pin
docs/               — user + internals docs; history.md holds the retired contract
contrib/            — optional sidecars: aeorchestrator (templates only, no code)
.github/workflows/  — rust lanes (both platforms) + dispatch-only release-proof lanes
Cargo.toml          — one crate, bin + lib, both named `ae`. No workspace
rust-toolchain.toml — compiler pin, profile, components, both targets
clippy.toml         — tests-only unwrap/expect relaxation + the two capability denies.
                      deny.toml — supply chain. taplo.toml — TOML scope. .cargo/ — aliases,
                      musl linker, cargo-mutants config
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

Other rules of the loop:

- **Live probes go in the `ae-dev` namespace** — `~/.local/bin/ae-dev`: own `~/.ae-dev` home
  and config, own tmux server (`-L ae-dev`), checkout binary. NEVER the default server or `~/.ae`.
- **Workers get their own git worktree.** `git worktree add <path> -b <branch> main`.
- **One writer per file.** Other agents edit this tree concurrently; coordinate before
  reverting or overwriting anything you did not change.
- **No lifecycle commands against a live session you do not own** — no `ae end`, `ae stop`,
  `ae rm`, no `retire` of someone else's agent.
- **Never `export HOME=… AE_HOME="$HOME/.ae"` in ONE statement.** The shell expands every
  word before any assignment, so `AE_HOME` binds to the REAL home. This clobbered `~/.ae`
  twice. Separate the statements, or assign both from the literal temp path.
- **CI pins too**: runner images are exact (`ubuntu-24.04`, `macos-15`, never `-latest`), and
  actions are first-party and SHA-pinned with the version in a trailing comment.
- **Port behaviour, not code.** Drop features on the way rather than transliterating.

## Hard rules — never do these

- `unsafe_code` is **forbid**. There is no exception worth having.
- No `unwrap()` / `expect()` in production code. `-D warnings` makes it fail the gate.
- No `std::process::Command` outside the enumerated doors. The PRODUCT has four —
  `src/transport.rs`, `src/run.rs`, `src/upgrade.rs`, `src/install.rs` — and the suite has
  twelve, across `tests/it/`'s `cli.rs`, `install.rs`, `shape.rs`, `parity.rs` and `doors.rs`.
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
  meta, journals, archives and anything hand-editable are hostile input.

## Toolchain pins

Pins, not channels. CI, laptop and agent sandbox must resolve to the same compiler.

| What | Pin | Declared in |
|---|---|---|
| Compiler | `1.97.1` (exact release) | `rust-toolchain.toml` |
| Edition / MSRV | `2024` / `rust-version = "1.97.1"` | `Cargo.toml` |
| Profile + components | `minimal` + rustfmt, clippy, llvm-tools | `rust-toolchain.toml` |
| Targets | `aarch64-apple-darwin`, `x86_64-unknown-linux-musl` | `rust-toolchain.toml`, justfile, `deny.toml` |
| Dev tools | nextest `0.9.143`, taplo `0.10.0`, deny `0.20.2`, mutants `27.1.0`, llvm-cov `0.9.0`, vet `0.10.2` | justfile `*_VERSION` — the single source |
| `just` | `1.57.0` | justfile `JUST_VERSION`; CI reads the pin from there |

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
| `just rust-build-release` / `bundles` | native release binary (native only, a bare clone must build) / both platform bundles + `SHA256SUMS` into `dist/` (needs the musl cross toolchain) |
| `just release` | the whole release, locally. Pre-flight refuses before any state is written |

`ae upgrade` (and `ae _install`, the same publish) is not only a binary swap: between the
new version directory and the repointed link it migrates EVERY session under
`<HOME>/.ae/sessions`, running or stopped — the chain, then `ae_core`/`ae_core_version`/
`ae_version` rewritten, then the helper links re-rendered, then the watchdog and the
Telegram bridge of a running one restarted on the new core. Agent panes are never touched:
they run the agent tool, not ae.

`<HOME>/.ae`, not `<AE_HOME>`. A publish is `$HOME`-pinned end to end — the version
directories and `~/.local/bin/ae` are — and in the INSTALLED shape, the only one an upgrade
happens in, the two are the same directory. A checkout run with `AE_HOME` elsewhere would
publish into `$HOME/.ae` and migrate the sessions there; that is why a live upgrade probe
belongs in a sandboxed `$HOME`, not in the `ae-dev` namespace.

Nothing is written until every session has been asked, so a session that cannot be migrated
aborts the publish, by name, with the old link intact and no session repointed. After the
journal is removed — not before, because the prune can otherwise delete the rollback target
the journal names — every unreferenced `versions/<V>` is deleted, so there is one installed
version and no relink-to-yesterday rollback. `ae list` marks a session the publish did not
reach.

**`ae upgrade` hands the publish to the DOWNLOADED core** (`<bundle>/ae-core _install
--from`), the way the `install` bootstrap already does. The migration steps for versions
N..M live in the core being INSTALLED; a publish run in-process by the OLD core would
migrate with the rules of the release it is replacing, and on the first real schema change
would have no step to run at all. One consequence is unavoidable and one-time: upgrading
FROM a core that predates this ruling runs that core's publish, which has no sweep, so those
sessions arrive unmigrated and are refused on resume until they are ended.

## Session helpers

The core LINKS 21 names into `~/.ae/sessions/<name>/`. Every one is a **symlink to the core
binary**; the core dispatches on `argv[0]`'s basename and derives the session from its
dirname. Names and argv are the compatibility contract.

| Helper | Purpose |
|---|---|
| `send <agent> <msg>` | Deliver a message to another agent's pane. Refuses a dead pane, defers on busy/human input, verifies the submit |
| `ask <agent> <question>` | Tracked request with a request ID and an exact reply command |
| `review <agent> <request>` | Ask for a critical review, findings first |
| `reply <request-id> <msg>` | Reply to a logged ask/review. Verified against the request's stored slot |
| `requests [mine\|inbox\|all]` | Inspect pending and replied requests without peeking panes |
| `state <working\|waiting-user\|blocked\|done> [reason]` | Declare work state; shows in `ae list`. Only `done`, `waiting-user` and `blocked` quiet the watchdog — `working` does not. `mark-done [msg]` = `state done` plus the legacy `done` event |
| `say <text>` | Push a line to the human's Telegram chat. Pane output is NOT forwarded |
| `memo add [--topic t] <text>` / `memo read` / `memo tail [n]` | Durable shared session memory |
| `goal [text\|--clear]` | The session's one-line objective. Survives resume; shown in `ae list` |
| `peek <agent> [lines]` / `peak` | Capture recent pane output. Inspection only, never a reply channel |
| `agents [--all]` | List agents with pane IDs and processes. `focus <agent>` switches tmux focus |
| `interrupt <agent> [msg]` | Cancel current generation, optionally send new instructions |
| `spawn <name> --using <profile> [prompt]` | Add an agent to the workspace |
| `retire <name>` \| `retire %pane` | Remove a spawned agent. Exact name only; `main`/`worker` refuse |
| `_register-sid` | codex's own session-id handshake. The one helper no human types |
| `watchdog`, `events-tail`, `loop` | The two monitor panes' whole command (`loop` = deprecated alias) |

**Call a helper by its FULL PATH.** No `/` in `argv[0]` means no session to derive, and the
core exits 2 rather than guessing. That is why they are not on `PATH`.
Name resolution takes the exact name, `%pane-id`, or `session:agent` / `@session:agent`.

## Agent tool capabilities

| | Claude Code | Codex | Gemini CLI | Antigravity (`agy`) | Grok Build | OpenCode |
|---|---|---|---|---|---|---|
| **Prompt injection** | `--append-system-prompt` | `-c developer_instructions=` | `-i` | none — rides `-i` as a user turn | none — rides positional `[PROMPT]`; never `--system-prompt-override` | `OPENCODE_CONFIG` json `instructions` |
| **Session id at launch** | `--session-id UUID` | none | none | none | `--session-id UUID` | none |
| **Id capture** | immediate | post-launch: sid file, token scan, cwd scan, TUI header | post-launch chat-history scan | post-launch: `<id>.db` bytes with a token, else `cli-*.log`. A token miss stays `pending` | immediate | post-launch `session list --format json` |
| **Exact resume** | `--resume UUID` | `resume UUID` (subcommand) | `--resume UUID` | `--conversation UUID` | `--resume UUID` | `--session ID` |
| **Resume fallback** | `--continue` | fresh start | `--resume latest` | `--continue` | `--continue` | `--continue` |
| **TUI modelled for delivery** | yes | yes | no | no | no | no |
| **`_run` re-run** | exact resume when the recorded id passes the tool's store probe (or the tool has no probe); a gone conversation takes the fallback above | same | same | same | same | same |

- A drawn input box is not an initialized tool. Paste-driven delivery is gated by
  `src/deliver.rs::input_ready` / `wait_input_ready`; a timeout is a loud, durable failure.
- **codex's rollout does not exist until the first USER turn.** Measured on codex-cli
  0.153.2 (2026-09-04): ae's exact argv with no positional prompt writes nothing under
  `~/.codex/sessions/<day>/` for 30s, and the header carries no session id before that
  turn either — so both the token scan and the header scrape have nothing to find. The
  turn stays, and it is PASSIVE: wording and reason at `src/launch.rs::initial_prompt_for`.
- **agy's trust modal blocks the pane** until a human answers, and its trust list is
  exact-path. ae's context survives it (argv), a pasted brief would not.
- Meta v2 roster: `seat.<slot>` / `profile.<slot>` / `agent_bin.<slot>` / `harness_session.<slot>`.
  A legacy `agent.<slot>` row is refused and recorded `degraded: true`, never migrated.

## Environment doors

`src/doors.rs` is the ENTRY/SHAPE surface — the facts the deleted wrapper used to hand over.
Other ambient reads are named doors at their own use site, each carrying its reason and each
inventoried by the clippy `disallowed-methods` boundary. Never read the world ad hoc.

| Door | Decides | Read in |
|---|---|---|
| `HOME` | where ae state lives | both shapes |
| `PWD` | the caller's working directory | both |
| `AE_HOME` | relocates ALL ae state | CHECKOUT only |
| `CONFIG_FILE` | which global config is read | CHECKOUT only |
| `AE_TMUX_SERVER` + `AE_TMUX_SERVER_KIND` | which tmux server a launch lands on | CHECKOUT only |
| `AE_NO_AUTOSTART` | start neither companion | both |
| `TMUX` / `TMUX_PANE` | which pane this shell is, for `stop` and `watchdog` | both |

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
| Session name `^[A-Za-z0-9][A-Za-z0-9_-]{0,127}$`, checked before any side effect | `src/session_launch/name.rs::is_session_name` |
| Agent name `^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$`; it reaches a system prompt, so it is an allowlist | `src/config.rs::is_agent_name` |
| That name is re-validated at the interpolation site, fail-quiet | `src/render.rs::context_document` |
| Dispatch is on `argv[0]`'s basename; no `/` means exit 2, never a guess | `src/shim.rs` |
| `current_exe()` has exactly ONE caller | `src/shape.rs::resolved_exe` |
| `launch.<slot>.started` decides create-vs-resume, before the exec | `src/run.rs` |
| The install gate is STRUCTURAL and hashes nothing. Every command and helper passes it EXCEPT `version` and `upgrade`, which diagnose and repair a broken install | `src/shape.rs`, ordered in `src/lib.rs::run` |
| The one hashing site: both members re-digested against `SHA256SUMS` before publication | `src/install.rs` |
| Published dir 0555, members 0555/0444; `~/.local/bin/ae` is the current pointer | `src/install.rs` |
| Every session meta carries `meta_version=<N>`; the chain steps N->N+1 and runs wherever the core touches a session. A missing row is the pre-version past: REFUSED at resume, REPORTED at stop and end, which must never be blocked. A publish migrates and repoints EVERY session before it moves the command link, then deletes every `versions/<V>` no meta records | `src/migrate.rs`, called from `src/install.rs::publish_steps`, `src/session_launch.rs`, `src/lifecycle.rs` and `src/lifecycle/end.rs` |
| A harness session id is a NAME: the purge proves it against the archive UUID grammar before it builds a path | `src/lifecycle/end.rs::purge_conversation_files`; grammar in `src/archive.rs::canonical_uuid` |
| A monitor sweep may act only on the CALLER'S own session (`$TMUX_PANE`) | `src/monitor.rs` |
| Every tmux format uses a printable pipe separator, never a control byte — tmux 3.4 octal-escapes those. Each format literal is written out; `SLOTS_FORMAT` deliberately uses an unspaced pipe | `src/tmux.rs` (`FIELD_SEPARATOR` is the parser delimiter) + the control-char-free test over every format constant |
| The server pair is read by SET, not by nonempty; an untypeable pair is refused | `src/doors.rs` |
| Control bytes never reach JSON raw: they are written as JSON escapes | `src/json.rs` |
| Exit codes: `0` success, `2` usage error, `1` everything else | `src/cli.rs` + `src/lib.rs::run`; `src/main.rs` only maps the byte |
| Archive on `ae end` is MANDATORY: a failed archive fails the end with state intact | `src/lifecycle/end.rs::archive_step`, ordered before cleanup; `src/archive/publish.rs` only publishes |
| An archive under `~/.ae/archive/<uuid>/` is INERT — data only, never an executable file | `src/archive/store.rs` (`write_file_0600`, `mkdir_0700`) |

Role doctrine: [docs/gatekeeping.md](docs/gatekeeping.md) before gating or reviewing;
[docs/design-patterns.md](docs/design-patterns.md) for the coordination patterns.

## Config

INI-style, one regex parser, `src/config.rs`. No TOML/YAML/JSON parsing. Do not extend it.

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
