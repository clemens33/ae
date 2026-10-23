# Development

ae ships an immutable versioned Rust core, with `~/.local/bin/ae` pointing directly to it.
The repo contains that product, its installer and release lanes, the Rust test suite,
and this docs site.

## Layout

```
justfile            — dev/release pipeline (just check, just test, just release)
cliff.toml          — git-cliff config (CalVer-compatible changelog)
tests/it/           — the single Rust integration target; `doors.rs`, `gate.rs` and
                      `parity.rs` are about the repository rather than the product
tests/fixtures/     — frozen inputs the suites read (session shapes, list goldens)
install             — the bootstrap (download, verify, extract, `ae-core _install`); the
                      product's only bash file, 83 lines
Cargo.toml          — Rust package (bin + lib, both `ae`)
src/                — Rust core sources
README.md           — user-facing intro
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
| [git-cliff](https://github.com/orhun/git-cliff) | changelog from plain subjects on the first-parent chain — a merged slice contributes its merge subject |
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
just test             # compile every fuzz target + unit + integration
just test             # the whole test surface (= just rust-check)
just check            # shellcheck + shfmt over install, the one bash file
just install          # checkout-mode immutable versioned install
just rust-setup       # install pinned Rust toolchain and dev tools
just rust-check       # Rust fmt, clippy, compile every fuzz target, nextest, doctests
just rust-prune       # report stale ae build output here; apply=yes cleans it
just version          # current AE_VERSION
just bundles          # both platform bundles + release SHA256SUMS into ./dist
just release          # full release pipeline (CalVer → bundles → tag → gh release)
just docs             # serve the docs site locally on http://localhost:8000
just docs-build       # build the static site into ./site
```

**The gate on real Linux, locally.** `just rust-linux` runs `just rust-setup` and
`just test` inside a local Linux container, natively arm64. The checkout is bound
read-only, the CARGO_HOME/RUSTUP_HOME/CARGO_TARGET_DIR caches live in named
volumes (`ae-linux-arm64-*`; remove them by name for a cold run), the container
runs non-root as the invoking uid, and everything it creates carries the
`ae.lane=linux` label. The base image and the two host-fetched binaries are
digest-pinned in the justfile. The target volume is shared by every checkout, so
the container rebuilds the `ae` package from the mounted tree
(`cargo clean --locked -p ae`) before the gate runs — without it, one checkout
could test another's build (#190). `just rust-linux-smoke` executes the x86_64 musl
bundle from `./dist` in an amd64 container — the run proof `just bundles` cannot
make on an arm64 macOS host, where its byte search is the degraded form. (An amd64 GATE
was tried and dropped: under QEMU user emulation `rust-setup` cannot build
cargo-nextest, whose aws-lc-sys build script segfaults.)

## Tests

**The whole test surface is Rust.** `just rust-check` is format, lint, a locked stable
`cargo check --bins` of every fuzz target, `cargo nextest` over
the unit tests and the single `tests/it` integration target, and `cargo test --doc` for the
doctests. `just test` is that same command — there is no inner-loop/gate split any more,
because the whole suite now costs what one scoped bash domain used to. `just rust-mutants`
asks the harder question: whether those tests would ever go red. The `menu` module drives a
real tmux server and real attached clients, so `.config/nextest.toml` keeps all of it in one
small test-group: parallel load can starve a client the test is waiting on, and the group
bounds that contention without weakening a single assertion.

`just rust-fuzz-check` also runs in CI, using the gate's target cache. A cold registry
fetches the independent fuzz graph; `libfuzzer-sys` builds C++ even under `cargo check`,
so a C++ compiler is required. This graph remains outside product deny/vet. No nightly
or cargo-fuzz is needed; fuzz execution remains human-run (`fuzz/README.md`).

Slice Z4 retired the bash suites. Every `tests/integration` section and every live
`tests/unit` block was matched against a Rust test that already proved the same invariant,
or ported as one. What was ported is BEHAVIOUR, not test count: one strong end-to-end test
per invariant, driving the real binary against a real tmux server, rather than a
transliteration of a 40-assertion bash section.

