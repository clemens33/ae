# Development

ae ships one public wrapper over an immutable versioned Rust core.
The repo contains that product, its installer and release lanes, wrapper/installer tests,
Rust tests, and this docs site.

## Layout

```
justfile            — dev/release pipeline (just check, just test, just release)
cliff.toml          — git-cliff config (CalVer-compatible changelog)
tests/it/           — the single Rust integration target; `doors.rs`, `gate.rs` and
                      `parity.rs` are about the repository rather than the product
tests/fixtures/     — frozen inputs the suites read (session shapes, list goldens)
install             — the bootstrap (download, verify, extract, `ae-core _install`); the
                      product's only bash file, 79 lines
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
| [gh](https://cli.github.com/) | GitHub CLI — a hard prerequisite of `just release`, which attaches the assets itself |
| [x86_64-unknown-linux-musl-gcc](https://github.com/messense/macos-cross-toolchains) | musl cross toolchain — needed by `just bundles` only. `brew install messense/macos-cross-toolchains/x86_64-unknown-linux-musl` |
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
just test             # the whole test surface (= just rust-check)
just check            # shellcheck + shfmt over install, the one bash file
just install          # checkout-mode immutable versioned install
just rust-setup       # install pinned Rust toolchain and dev tools
just rust-check       # Rust fmt, clippy, nextest, and doctests
just version          # current AE_VERSION
just bundles          # both platform bundles + release SHA256SUMS into ./dist
just release          # full release pipeline (CalVer → bundles → tag → gh release)
just docs             # serve the docs site locally on http://localhost:8000
just docs-build       # build the static site into ./site
```

## Tests

**The whole test surface is Rust.** `just rust-check` is format, lint, `cargo nextest` over
the unit tests and the single `tests/it` integration target, and `cargo test --doc` for the
doctests. `just test` is that same command — there is no inner-loop/gate split any more,
because the whole suite now costs what one scoped bash domain used to. `just rust-mutants`
asks the harder question: whether those tests would ever go red.

Slice Z4 retired the bash suites. Every `tests/integration` section and every live
`tests/unit` block was matched against a Rust test that already proved the same invariant,
or ported as one. What was ported is BEHAVIOUR, not test count: one strong end-to-end test
per invariant, driving the real binary against a real tmux server, rather than a
transliteration of a 40-assertion bash section.

`tests/it` runs against real servers. Its rigs create a tmux session on their own socket
under a short `/tmp` path (`sun_path` is 104 bytes on macOS), launch real panes, and tear
everything down in a `Drop` so a failed assertion cannot leave a server behind for the next
timing-sensitive test. `doctor::doctor_refresh_republishes_the_shims_the_manifest_and_the_core_pin`
is the canary for the link set: it clobbers a helper, refreshes, and pins the link targets
against what the core links at launch, so the refresh entry and the launch entry cannot
drift. It asserts the set is EXACTLY the core's list, never `>= N`, so an artifact quietly
appearing or vanishing is a failure rather than a different number.

Three modules are about the REPOSITORY rather than the product:

* `tests/it/doors.rs` — the capability boundary. `clippy.toml` denies
  `std::process::Command` and a short list of `std::fs` readers outside a named few files;
  these tests ask clippy itself, under `--force-warn`, whether that still holds, and pin
  that `git` and `ps` each have exactly one product caller.
* `tests/it/gate.rs` — the two build files ae owns. It proves the `lint` recipe redirects
  shellcheck's stdin (issue #67: the linter reads fd 0, an agent harness hands it a socket,
  and the process then blocks forever), that `install` carries no GNU-only coreutils, that
  the `bundle` recipe is the one definition of a bundle, that `just bump` derives its
  sequence from the tags, that the musl cross compiler has one spelling across the justfile
  and `.cargo/config.toml`, and that `just release` proves its push rights before the bump
  and builds both bundles before the tag.
* `tests/it/parity.rs` — the suite's ONE child-process door. It was the plumbing of a
  bash-versus-core parity harness; the harness went with the bash it compared against, and
  the door stayed because every real-server test runs through it.

