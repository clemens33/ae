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
docs/               — this site (MkDocs Material); `docs/reference/contract.md` holds the
                      detail AGENTS.md indexes (module map, helpers, tool notes, doors,
                      invariants, config)
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

### Pinned versions

AGENTS.md carries the rule: pins, not channels, and the files below are the single sources. This is the full list with its versions; where it and a source disagree, the source wins.

| What | Pin | Declared in |
|---|---|---|
| Compiler | `1.97.1` (exact release) | `rust-toolchain.toml` |
| Edition / MSRV | `2024` / `rust-version = "1.97.1"` | `Cargo.toml` |
| Profile + components | `minimal` + rustfmt, clippy, llvm-tools | `rust-toolchain.toml` |
| Targets | `aarch64-apple-darwin`, `x86_64-unknown-linux-musl` | `rust-toolchain.toml`, justfile, `deny.toml` |
| Dev tools | nextest `0.9.151`, taplo `0.10.0`, deny `0.20.2`, mutants `27.1.0`, llvm-cov `0.9.0`, vet `0.10.2`, git-cliff `2.13.1`; cargo-fuzz `0.13.2` on request, not by `rust-setup` | justfile `*_VERSION` — the single source |
| Fuzz compiler | `nightly-2026-08-20` (exact). The product compiler does not move; there is no `fuzz/rust-toolchain.toml`, the lane passes `cargo +<pin>` | justfile `FUZZ_TOOLCHAIN` — the only source |
| `just` | `1.57.0` | justfile `JUST_VERSION`; CI reads the pin from there |
| tmux floor | `3.4` — a launch and the picker REFUSE below it; `list`/`version`/`doctor`/`upgrade` do not. Ubuntu 24.04 and Homebrew both package a tmux that clears it, so CI installs the package | `src/tmux_floor.rs`; CI step in `.github/workflows/rust.yml` |

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
just pre              # ADVISORY: fmt + clippy + unit + guard tests, NOT the gate
just lane-lock status # the gate lane lock: claim <name> [secs] | release <name> | status
just check            # shellcheck + shfmt over install, the one bash file
just install          # checkout-mode immutable versioned install
just rust-setup       # install pinned Rust toolchain and dev tools
just rust-check       # Rust fmt, clippy, compile every fuzz target, nextest, doctests
just rust-prune       # report stale ae build output here; apply=yes cleans it
just rust-stress 'test(=menu::x)' 20 600  # one named filter, repeated, bounded (see Stress runs)
just version          # current AE_VERSION
just bundles          # both platform bundles + release SHA256SUMS into ./dist
just test-macos       # the Mac boundary subset (.config/macos-subset); print=yes prints it, runs nothing
just release-proof    # the hp's whole green receipt for this commit over ssh, then just test-macos
just release          # check → release-proof → rust-deny → CalVer → bundles → smoke → tag → gh release
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
could test another's build (#190). Before anything runs, the container compares
the mounted checkout with what the host recorded — HEAD, tree id and the count of
dirty paths — and stops with exit 3 on a mismatch, so a lane run is evidence for
the tree it names. For a dirty tree that identity is HEAD plus the count only,
never the content, so exact evidence needs a clean checkout. `just rust-linux-smoke` executes the x86_64 musl
bundle from `./dist` in an amd64 container — the run proof `just bundles` cannot
make on an arm64 macOS host, where its byte search is the degraded form. (An amd64 GATE
was tried and dropped: under QEMU user emulation `rust-setup` cannot build
cargo-nextest, whose aws-lc-sys build script segfaults.)

