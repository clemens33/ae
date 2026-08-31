set dotenv-load
set positional-arguments

CI := env("CI", "false")
GIT_REMOTE := env("GIT_REMOTE", "origin")
default_branch := "main"

# ── Development ──────────────────────────────────────────────────────

# Run all quality checks
check: lint format-check

# The contrib aemonitor/aewatch helpers are Python, not shell; only their bash
# runners are linted here. The e2e-ai harness + scenario drivers are linted but
# NEVER run by `check`.
#
# `< /dev/null` IS THE FIX FOR #67, not tidiness. shellcheck reads stdin when fd
# 0 is open, and an agent harness hands its tool calls a UNIX SOCKET. If that
# socket's peer has not closed by the time the read happens, it never returns
# EOF and the process blocks FOREVER at 0.0% CPU — wedges observed at 4h40m,
# 8h40m, 16h52m and 18h33m beside successful runs of the same command, with
# nothing in between: a race on the peer's close, not slowness. Every input is
# already named on the argv, so this costs nothing and removes the race.
# Reproduce on demand with a fifo no one ever closes; a plain rerun passes by
# luck most of the time. tests/unit pins the redirect structurally, because it
# reads like line-noise to the next person tidying the recipe.

# Lint with shellcheck
lint:
    shellcheck -x ae tests/unit tests/integration tests/aemonitor tests/aewatch install \
        contrib/ae-next/ae-next contrib/ae-next/install contrib/ae-next/install-remote \
        tests/e2e/ai/lib.sh tests/e2e/ai/run_scenario.sh \
        $(find tests/e2e/ai/scenarios -name steps.sh) < /dev/null

# `< /dev/null` here is insurance, not a fix: shfmt reads stdin only when given
# no paths, and it is given two. Deliberately NOT pinned by a test — pinning
# insurance as contract makes the next reader treat it as load-bearing.

# Check formatting (shfmt, diff mode)
format-check:
    shfmt -d -i 4 -ci ae install contrib/ae-next/ae-next contrib/ae-next/install contrib/ae-next/install-remote < /dev/null

# Auto-format
format:
    shfmt -w -i 4 -ci ae install contrib/ae-next/ae-next contrib/ae-next/install contrib/ae-next/install-remote

# ── Testing ──────────────────────────────────────────────────────────

# Run all tests
test: test-unit test-integration test-aemonitor test-aewatch

# Unit tests (pure functions, no deps)
test-unit:
    bash tests/unit

# Integration tests (requires tmux, git)
test-integration:
    AE_ITEST_FULL=1 bash tests/integration

# contrib aemonitor helper tests (requires python3; deterministic fixtures)
test-aemonitor:
    bash tests/aemonitor

# contrib aewatch sidecar tests (stdlib unittest; skips if Python < 3.11).
# This is the FULL PHASE GATE — it runs the slow bash-vs-python dual-run oracle.
test-aewatch:
    bash tests/aewatch

# FAST commit inner loop: AEWATCH_FAST=1 skips the subprocess-backed bash-oracle
# dual-runs, leaving the pure-Python surface (seconds, not minutes). NOT the phase
# gate — run `just test-aewatch` (+ contracts validate + check + git diff -- ae)
# for that.
test-aewatch-fast:
    AEWATCH_FAST=1 bash tests/aewatch

# AI-driven e2e (OPT-IN: needs AE_E2E_AI=1, runs REAL agents against your real
# subscription — real tokens, your live rate budget). NOT part of `check`/`test`.
test-ai *args="tests/e2e/ai/scenarios":
    AE_E2E_AI={{ env_var_or_default("AE_E2E_AI", "") }} tests/e2e/ai/run_scenario.sh {{ args }}

