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

`tests/unit` extracts pure functions from `ae` via `awk`, sources them, and asserts behavior with a tiny `assert_eq` harness. No external dependencies. Heredoc bodies (loop, send, ask, requests, etc.) are extracted by their EOF marker for behavioral tests.

`tests/integration` spins up real tmux sessions, exercises the full lifecycle (create, send, ask, reply, stop, resume, end), and tears down. Requires tmux and git.

```bash
bash tests/unit          # ~220+ assertions, < 1s
bash tests/integration   # ~45 scenarios, ~30s
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
codex exec --full-auto -o .local/cross-review.md "Review uncommitted changes critically..."
```

Or from Codex, call Claude. The point is that any meaningful diff gets a second pair of eyes from a different model architecture. See `AGENTS.md` for the full protocol.

## Philosophy reminders

- ae must remain a single bash script. No compiled languages, no runtimes.
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
