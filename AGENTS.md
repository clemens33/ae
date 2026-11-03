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

Automation features are allowed again, but the focus stays on code quality, architecture and guardrails: a new one must earn its place by daily use.

One Rust core in one immutable versioned install, published read-only. tmux is the runtime.
Rationale, retired rules and measurement narrative: [docs/history.md](docs/history.md).
Product direction: [VISION.md](VISION.md).

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
   There is no per-commit fast/slow split; run the whole thing. `just pre` is ADVISORY, is NOT
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
before it moves the command link, then prunes unreferenced versions.
A publish is `$HOME`-pinned, so a checkout run whose state root differs
REFUSES `ae upgrade` before downloading anything.
Live upgrade probes therefore go in a sandboxed `$HOME`, never `ae-dev`. Why and the rest:
[docs/upgrade.md](docs/upgrade.md).

Other rules of the loop:

- **Persistent development goes in the `ae-dev` namespace** — `~/.local/bin/ae-dev`: own
  `~/.ae-dev` home and config, own tmux server (`-L ae-dev`), checkout binary. NEVER the
  default server or `~/.ae`. A one-shot launch probe gets a fresh private `TMUX_TMPDIR`, clears
  `TMUX` / `TMUX_PANE`, and uses scratch state; never let checkout `-L ae` reach the real socket:
  `probe=$(mktemp -d)` then
  `TMUX_TMPDIR="$probe" AE_HOME="$probe/state" CONFIG_FILE="$probe/config" env -u TMUX -u TMUX_PANE target/debug/ae …`.
  `TMUX_TMPDIR` must EXIST — tmux silently falls back to the real socket dir otherwise (3.7b).
- **A session's LOOK is session-scoped, and it has four writers with one job each.**
  `src/theme.rs` owns the palettes, the seven marks and every format. A LAUNCH writes the
  layout, the look facts and the attention SEED, and stamps each WINDOW. A RENAME rewrites the
  layout and the facts and leaves every verdict alone. The WATCHDOG owns the verdicts,
  refreshing them every cycle, and rewrites or UNSETS the layout only when `@ae_look_stamp`
  says the look has changed under it. The ADOPTER, any running watchdog, writes
  `theme::FLEET_STRIP_OPTION`, and nothing else, into every same-server ae session that has
  no live watchdog of its own. Any change to a status format or layout option MUST bump
  `theme::FORMAT_VERSION`, because that stamp change is the only thing that makes the
  watchdog repaint a running session's layout. Never write a global (`-g`) option, never
  `#()` in a format, and prove a look change by rendering it in `ae-dev` before it touches a
  live session. The launch also stamps, by main-pane ID, a session-scoped focus hook guarded
  by its captured session ID and a lead-pair window-scoped resize hook, never by name or
  globally; both apply with `theme = off` and are not part of the look. Launch and upgrade add
  `xterm-ghostty:hyperlinks` once, preserving existing terminal features, reassert the
  canonical server-global input map and remove stock right-click menus on ae-owned servers
  only; the user's tmux is untouched. A bind cannot be session-scoped. Mouse, picker, jump and
  seat-view bindings: [docs/internals/theme.md](docs/internals/theme.md).
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
- No `std::process::Command` outside the enumerated doors: in the product only
  `src/transport.rs`, `src/run.rs`, `src/upgrade.rs` (TWICE) and `src/install.rs`.
  `tests/it/doors.rs` pins the exact per-file counts, so a new door is a review, not a diff.
  Same for the world-reading methods in `clippy.toml`'s `disallowed-methods`: each lives at a
  named door carrying its reason. Inventories and the counting rule:
  [Environment doors](docs/reference/contract.md#environment-doors).
- No clap, serde, anyhow, thiserror, chrono or nix. Adding any runtime dependency is a
  ruling, not a commit (researched line and triggers: docs/history.md §11). Carve-outs:
  thiserror TRANSITIVELY only (kasuari and ratatui-core, under `ae app`), never a direct ae
  dependency and no ae code imports it; #251 owner ruling permits exact-pinned pulldown-cmark
  (default features off), tables + strikethrough only; inert output, no tui-markdown.
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

Pins, not channels: `rust-toolchain.toml` and the justfile `*_VERSION` are the single sources, so CI, laptop and agent sandbox resolve to the same compiler. Versions and where each is declared: [docs/development.md](docs/development.md#pinned-versions).
The fuzz compiler is a separate exact nightly (justfile `FUZZ_TOOLCHAIN`, the only source): the product compiler does not move, there is no `fuzz/rust-toolchain.toml`, the lane passes `cargo +<pin>`.
tmux floor `3.4`: a launch and the picker REFUSE below it; `list`/`version`/`doctor`/`upgrade` do not (`src/tmux_floor.rs`).

`--locked` on every graph-consuming lane. Spellings differ: `cargo deny --locked check`
(global option), `cargo mutants --cargo-arg=--locked` (no native flag), `cargo fmt` exempt.

## Lanes

Every recipe, in full: [docs/development.md](docs/development.md#lane-reference). Those with a rule:

| Recipe | What it is |
|---|---|
| `just test` / `just rust-check` | **the gate**: `rust-fmt-check` + `rust-lint` + `rust-fuzz-check` + `rust-test` (nextest AND doctests: nextest does not run doctests) |
| `just pre` | **ADVISORY, NOT the gate**, never cited as it: `rust-fmt-check` + `rust-lint` + the unit tests + the `doors`, `floor` and `gate` guard modules of `tests/it`, through the lane's filtered arm. No doctests, no fuzz compile, no retries; `just test` stays exact |
| `just lane-lock claim <name> [secs]` / `release <name>` / `status` | the gate lane lock, one gate per machine: the MAIN checkout's `.local/locks/gate` (resolved from a linked worktree). `release` refuses any other owner; it never steals or clears a stale lock, it reports one (`status` exit 1 while held) |
| `just rust-mutants` | does the suite discriminate? A product slice runs it `--in-diff` over its own diff before freeze and answers every survivor in the freeze report (test added, or named equivalent); budget bounded, LOCAL, no percentage ever cited or gated (docs/development.md §Mutation testing). `rust-cov` reports, never gates |
| `just rust-fuzz target=<name> secs=60` / `rust-fuzz-all secs=60` | human-run cargo-fuzz over the hostile parsers, on an exact nightly. REPORTS — CI carries no nightly and never runs it (`fuzz/README.md`) |
| `just rust-deny` / `rust-vet` | supply chain. The TLS graph is EXEMPTED, not audited (docs/history.md §11) |
| `just rust-prune` / `rust-prune apply=yes` | THIS checkout's stale `ae` build output: the report is the default and removes nothing. Own tree only, never during a gate (`docs/development.md`) |
| `just rust-linux` / `rust-linux-26` / `rust-linux-smoke` / `harness-smoke [control]` | ON DEMAND, never reached by `just test`, `just pre` or `just release`. `harness-smoke` runs a few short turns against the REAL harness binaries and logins and is never in the gate, CI or mutants; control = hang shim, no spend |