`tests/it` runs against real servers. Every scratch root comes from one owner,
`cli::OwnedScratch::root`: `<base>/ae-it-<pid>/<family>.<tag>`, the base `AE_TEST_TMPDIR` or
`/tmp` — short, because a tmux socket beneath it must fit `sun_path` (104 bytes on macOS), and
the owner refuses a root that leaves no room. The root is registered with the lane's reaper
before any tmux starts there, and its `Drop` kills every server beneath it before removing it.
A test killed outright is reaped when its lane exits, unless an orphaned child still holds its
root 30 s later: then the lane fails and keeps itself, and the next lane retries the reap.
A root the sweep cannot delete because a child is still writing into it is named (`note:
kept …`) and left for the next lane: that transient race never turns an all-green lane red.
`cli::no_test_file_rolls_its_own_scratch_root` refuses a root built by hand. Parallelism is
config: nextest runs at most eight tests (`.config/nextest.toml`), `just rust-mutants` one
mutant at a time on at most two tests, each killed after 60 s (the nextest `mutants` profile,
which `.cargo/mutants.toml` selects, with SIGKILL at the bound and no grace period;
`NEXTEST_TEST_THREADS=1` tightens it, and the lane caps anything wider than two. Arguments
after the lane's `--` reach nextest as typed, so `-- --test-threads N` is the explicit,
unscanned way past the cap). A mutant that grows a buffer without bound outruns any timeout (about
2 GB/s, and macOS has no usable rlimit), so the mutants lane also polls its `cargo-mutants`
subtree twice a second and SIGKILLs a test binary above `AE_MUTANTS_RSS_MAX_MB` (default 2048),
printing the kill and appending it to `mutants.out/ae-rss-kills.log`: nextest sees a failed test,
so cargo-mutants calls that mutant caught; a freeze reclassifies it from the receipt as memory
growth. The lane sizes itself at start from the free
cores: `jobs-budget` when that helper is on PATH and prints one positive integer of at most
three digits, else the online CPU count. An unset `NEXTEST_TEST_THREADS` becomes min(free cores, 8), an unset
`CARGO_BUILD_JOBS` the free cores (left to cargo when the helper is absent); an explicit
value of either wins, so CI and a bare clone size as before. The choice and its source
(`budget`, `cpus`, `env`; an unset jobs value reads `unset (cargo default)`) print on stderr, and the full test, coverage and mutants runs start
cargo at `nice -n 10` (a filtered `just test <filter>` does not). The lane runs cargo in a session of
its own, with no controlling terminal, so no test can take the terminal `just test` was typed
in for a tmux pane it sits in (#149); a `cargo nextest` run straight from a terminal has no
such guarantee. Ctrl-C still stops cargo, because the lane passes the signal on; Ctrl-Z
suspends only `just`, never the detached cargo. `doctor::doctor_refresh_republishes_the_shims_the_manifest_and_the_core_pin`
is the canary for the link set: it clobbers a helper, refreshes, and pins the link targets
against what the core links at launch, so the refresh entry and the launch entry cannot
drift. It asserts the set is EXACTLY the core's list, never `>= N`, so an artifact quietly
appearing or vanishing is a failure rather than a different number.

Each `just rust-mutants` invocation uses a fresh `target` below its lane scratch
for both Cargo final and intermediate artifacts. The lane replaces inherited
`CARGO_TARGET_DIR`, `CARGO_BUILD_TARGET_DIR` and `CARGO_BUILD_BUILD_DIR`, overriding
ordinary Cargo file config. Explicit Cargo argument redirects, including extra
Cargo arguments in `mutants.toml`, remain the caller's responsibility and sit
outside this guarantee. Existing pass-through stays unchanged and Cargo decides
precedence. The location of `mutants.out` stays unchanged.

This builds cold each time and can consume several GB under `AE_TEST_TMPDIR`
(default `/tmp`). Normal lane cleanup removes those artifacts; SIGKILL leftovers
can remain until a later lane successfully reaps them. Mutants refuses `{` or `}`
in its scratch path before Cargo starts, because Cargo templates build-dir paths.
The existing RSS matcher splits the executable path at whitespace, so use an
unspaced scratch base to retain that protection.

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
  and builds both bundles before the tag. It also pins the dev profile's line-table
  debuginfo and the opt-in `debugging` profile, proves with a real panic that `file:line`
  survives for an ae frame and a dependency frame, and proves both clippy guards run their
  child cargo with `CARGO_INCREMENTAL=0`.
* `tests/it/parity.rs` — the suite's ONE child-process door. It was the plumbing of a
  bash-versus-core parity harness; the harness went with the bash it compared against, and
  the door stayed because every real-server test runs through it.

Every guard in those three is exercised against deliberately broken input as well as the
real file. A rule that matches nothing is indistinguishable from a clean tree, so the red
cases are the half that makes a green run mean something.

## Build disk

Version churn dominates what `target/` keeps. The package version is part of every ae unit's
identity (`CARGO_PKG_VERSION` is embedded in the crate), so each CalVer bump gives the lib,
the bin, both unit-test harnesses and the `it` target a fresh hash, in the clippy check tree
as well as the build tree, and neither Cargo nor rustc removes the identities of earlier
versions. Source edits also grow a version's retained incremental sessions, but only within
that version; the number of versions has no bound.

Measured on 2026-10-20 at v2026.10.97 as `du -sk` of each tree on its own, before and after
this change. The figures are independent measurements, not parts of one sum: hard links make
them overlap.

| After one gate on a fresh `target/` | before | after |
|---|---:|---:|
| whole `target/` | 3.24 GiB | 2.22 GiB |
| `debug/incremental` | 1.80 GiB | 1.61 GiB |
| `debug/deps` | 1.02 GiB | 0.75 GiB |
| each guard tree | 0.44 GiB | 0.07 to 0.08 GiB |

| Added by each further version | before | after |
|---|---:|---:|
| `debug/incremental` | 1.80 GiB | 1.61 GiB |
| `debug/deps` | 0.84 GiB | 0.59 GiB |
| each guard tree | 384 to 402 MiB | 11 to 15 MiB |

In the second table the before figures for the guard trees come from scratch trees that ran
each guard's own command with incremental compilation on; the after figures come from the
real guard tests. The main checkout's `target/` had reached 175 GB with 53 cached ae versions; how long ago it
was last cleaned was not measured, and that total also holds the release and musl trees, so
it is not only these dev identities.

What keeps each release smaller:

* **Line-table debuginfo.** `[profile.dev] debug = "line-tables-only"`; `test` inherits it
  and it applies to dependencies. Panic and test-failure backtraces keep `file:line` for ae
  and for dependencies; only the variable and type descriptions a debugger reads are gone.
  For a debugger use the opt-in profile: `cargo build --profile debugging` or
  `cargo nextest run --cargo-profile debugging`. It builds into its own `target/debugging`
  tree and no lane uses it. Do not add a `[profile.dev.package."*"]` override (it would also
  win over `debugging`) and never `debug = 0` (it loses every frame's `file:line`).
* **The two clippy guards run without incremental state.** `tests/it/doors.rs` and
  `tests/it/phase3.rs` each run a nested `cargo clippy --force-warn …` and read its JSON.
  Each owns a target directory (`target/force-warn-guard`, `target/world-read-guard`)
  because the lint arguments differ from the lane's: measured on 2026-10-20, sharing one
  directory made every switch between `-D warnings` and `--force-warn` recheck the ae package
  (about 40 s against 0.1 s when nothing changed), although the diagnostics stayed correct.
  Their incremental cache is the part nothing reuses across releases (384 to 402 MiB per guard
  per release against 11 to 15 MiB without it), so both children set `CARGO_INCREMENTAL=0`. The
  price is paid when the crate's source or version changes: the guard's clippy then rechecks
  the crate without incremental reuse. Measured on 2026-10-20 on a quiet machine, running only
  the two guard tests after touching one source file: 18.2, 19.5 and 18.9 s with it against
  9.8, 10.0 and 9.5 s without, a median of 9.4 s more per run over three paired runs (the
  two guards run side by side; a whole-gate comparison was too noisy to resolve a difference
  that small).

What this does not do by itself: these changes shrink the bytes a release leaves behind; they
do not remove old releases. `just rust-prune` does, for the checkout it is run in, from that
checkout's root, once per checkout (a worktree is its own checkout). It drops the ae
package's artifacts of every version while keeping the compiled dependencies, so the next
`just test` rebuilds only ae, in a minute or two. Measured on 2026-10-20 in a target holding
three ae versions, `cargo clean -p ae --profile dev` took 3.4 s: it removed all 30
incremental directories (6.04 GiB), every `ae-*`, `libae-*`, `it-*` and `libit-*` file in
`debug/deps` (1.95 GiB down to 0.15 GiB) and the ae units' fingerprints, and all 373
compiled-dependency files stayed.

`just rust-prune` only reports; `just rust-prune apply=yes` cleans. It runs five cleans, each
`cargo clean --locked -p ae` with `--profile dev`, `--profile release`, `--profile release
--target x86_64-unknown-linux-musl`, and `--profile dev --target-dir target/force-warn-guard`
or `target/world-read-guard` for the two guard trees, after a `--dry-run --verbose` of each.
The guards come first, and a refusal exits 1 with one line naming the one that fired:

* a redirect variable (`CARGO_TARGET_DIR`, `CARGO_BUILD_TARGET_DIR`, `CARGO_BUILD_BUILD_DIR`,
  `CARGO_BUILD_TARGET`) is set;
* `cargo metadata`, asked from the checkout, puts the target or build directory anywhere but
  `target/` (a `build.target-dir` or `build.build-dir` in a cargo config, a parent
  directory's included; cargo finds config from the working directory, not from
  `--manifest-path`, which is why the recipe judges only the tree it runs in);
* `target/` or a tree the recipe cleans is a link;
* a dry-run's stdout, the paths cargo would remove, holds a non-empty line that is not an
  absolute path (a relative one such as `../outside/x` is refused, never skipped), one that
  lies outside `target/` (outside the guard's own directory for the two guard cleans; the
  `--target` clean lists the host units under `target/release` too), has a `.` or `..`
  segment, or goes through a link;
* a dry-run's stderr, which is diagnostics and never an inventory, does not hold exactly one
  valid `Summary <n> file[s]` line (`, <size> total` after it), or claims files while stdout
  lists none. A tree with nothing to remove has an empty stdout and `Summary 0 files`, and
  passes;
* cargo itself refuses a `--dry-run` (a `target-dir` without its `CACHEDIR.TAG` is one) and
  the recipe prints cargo's message and stops;
* a process outside this run's own ancestry (the recipe shell, `just`, the invoking shell)
  has an argv naming the checkout or, when `lsof` is installed, a working directory inside
  it, which includes the other end of a pipe (`just rust-prune | tail` is refused). `ps` is
  required; without `lsof` the run says so once on stderr and judges by argv alone. A gate
  between its phases, with no child alive, cannot be seen: do not run the recipe during one.

The recipe never removes a file itself: cargo does. To reclaim the compiled dependencies too,
delete the whole `target/` of a checkout nobody is building in. `--dry-run` prints a summary
of what a cargo command would remove; add `--verbose` to list the paths.

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

1. Pre-flight: clean working tree; on `main`; `gh` authenticated **and** `repos/<owner>/<repo>` reporting `permissions.push=true`; the musl cross compiler present; fetch tags, pull rebase.
2. `just check` (shellcheck + shfmt).
3. `just test` (the Rust suite; minutes).
4. `just bump` updates the version — `Cargo.toml` and `Cargo.lock`, which since slice Z3 are the only files that hold one — and rewrites the README and docs/index release badges. It refuses to proceed if the pre-release badge or the obsolete checkout-install prose is still standing in either file; a real build-from-source section is not what it is looking for.
5. `just bundles` builds both halves into `dist/` and proves them.
6. `git-cliff` → `CHANGELOG.md` + release-body.
7. Commit, tag locally, write the notes file and check every asset, push the branch, then `gh release create --target <sha>` with the three assets and the changelog section as `--notes-file`.

**Why the order is what it is.** Everything that can refuse refuses in step 1, before a
version file is written and long before a tag exists — an authenticated `gh` account is not
the same thing as one that can push here, which is why the API is asked rather than the
login trusted, and the branch is checked there rather than after five files have been
rewritten. The bundles are built in step 5, still before anything is pushed.

**The remote tag is created by the release, not pushed ahead of it.** A `git push <tag>`
that lands and a `gh release create` that then fails publishes a version with nothing behind
it: `install` resolves the latest release, finds no `SHA256SUMS`, and the install one-liner
is broken for everyone. So step 7 pushes only the branch and lets
`gh release create --target <sha>` create the tag object and the release in one call with
the assets attached. If that call fails, the remote has no tag, the local one survives, and
the retry command is printed; a re-run against an existing release uploads into it with
`--clobber`. `tests/it/gate.rs` pins the whole order and refuses a `git push` of the tag.

**What `just bundles` proves, and what it does not.** It builds `darwin-arm64` natively and
`linux-x86_64-musl` against the Homebrew cross toolchain, both `--locked`; runs `--version`
on the native half; and proves the musl half static with `llvm-readobj --program-headers`
from the pinned `llvm-tools` component (no `PT_INTERP`) plus `file`. It cannot RUN the musl
binary — a macOS kernel will not exec an ELF — so **the musl run proof stays on the Linux CI
leg** (`.github/workflows/rust.yml`), which executes the core, checks its exit codes, and
resolves a real name through musl's NSS-less `getaddrinfo`. `just bundle` is honest about
the same gap: it asks a native member for its `--version` and says so out loud when it
degrades to a byte search for a foreign one.

`.github/workflows/release.yml` is kept as a **dispatch-only, proof-only** lane for that Linux
run proof and the macOS build proof. It uploads the runner-built bundles as workflow artifacts
for inspection alongside the assets published by `just release`; those tarballs are not expected
to be byte-identical to the laptop-built assets, and `SHA256SUMS` remains the reference for the
published files rather than a reproducibility claim. It never creates, updates or overwrites
release assets. Dispatch it at a tag — `gh workflow run release.yml --ref v<version>` — because
every step derives the version from `GITHUB_REF_NAME`.

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

- ae's Rust core owns runtime behavior; the only Bash file is the policy-frozen `install`
  bootstrap.
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
