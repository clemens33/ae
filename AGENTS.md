# ae

Before touching a module, read its matching topic below.
Module map: [Structure](docs/reference/contract.md#structure).

| When | Read first |
|---|---|
| gating or reviewing | run [The protocol](docs/gatekeeping.md#the-protocol) and [the second protocol](docs/gatekeeping.md#the-second-protocol--reviewing-a-claim-a-criterion-or-a-body-of-evidence); read an entry from [Which entry do I need?](docs/gatekeeping.md#which-entry-do-i-need) when its symptom matches |
| coordination design | [docs/design-patterns.md](docs/design-patterns.md) |
| src/shim.rs, helper commands | [Session helpers](docs/reference/contract.md#session-helpers) |
| src/tool.rs, adapters, src/run.rs, src/launch.rs | [Agent tool capabilities](docs/reference/contract.md#agent-tool-capabilities) |
| src/doors.rs, src/transport.rs, environment reads | [Environment doors](docs/reference/contract.md#environment-doors) |
| src/theme.rs, status formats, tmux hooks and binds | [docs/internals/theme.md](docs/internals/theme.md) |
| src/upgrade.rs, `ae upgrade` | [docs/upgrade.md](docs/upgrade.md) |
| shell commands on macOS or Linux | [GNU vs BSD](docs/reference/contract.md#gnu-vs-bsd--check-the-command-you-are-about-to-type) |
| any src/ module; find its named owner before editing | [Key invariants](docs/reference/contract.md#key-invariants) |
| src/config.rs, src/meta.rs, config grammar | [Config](docs/reference/contract.md#config) |

Automation features are allowed again, but the focus stays on code quality, architecture and guardrails: a new one must earn its place by daily use.

One Rust core in one immutable versioned install, published read-only; tmux is the runtime.
Rationale and retired rules: [docs/history.md](docs/history.md). Product direction: [VISION.md](VISION.md).

## What ae is NOT

- Not a CI/CD pipeline. Use your existing workflow for that.
- Not a billing system. `ae usage` prices the agents' own transcripts at list price, offline;
  it never meters, caps or bills, and never substitutes a profile; the lead steps down.
- Not a logging system. tmux already does `capture-pane` and `pipe-pane`.
- Not a git workflow tool. It does the minimum (commit + push), nothing more.
- Not a plugin framework. Wrap `ae` in a script if you need custom behavior.

## How to work here

The loop, in order:

1. `just test` is the gate: exactly `just rust-check`, run whole, with no per-commit fast/slow split.
   `just pre` is ADVISORY and never cited as the gate.
2. `just check` only when you touched `install` (shellcheck + shfmt).
3. Cross-model review BEFORE the commit, for anything significant. Route it through ae
   (`<session>/ask`, `<session>/review`, `<session>/spawn <alias>:reviewer`) — never shell
   out to another CLI, never a harness-internal subagent.
4. Commit on green, after the review findings are answered. Plain message, no trailers,
   no AI attribution of any kind.
5. `just release` runs locally on one machine: check, test, rust-deny, CalVer bump, both bundles,
   tag, push, assets. Nothing waits on a runner.

Ship small vertical slices: commit, release and roll out standalone intermediate state or
sub-features instead of waiting for the whole feature. A slice ships when it is green, reviewed
and leaves the product coherent; `just test`, cross-model review, the freeze protocol and every
hard rule stay mandatory.

`ae upgrade` migrates and relinks every placeable session before it moves the command link,
then prunes unreferenced versions. A publish is `$HOME`-pinned, so a checkout whose state root differs refuses `ae upgrade` before it downloads
anything. Run live upgrade probes in a sandboxed `$HOME`, never in `ae-dev`.

Other rules of the loop:

- Persistent development goes in the `ae-dev` namespace: `~/.local/bin/ae-dev`, with its own
  `~/.ae-dev` home and config, its own tmux server (`-L ae-dev`) and the checkout binary; never
  the default server or `~/.ae`. A one-shot launch probe gets a fresh private `TMUX_TMPDIR`,
  clears `TMUX` / `TMUX_PANE` and uses scratch state, so checkout `-L ae` never reaches the real
  socket: `probe=$(mktemp -d)` then
  `TMUX_TMPDIR="$probe" AE_HOME="$probe/state" CONFIG_FILE="$probe/config" env -u TMUX -u TMUX_PANE target/debug/ae …`.
  `TMUX_TMPDIR` must exist: otherwise tmux silently falls back to the real socket dir (tmux 3.7b).
- **A session's LOOK is session-scoped, and it has four writers with one job each.**
  `src/theme.rs` owns the palettes and every format. A LAUNCH writes the layout, the look facts
  and the attention SEED, and stamps each WINDOW. A RENAME rewrites the layout and the facts and
  leaves every verdict alone. The WATCHDOG owns the verdicts, refreshing them every cycle, and
  rewrites or UNSETS the layout only when `@ae_look_stamp` says the look has changed under it.
  The ADOPTER, any running watchdog, writes `theme::FLEET_STRIP_OPTION`, and nothing else, into
  every same-server ae session that has no live watchdog of its own. Any change to a status
  format or layout option MUST bump `theme::FORMAT_VERSION`, because that stamp change is the
  only thing that makes the watchdog repaint a running session's layout. Never write a global
  (`-g`) option, never `#()` in a format, and prove a look change by rendering it in `ae-dev`
  before it touches a live session. Binds are server-global, never session-scoped: on ae-owned
  servers only, launch and upgrade reassert the canonical input map, remove the stock
  right-click menus and add `xterm-ghostty:hyperlinks` once, preserving existing terminal
  features; the user's tmux is untouched.
- **Workers get their own git worktree.** `git worktree add <path> -b <branch> main`.
- **Land a slice with `git merge --ff-only`** after the worker rebases onto the main tip. Never `--no-ff`: `just release` runs `git pull --rebase` and flattens merge commits.
- **One writer per file.** Other agents edit this tree concurrently; coordinate before
  reverting or overwriting anything you did not change.
- **No lifecycle commands against a live session you do not own** — no `ae end`, `ae stop`,
  `ae rm`, no `retire` of someone else's agent. An unforced `ae stop` or `ae end` from inside a
  session asks the attached human's tmux client to confirm; never answer that prompt or pass
  `-y` / `-f` unless the human explicitly authorized it.
- **Never `export HOME=… AE_HOME="$HOME/.ae"` in ONE statement.** The shell expands
  `$HOME` before the assignment, so `AE_HOME` points at the real `~/.ae` and clobbers it.
  Use two statements, or assign both from the literal temp path.
- **CI pins too**: runner images are exact (`ubuntu-24.04`, `macos-15`, never `-latest`), and
  actions are first-party and SHA-pinned with the version in a trailing comment.
- **Port behaviour, not code.** Drop features on the way rather than transliterating.
- A bug report or its triage first collects the facts listed in
  [docs/troubleshooting.md §Filing a bug](docs/troubleshooting.md#filing-a-bug).

## Hard rules — never do these

- `unsafe_code` is **forbid**.
- No `unwrap()` / `expect()` in production code. `-D warnings` makes it fail the gate.
- No `std::process::Command` and no `clippy.toml` `disallowed-methods` world read outside the
  enumerated doors; each door is named and carries its reason. `tests/it/doors.rs` pins the exact
  per-file counts, so a new door is a review, not a diff. Inventory and counting rule:
  [Environment doors](docs/reference/contract.md#environment-doors).
- No clap, serde, anyhow, thiserror, chrono or nix. A new runtime dependency is a ruling, not a
  commit (docs/history.md §11). Carve-outs: thiserror only transitively (kasuari and
  ratatui-core, under `ae app`), never a direct dependency or an import in ae code; the #251
  owner ruling permits exact-pinned pulldown-cmark (default features off), tables +
  strikethrough only, inert output, no tui-markdown.
- **No new Bash.** `install` is policy-frozen, and no other bash file may be added. The one
  shell literal is the chat's pane argv `src/console/toggle.rs::TTY_SETUP` (termios needs libc
  or `unsafe`): not a bash file, not a door, one site. The chat's and the
  app's `!<command>` runs the human's own line, through the fixed `/bin/sh -c` outside tmux or
  the tmux server's default shell in a popup; ae authors no script text there.
- **No Python anywhere in the product.**
- **Never write through a helper path.** `>`, `>>`, `chmod`, `cp`, `sed -i` FOLLOW the
  symlink and corrupt the shared core binary. `rm -f` first. Fixtures are where this bites.
- Pass a helper message as one single-quoted shell argument, and write a literal single quote
  as `'\''`: double quotes still expand `$`, backticks and `$(…)`.
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
