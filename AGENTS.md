# ae

Before touching a module, read its matching topic below.
Module map: [Structure](docs/reference/contract.md#structure).

## Read before acting

Role doctrine: [docs/gatekeeping.md](docs/gatekeeping.md) before gating or reviewing;
[docs/design-patterns.md](docs/design-patterns.md) for the coordination patterns.

Doors: every ambient read and process start uses a named door carrying its reason; never read the world ad hoc. See [Environment doors](docs/reference/contract.md#environment-doors).

| Touching | Read first |
|---|---|
| src/shim.rs, helper commands | [Session helpers](docs/reference/contract.md#session-helpers) |
| src/tool.rs, adapters, src/run.rs, src/launch.rs | [Agent tool capabilities](docs/reference/contract.md#agent-tool-capabilities) |
| src/doors.rs, src/transport.rs, environment reads | [Environment doors](docs/reference/contract.md#environment-doors) |
| shell commands on macOS or Linux | [GNU vs BSD — check the command YOU are about to type](docs/reference/contract.md#gnu-vs-bsd--check-the-command-you-are-about-to-type) |
| any src/ module; find its named owner before editing | [Key invariants](docs/reference/contract.md#key-invariants) |
| src/config.rs, src/meta.rs, config grammar | [Config](docs/reference/contract.md#config) |

New automation features are paused until a daily-use inventory exists. Only the human may lift this pause.

One Rust core in one immutable versioned install, published read-only. tmux is the runtime.
The only Bash in the product is `install`, the 83-line bootstrap that publishes the core.

This file is the CURRENT contract: what to do, what never to do, who owns each rule. The
reasoning, the retired rules and every measurement narrative are in
[docs/history.md](docs/history.md). Product direction: [VISION.md](VISION.md).

## What ae is

- A thin wrapper around tmux. Not a framework, not a platform.
- Built for daily productivity, not completeness. If it does not save time on every use, cut it.
- Resistant to features. If tmux already does it, do not re-implement it.
- The ownership map fits in one sitting. Keep it that way.
- One crate with modules, because that is how a Rust program stays readable.

## What ae is NOT

- Not a CI/CD pipeline. Use your existing workflow for that.
- Not a billing system. `ae usage` prices the agents' own transcripts at list price, offline;
  it never meters, caps or bills, and never substitutes a profile; the lead steps down.
- Not a logging system. tmux already does `capture-pane` and `pipe-pane`.
- Not a git workflow tool. It does the minimum (commit + push), nothing more.
- Not a plugin framework. Wrap `ae` in a script if you need custom behavior.

## How to work here

The loop, in order:

1. `just test` — THE GATE. Exactly `just rust-check` (fmt-check + lint + fuzz compile + test).
   There is no per-commit fast/slow split; run the whole thing. `just pre` (fmt + clippy + unit +
   guard tests) is ADVISORY: it learns early that a format, lint or guard check will fail, is NOT
   the gate and is never cited as it. The lane sizes its threads and build jobs
   from `jobs-budget` when installed and runs cargo niced (docs/development.md).
2. `just check` — only when you touched `install` (shellcheck + shfmt).
3. Cross-model review BEFORE the commit, for anything significant. Route it through ae
   (`<session>/ask`, `<session>/review`, `<session>/spawn <alias>:reviewer`) — never shell
   out to another CLI, never a harness-internal subagent.
4. Commit on green, after the review findings are answered. Plain message, no trailers,
   no AI attribution of any kind.
5. `just release` — local, one machine: check, test, rust-deny, CalVer bump, both bundles, tag, push,
   assets. Nothing waits on a runner.

Prefer small vertical slices and fast iterations. Commit, release, push and roll out
standalone intermediate state or sub-features rather than waiting for a feature to finish.
A slice ships when it is green, reviewed and leaves the product coherent — not when its feature
is finished. This changes when we ship, not what we skip: `just test`, cross-model review, the
freeze protocol and every hard rule remain mandatory.

An upgrade is not a binary swap: it migrates, repoints and relinks every placeable session
before it moves the command link, reports and skips stopped unplaceable sessions, then prunes
unreferenced versions. A seat's pane line names the command link when installed and stays runnable across the upgrade.
A running installed `ae app` follows the link itself once idle: in its session's chat window tmux
respawns its pane, any other app execs the linked core in place (same pid, its shell kept); a
chat window whose proof fails only hints. ae's own app starts carry the internal `--app-own`
marker (the landings, an exec restart with no view), a view carrier, or a stamped chat pane; only a
human-typed `ae app` prints its one-line stderr hint.
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
  upgrade add `xterm-ghostty:hyperlinks` once, preserving existing terminal features,
  reassert the canonical server-global input map and remove stock right-click menus on
  ae-owned servers only; the user's tmux is untouched. That map also binds root `F12` and `C-]`,
  which detach only the client the session's `@ae_view_client` names (the `ae app` seat view
  outside tmux) and send the key on for every other client. On tmux 3.5+,
  `MouseDown1Status` sends session/window ranges through tmux navigation, `ae` / `ae-more` to
  the fleet picker and `ae-settings` to settings; `MouseDown3Status` sends those menu ranges to
  their matching menus and session ranges to the guarded Flip menu, and stale `MouseUp*Status`
  bindings are removed. On tmux 3.4, Down keeps navigation but does nothing on menu ranges, while
  Up opens the matching keyboard-driven menu or Flip menu after release. `<prefix> a` (default
  `C-b a`) opens the fleet picker without colliding with
  stock tmux's pane keys. The picker carries the invoking client explicitly, resolves that client's
  current session for its transient open marker, and a row switches to the captured session id
  before selecting its still-member lead pane. `<prefix> O` jumps the pressing client to the
  orchestrator's lead pane and back (`src/jump.rs`). A bind cannot be session-scoped.
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
  downloaded core's own `_install`), `src/install.rs` — and the suite has fifteen, across
  `tests/it/`'s `cli.rs`, `install.rs`, `migrate.rs`, `shape.rs`, `parity.rs` and `doors.rs`.
  `tests/it/doors.rs` pins the exact per-file counts, so a new door is a review, not a diff.
  Same for the world-reading methods in `clippy.toml`'s `disallowed-methods`: each lives at a
  named door carrying its reason. A new caller of an existing door moves neither inventory:
  `tests/it/phase3_probe_spec.rs` is the FILE-level world-read inventory, `tests/it/doors.rs` counts
  disallowed-TYPES relaxation SITES per file, so reads in an already-listed file add no file
  and calls through an existing door add no site.
- No clap, serde, anyhow, thiserror, chrono or nix. Adding any runtime dependency is a
  ruling, not a commit. See docs/history.md §11 for the researched line and its triggers.
  thiserror is carved out TRANSITIVELY only (kasuari and ratatui-core, under `ae app`): it is
  never a direct ae dependency and no ae code imports it (docs/history.md §11).
  #251 owner ruling permits exact-pinned pulldown-cmark (default features off),
  tables + strikethrough only; inert output, no tui-markdown (docs/history.md §11).
- **No new Bash.** `install` is policy-frozen. There is no other bash file and none may
  be added. The ONE shell literal is the chat's pane argv `src/console/toggle.rs::TTY_SETUP`
  (`stty`, then `exec`; termios needs libc or `unsafe`): not a bash file, not a door, one site.
  The chat's and the app's `!<command>` runs the HUMAN's own line, through the fixed
  `/bin/sh -c` outside tmux or the tmux server's default shell in a popup; ae authors no
  script text there, and `TTY_SETUP` stays the one ae shell literal.
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
| `just test` / `just rust-check` | **the gate**: `rust-fmt-check` + `rust-lint` + `rust-fuzz-check` + `rust-test` |
| `just pre` | **ADVISORY, NOT the gate**, never cited as it: `rust-fmt-check` + `rust-lint` + the unit tests + the `doors`, `floor` and `gate` guard modules of `tests/it`, through the lane's filtered arm. No doctests, no fuzz compile, no retries; `just test` stays exact |
| `just lane-lock claim <name> [secs]` / `release <name>` / `status` | the gate lane lock, one gate per machine: the MAIN checkout's `.local/locks/gate` directory (resolved from a linked worktree), whose `owner` line the recipe creates EXCLUSIVELY: that create is the claim, because `mkdir` is not atomic on every host (Ubuntu 26.04's uutils reports success for a directory that exists) and there the hand protocol has no owner proof; the `next` queue file. Same on-disk protocol as the hand one. `release` refuses any other owner; it never steals or clears a stale lock, it reports one (`status` exit 1 while held) |
| `just check` | the bash lane: shellcheck + shfmt over `install` |
| `just rust-fmt` / `rust-fmt-check` | `cargo fmt` + `taplo fmt` |
| `just rust-lint` | `cargo clippy --locked --all-targets --all-features -- -D warnings` + `taplo lint` |
| `just rust-test` | `cargo nextest run --locked` **and** `cargo test --doc --locked`. Both. nextest does not run doctests |
| `just rust-fuzz-check` | product compiler, locked `cargo check --bins` over every fuzz target; no execution. Uses inherited `CARGO_TARGET_DIR` or root `target` |
| `just rust-deny` / `rust-vet` | supply chain. The TLS graph is EXEMPTED, not audited (docs/history.md §11) |
| `just rust-mutants` | does the suite discriminate? A product slice runs it `--in-diff` over its own diff before freeze and answers every survivor in the freeze report (test added, or named equivalent); budget bounded, LOCAL, no percentage ever cited or gated (docs/development.md §Mutation testing). CI runs it diff-bounded on path-matching pushes to main and PRs targeting main when a source diff and usable base exist; the full lane is opt-in manual dispatch (`mutants=true`). Bounded in config: 2 tests, 60 s kill, RSS watcher (`AE_MUTANTS_RSS_MAX_MB`, default 2048). `rust-cov` reports, never gates |
| `just rust-fuzz target=<name> secs=60` / `rust-fuzz-all secs=60` | human-run cargo-fuzz over the hostile parsers, on an exact nightly. Refuses on an unpinned tool or a stale `fuzz/Cargo.lock`, ends on the cutover evidence line, and REPORTS — CI carries no nightly and never runs it (`fuzz/README.md`) |
| `just rust-build-release` / `bundles` | native release binary (native only, a bare clone must build) / both platform bundles + `SHA256SUMS` into `dist/` (needs the musl cross toolchain) |
| `just rust-prune` / `rust-prune apply=yes` | THIS checkout's stale `ae` build output: the report is the default and removes nothing, `apply=yes` runs five fixed `cargo clean -p ae` cleans, each proved by a `--dry-run` path check first. Refuses a redirected target dir, a non-root cwd, a linked tree, a process using the checkout or a dry run it cannot prove (a stdout line that is not an absolute path inside its tree, no single valid `Summary`). Own tree only, never during a gate (`docs/development.md`) |
| `just release` | the whole release, locally: check, test, rust-deny before the bump. Pre-flight refuses before any state is written |
| `just rust-linux` / `rust-linux-26` / `rust-linux-smoke` | ON DEMAND, never reached by `just test`, `just pre` or `just release`: the gate on real Linux, native arm64, in a local container (read-only checkout, `ae-linux-arm64-*` cache volumes, non-root, the mounted source checked against the host's HEAD, tree and dirty count) / the same on Ubuntu 26.04, own `ae-linux26-arm64-*` volumes; `rust-linux-26` (bus `stall`, the default since that arm went green, #267) adds a stalling session bus proved before the build, `rust-linux-26 off` is the validated-host arm / the x86_64 musl release bundle from `./dist` executed in an amd64 container. arm64 only: the x86_64 proof of Ubuntu 26.04 is a human run on x86_64 hardware |
| `just harness-smoke [control]` | ON-DEMAND live smoke of the adapter rows against the REAL harness binaries and logins (a few short turns per tool); never in the gate, CI or mutants; control = hang shim, no spend |