**Ubuntu 26.04, two arms.** `just rust-linux-26 [bus]` is the same gate on a
digest-pinned Ubuntu 26.04 image, with its own cache volumes
(`ae-linux26-arm64-*`; it never shares a cache with the 24.04 lane). The `bus`
parameter is `stall` by default, or `off` (the validated-host arm: the container as
it is, no bus set up); anything else exits 2 on the host before Docker starts.
The `stall` arm reproduces a slow host: 26.04 ships tmux 3.6, which asks the
session bus for a systemd transient scope whenever it spawns or respawns a pane
child, and where the user manager answers slowly each of those waits out that
timeout. On that arm a private `dbus-daemon` is the only session bus and the
activation of `org.freedesktop.systemd1` never completes. The stall is PROVEN
before the first build step: a method call to that name must end in a reply timeout
(`NoReply`) after at least two seconds, otherwise the run stops with exit 4, so a
green stall run cannot silently have run without it. The stall arm is the default
since it went green (#267). The 24.04 lane runs with no bus at
all. Both lanes are on demand: `just test`, `just pre` and `just release` never run
them. They are arm64 only; the x86_64 proof of Ubuntu 26.04 is a human run on
x86_64 hardware, and nothing here claims it.

## Lane reference

Every `just` recipe of the Rust lanes and what it is. AGENTS.md carries the short form of the rows that hold a rule (the gate, `just pre`, the lane lock, mutants, fuzz, deny/vet, prune, the on-demand lanes); every row is here in full.

| Recipe | What it is |
|---|---|
| `just rust-setup` | bootstrap: toolchain + pinned tools. Idempotent, and CI asserts it |
| `just test` / `just rust-check` | **the whole gate**: `rust-fmt-check` + `rust-lint` + `rust-fuzz-check` + `rust-test`. The hp runs it for every release ([Release proof](#release-proof)) |
| `just test-affected [base=<ref>] [print=yes]` | **the merge gate** (M1, one hop): `rust-fmt-check` + `rust-lint` + `rust-fuzz-check`, then every unit test, the `doors`, `floor` and `gate` guard modules and the integration modules the change reaches, through the lane's filtered arm, plus the doctests when a `src/` file changed and `just check` when `install` changed. The whole lane when the selector cannot place the change. A clean committed tree that does not move, no receipt. `print=yes` measures the working tree, prints the selection and runs nothing (see [The merge gate](#the-merge-gate)) |
| `just pre` | **ADVISORY, NOT the gate**, never cited as it: `rust-fmt-check` + `rust-lint` + the unit tests + the `doors`, `floor` and `gate` guard modules of `tests/it`, through the lane's filtered arm. No doctests, no fuzz compile, no retries; `just test` stays exact |
| `just lane-lock claim <name> [secs]` / `release <name>` / `status` | the gate lane lock, one gate per machine: the MAIN checkout's `.local/locks/gate` directory (resolved from a linked worktree), whose `owner` line the recipe creates EXCLUSIVELY: that create is the claim, because `mkdir` is not atomic on every host (Ubuntu 26.04's uutils reports success for a directory that exists) and there the hand protocol has no owner proof; the `next` queue file. Same on-disk protocol as the hand one. `release` refuses any other owner; it never steals or clears a stale lock, it reports one (`status` exit 1 while held) |
| `just check` | the bash lane: shellcheck + shfmt over `install` |
| `just rust-fmt` / `rust-fmt-check` | `cargo fmt` + `taplo fmt` |
| `just rust-lint` | `cargo clippy --locked --all-targets --all-features -- -D warnings` + `taplo lint`, cargo at nice 10 with `CARGO_BUILD_JOBS` from the one sizing helper below |
| `just rust-test` | `cargo nextest run --locked` **and** `cargo test --doc --locked -- --test-threads <the lane's thread count>`. Both. nextest does not run doctests, and rustdoc starts one doctest per core unless told |
| `just rust-fuzz-check` | product compiler, locked `cargo check --bins` over every fuzz target; no execution. Uses inherited `CARGO_TARGET_DIR` or root `target`. Nice 10, jobs from the one sizing helper below, then capped at four |
| `just rust-deny` / `rust-vet` | supply chain. The TLS graph is EXEMPTED, not audited (docs/history.md §11) |
| `just rust-mutants` | does the suite discriminate? A product slice runs it `--in-diff` over its own diff before freeze and answers every survivor in the freeze report (test added, or named equivalent); budget bounded, LOCAL, no percentage ever cited or gated (docs/development.md §Mutation testing). CI runs it diff-bounded on path-matching pushes to main and PRs targeting main when a source diff and usable base exist; the full lane is opt-in manual dispatch (`mutants=true`). Bounded in config: 2 tests, 60 s kill (three named tests 180 s, below), RSS watcher (`AE_MUTANTS_RSS_MAX_MB`, default 2048). `rust-cov` reports, never gates |
| `just rust-fuzz target=<name> secs=60` / `rust-fuzz-all secs=60` | human-run cargo-fuzz over the hostile parsers, on an exact nightly. Refuses on an unpinned tool or a stale `fuzz/Cargo.lock`, ends on the cutover evidence line, and REPORTS — CI carries no nightly and never runs it (`fuzz/README.md`) |
| `just rust-build-release` / `bundles` | native release binary at `target/release/ae` (host only, a bare clone must build) / both platform bundles + `SHA256SUMS` into `dist/` from one two-target cargo build (native binary under `target/aarch64-apple-darwin/release/`; needs the musl cross toolchain) |
| `just rust-stress <filterset> [count=10] [secs=900]` | the BOUNDED repeat run: nextest `--stress-count` over ONE named filterset (a module or a test, never the suite), through the isolated lane. Refuses to start above load1 14 or when load1 cannot be read (status 1), refuses an empty or `all()` filter, a count or bound that is not a plain integer, or a host without `timeout`/`gtimeout` (status 2); threads are the lane's sizing (below), cargo runs at nice 10 under `timeout -k 5 <secs>` (status 124 when the bound ends it) and the lane reaps what it started. It judges the run's own JUnit report, not nextest's exit status alone: status 100 when any iteration failed, status 1 when one did not run or the report is missing (nextest 0.9.143 exits 0 when only the last iteration passed). Never in the gate, CI or mutants. See Stress runs |
| `just rust-prune` / `rust-prune apply=yes` | THIS checkout's stale `ae` build output: the report is the default and removes nothing, `apply=yes` runs six fixed `cargo clean -p ae` cleans, each proved by a `--dry-run` path check first. Refuses a redirected target dir, a non-root cwd, a linked tree, a process using the checkout or a dry run it cannot prove (a stdout line that is not an absolute path inside its tree, no single valid `Summary`). Own tree only, never during a gate (`docs/development.md`) |
| `just test-macos` / `test-macos print=yes` | the Mac boundary subset: every unit test plus the integration modules and exact tests `.config/macos-subset` names, through the lane's filtered arm (no doctests, no receipt). `print=yes` prints the filterset and the selected tests and runs nothing. A malformed manifest refuses. The `gate::macos_subset` guard keeps the manifest complete (see [Release proof](#release-proof)) |
| `just release-proof [expect=<head>:<tree>]` | the release's tests: the hp's whole green gate-receipt for this clean HEAD, read over ssh and checked strictly, then `just test-macos`. Any doubt is one `release-proof: refused:` line. Needs `AE_RELEASE_HP` and `AE_RELEASE_HP_ROOT` (see [Release proof](#release-proof)) |
| `just release` | the whole release, locally: check, `release-proof`, rust-deny and an identity recheck before the bump. Pre-flight refuses before any state is written, a missing hp destination included |
| `just rust-linux` / `rust-linux-26` / `rust-linux-smoke` | ON DEMAND, never reached by `just test`, `just pre` or `just release`: the gate on real Linux, native arm64, in a local container (read-only checkout, `ae-linux-arm64-*` cache volumes, non-root, the mounted source checked against the host's HEAD, tree and dirty count) / the same on Ubuntu 26.04, own `ae-linux26-arm64-*` volumes; `rust-linux-26` (bus `stall`, the default since that arm went green, #267) adds a stalling session bus proved before the build, `rust-linux-26 off` is the validated-host arm / the x86_64 musl release bundle from `./dist` executed in an amd64 container. arm64 only: the x86_64 proof of Ubuntu 26.04 is a human run on x86_64 hardware |
| `just harness-smoke [control]` | ON-DEMAND live smoke of the adapter rows against the REAL harness binaries and logins (a few short turns per tool); never in the gate, CI or mutants; control = hang shim, no spend |

## Tests

**The whole test surface is Rust.** `just rust-check` is format, lint, a locked stable
`cargo check --bins` of every fuzz target, `cargo nextest` over
the unit tests and the single `tests/it` integration target, and `cargo test --doc` for the
doctests. `just test` is that same command, the whole gate, and the hp runs it for every release.
A merge runs `just test-affected`, the same checks with only the tests the change reaches (see
[The merge gate](#the-merge-gate)). `just rust-mutants`
asks the harder question: whether those tests would ever go red (see Mutation testing below). The `menu` module drives a
real tmux server and real attached clients, so `.config/nextest.toml` keeps all of it in one
small test-group: parallel load can starve a client the test is waiting on, and the group
bounds that contention without weakening a single assertion.

**`just pre` is advisory, not the gate.** It runs `rust-fmt-check`, `rust-lint`, the unit tests
(library and bin) and the `doors`, `floor` and `gate` guard modules of `tests/it` through the
lane's filtered arm, so a format, lint or guard failure shows in minutes less than the gate
needs to reach it. It skips the rest of `tests/it`, the fuzz compile and the doctests, retries
nothing, and a green `just pre` is never cited as a green gate: only `just test-affected` (the
merge gate) and `just test` (the whole gate) are.

**The lane lock.** One machine runs one gate at a time. `just lane-lock claim <name> [secs]`
takes the lock (waiting up to `secs`, printing the holder every 30 s), `release <name>` gives it
back, `status` names the holder and its age. The lock is the main checkout's
`.local/locks/gate` directory holding one `owner` line (`<name> <UTC timestamp>`), which the
recipe creates exclusively: that create is the claim, not the `mkdir` before it, because a
`mkdir` is not atomic on every host (Ubuntu 26.04's uutils reports success for a directory that
exists, so two claimants can both get past it). The hand-typed `mkdir` has no such proof there;
on such a host only the recipe is exclusive. `.local/locks/next` is the queue (empty = anyone). Resolved from a
linked worktree, so every checkout meets the same lock, and the same protocol as the hand-typed
one, so a holder on either spelling blocks the other. `release` refuses any owner but `<name>`
and leaves the lock intact, and works inside the directory it checked: a release that stalls
cannot remove the lock of whoever claimed after it, however the first lock was released (this
recipe or the hand protocol), because the directory it stands in is gone. One tiny window is
named and accepted: a claim creates the `gate` directory before it writes its `owner` line, so
a release whose last identity check and final `rmdir` straddle the lock being released by
someone else and re-claimed could remove that still-empty new directory. The hand protocol
retires once the recipe is the only spelling. Nothing here steals or
clears a lock: one that looks abandoned (`status` says `suspect`) is reported, and the holder,
the lead or a human clears it by hand.

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
config: a bare `cargo nextest run` with no `NEXTEST_TEST_THREADS` exported runs at most four
tests (`.config/nextest.toml`; the isolated lane sizes itself below and reaches its lane ceiling only on a
quiet machine), `just rust-mutants` one
mutant at a time on at most two tests, each killed after 60 s (the nextest `mutants` profile,
which `.cargo/mutants.toml` selects, with SIGKILL at the bound and no grace period;
`NEXTEST_TEST_THREADS=1` tightens it, and the lane caps anything wider than two. Arguments
after the lane's `--` reach nextest as typed, so `-- --test-threads N` is the explicit,
unscanned way past the cap). Three named tests are the one finite exception: each is killed
after 180 s (90 s x 2, grace zero) by its own block in the same profile, because they passed
in 67-87 s in a whole-gate run and the 60 s kill ended valid runs (#234). A timeout is
termination and never a caught mutant, and a hang costs up to 120 s more per mutant for each of
them. `the_mutants_timeout_exceptions_are_named_and_finite` (`tests/it/gate.rs`) reads the
config and the test names compiled into its own binary, which it lists once with `--list
--format terse` under a 30 s bound; the listing must exit 0 with an EMPTY stderr, so a benign
new startup line on stderr makes the guard fail closed until the cause is read. It refuses any
other name, selector, kill, grace, default or linux timeout change, or form it cannot classify;
widening needs an explicit ruling and a policy amendment. A mutant that grows a buffer without bound outruns any timeout (about
2 GB/s, and macOS has no usable rlimit), so the mutants lane also polls its `cargo-mutants`
subtree twice a second and SIGKILLs a test binary above `AE_MUTANTS_RSS_MAX_MB` (default 2048),
printing the kill and appending it to `mutants.out/ae-rss-kills.log`: nextest sees a failed test,
so cargo-mutants calls that mutant caught; a freeze reclassifies it from the receipt as memory
growth.

ONE helper sizes every heavy recipe. The `SIZING` shell text in the `justfile` is included by the test lane, `rust-lint` and `rust-fuzz-check`, so the arithmetic exists once and `tests/it/gate.rs` pins that no recipe carries a copy. `rust-lint` and `rust-fuzz-check` run cargo at nice 10 with `CARGO_BUILD_JOBS` from it (the fuzz check then caps itself at four jobs), and the doctest step of the lane takes `--test-threads` from the lane's thread count: without it rustdoc starts one doctest per core (12 of 12 at once measured in a scratch crate, 2 with the flag). The sizing starts at the free
cores: `jobs-budget` when that helper is on PATH and prints one positive integer of at most
three digits, else the online CPU count. Each count is the LEAST of its ceilings. Threads:
`jobs-budget` (the CPU count without it), the lane ceiling (`lane_threads` in `SIZING`, sixteen: the thread matrix of the speed plan measured 111 s
at 16 threads against 132 s at 12 on its sample, with no failure) and an inherited
`NEXTEST_TEST_THREADS`. The lane then reads load1 once at its start: above `load_ceiling` (14, the start ceiling of
a gate) or unreadable it lowers the threads to `loaded_threads` (eight, the old default) and says so on stderr
(`lane test: load1 21.30 is above 14: nextest threads at most 8`, and `was 12, capped` in the sizing line when it
lowered anything). That guard only lowers: it never refuses, waits or aborts, because an unattended gate has to
start, and an explicit `AE_TEST_THREADS` replaces it with the rest of the rule. The three values are named once
each at the top of the helper, so a new ceiling is a one-line change. Cargo jobs: `jobs-budget` and an inherited `CARGO_BUILD_JOBS`
(without `jobs-budget` there is no live ceiling and an inherited value stands, else cargo's own
default). An inherited export can lower a lane and never lift it: a shell profile that exports a
wide count (a fixed 16, say) cannot push a gate past what is free. Only
one positive integer of at most three digits is a ceiling; a relative value (`-2`), `num-cpus`,
`default` or an oversize one is no ceiling, is reported as ignored and never enters the
arithmetic, and the lane falls back to the budget path. The lane exports the result, so it also
overrides a `[build] jobs` in a cargo config. The deliberate per-run override is the lane's own
`AE_TEST_THREADS` and `AE_TEST_JOBS` (one positive integer of at most three digits each; a
malformed one refuses with status 2): it REPLACES the rule, so it can lift a count, and the mutants lane still caps
threads at two. The choice and every ceiling that applied print on stderr, as `threads 6
(budget; env 16)` (the budget set it, an inherited 16 was above), `threads 3 (env; budget 6)`
(the inherited 3 was lower), `threads 6 (budget; env -2 ignored)` or `override`; an unset jobs
value reads `unset (cargo default)`. A bare `cargo nextest run` outside the lane is NOT sized
by any of this: nextest reads `NEXTEST_TEST_THREADS` from the environment before the config
default, so a shell that exports the variable (a fixed 16, or `-2` meaning the CPU
count minus two) runs that many tests whatever `.config/nextest.toml` says, and the four applies
only where nothing is exported (CI, a bare clone). Run the gate through `just test`, or pass
`--test-threads`. The full test, coverage and mutants runs start
cargo at `nice -n 10` (a filtered `just _tmux-isolated test -E <filterset>` does not). The lane runs cargo in a session of
its own, with no controlling terminal, so no test can take the terminal `just test` was typed
in for a tmux pane it sits in (#149); a `cargo nextest` run straight from a terminal has no
such guarantee. Ctrl-C still stops cargo, because the lane passes the signal on; Ctrl-Z
suspends only `just`, never the detached cargo. `doctor::doctor_refresh_republishes_the_shims_the_manifest_and_the_core_pin`
is the canary for the link set: it clobbers a helper, refreshes, and pins the link targets
against what the core links at launch, so the refresh entry and the launch entry cannot
drift. It asserts the set is EXACTLY the core's list, never `>= N`, so an artifact quietly
appearing or vanishing is a failure rather than a different number.

### The merge gate

The human ruled on 2026-11-12 that a merge runs only the relevant tests, and that a release runs
the whole suite on the hp and the Mac boundary subset on the Mac (docs/history.md §10, "Release
tests on two machines"). `just test-affected` is that merge gate, model M1: it follows one hop
from the change to the tests. It measures the change from `git merge-base HEAD <base>` (`base=main`
by default) and runs, in this order:

1. `rust-fmt-check`, `rust-lint` and `rust-fuzz-check`, as `just test` runs them.
2. The selection. The recipe writes the change as framed records (paths, statuses, the text of
   every `.rs` file under `src/` and `tests/`, the merge-base text of each modified one), and the
   selector `examples/affected.rs` answers with one verdict. The selector source is
   `tests/it/gate/affected.rs`, and `tests/it/gate/affected/` holds its frozen acceptance.
3. A subset verdict runs every unit test, the guard modules `doors`, `floor` and `gate` and the
   integration modules the change reaches, through the lane's filtered arm, and then the doctests
   when a `src/` file changed. A whole verdict runs the bare lane, which runs the doctests itself.
4. `just check` when `install` changed, under both verdicts.

The checkout must be clean (untracked files count), and HEAD and its tree must stay the same from
the start to the end. Otherwise, and when git cannot say, the recipe prints one
`test-affected: refused: <why>` line and exits 1. It never writes a gate receipt: only a whole
`just test` vouches for a tree.

The recipe needs bash 4.4 or newer. Under an older bash, such as the macOS stock `/bin/bash` 3.2,
its first command prints `test-affected: refused: needs bash 4.4 or newer, found <version>` and
exits 1, before bash reads any syntax it cannot parse. Without that refusal, bash 3.2 stops at the
first such construct and exits 0 through the recipe's `EXIT` trap (#337). Put Homebrew bash first on
`PATH` (`/opt/homebrew/bin` on Apple Silicon) for the run. `tests/it/gate/bashfloor.rs` pins the
refusal and the rule that every recipe with bash 4 syntax opens with it.

**What a change selects.** A changed integration file selects every module that owns it. A changed
library module selects the integration files that name it: an `ae::` path to it or below it, a
`use ae::…` of it, or a name that `src/lib.rs` or its parent module re-exports from it with
`pub use`. Every changed path is also data: an `include!`, `include_str!` or `include_bytes!`, a
`#[path]` or a string literal that names it (or its base name) selects the reader, and a library
module that includes it counts as changed. Docs, top-level Markdown, `fuzz/` and
`.config/macos-subset` add nothing when nobody names them. A selected module becomes a filter term
when a test name is under it, or when a `macro_rules!` body in it writes a `#[test] fn $name`. A
selected module with neither is explained by its `why` lines and adds no term.

**What runs the whole lane.** Each reason prints as `test-affected: whole: <id>: <detail>`:
`selector-source` (the selector, its walk or its example), `build-input` (`Cargo.toml`,
`Cargo.lock`, `build.rs`, `rust-toolchain.toml`, `.cargo/`), `justfile`, `config` (`.config/`
except the Mac manifest), `crate-root` (a change to `src/main.rs`, or to `src/lib.rs` or
`tests/it/main.rs` other than an added `mod` of an added file), `module-map` (a file whose owners
change), `shared-rig` (a changed item other than a `#[test]` fn in an integration file that other
modules reference), `deleted` and `renamed` (a Rust file), `macro` (a changed `macro_rules!`),
`unknown-path` (a path no rule places) and `stream` (a change stream the selector cannot decode). A
failed producer, a failed selector or an answer that is not exactly the protocol runs the whole lane
and `just check` too (`producer`, `selector-exit`, `selector-output`): nothing then says that
`install` did not change.

**The M1 gaps.** The human accepted three, with the release's whole hp run as the backstop:

- a test that reaches a changed module only through another module (A → B → C);
- a test that drives only the binary: most integration tests run `ae` as a process and name no
  library module (3752 of 4022 when the plan was measured);
- a path built at run time or by a macro.

Every subset prints `test-affected: not selected: <N> of <M> integration tests (M1 gap)`: the
statically enumerated integration tests outside every selected module. A test that a macro writes
has no static name, so the counter leaves it out on both sides.

`just test-affected print=yes` measures the working tree instead, staged, unstaged and untracked
changes included. It prints the selector's answer with every reason and every `why` line (owner,
`file:line`, rule, target) and runs nothing else.

**Dry runs.** The selector on five slices that landed before it, 2026-11-13:

| Slice | Range | Verdict | Selection | Not selected (slice tree) | Selected tests (current inventory) | Subset nextest on the hp |
|---|---|---|---|---|---|---|
| wtgc | `6812606a..cf7fa4ff` | whole: `crate-root` `src/lib.rs`, `justfile` | 8.4 s | — | all | whole lane |
| seatproof | `7b65e90c..d067f40d` | subset: `seatproof_spec` + guards, no doctests | 8.4 s | 3756 of 3974 | 6479 of 10289 | 180.5 s |
| cliword | `7e533bcd..5c6b51a9` | subset: 27 owners in 26 terms (`cli`, `entry`, `session_launch`, …), doctests | 8.6 s | 3035 of 3978 | 7234 of 10289 | 269.5 s + doctests |
| paneerr | `27c863c6..dfb95d41` | subset: 21 owners (`transport`, `session_launch`, …), doctests | 8.9 s | 3145 of 4026 | 7140 of 10289 | 336.2 s + doctests |
| chatday | `e21d2ba5..86a0863d` | whole: `shared-rig` `tests/it/app_mode.rs` | 9.0 s | — | all | whole lane |

- **Selection** is the warm wall time of `just test-affected base=<base> print=yes` on the Mac at
  the slice's tip: the producer, which reads every source file, and the debug-built selector.
- **Not selected** is the selector's own counter on the slice's tree.
- **Selected tests** are list matches, not an executed gate: the slice's filterset against the
  `cargo nextest list` of the current tree (6102 unit tests, 4187 integration tests).
- **Subset nextest on the hp** is one executed run of that filterset on the current tree through
  the lane's filtered arm, with other agents' lanes running beside it. The hp listed 6478 and 7139
  tests for seatproof and paneerr, one fewer than the Mac. The doctests (110) took 7 s on the hp.
- **Whole-run reference** (a timing reference, not a green gate): the P1 whole `just test` on the
  hp on 2026-11-12 ran its nextest phase in 852.9 s for 10067 tests and ended with one failure.
- `rust-fmt-check`, `rust-lint` and `rust-fuzz-check` run in both lanes alike and are not in these
  numbers.

### The gate ledger

A whole `just test` (no filter) that ends green prints what it cost, on stderr, after the doctests:
the test count, the wall time, the TEST-SECONDS (the sum of the per-test times), the THREAD-SECONDS
(wall time times the lane's threads, so the gap to the test-seconds is the time a thread sat idle),
the 30 slowest tests and the change against the previous entry. It then appends one tab-separated
line to `.local/gate-ledger.tsv` in the MAIN checkout (found like the lane lock, so a linked worktree
writes the one ledger). The numbers come from nextest's own JUnit report, which
`[profile.default.junit]` in `.config/nextest.toml` writes to `target/nextest/default/junit.xml`; the
lane deletes the old file before the run, so a crashed nextest cannot be read as a new one. The ledger
is a report and never a failure: a missing or unreadable report, no git checkout or an unwritable
`.local` is one `note:` line and the lane keeps the gate's exit status. A filtered run writes nothing, and
a red run never reaches the report, so the ledger holds green whole gates only. `tests/it/gate.rs` pins the report, the append from a
linked worktree, the silence of a filtered or red run and the order of the lane's commands.

### The gate receipt

A whole `just test` (no filter, exit 0) on a clean committed tree ends by writing a gate-receipt:
one file, `.local/gate-receipts/<tree hash>` in the MAIN checkout (found like the lane lock, so a gate
run in a slice worktree is the one a release in the main checkout finds, and the tree hash survives a
fast-forward merge). A release reads the hp's receipt so that the whole suite runs on the hp and only
the boundary subset on the Mac (see [Release proof](#release-proof)).

The writer is the last act of `rust-check`, after its four dependencies, and nowhere else. A filtered
run, `just rust-test`, `just pre`, the bare lane and a red run never reach it, so none of them leaves a
receipt and a red run neither writes one nor touches the last one. A dirty tree (staged, unstaged or
untracked) writes none, because a receipt names a committed tree.

A receipt also vouches only for the tree the whole run tested. `just` records `HEAD`, the tree hash and
whether the tree was clean when it STARTS (the top-level `RUN_START` variable: `just` evaluates every
top-level assignment before it runs any recipe, so the snapshot precedes the first dependency of
`rust-check` without a new dependency in the gate). The writer writes only when the run started on a
clean tree and, when the run ENDS, the tree is still clean with the same `HEAD` and tree hash. It checks
a second time right before it publishes, after its slow probes. A commit, a reset or a cleanup while the
tests ran writes none, and one `gate-receipt: not written:` line says why. A change that is undone
before the run ends is invisible to this check. The snapshot costs one `git status` (without the git
lock) per `just` invocation and needs `sh` and `git` on `PATH`; outside a git checkout it is empty and
the writer writes none.

The file is published with a same-directory temp and `mv`, and read back by the strict reader first.
The receipt is a report and never a failure: an unwritable `.local` or an unreadable fact is one
`gate-receipt:` line on stderr and the gate keeps its green exit status.

The file holds fourteen `key=value` lines in a fixed order, printable ASCII, one LF each:
`version tree head lock_hash toolchain_pin rustc nextest tmux harness inventory inventory_count scratch
host time`. `lock_hash` and `toolchain_pin` are the sha256 of `Cargo.lock` and `rust-toolchain.toml`.
`inventory` is the sha256 of the sorted `cargo nextest list` lines and `inventory_count` their number.
`scratch` is the filesystem kind under the lane's scratch base (`AE_TEST_TMPDIR`, else `/tmp`: the base the lane itself uses, so a release run with another `AE_TEST_TMPDIR` than the gate costs one full gate). `harness` records seven
`name=first line of --version` entries (or `name=absent`) and never decides.

A receipt is never overwritten by a red run, so a later red run does not invalidate a fresh green
receipt for the same tree: the green run tested exactly that tree, and the red one only adds that some
test is flaky or depends on the machine. Receipts are a few hundred bytes, one per gated tree, and safe
to delete at any time. `tests/it/gate/receipt_spec.rs` is the frozen acceptance for the writer and the
strict reader.

### Release proof

The human ruled the split on 2026-11-12: during a merge only the relevant tests run, and during a
release the full suite runs only on the hp and the relevant macOS boundary subset on the Mac
(docs/history.md §10, "Release tests on two machines"). `just release` therefore never runs the whole suite on the Mac. It calls
`just release-proof expect=<head>:<tree>` with the clean pre-bump identity, after `just check` and
before `just rust-deny`.

**The hp half.** Run a whole green `just test` on the hp at exactly the commit you will release. It
writes `.local/gate-receipts/<tree hash>` in the hp's main checkout. `release-proof` reads that receipt
with `timeout 30 ssh -o BatchMode=yes -o ConnectTimeout=10 -- "$AE_RELEASE_HP" ...`; it never relaxes
host-key checking. It refuses unless ALL of these hold:

- the receipt is at most 8192 bytes and parses under the strict grammar of [The gate receipt](#the-gate-receipt);
- `tree` and `head` equal the clean checkout here;
- `lock_hash`, `toolchain_pin` and `rustc` equal this Mac's values;
- the `nextest` version token equals the justfile's `NEXTEST_VERSION` pin;
- `host` equals the name the hp answered with, and it is not this Mac's `hostname -s`;
- the receipt is at most 24 h old and at most 300 s in the future (clock skew).

`tmux`, `harness`, `scratch`, `inventory` and `inventory_count` are recorded only: the two machines
differ there by design.

**The Mac half.** `just test-macos` runs every unit test plus the integration modules and exact tests
`.config/macos-subset` names: the tests that touch what differs on a Mac (OS conditionals, the process
table, real tmux clients, justfile recipes, fsync, the clipboard, archive tools). The guard
`tests/it/gate/macos_subset.rs` derives that boundary from the source, and the gate refuses a manifest
that misses a marker site, a member of the nextest `tmux-clients` group or a shared boundary helper
(each helper needs its own pinned test, one that provably calls it: a same-named method on another
type does not count). `just test-macos print=yes` prints the filterset and the
selected tests and runs nothing.

**The identity.** The checkout must be clean (no staged, unstaged or untracked change) and keep the
same HEAD and tree while the hp is asked, while the subset runs and through `just rust-deny`; the
release rechecks it right before `just bump`. Any change, and any git read that fails, refuses.

**Configuration.** `AE_RELEASE_HP` names the hp as `host` or `user@host`, and `AE_RELEASE_HP_ROOT` is
the absolute path of the hp's main checkout (no `..` segment). Neither has a default and neither lives
in a tracked file: an unset or malformed value refuses before any ssh, and the release refuses in its
pre-flight. Every refusal is one `release-proof: refused: <why>` line and exit 1. Nothing falls back to
the whole suite on the Mac.

**The human-only override.** `AE_RELEASE_HP=local` releases without the hp. It prints
`release-proof: DEVIATION: whole suite on this Mac, no hp proof`. A fresh local receipt of this tree
(the old local match: same host, every deciding fact equal, at most 24 h old) then vouches without a
test run; otherwise the whole `just test` runs on the Mac, and `just test-macos` does not run again.
Agents never set `AE_RELEASE_HP=local`: it is the human's call when the hp is unavailable.

After the proof, `rust-fmt-check`, `rust-lint`, `rust-fuzz-check` and the doctests run again on the
bumped tree, and `cargo check --locked`, the bundles and a bounded smoke of the built binary
(`--version` and `doctor`, 10 s each) follow. The bump moves the ae version that `fuzz/Cargo.lock`
records, and `rust-fuzz-check` runs `--locked`, so the release refreshes that lock and proves that only
the ae version changed right after the bump, before the first of those checks. After the fuzz-lock
refresh and before the version commit the release proves that every tracked path that differs from
HEAD is one of the six version paths (`Cargo.toml`, `Cargo.lock`, `fuzz/Cargo.lock`, `README.md`,
`docs/index.md`, `CHANGELOG.md`); any other path stops the release by name, before the commit and the
tag. `tests/it/gate/receipt_spec/releaseproof_spec.rs` and `tests/it/gate/macos_subset_spec.rs` are
the frozen acceptance for this section.

### Stress runs

A flake hunt repeats the REAL tests under bounds; it never loads the machine on purpose.
`just rust-stress <filterset> [count] [secs]` is the one recipe for it: nextest
`--stress-count` over one named filterset (a module or a test, not the suite) in the isolated
lane. Every bound is in the recipe: it refuses to start while load1 is above 14 (the start
ceiling of a gate) or unreadable, before any lane state exists; `--test-threads` is the lane's
sizing above (the least of the live budget, the lane ceiling and an inherited export); cargo
runs at `nice -n 10` under `timeout -k 5 <secs>`, so the bound ends the whole process group
(status 124) and the lane's cleanup reaps the scratch and its tmux servers; the count is at most
three digits and the bound at most five. The test-group caps in `.config/nextest.toml` apply
unchanged. `tests/it/gate.rs` pins the refusal at 14.01 against the start at 14.00, the exact
argv, the niceness, the 124 and that no child of cargo survives. Synthetic CPU burners (a
matrix of `yes > /dev/null`, busy loops) are not committed and not a flake signal: they measure
the scheduler, not the tests. The same rule applies to any ad-hoc stress run: size it with
`jobs-budget`, nice 10, a timeout, a start at load1 14 or lower and no child left behind.

The exit status of nextest 0.9.143 is no verdict for a stress run: with fail-fast off it exits 0
when the LAST iteration passed, whatever the earlier ones did (fixed upstream in 0.9.148, issue
327). So the lane deletes the old `target/nextest/default/junit.xml` first, refuses to start if it
stays, and after a zero exit reads the fresh report: each `<testsuite name="<binary>@stress-N">`
is one binary in one iteration. Every iteration must be there with tests in it and none may carry a
failure, an error or a retry that only passed. A failed iteration exits 100 (nextest's own
failed-run status). An iteration that did not run, or a report that is missing, a symlink or cut
short, exits 1. Both name the iterations from 1, as nextest's log counts them. A non-zero status
from nextest or the bound (124) is the lane's status and is named. `tests/it/gate.rs` pins both
(`a_stress_run_fails_when_the_report_shows_a_failed_or_missing_iteration` and
`a_stress_run_judges_only_a_fresh_report_and_keeps_nextests_own_status`).

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
  that `git` and `ps` each have exactly one product caller. Registered below it,
  `tests/it/doors/layers.rs` is the LAYER RATCHET: every top-level module has a reviewed
  layer, and a `crate::` path that names a higher layer must be pinned in
  `tests/it/doors/layers_allow.rs` (exact file, spelling and count, so the pins only
  shrink; the typed argv seals of the door layer are listed apart as permanent
  handshakes). It reads path spellings only, and its doc comment states what it does not
  see. It runs in every `just test-affected` because it lives under `doors`.
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

### Live harness smoke

Every other test drives a fake tool. Vendors change their TUI, argv and session stores weekly, so
ae's adapter rows (input readiness, frame classifier, id capture, exact resume) can break silently
and the first witness is a live seat. `just harness-smoke` asks the REAL binaries, with their real
logins, on demand. It is **never part of the gate**: not a dependency of `test`, `pre`,
`rust-check`, mutants or CI. The two tests behind it (`tests/it/harness_smoke.rs`) are
`#[ignore]`d and need `AE_HARNESS_SMOKE=1`, which only the recipe sets; `tests/it/gate.rs` pins
that no other recipe or workflow can reach them.

Per tool, through the public verbs, in a scratch world (private tmux socket, scratch `AE_HOME` and
`CONFIG_FILE`, never the `ae` server, `~/.ae` or `ae-dev`; the real `HOME` carries the logins):
`bin` → `version` → `launch` → `ready` (a quiet input frame: the classifier says idle, or it is a
fresh claude start before its first turn, which the classifier cannot judge; any other frame it has
no verdict for, such as a running compaction, is not quiet) → `id` (session id captured) → `turn1`
(sent only into a quiet frame; one delivered turn, answered) → `idle` (the classifier proves a
finished turn) → `stop` → `resume` (same id, journal says resumed) →
`turn2` (a recall turn: the resumed conversation must still hold turn1's answer). `stop` needs the
product's strict absence proof; `resume` needs a NEW parsed `conversation-resumed` decision for the
same id in ae's journal (`conversation-unproven` for a tool whose store has no probe, where the
recall turn is the proof). A tool reads `PASS` only when every step passed; `FAIL(step)`,
`LIMIT(step)` (a vendor usage line) and `SKIP(step)` (not installed, or no frame grammar to
observe) name the step. Nothing is green by omission. Exit status is non-zero on any FAIL.

Spend: this runner sends two short turns per tool, never retried, on the cheapest model each; ae's
own launch turn (codex's registration turn, on create and on resume) comes on top. The report
counts both: `runner=` is what this runner sent, `ae=` the launch/resume ATTEMPTS that carry ae's
own turn (an upper bound: its delivery is not observed, and a tool that was absent or never
launched counts none). Knobs: `AE_HARNESS_SMOKE_TOOLS` (default `claude,codex,muse,opencode`;
`agy`, `grok`, `gemini` are opt-in; an unknown, empty or repeated name refuses before any state is
created), `AE_HARNESS_SMOKE_<TOOL>_CMD` (the profile command), `AE_HARNESS_SMOKE_CLAUDE_HOME` (the
claude login, for example `~/.claude-p`), `AE_HARNESS_SMOKE_STEP_SECS`, `_TOOL_SECS`,
`_TOTAL_SECS` (a tool started late gets only what is left of the total), `AE_HARNESS_SMOKE_REPORT`
(a directory for the TSV report; asked for and unwritable is a failure). Raw tool output is never
printed: a version is reduced to its numeric token and a refused verb to its exit code. The one
thing kept is the last pane frame a wait on the pane saw (the first frame, the frame after a
resume, the quiet before a send, the answer, the idle frame after it) when that wait failed, an
outer bound cutting it included, so a smoke never fails blind: with the report directory set it
is written there as `frame-<stamp>-<tool>.txt` (owner-only, created exclusively, at most 80 rows
of 240 characters and 16 KiB, every control character but the row break and tab shown as U+FFFD),
and without one the report says none was kept.
`just harness-smoke control` runs the hang control: a `claude` shim that answers `--version` and
then hangs must read `FAIL(ready)` inside its bound, with its last frame kept, and again under an
outer bound shorter than the wait; it spends nothing and ignores every command
and login override, so it can never reach a real binary. The recipe runs one test at a time,
niced, sized by `jobs-budget`.

Bounds. Each step has a deadline (`ready` gets three minutes while the watchdog runs: a tool that
opens on a folder-trust block, claude, is answered by the watchdog's 60 s cycle); each tool has a hard outer bound (its driver runs on its own
thread, and on expiry the run is cancelled, so an abandoned worker can start no launch, send or
resume afterwards); the whole run has a total bound after which the remaining tools read `SKIP`.
Cleanup of the private tmux server and the scratch is synchronous and is measured and reported,
not promised to be instantaneous.

Housekeeping. The work directory is one fixed canonical path under an exclusive lock, so a
vendor's trust list gains one entry, not one per run; a second run refuses while the lock stands,
and a stale lock is reported and cleared by hand, never stolen. The vendors write to their real
config: claude records the folder trust in `<config_home>/.claude.json` (ae's `auto_trust` answers
the modal) and the conversation under `<config_home>/projects/`; codex writes a rollout under
`~/.codex/sessions/`. The conversations a run leaves are NAMED in the report, never purged. Pane
text is read in memory and only decides; the report carries fixed words, versions and ids, never a
transcript, a token or a credentials path.

## Mutation testing

A green suite says the tests pass, not that they would ever go red. `just rust-mutants` is
the lane that asks. It is a LOCAL lane, never a gate and never a score: the workflow in CI
(diff-bounded per push, full lane by manual dispatch) is unchanged and is not part of the
rules below. No percentage, ratio or threshold of mutants is ever cited, reported or
enforced; a result is the list of survivors and what each one turned out to be.

**A product slice mutates its own diff before the freeze.** A slice that changes `src/`
runs the lane over exactly its COMMITTED diff, under the lane lock like the gate, and
answers every survivor in its freeze report:

```bash
git status --short                 # must print nothing: if it does, stop and commit first
base=<sha>                         # the pre-slice commit the brief records; the report names it
git merge-base --is-ancestor "$base" HEAD   # exit 0, or stop
git diff "$base" HEAD -- src > "$PWD/.local/<slice>/slice.diff"
cargo mutants --list --in-diff "$PWD/.local/<slice>/slice.diff" > "$PWD/.local/<slice>/mutants.list"
wc -l < "$PWD/.local/<slice>/mutants.list"   # the count, before the cost
just rust-mutants --in-diff "$PWD/.local/<slice>/slice.diff"
```

`just rust-mutants` hands every argument to `cargo mutants` as the one word you typed, so a regex with a space or a `|` needs only the shell quoting you would give `cargo mutants` itself (`-F 'fn a b|fn c'`).

Run the lines one at a time and stop at a nonzero exit: a failed listing is not a count of
zero. The base is the recorded pre-slice commit, never a guess: `git merge-base main HEAD`
is a fork point, which is `HEAD` on `main` or after the slice has landed (the slice's own
diff disappears) and reaches back over earlier slices on a stacked branch, so use it only
after proving it equals the recorded commit. `mutation: nothing run (<why>)` is for a slice
whose diff really holds no mutatable `src/` code; a slice that could not be run (no
recorded base, no lane) reports an explicit verification gap instead, never a clean line.

* **Budget.** Provisional until a run on the current lane is measured, and no archived run
  measures it. The archive (September and October 2026, 54 runs, 2821 mutants) mixes scopes
  and settings: 34 runs, 1660 mutants, ran the whole suite (four or eight test threads, or
  the nextest default) and cost on average 40 s per caught mutant (median 10 s), 425 s per
  survivor (median 314 s), 3 s per unviable one, after a baseline test run of 193 to 661 s
  (median 312 s); the other 20 runs, 1161 mutants, filtered the tests to the code they
  mutated and cost 126 s per mutant on average. A survivor costs about one whole test run
  of its lane, so the cost follows the number of survivors more than the number of
  mutants. A full-suite baseline is now projected, not measured, at 55 to 60 minutes at two
  test threads on 8043 tests, so a slice filters its tests to the code it mutates with
  `-- -E` and a survivor of that filtered set is reported as such, never as final. A
  slice's provisional budget is 40 mutants, about an hour with its baseline
  run. A larger diff is cut to the code that carries the slice's logic (`-f <file>`,
  `-F <regex>`, or `--shard k/n` with `--sharding round-robin` for a spread over the diff)
  and the freeze names what was not run; the lead may raise the budget. A later shard of
  the same tree may pass `--baseline=skip` once an earlier shard's baseline passed. `-F`
  does not filter struct-field mutants (cargo-mutants 27.1): they ride along from every
  selected file, so trust the `--list` count, not the regex. `just rust-mutants` hands each
  argument to `cargo mutants` as the one word you typed, so a regex is quoted once: `-F '(a|b)'`.
* **Survivors.** Each survivor in the diff is answered in the freeze report, one line:
  killed by a test the slice added (name it), or named equivalent (say why no behaviour
  differs), or dead code (and removed), or timeout or unviable. A timeout is not a kill.
  A mutant the RSS watcher killed (`mutants.out/ae-rss-kills.log`) reads as caught but is
  memory growth, and is reported as that.
* **Added tests** come from the requirement or the invariant, never from what the code
  says (the test-oracle rule), and each is shown to fail on its mutant. Run that mutant
  alone, and only if `--list` with the same arguments selects exactly one mutant; `-F`
  cannot isolate a struct-field mutant, so for those, and whenever the count is not one,
  apply the mutant's patch (`cargo mutants --list --diff`) by hand in a throwaway scratch
  tree and run the named test there. Inspect the outcome and the test's own failure: it
  survived before the test, and the new test fails on it. A survivor is never answered by
  weakening or deleting a test.
* **Known gap.** `--in-diff` selects source mutants only: a weakened test, or a change to
  anything that shapes how tests run, is invisible to it. A slice that changes tests reads
  that diff by eye, and a slice with no `src/` change has nothing to mutate.

**A campaign is a scoped, budgeted look at code that has never been mutated.** The lead
orders it; it is not part of any slice. Scope is a fixed set of functions in three to six
files, chosen by blast radius and by the absence of earlier mutation evidence, counted with
`--list` first and sized to the wall cap (3 hours unless the lead raises it; about 100
mutants plus their baselines is the provisional size, on the same terms as the slice
budget). It runs sharded under the lane lock, one claim per shard. Every survivor is triaged
on one line in a `triage.md` beside the run as equivalent (why), test gap (where, and the
oracle that would kill it), dead code, or timeout/unviable, and a reviewer on another
provider spot-checks the equivalent claims. A test gap is closed by a fix round that adds
tests only. A product bug found on the way stops the campaign and becomes its own slice.

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

`just rust-prune` only reports; `just rust-prune apply=yes` cleans. It runs six cleans, each
`cargo clean --locked -p ae` with `--profile dev`, `--profile release`, `--profile release
--target aarch64-apple-darwin` (where `just bundles` puts the native binary), `--profile release
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

1. Pre-flight: clean working tree; on `main`; `gh` authenticated **and** `repos/<owner>/<repo>` reporting `permissions.push=true`; the musl cross compiler present; a usable `AE_RELEASE_HP` and `AE_RELEASE_HP_ROOT`; fetch tags, pull rebase.
2. `just check` (shellcheck + shfmt).
3. `just release-proof` (see [Release proof](#release-proof)): the hp's whole green receipt for exactly this commit, over ssh, then the Mac boundary subset `just test-macos`. fmt-check, lint, fuzz-check and the doctests run again after the bump.
4. `just rust-deny` audits advisories, licenses, bans and sources (`cargo deny --locked check`). The advisory DB needs network; an offline release refuses here, before the bump. The identity recheck follows: the checkout must still be clean on the proved HEAD and tree.
5. `just bump` updates the version — `Cargo.toml` and `Cargo.lock`, which since slice Z3 are the only files that hold one — and rewrites the README and docs/index release badges. It refuses to proceed if the pre-release badge or the obsolete checkout-install prose is still standing in either file; a real build-from-source section is not what it is looking for.
6. `just bundles` builds both halves into `dist/` and proves them; a bounded smoke (`--version` and `doctor`, 10 s each) of the native binary follows.
7. `git-cliff` → `CHANGELOG.md` + release-body.
   Before the commit the release proves that only the six version paths changed.
8. Commit, tag locally, write the notes file and check every asset, push the branch, then `gh release create --target <sha>` with the three assets and the changelog section as `--notes-file`.

**Why the order is what it is.** Everything that can refuse refuses in step 1, before a
version file is written and long before a tag exists — an authenticated `gh` account is not
the same thing as one that can push here, which is why the API is asked rather than the
login trusted, and the branch is checked there rather than after five files have been
rewritten. The bundles are built in step 6, still before anything is pushed.

**The remote tag is created by the release, not pushed ahead of it.** A `git push <tag>`
that lands and a `gh release create` that then fails publishes a version with nothing behind
it: `install` resolves the latest release, finds no `SHA256SUMS`, and the install one-liner
is broken for everyone. So step 8 pushes only the branch and lets
`gh release create --target <sha>` create the tag object and the release in one call with
the assets attached. If that call fails, the remote has no tag, the local one survives, and
the retry command is printed; a re-run against an existing release uploads into it with
`--clobber`. `tests/it/gate.rs` pins the whole order and refuses a `git push` of the tag.

**What `just bundles` proves, and what it does not.** It builds `darwin-arm64` natively and
`linux-x86_64-musl` against the Homebrew cross toolchain in ONE `cargo build --release --locked --target aarch64-apple-darwin --target x86_64-unknown-linux-musl` (one jobserver, one build-directory lock, the two single-codegen-unit `ae` compiles side by side; `lto = "thin"` and `codegen-units = 1` unchanged), so the native binary is `target/aarch64-apple-darwin/release/ae` and the musl one `target/x86_64-unknown-linux-musl/release/ae`; runs `--version`
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
