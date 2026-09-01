# Development

ae ships one public wrapper over an immutable versioned Rust core and minimal policy-frozen
Bash pane glue.
The repo contains that product, its installer and release lanes, glue/installer tests, Rust
tests, and this docs site.

## Layout

```
ae                  — the Bash pane glue (bundled as `ae-glue`)
ae-entry            — public wrapper source (installed and bundled as `ae`)
justfile            — dev/release pipeline (just check, just test, just release)
cliff.toml          — git-cliff config (CalVer-compatible changelog)
tests/unit          — pure-function unit tests (bash, no deps)
tests/integration   — integration tests (requires tmux, git)
tests/it/           — Rust integration target
install             — canonical checksum-verifying versioned installer
Cargo.toml          — Rust package (bin + lib, both `ae`)
src/                — Rust core sources
README.md           — short user-facing intro
AGENTS.md           — project-level rules for agents working in this repo
CLAUDE.md           — symlink to AGENTS.md for Claude Code
docs/               — this site (MkDocs Material)
mkdocs.yml          — docs site config
```

## Toolchain

| Tool | Purpose |
|---|---|
| [just](https://github.com/casey/just) | task runner |
| [rustup](https://rustup.rs/) | pinned Rust toolchain bootstrap |
| cargo / rustfmt / clippy | Rust build, format, and lint tooling (provisioned by `just rust-setup`) |
| [shellcheck](https://github.com/koalaman/shellcheck) | bash linter, pinned to **0.11.0** and enforced by `just lint` |
| [shfmt](https://github.com/mvdan/sh) | bash formatter (indent=4, case-indent) |
| [git-cliff](https://github.com/orhun/git-cliff) | changelog from conventional commits |
| [gh](https://cli.github.com/) | GitHub CLI for releases |
| [mkdocs-material](https://squidfunk.github.io/mkdocs-material/) | docs site (optional, only for `just docs`) |

## Common commands

**What the bash lint lane does and does not cover.** `just lint` pins shellcheck to 0.11.0
and refuses to run against any other version — the finding set moves between releases, so an
unpinned linter is a gate that changes without a commit. It then enforces a severity floor of
`warning`: warnings and errors fail the lane, and everything below warning — informational
and style diagnostics alike — does not. A green `just
lint` therefore does **not** mean plain `shellcheck` reports nothing. It currently leaves 93
info-level notes standing — 55 SC2031 and 38 SC2030, all in the suites, where the subshells
are deliberate isolation and restructuring them would trade real test safety for a clean
linter page. The SC2016 and SC2329 sites carry their own reasoned comments instead.

```bash
just check            # shellcheck + shfmt (diff mode)
just format           # shfmt -w
just test             # unit + integration
just test-unit        # pure-function unit tests (bash only, no tmux)
just test-integration # tmux + git required
just install          # checkout-mode immutable versioned install
just rust-setup       # install pinned Rust toolchain and dev tools
just rust-check       # Rust fmt, clippy, nextest, and doctests
just version          # current AE_VERSION
just release          # full release pipeline (CalVer → tag → gh release)
just docs             # serve the docs site locally on http://localhost:8000
just docs-build       # build the static site into ./site
```

## Tests

`tests/unit` covers the Bash pane glue, the public-wrapper/installer contract, and helper templates with a tiny `assert_eq` harness. Session-helper logic (watchdog, send, ask, requests, …) lives in the top-level **template library** section of `ae` — real column-0 functions emitted into generated helpers via `declare -f` — so tests extract and exercise that glue directly; there are no helper heredocs to parse anymore (only three trivial exec shims remain heredocs). Two builder helpers (`_build_lib_from_source`, `_build_helper_from_source`) reconstruct a runnable `_lib`/helper from the emission's own prologue + `declare -f` list when a test needs the full artifact.

When adding or changing a helper, the guard suite enforces the emission invariants: every `declare -f` list ends with its `helper_<name>_main`, every emitted name has exactly one top-level definition, the template `helper_*` set equals the emitted union, and the whole template library must source silently under `set -u` (an executable leak would run on every ordinary `ae` invocation).

`tests/integration` spins up real tmux sessions, exercises the full lifecycle (create, send, ask, reply, stop, resume, end), and tears down. Requires tmux and git. The `doctor --refresh` scenario doubles as the declare-f canary: it clobbers generated helpers, refreshes, and runs the regenerated artifacts end-to-end (including a watchdog stop → start cycle, since a running watchdog keeps its loaded body until restarted).

```bash
bash tests/unit          # ~650+ assertions, pure bash (no tmux needed)
bash tests/integration   # ~60 scenarios, real tmux sessions
```

## Releases

SemVer-compatible CalVer in `YYYY.M.N` format, with the sequence derived from matching Git
tags and reset each month. `just release` is the full pipeline:

The bump is recover-or-refuse: durable backups restore version files on a handled failure;
an untrappable interruption leaves `.ae-bump-recovery`, and the next bump stops until it is
recovered with `just bump-recover`.

1. Pre-flight: clean working tree, fetch tags, pull rebase.
2. `just check` (shellcheck + shfmt).
3. `just test` (unit + integration).
4. `just bump` updates all four version-bearing files — `_AE_ENTRY_VERSION` in `ae-entry`, `AE_VERSION` in `ae`, the Cargo package and lockfile — and rewrites the README and docs/index release badges. It refuses to proceed if the pre-release badge or the obsolete checkout-install prose is still standing in either file; a real build-from-source section is not what it is looking for.
5. `git-cliff` → `CHANGELOG.md` + release-body.
6. Commit, tag, push, `gh release`.

## Cross-model code review

Significant changes go through cross-model review before commit. Inside an ae session, route the
review through its visible, steward-monitored helpers:

```bash
review codex:reviewer "Review uncommitted changes critically; return findings first."
# or: spawn codex:reviewer "Review uncommitted changes critically; return findings first."
```

Outside an ae session, use the direct CLI fallback:

```bash
codex exec --sandbox read-only -o .local/cross-review.md "Review uncommitted changes critically..."
```

Or from Codex, call Claude. The point is that any meaningful diff gets a second pair of eyes from a different model architecture. See `AGENTS.md` for the full protocol.

Review invocations run **read-only** — a `--full-auto` reviewer is a tree mutator (one reverted an in-flight fix while "probing" pre-fix behavior). Reserve write access for explicitly authorized workers in isolated checkouts.

## Philosophy reminders

- ae is a public wrapper, a Rust core, and the Bash pane glue; no new Bash features return to the glue.
- Config is INI-style with a simple regex parser. Don't add TOML/YAML/JSON parsing.
- The installed runtime is an immutable matched set; checkout development additionally needs rustup and just. (Docs tooling remains optional.)
- Session state lives in `~/.ae/sessions/`. Working directories stay clean.
- No AI tool attribution in commits.
- Keep the script lean. If it's getting bloated, cut, don't add.

## Where the doctrine lives

- **`AGENTS.md`** in the repo: project-level rules.
- **`~/.claude/CLAUDE.md`** (user-level): personal preferences, cross-model review protocol, S/M/L workflow doctrine.
- **`~/.claude/skills/`**: reusable workflow recipes.

For Claude Code, `CLAUDE.md` is a symlink to `AGENTS.md` so both tools see the same rules.