# ── Coexistence: ae-next (pre-P5) ────────────────────────────────────
# Installs a SECOND command, `ae-next`, that runs this branch's hybrid (bash glue
# + Rust core) with its own state home (~/.ae-next), its own tmux server and an
# immutable copy of the core under ~/.ae-next/core/<version>/. It never touches
# ~/.local/bin/ae, ~/.ae, or the `install` script — dogfooding the rewrite is not
# supposed to cost you the ae you work with.
#
# Retires at the P5 entry flip, together with contrib/ae-next itself.
# Design and canary plan: docs/migration/coexistence.md

# Install/upgrade the ae-next command beside your installed ae
next-install:
    contrib/ae-next/install

# ── Version ──────────────────────────────────────────────────────────

# Show current version
version:
    @grep -m1 '^AE_VERSION=' ae | cut -d'"' -f2

# Restore a previously interrupted CalVer bump from durable backups.
bump-recover:
    #!/usr/bin/env bash
    set -euo pipefail
    RECOVERY_DIR=".ae-bump-recovery"
    paths=(ae Cargo.toml Cargo.lock)
    staged=()
    if [[ ! -d "$RECOVERY_DIR" ]]; then
        echo "Error: ${RECOVERY_DIR} is not present; nothing to recover" >&2
        exit 1
    fi
    if [[ ! -f "$RECOVERY_DIR/backups-ready" ]]; then
        echo "Error: ${RECOVERY_DIR}/backups-ready is missing; marker retained and live files unchanged" >&2
        exit 1
    fi

    cleanup() {
        local status="$?"
        local restore_tmp
        for restore_tmp in "${staged[@]}"; do
            if [[ -n "$restore_tmp" && -e "$restore_tmp" ]] && ! rm -f "$restore_tmp"; then
                status=1
            fi
        done
        return "$status"
    }
    trap cleanup EXIT

    for index in "${!paths[@]}"; do
        path="${paths[$index]}"
        name="${path##*/}"
        backup="$RECOVERY_DIR/${name}.orig"
        if [[ ! -f "$backup" ]]; then
            echo "Error: missing ${backup}; marker retained and live files unchanged" >&2
            exit 1
        fi
        if ! restore_tmp="$(mktemp "${path}.bump-recover.XXXXXX")"; then
            echo "Error: could not stage ${path}; marker retained and live files unchanged" >&2
            exit 1
        fi
        staged+=("$restore_tmp")
        if ! cp -p "$backup" "$restore_tmp"; then
            echo "Error: could not stage ${path}; marker retained and live files unchanged" >&2
            exit 1
        fi
        if ! cmp -s "$restore_tmp" "$backup"; then
            echo "Error: staged restore verification failed for ${path}; marker retained and live files unchanged" >&2
            exit 1
        fi
    done

    for index in "${!paths[@]}"; do
        path="${paths[$index]}"
        if ! mv "${staged[$index]}" "$path"; then
            echo "Error: could not restore ${path}; marker retained" >&2
            exit 1
        fi
    done

    if ! rm -rf "$RECOVERY_DIR"; then
        echo "Error: restored files but could not remove ${RECOVERY_DIR}; marker retained" >&2
        exit 1
    fi
    trap - EXIT

