set dotenv-load
set positional-arguments

CI := env("CI", "false")
GIT_REMOTE := env("GIT_REMOTE", "origin")
default_branch := "main"

# ── Development ──────────────────────────────────────────────────────

# Run all quality checks
check: lint format-check

# Lint with shellcheck (the contrib aemonitor/aewatch helpers are Python, not
# shell; only their bash runners are linted here).
# The e2e-ai harness + scenario drivers are linted here but NEVER run by `check`.
lint:
    shellcheck -x ae tests/unit tests/integration tests/aemonitor tests/aewatch install \
        tests/e2e/ai/lib.sh tests/e2e/ai/run_scenario.sh \
        $(find tests/e2e/ai/scenarios -name steps.sh)

# Check formatting (shfmt, diff mode)
format-check:
    shfmt -d -i 4 -ci ae install

# Auto-format
format:
    shfmt -w -i 4 -ci ae install

# ── Testing ──────────────────────────────────────────────────────────

# Run all tests
test: test-unit test-integration test-aemonitor test-aewatch

# Unit tests (pure functions, no deps)
test-unit:
    bash tests/unit

# Integration tests (requires tmux, git)
test-integration:
    bash tests/integration

# contrib aemonitor helper tests (requires python3; deterministic fixtures)
test-aemonitor:
    bash tests/aemonitor

# contrib aewatch sidecar tests (stdlib unittest; skips if Python < 3.11)
test-aewatch:
    bash tests/aewatch

# AI-driven e2e (OPT-IN: needs AE_E2E_AI=1, runs REAL agents against your real
# subscription — real tokens, your live rate budget). NOT part of `check`/`test`.
test-ai *args="tests/e2e/ai/scenarios":
    AE_E2E_AI={{ env_var_or_default("AE_E2E_AI", "") }} tests/e2e/ai/run_scenario.sh {{ args }}

# ── Version ──────────────────────────────────────────────────────────

# Show current version
version:
    @grep -m1 '^AE_VERSION=' ae | cut -d'"' -f2

# Compute next release version using CalVer: YYYY.MM.BUILD
bump:
    #!/usr/bin/env bash
    set -euo pipefail
    CURRENT=$(grep -m1 '^AE_VERSION=' ae | cut -d'"' -f2)
    YEAR_MONTH="$(date +%Y.%m)"
    BUILD=1
    if [[ "$CURRENT" =~ ^([0-9]{4})\.([0-9]{2})\.([0-9]+)$ ]]; then
        CURRENT_YEAR_MONTH="${BASH_REMATCH[1]}.${BASH_REMATCH[2]}"
        if [[ "$CURRENT_YEAR_MONTH" == "$YEAR_MONTH" ]]; then
            BUILD=$((BASH_REMATCH[3] + 1))
        fi
    fi
    echo "${YEAR_MONTH}.${BUILD}"

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

    # Update version in script
    sed -i "s/^AE_VERSION=\".*\"/AE_VERSION=\"$VERSION\"/" ae

    # Update version badge in README
    sed -E -i "s/release-[0-9]+\\.[0-9]+\\.[0-9]+/release-$VERSION/" README.md 2>/dev/null || true

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