Every guard in those three is exercised against deliberately broken input as well as the
real file. A rule that matches nothing is indistinguishable from a clean tree, so the red
cases are the half that makes a green run mean something.

## Releases

SemVer-compatible CalVer in `YYYY.M.N` format, with the sequence derived from matching Git
tags and reset each month.

**The whole release happens on one machine (ruled 2026-09-04).** `just release` builds both
platform bundles locally and attaches them itself, so GitHub Actions is not on the critical
path and a queued or red run cannot hold a release up. Prerequisites are `gh` logged in as
an account with push rights and the musl cross toolchain; both are checked in the pre-flight.

The bump is recover-or-refuse: durable backups restore version files on a handled failure;
an untrappable interruption leaves `.ae-bump-recovery`, and the next bump stops until it is
recovered with `just bump-recover`.

1. Pre-flight: clean working tree; `gh` authenticated **and** `repos/<owner>/<repo>` reporting `permissions.push=true`; the musl cross compiler present; fetch tags, pull rebase.
2. `just check` (shellcheck + shfmt).
3. `just test` (the Rust suite; minutes).
4. `just bump` updates the version — `Cargo.toml` and `Cargo.lock`, which since slice Z3 are the only files that hold one — and rewrites the README and docs/index release badges. It refuses to proceed if the pre-release badge or the obsolete checkout-install prose is still standing in either file; a real build-from-source section is not what it is looking for.
5. `just bundles` builds both halves into `dist/` and proves them.
6. `git-cliff` → `CHANGELOG.md` + release-body.
7. Commit, tag, push, then `gh release create` with the three assets and the changelog section as `--notes-file`.

**Why the order is what it is.** Everything that can refuse refuses in step 1, before a
version file is written and long before a tag exists — an authenticated `gh` account is not
the same thing as one that can push here, which is why the API is asked rather than the
login trusted. The bundles are built in step 5, still before the tag, so a failed cross
build costs a `git checkout` of two version files rather than an orphan tag with no assets
behind it. `tests/it/gate.rs` pins that order.

**What `just bundles` proves, and what it does not.** It builds `darwin-arm64` natively and
`linux-x86_64-musl` against the Homebrew cross toolchain, both `--locked`; runs `--version`
on the native half; and proves the musl half static with `llvm-readobj --program-headers`
from the pinned `llvm-tools` component (no `PT_INTERP`) plus `file`. It cannot RUN the musl
binary — a macOS kernel will not exec an ELF — so **the musl run proof stays on the Linux CI
leg** (`.github/workflows/rust.yml`), which executes the core, checks its exit codes, and
resolves a real name through musl's NSS-less `getaddrinfo`. `just bundle` is honest about
the same gap: it asks a native member for its `--version` and says so out loud when it
degrades to a byte search for a foreign one.

`.github/workflows/release.yml` is kept as a **dispatch-only** lane for that Linux run
proof. Dispatch it at a tag — `gh workflow run release.yml --ref v<version>` — because every
step derives the version from `GITHUB_REF_NAME`.

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

- ae is a public wrapper over a Rust core; no new Bash features return to the wrapper.
- Config is INI-style, parsed by the core (`src/config.rs`). Don't add TOML/YAML/JSON parsing.
- The installed runtime is an immutable matched set; checkout development additionally needs rustup and just. (Docs tooling remains optional.)
- Session state lives in `~/.ae/sessions/`. Working directories stay clean.
- No AI tool attribution in commits.
- Keep the script lean. If it's getting bloated, cut, don't add.

## Where the doctrine lives

- **`AGENTS.md`** in the repo: project-level rules.
- **`~/.claude/CLAUDE.md`** (user-level): personal preferences, cross-model review protocol, S/M/L workflow doctrine.
- **`~/.claude/skills/`**: reusable workflow recipes.

For Claude Code, `CLAUDE.md` is a symlink to `AGENTS.md` so both tools see the same rules.
