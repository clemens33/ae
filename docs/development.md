# Development

ae is a single bash script. The repo is mostly that script + tests + this docs site.

## Layout

```
ae                  — the script (everything lives here)
justfile            — dev/release pipeline (just check, just test, just release)
cliff.toml          — git-cliff config (CalVer-compatible changelog)
tests/unit          — pure-function unit tests (bash, no deps)
tests/integration   — integration tests (requires tmux, git)
install             — symlink / curl|bash installer
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
| [shellcheck](https://github.com/koalaman/shellcheck) | bash linter |
| [shfmt](https://github.com/mvdan/sh) | bash formatter (indent=4, case-indent) |
| [git-cliff](https://github.com/orhun/git-cliff) | changelog from conventional commits |
| [gh](https://cli.github.com/) | GitHub CLI for releases |
| [mkdocs-material](https://squidfunk.github.io/mkdocs-material/) | docs site (optional, only for `just docs`) |

## Common commands

```bash
just check            # shellcheck + shfmt (diff mode)
just format           # shfmt -w
just test             # unit + integration
just test-unit        # pure-function unit tests (bash only, no tmux)
just test-integration # tmux + git required
just version          # current AE_VERSION
just release          # full release pipeline (CalVer → tag → gh release)
just docs             # serve the docs site locally on http://localhost:8000
just docs-build       # build the static site into ./site
```

## Tests

`tests/unit` extracts pure functions from `ae` via `awk`, sources them, and asserts behavior with a tiny `assert_eq` harness. No external dependencies. Session-helper logic (watchdog, send, ask, requests, …) lives in the top-level **template library** section of `ae` — real column-0 functions emitted into the generated helpers via `declare -f` — so tests extract and exercise those functions directly; there are no helper heredocs to parse anymore (only three trivial exec shims remain heredocs). Two builder helpers (`_build_lib_from_source`, `_build_helper_from_source`) reconstruct a runnable `_lib`/helper from the emission's own prologue + `declare -f` list when a test needs the full artifact.

When adding or changing a helper, the guard suite enforces the emission invariants: every `declare -f` list ends with its `helper_<name>_main`, every emitted name has exactly one top-level definition, the template `helper_*` set equals the emitted union, and the whole template library must source silently under `set -u` (an executable leak would run on every ordinary `ae` invocation).

`tests/integration` spins up real tmux sessions, exercises the full lifecycle (create, send, ask, reply, stop, resume, end), and tears down. Requires tmux and git. The `doctor --refresh` scenario doubles as the declare-f canary: it clobbers generated helpers, refreshes, and runs the regenerated artifacts end-to-end (including a watchdog stop → start cycle, since a running watchdog keeps its loaded body until restarted).

```bash
bash tests/unit          # ~650+ assertions, pure bash (no tmux needed)
bash tests/integration   # ~60 scenarios, real tmux sessions
```

## Releases

CalVer in `YYYY.MM.BUILD` format. `just release` is the full pipeline:

1. Pre-flight: clean working tree, fetch tags, pull rebase.
2. `just check` (shellcheck + shfmt).
3. `just test` (unit + integration).
4. Bump `AE_VERSION` in `ae` and the README badge.
5. `git-cliff` → `CHANGELOG.md` + release-body.
6. Commit, tag, push, `gh release`.

## Cross-model code review

Significant changes go through cross-model review before commit. From a Claude Code session in the repo:

```bash
codex exec --sandbox read-only -o .local/cross-review.md "Review uncommitted changes critically..."
```

Or from Codex, call Claude. The point is that any meaningful diff gets a second pair of eyes from a different model architecture. See `AGENTS.md` for the full protocol.

Review invocations run **read-only** — a `--full-auto` reviewer is a tree mutator (one reverted an in-flight fix while "probing" pre-fix behavior). Reserve write access for explicitly authorized workers in isolated checkouts.

## Philosophy reminders

- ae must remain a single bash script. No compiled languages, no runtimes. (A decision with reasons, not dogma — see "Revisit triggers" in `AGENTS.md`.)
- Config is INI-style with a simple regex parser. Don't add TOML/YAML/JSON parsing.
- No dependencies beyond bash ≥ 4.0, tmux, and git. (Docs toolchain is *optional* and never required at runtime.)
- Session state lives in `~/.ae/sessions/`. Working directories stay clean.
- No AI tool attribution in commits.
- Keep the script lean. If it's getting bloated, cut, don't add.

## Where the doctrine lives

- **`AGENTS.md`** in the repo: project-level rules.
- **`~/.claude/CLAUDE.md`** (user-level): personal preferences, cross-model review protocol, S/M/L workflow doctrine.
- **`~/.claude/skills/`**: reusable workflow recipes.

For Claude Code, `CLAUDE.md` is a symlink to `AGENTS.md` so both tools see the same rules.