# Compute next release version using SemVer-compatible CalVer: YYYY.M.N.
# The sequence is tag-derived, so a stale working tree version cannot cause a
# duplicate publication. Version files use durable backups and recover-or-refuse
# publication; stdout remains the VERSION-only contract for just release.
bump:
    #!/usr/bin/env bash
    set -euo pipefail
    RECOVERY_DIR=".ae-bump-recovery"
    if [[ -e "$RECOVERY_DIR" ]]; then
        echo "Error: stale ${RECOVERY_DIR} exists; recover before starting another bump." >&2
        echo "run just bump-recover." >&2
        exit 1
    fi

    YEAR_MONTH="$(date -u +%Y).$((10#$(date -u +%m)))"
    TAG_PREFIX="v${YEAR_MONTH}."
    TAG_RE="^v${YEAR_MONTH//./\\.}\.([0-9]+)$"
    MAX=0
    while IFS= read -r tag; do
        if [[ "$tag" =~ $TAG_RE ]]; then
            sequence="${BASH_REMATCH[1]}"
            if ((10#$sequence > MAX)); then
                MAX=$((10#$sequence))
            fi
        fi
    done < <(git tag --list "${TAG_PREFIX}*")

    VERSION="${YEAR_MONTH}.$((MAX + 1))"
    TAG="v${VERSION}"
    if git show-ref --verify --quiet "refs/tags/${TAG}"; then
        echo "Error: release tag ${TAG} already exists" >&2
        exit 1
    fi

    TMP_DIR="$(mktemp -d .ae-bump.XXXXXX)"
    if ! mkdir "$RECOVERY_DIR"; then
        rm -rf "$TMP_DIR"
        echo "Error: could not create ${RECOVERY_DIR}" >&2
        exit 1
    fi

    recover() {
        local path name restore_tmp recovery_rc=0
        if [[ ! -f "$RECOVERY_DIR/backups-ready" ]]; then
            rm -f "$RECOVERY_DIR"/*.orig "$RECOVERY_DIR"/*.orig.tmp.* "$RECOVERY_DIR"/backups-ready.tmp.* || true
            return 0
        fi
        for path in ae Cargo.toml Cargo.lock; do
            name="${path##*/}"
            if [[ ! -f "$RECOVERY_DIR/${name}.orig" ]]; then
                recovery_rc=1
                continue
            fi
            restore_tmp="$RECOVERY_DIR/${name}.restore.$$"
            if ! cp -p "$RECOVERY_DIR/${name}.orig" "$restore_tmp" ||
                ! mv "$restore_tmp" "$path"; then
                rm -f "$restore_tmp" || true
                recovery_rc=1
            fi
        done
        return "$recovery_rc"
    }
    cleanup() {
        local status="$?"
        local recovery_status=0
        # An interrupted or failed publication restores every backed-up file
        # before either temporary data or the durable recovery marker is removed.
        if [[ "${BUMP_PUBLISHED:-0}" != 1 ]]; then
            recover || recovery_status="$?"
        fi
        rm -rf "$TMP_DIR" || recovery_status=1
        if ((recovery_status == 0)); then
            rm -rf "$RECOVERY_DIR" || recovery_status=1
        fi
        if ((recovery_status != 0)); then
            echo "Error: bump recovery failed; ${RECOVERY_DIR} retained for manual recovery" >&2
            return "$recovery_status"
        fi
        return "$status"
    }
    trap cleanup EXIT
    backup_files() {
        local path name backup_tmp
        for path in ae Cargo.toml Cargo.lock; do
            name="${path##*/}"
            backup_tmp="$RECOVERY_DIR/${name}.orig.tmp.$$"
            cp -p "$path" "$backup_tmp"
            mv "$backup_tmp" "$RECOVERY_DIR/${name}.orig"
        done
        for path in ae Cargo.toml Cargo.lock; do
            name="${path##*/}"
            if ! cmp -s "$RECOVERY_DIR/${name}.orig" "$path"; then
                echo "Error: backup verification failed for ${path}" >&2
                return 1
            fi
        done
        : >"$RECOVERY_DIR/backups-ready.tmp.$$"
        mv "$RECOVERY_DIR/backups-ready.tmp.$$" "$RECOVERY_DIR/backups-ready"
    }
    backup_files
    for path in ae Cargo.toml Cargo.lock; do
        name="${path##*/}"
        cp -p "$path" "$TMP_DIR/$name"
    done

    # Redirect into mode-preserving copies: replacing ae with a freshly-created
    # redirection target would silently drop its executable bit.
    cp -p "$TMP_DIR/ae" "$TMP_DIR/ae.next"
    sed "s/^AE_VERSION=\".*\"/AE_VERSION=\"$VERSION\"/" \
        "$TMP_DIR/ae" >"$TMP_DIR/ae.next"
    mv "$TMP_DIR/ae.next" "$TMP_DIR/ae"
    cp -p "$TMP_DIR/Cargo.toml" "$TMP_DIR/Cargo.toml.next"
    awk -v version="$VERSION" '
        !done && /^version = "/ { print "version = \"" version "\""; done=1; next }
        { print }
    ' "$TMP_DIR/Cargo.toml" >"$TMP_DIR/Cargo.toml.next"
    mv "$TMP_DIR/Cargo.toml.next" "$TMP_DIR/Cargo.toml"
    cp -p "$TMP_DIR/Cargo.lock" "$TMP_DIR/Cargo.lock.next"
    awk -v version="$VERSION" '
        function flush(    i, name, source, versions, replaced) {
            if (n == 0) return
            name = ""
            source = 0
            versions = 0
            for (i = 1; i <= n; i++) {
                if (lines[i] == "name = \"ae\"") name = "ae"
                if (lines[i] ~ /^source = /) source = 1
                if (lines[i] ~ /^version = "/) versions++
            }
            if (name == "ae" && !source) {
                roots++
                if (versions != 1) invalid = 1
                if (roots == 1) {
                    replaced = 0
                    for (i = 1; i <= n; i++) {
                        if (!replaced && lines[i] ~ /^version = "/) {
                            print "version = \"" version "\""
                            replaced = 1
                        } else {
                            print lines[i]
                        }
                    }
                    if (!replaced) invalid = 1
                } else {
                    for (i = 1; i <= n; i++) print lines[i]
                }
            } else {
                for (i = 1; i <= n; i++) print lines[i]
            }
            delete lines
            n = 0
        }
        /^\[\[package\]\]$/ {
            flush()
            lines[++n] = $0
            next
        }
        { lines[++n] = $0 }
        END {
            flush()
            if (roots != 1 || invalid) exit 1
        }
    ' "$TMP_DIR/Cargo.lock" >"$TMP_DIR/Cargo.lock.next"
    mv "$TMP_DIR/Cargo.lock.next" "$TMP_DIR/Cargo.lock"

    grep -q '^AE_VERSION="'"$VERSION"'"$' "$TMP_DIR/ae"
    grep -q '^version = "'"$VERSION"'"$' "$TMP_DIR/Cargo.toml"
    awk -v version="$VERSION" '
        function flush(    i, name, source) {
            if (n == 0) return
            name = ""
            source = 0
            for (i = 1; i <= n; i++) {
                if (lines[i] == "name = \"ae\"") name = "ae"
                if (lines[i] ~ /^source = /) source = 1
            }
            if (name == "ae" && !source) {
                roots++
                for (i = 1; i <= n; i++) {
                    if (lines[i] == "version = \"" version "\"") matches++
                }
            }
            delete lines
            n = 0
        }
        /^\[\[package\]\]$/ {
            flush()
            lines[++n] = $0
            next
        }
        { lines[++n] = $0 }
        END {
            flush()
            exit !(roots == 1 && matches == 1)
        }
    ' "$TMP_DIR/Cargo.lock"

    for path in ae Cargo.toml Cargo.lock; do
        name="${path##*/}"
        if ! mv "$TMP_DIR/$name" "$path"; then
            echo "Error: could not publish ${path}; recover-or-refuse marker retained if recovery fails" >&2
            exit 1
        fi
    done
    BUMP_PUBLISHED=1
    trap - EXIT
    rm -rf "$TMP_DIR" "$RECOVERY_DIR"
    printf '%s\n' "$VERSION"

# ── Changelog ────────────────────────────────────────────────────────

# Generate full CHANGELOG.md from git history
changelog:
    git-cliff -o CHANGELOG.md

# ── Release ──────────────────────────────────────────────────────────

# Full release pipeline: check → test → bump → changelog → tag → gh release
# Usage: just release
release:
    #!/usr/bin/env bash
    set -euo pipefail

    # Pre-flight: clean working tree (staged, unstaged, AND untracked)
    if ! git diff --quiet HEAD || [ -n "$(git ls-files --others --exclude-standard)" ]; then
        echo "Error: uncommitted or untracked changes" >&2; exit 1
    fi
    git fetch {{GIT_REMOTE}} --tags
    git pull {{GIT_REMOTE}} {{default_branch}} --rebase

    # Gate 1: Quality
    just check

    # Gate 2: Tests
    just test

    # Version
    VERSION=$(just bump)
    echo "Releasing v$VERSION"
    # Re-parse and compile after bump before any changelog or tag publication.
    cargo check --locked

    # Update the README badge. `just bump` preserves ae's executable bit.
    sed_i() {
        local f="$1"; shift
        cp -p "$f" "$f.tmp.$$" || return 1
        sed "$@" "$f" > "$f.tmp.$$" && mv "$f.tmp.$$" "$f" || { rm -f "$f.tmp.$$"; return 1; }
    }
    sed_i README.md -E "s/release-[0-9]+\\.[0-9]+\\.[0-9]+/release-$VERSION/" 2>/dev/null || true
    # Guard the guard: a release must never publish ae without its exec bit.
    [ -x ae ] || { echo "Error: ae lost its executable bit during version bump" >&2; exit 1; }

    # Generate changelog
    TAG="v$VERSION"
    git-cliff --tag "$TAG" -o CHANGELOG.md
    RELEASE_BODY=$(git-cliff --tag "$TAG" --unreleased --strip header)
    RELEASE_BODY="${RELEASE_BODY:-Release $TAG}"

    # Commit
    BRANCH=$(git rev-parse --abbrev-ref HEAD)
    if [ "$BRANCH" != "{{default_branch}}" ]; then
        echo "Error: releases must be from {{default_branch}} (currently on $BRANCH)" >&2; exit 1
    fi

    git add CHANGELOG.md
    git add -u
    git diff --cached --quiet || git commit -m "chore(release): $TAG"

    # Tag + push
    git tag "$TAG"
    git push {{GIT_REMOTE}} "$TAG"
    git push {{GIT_REMOTE}} {{default_branch}}

    # GitHub release (best-effort, requires gh CLI with repo access)
    if command -v gh &>/dev/null; then
        REPO=$(gh repo view --json nameWithOwner -q .nameWithOwner 2>/dev/null || true)
        if [[ -n "$REPO" ]]; then
            gh api "repos/$REPO/releases" \
                -f tag_name="$TAG" -f target_commitish={{default_branch}} -f name="$TAG" \
                -f body="$RELEASE_BODY" -f make_latest=true > /dev/null 2>&1 \
                && echo "GitHub release created" \
                || echo "Warning: GitHub release failed (tag pushed, create release manually)" >&2
        fi
    fi

    echo "Released $TAG"

# ── Install ──────────────────────────────────────────────────────────

# Install ae (symlink to ~/.local/bin)
install:
    ./install

# ── Docs ─────────────────────────────────────────────────────────────
# Optional. Requires `pip install mkdocs-material`.

# Serve the docs site locally with live reload
docs:
    mkdocs serve

# Build the static docs site into ./site (gitignored)
docs-build:
    mkdocs build --strict

# ── Quick Reference ──────────────────────────────────────────────────

# Show available recipes
help:
    @just --list

# ── Rust (rewrite branch, epic #79 / #80) ────────────────────────────
# The bash-era recipes above stay while bash ae is frozen. Everything below is
# prefixed `rust-` and touches nothing they own.
#
# PINS ARE THE CONTRACT. This block is the single source of truth for dev-tool
# versions; `rust-setup` installs exactly these and nothing else. The compiler
# pin lives in rust-toolchain.toml (cargo/rustup read it, just does not).
#
# EVERY graph-consuming cargo invocation below passes `--locked`. Without it
# cargo will happily UPDATE Cargo.lock to satisfy a build and then report green —
# a committed lockfile that no lane ever enforces is decoration. `cargo fmt` is
# the one exception: it does not resolve the dependency graph.
# Two spellings differ and are not interchangeable:
#   cargo-deny takes it as a GLOBAL option  -> `cargo deny --locked check`
#     (`cargo deny check --locked` exits 2 — measured)
#   cargo-mutants does not accept it at all -> `--cargo-arg=--locked` passes it
#     through to the cargo it drives (measured; bare `--locked` is rejected)

# `just` is a PREREQUISITE of the bootstrap contract, not something rust-setup
# installs — you cannot run a recipe that installs the tool running the recipe.
# The pin is recorded HERE anyway so CI has ONE source of truth to read, and so a
# bump re-keys the CI tool cache instead of silently reusing the old binary.
JUST_VERSION := "1.57.0"

NEXTEST_VERSION := "0.9.143"
TAPLO_VERSION := "0.10.0"
DENY_VERSION := "0.20.2"
MUTANTS_VERSION := "27.1.0"
LLVM_COV_VERSION := "0.9.0"
# Arrived with the FIRST runtime dependency (P4.3): cargo-deny gates the graph's
# advisories/licenses/bans/sources, cargo-vet gates its PROVENANCE (who reviewed
# the code). See `rust-vet`.
VET_VERSION := "0.10.2"

# The foreign target. musl, not gnu — the epic promises a STATIC zero-dep binary
# and gnu is not that (see rust-toolchain.toml for the NSS caveat). Compile-smoke
# only: `cargo check` never links, so nothing produced here can be mistaken for a
# runnable artifact.

RUST_CROSS_TARGET := "x86_64-unknown-linux-musl"

# Bootstrap: rustup toolchain + the pinned dev tools. Idempotent — every tool is
# version-checked first, so a second run installs nothing. Honest prerequisites:
# rustup and just, nothing else.

# Install the pinned Rust toolchain + dev tools (idempotent)
rust-setup:
    #!/usr/bin/env bash
    set -euo pipefail

    command -v rustup >/dev/null || { echo "error: rustup is required (https://rustup.rs)" >&2; exit 1; }

    # No toolchain argument: rustup resolves the ACTIVE toolchain, which in this
    # directory is rust-toolchain.toml — channel, profile, components and both
    # targets come from the pin rather than being restated here.
    echo "==> toolchain (rust-toolchain.toml)"
    rustup toolchain install --no-self-update

    # `cargo install` compiles from source and is slow, so it runs only on an
    # actual version mismatch. Each probe prints the tool's own reported version
    # and matches the pin as a WHOLE WORD — a prefix match would accept 0.9.1430.
    ensure() {
        local crate="$1" want="$2" probe="$3"
        local have
        have="$(eval "$probe" 2>/dev/null | tr -c '0-9A-Za-z.' '\n' | grep -Fx "$want" || true)"
        if [ -n "$have" ]; then
            printf '    ok   %-16s %s\n' "$crate" "$want"
            return 0
        fi
        printf '    ==>  %-16s installing %s\n' "$crate" "$want"
        cargo install --locked --version "$want" "$crate"
    }

    echo "==> pinned dev tools"
    ensure cargo-nextest  "{{ NEXTEST_VERSION }}"  'cargo nextest --version'
    ensure taplo-cli      "{{ TAPLO_VERSION }}"    'taplo --version'
    ensure cargo-deny     "{{ DENY_VERSION }}"     'cargo deny --version'
    ensure cargo-mutants  "{{ MUTANTS_VERSION }}"  'cargo mutants --version'
    ensure cargo-llvm-cov "{{ LLVM_COV_VERSION }}" 'cargo llvm-cov --version'
    ensure cargo-vet      "{{ VET_VERSION }}"      'cargo vet --version'

    echo "==> ready — run: just rust-check"

# The gate: everything a change must pass before it is offered for review
rust-check: rust-fmt-check rust-lint rust-test

# Check formatting in both languages of the build: Rust and TOML
rust-fmt-check:
    cargo fmt --all --check
    taplo fmt --check

# Auto-format Rust + TOML
rust-fmt:
    cargo fmt --all
    taplo fmt

# `-D warnings` is what makes [lints] a gate rather than a suggestion — which is
# also why unwrap_used/expect_used are declared "warn" in Cargo.toml and still
# fail here. --all-targets so tests are linted too.

# Clippy (-D warnings) + taplo lint
rust-lint:
    cargo clippy --locked --all-targets --all-features -- -D warnings
    taplo lint

# nextest does NOT run doctests. Doctests are KEPT — they are the executable half
# of the public docs — so they get their own invocation. Dropping that second
# line silently retires a whole lane.

# Run the test suite: nextest + doctests
rust-test:
    cargo nextest run --locked --all-features
    cargo test --doc --locked --all-features

# Coverage is a REPORT, not a gate. It becomes a gate the day a threshold is
# ratified, and not before.

# Coverage report (not a gate)
rust-cov:
    cargo llvm-cov nextest --locked --all-features

# The lane that asks whether the tests DISCRIMINATE, not just whether they pass.
# Agents write tests that pass; this is the check that costs them something.

# Mutation testing
rust-mutants *args:
    cargo mutants --cargo-arg=--locked {{ args }}

# The `--allow license-not-encountered` crutch is GONE as of the first real
# dependency (2026-08-29, P4.3 tracer A): the deny.toml allow-list is now
# minimal-to-encountered, so an unmatched entry is a genuine drift signal rather
# than the zero-dependency baseline noise it used to be. Do not re-add the flag;
# prune or extend the allow-list instead.

# Supply chain: advisories, licenses, bans, sources (policy in deny.toml)
rust-deny:
    cargo deny --locked check

# Supply-chain PROVENANCE: has a human reviewed each dependency's code? cargo-vet
# arrived with the first runtime dependency (P4.3), orthogonal to cargo-deny (do
# NOT also add cargo-audit — that duplicates deny's RustSec lane). Posture is the
# honest first setup: four trusted registries (mozilla, google, zcash, isrg) are
# imported and PINNED in supply-chain/imports.lock, so `--locked` re-checks
# offline in CI. The current graph is grandfathered as `exemptions` in
# supply-chain/config.toml — the audit BACKLOG, not an audited set: ring, rustls
# and ureq are the priority to delta-audit (their exact versions are newer than
# the registries certify), and cargo-deny already gates them for advisories. A
# dependency or version that no import covers fails HERE until it is audited or
# exemption'd — that is the point.
rust-vet:
    cargo vet --locked

# NATIVE is a real, runnable binary and is run here to prove it.
#
# THE MUSL COMPILE-SMOKE WAS REMOVED at the first dependency (2026-08-29, P4.3
# tracer A). It used to be a free `cargo check --target {{ RUST_CROSS_TARGET }}`
# that never linked. `ring` ended that: its build script compiles C for the
# target, so a musl `cargo check` now needs a musl C toolchain
# (`x86_64-linux-musl-gcc`) — trivial to add on the Linux CI leg (`musl-tools`),
# a heavy from-source cross-toolchain on a macOS laptop. Forcing every clone to
# install one to run `rust-build-release` is the wrong trade: the musl artifact
# is BUILT, LINKED, RUN and proven static on the Linux CI leg
# (.github/workflows/rust.yml), which is the only place it can link anyway. This
# recipe stays native-only so a bare clone builds without a cross toolchain.
# {{ RUST_CROSS_TARGET }} is still the pinned musl triple, used by that CI leg
# and deny.toml's graph.

# Release build: native binary (musl artifact is a Linux-CI-only proof — see above)
rust-build-release:
    cargo build --release --locked
    @echo "==> native binary: target/release/ae"
    @./target/release/ae --version

# bacon is deliberately NOT installed by rust-setup: a personal dev loop is not
# part of the bootstrap contract.

# Optional watch loop (requires bacon, installed separately)
rust-watch:
    bacon clippy-all
