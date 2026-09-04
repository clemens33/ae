set dotenv-load
set positional-arguments

CI := env("CI", "false")
GIT_REMOTE := env("GIT_REMOTE", "origin")
default_branch := "main"

# ── Development ──────────────────────────────────────────────────────

# Run all quality checks
check: lint format-check

# THE SUBJECT IS THE BASH THAT STILL EXISTS, and after slice Z4 that is ONE file.
# The bash suites are retired into `tests/it` and the contrib sidecars' runners
# went with the sidecars, so what is left to lint is `install` — the bootstrap
# that has to run before there is a core to run.
#
# `< /dev/null` IS THE FIX FOR #67, not tidiness. shellcheck reads stdin when fd
# 0 is open, and an agent harness hands its tool calls a UNIX SOCKET. If that
# socket's peer has not closed by the time the read happens, it never returns
# EOF and the process blocks FOREVER at 0.0% CPU — wedges observed at 4h40m,
# 8h40m, 16h52m and 18h33m beside successful runs of the same command, with
# nothing in between: a race on the peer's close, not slowness. Every input is
# already named on the argv, so this costs nothing and removes the race.
# Reproduce on demand with a fifo no one ever closes; a plain rerun passes by
# luck most of the time. `tests/it/gate.rs` pins the redirect structurally,
# because it reads like line-noise to the next person tidying the recipe.

# Enforce the linter pin. Kept OUT of `lint` deliberately: the #67 gate requires the
# lint recipe to contain exactly ONE `shellcheck` token, so that the `< /dev/null` on
# that line provably protects the invocation's stdin. A version probe inside the body
# would be a second token and would defeat the check that exists to stop the wedge.
_shellcheck-pin:
    #!/usr/bin/env bash
    set -euo pipefail
    want="0.11.0"
    # AVAILABILITY BEFORE VERSION. Under `set -e` with pipefail, probing the version of a
    # binary that is not installed aborts this recipe at rc 127 with the probe's stderr
    # already redirected, so `just` prints an exit code and NONE of the guidance below —
    # exactly the fresh machine that needs it most. Ask whether the tool exists first, and
    # let a probe that runs but prints nothing recognisable fall through to the same
    # message instead of killing the shell.
    if command -v shellcheck >/dev/null 2>&1; then
        have="$(shellcheck --version 2>/dev/null | awk '/^version:/ {print $2}' || true)"
    else
        have=""
    fi
    if [ "${have:-}" != "$want" ]; then
        echo "Error: the bash lint lane is pinned to shellcheck $want, found ${have:-no shellcheck on PATH}" >&2
        echo "       Its finding set moves between releases, so an unpinned linter is a gate" >&2
        echo "       that changes without a commit. Install $want (macOS: brew install shellcheck)" >&2
        echo "       and confirm with: shellcheck --version" >&2
        exit 1
    fi

# Lint with shellcheck.
# SEVERITY FLOOR: warning and error are the gate; everything BELOW warning — info and
# style — is advisory and is NOT enforced. State it plainly: a green run here does NOT
# mean the raw linter reports nothing. The SC2016 and SC2329 sites carry their own reasoned comments
# instead. Full rationale: docs/development.md.
lint: _shellcheck-pin
    shellcheck --severity=warning -x install < /dev/null

# `< /dev/null` here is insurance, not a fix: shfmt reads stdin only when given
# no paths, and it is given two. Deliberately NOT pinned by a test — pinning
# insurance as contract makes the next reader treat it as load-bearing.

# Check formatting (shfmt, diff mode)
format-check:
    shfmt -d -i 4 -ci install < /dev/null

# Auto-format
format:
    shfmt -w -i 4 -ci install

# ── Testing ──────────────────────────────────────────────────────────

# THE GATE, and it is one command. Slice Z4 retired the bash suites into
# `tests/it` and the contrib sidecars moved into the core, so ae's whole test
# surface is Rust: `just rust-check` is format, lint and every test together.
#
# Minutes, not the half hour the serial bash integration suite cost. There is no
# inner-loop/gate split any more because there is nothing left to narrow — the
# fast lane and the gate are the same command, which is the point.

# Run all tests — THE GATE
test: rust-check

# ── Version ──────────────────────────────────────────────────────────

# Show current version. Since slice Z3 the crate is the ONLY place a version
# word lives — the wrapper that held the other half of the pair is deleted.
version:
    @awk '/^\[/ { in_package = ($0 == "[package]") } in_package && /^version = "/ { gsub(/^version = "|"$/, ""); print; exit }' Cargo.toml

# Restore a previously interrupted CalVer bump from durable backups.
bump-recover:
    #!/usr/bin/env bash
    set -euo pipefail
    RECOVERY_DIR=".ae-bump-recovery"
    paths=(Cargo.toml Cargo.lock)
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
#
# Z3: it owns Cargo.toml and Cargo.lock, and nothing else. There is no second
# version word to move in step — `ae-entry` was deleted with the rest of the
# product's bash, and with it the whole class of "the pair disagrees" failure.
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
        for path in Cargo.toml Cargo.lock; do
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
        for path in Cargo.toml Cargo.lock; do
            name="${path##*/}"
            backup_tmp="$RECOVERY_DIR/${name}.orig.tmp.$$"
            cp -p "$path" "$backup_tmp"
            mv "$backup_tmp" "$RECOVERY_DIR/${name}.orig"
        done
        for path in Cargo.toml Cargo.lock; do
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
    for path in Cargo.toml Cargo.lock; do
        name="${path##*/}"
        cp -p "$path" "$TMP_DIR/$name"
    done

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

    for path in Cargo.toml Cargo.lock; do
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

# THE WHOLE RELEASE HAPPENS HERE, on this machine. GitHub Actions is no longer
# on the critical path: `just bundles` builds and proves both platform halves
# locally, and `gh release create` attaches them. The tag-triggered workflow is
# retained as a manually dispatched Linux run-proof lane, not as the publisher.
#
# THE ORDER IS THE SAFETY PROPERTY. Everything that can refuse — a dirty tree,
# a gh account without push rights, a missing cross toolchain — refuses in the
# pre-flight, before the bump writes a version file and long before a tag
# exists. Everything that can fail expensively — the gates, the bundles —
# happens before the tag too. A release that dies after `git push --tags` has
# published a version with no assets behind it, which is the half-done state
# this ordering exists to prevent.

# Full release pipeline: check → test → bump → bundles → changelog → tag → gh release
# Usage: just release
release:
    #!/usr/bin/env bash
    set -euo pipefail

    # Pre-flight: clean working tree (staged, unstaged, AND untracked)
    if ! git diff --quiet HEAD || [ -n "$(git ls-files --others --exclude-standard)" ]; then
        echo "Error: uncommitted or untracked changes" >&2; exit 1
    fi

    # PUBLICATION RIGHTS, PROVED BEFORE ANYTHING IS IRREVERSIBLE. Now that the
    # release is built and attached from here, `gh` is a hard prerequisite
    # rather than the best-effort afterthought it was when a runner published.
    #
    # `.permissions.push` is ASKED OF THE API, never inferred from a successful
    # login: an account can be authenticated, hold the `repo` scope, and still
    # have pull-only rights on this repository — which is the exact state this
    # laptop is in when the wrong one of two logged-in accounts is active.
    command -v gh >/dev/null 2>&1 || { echo "Error: gh is required to publish a release" >&2; exit 1; }
    gh auth status >/dev/null 2>&1 || { echo "Error: gh is not authenticated — gh auth login" >&2; exit 1; }
    REPO=$(gh repo view --json nameWithOwner -q .nameWithOwner)
    [ -n "$REPO" ] || { echo "Error: gh cannot resolve this repository" >&2; exit 1; }
    PUSH=$(gh api "repos/$REPO" --jq .permissions.push)
    if [ "$PUSH" != "true" ]; then
        echo "Error: the active gh account cannot push to $REPO (permissions.push=$PUSH)" >&2
        echo "       gh auth status            # which account is active" >&2
        echo "       gh auth switch --hostname github.com --user <account>" >&2
        exit 1
    fi

    # The cross toolchain `just bundles` links the Linux half with, checked in
    # the same breath and for the same reason: it is present or it is not, and
    # discovering that after the tag is pushed helps nobody.
    command -v {{ RUST_MUSL_CC }} >/dev/null 2>&1 || {
        echo "Error: {{ RUST_MUSL_CC }} not found — the linux-x86_64-musl bundle cannot be linked." >&2
        echo "       brew install messense/macos-cross-toolchains/x86_64-unknown-linux-musl" >&2
        exit 1
    }

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

    # Gate 3: the artifacts themselves. Both platform halves and the
    # release-level SHA256SUMS, built and proven HERE — this is what replaced
    # the tag-triggered workflow. It runs BEFORE the tag deliberately: a failed
    # cross build or a musl binary that is not static costs a `git checkout` of
    # two version files, not an orphan tag with nothing behind it.
    just bundles

    # Update the release badges.
    sed_i() {
        local f="$1"; shift
        cp -p "$f" "$f.tmp.$$" || return 1
        sed "$@" "$f" > "$f.tmp.$$" && mv "$f.tmp.$$" "$f" || { rm -f "$f.tmp.$$"; return 1; }
    }
    replace_badge_line() {
        local file="$1" pre_pattern="$2" prior_pattern="$3" post_line="$4" label="$5" source_pattern
        if grep -Eq "$pre_pattern" "$file"; then
            source_pattern="$pre_pattern"
        elif grep -Eq "$prior_pattern" "$file"; then
            source_pattern="$prior_pattern"
        else
            echo "Error: ${label} has no known pre-release or prior-release badge line" >&2
            exit 1
        fi
        sed_i "$file" -E -e "s@${source_pattern}@${post_line}@"
        if ! grep -Fxq "$post_line" "$file"; then
            echo "Error: ${label} did not become its expected release badge line" >&2
            exit 1
        fi
    }
    README_VERSION_PRE='^!\[Version: [0-9]+\.[0-9]+\.[0-9]+ untagged/pre-release\]\(https://img\.shields\.io/badge/version-[0-9]+\.[0-9]+\.[0-9]+%20untagged%2Fpre--release-blue\.svg\)$'
    README_VERSION_PRIOR='^\[!\[Release: [0-9]+\.[0-9]+\.[0-9]+\]\(https://img\.shields\.io/badge/release-[0-9]+\.[0-9]+\.[0-9]+-blue\.svg\)\]\(https://github\.com/clemens33/ae/releases\)$'
    README_VERSION_POST="[![Release: $VERSION](https://img.shields.io/badge/release-$VERSION-blue.svg)](https://github.com/clemens33/ae/releases)"
    README_INSTALL_PRE='^\[!\[Install\]\(https://img\.shields\.io/badge/install-checkout%20install-orange\.svg\)\]\(#install\)$'
    README_INSTALL_PRIOR='^\[!\[Install: curl \| bash\]\(https://img\.shields\.io/badge/install-curl%20%7C%20bash-orange\.svg\)\]\(#install\)$'
    README_INSTALL_POST='[![Install: curl | bash](https://img.shields.io/badge/install-curl%20%7C%20bash-orange.svg)](#install)'
    INDEX_VERSION_PRE="$README_VERSION_PRE"
    INDEX_VERSION_PRIOR="$README_VERSION_PRIOR"
    INDEX_VERSION_POST="$README_VERSION_POST"
    replace_badge_line README.md "$README_VERSION_PRE" "$README_VERSION_PRIOR" "$README_VERSION_POST" "README version"
    replace_badge_line README.md "$README_INSTALL_PRE" "$README_INSTALL_PRIOR" "$README_INSTALL_POST" "README Install"
    replace_badge_line docs/index.md "$INDEX_VERSION_PRE" "$INDEX_VERSION_PRIOR" "$INDEX_VERSION_POST" "docs/index version"
    if grep -En 'untagged/pre-release|checkout%20install|checkout install' README.md docs/index.md >&2; then
        echo "Error: pre-release badge or checkout-install prose remains; edit it deliberately before tagging" >&2
        exit 1
    fi
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

    # GitHub release. NOT best-effort any more: the tag is pushed, so the assets
    # have to land, and push rights were proved in the pre-flight — a failure
    # here is a real failure and is reported as one rather than as a warning
    # nobody reads. The three assets are the ones `install` fetches, so a
    # release object without them is a broken install command.
    NOTES_FILE="$(mktemp "${TMPDIR:-/tmp}/ae-release-notes.XXXXXX")"
    trap 'rm -f "$NOTES_FILE"' EXIT
    printf '%s\n' "$RELEASE_BODY" > "$NOTES_FILE"
    ASSETS=(
        "dist/ae-$VERSION-darwin-arm64.tar.gz"
        "dist/ae-$VERSION-linux-x86_64-musl.tar.gz"
        dist/SHA256SUMS
    )
    for asset in "${ASSETS[@]}"; do
        [ -f "$asset" ] || { echo "Error: $asset is missing — just bundles did not produce it" >&2; exit 1; }
    done
    # A re-run after a partial failure meets an existing release object. Upload
    # into it rather than refusing — the shape the publish job used, for the
    # same reason.
    if gh release view "$TAG" -R "$REPO" >/dev/null 2>&1; then
        gh release upload "$TAG" "${ASSETS[@]}" --clobber -R "$REPO"
    else
        gh release create "$TAG" "${ASSETS[@]}" \
            -R "$REPO" \
            --title "ae $VERSION" \
            --notes-file "$NOTES_FILE"
    fi

    echo "Released $TAG"

# ── Bundle ───────────────────────────────────────────────────────────

# Build one release bundle: ae-core + install + SHA256SUMS, tarred as
# ae-<version>-<platform>.tar.gz in the current directory.
#
# ONE SPELLING, and that is the whole point of the recipe. The release workflow
# calls this rather than open-coding the same cp/chmod/tar on each platform leg,
# so a change to what a bundle IS cannot land on one platform and miss the
# other — and the shape is runnable on a laptop, where the suites pin it.
#
# The bundle's SHA256SUMS covers its two EXECUTABLE members by bare basename.
# It is a different file from the release-level SHA256SUMS, which covers the
# tarballs: this one is copied verbatim into ~/.ae/versions/<v>/ at install time
# and is what the installed core validates itself against.
bundle version platform binary:
    #!/usr/bin/env bash
    set -euo pipefail
    version="{{ version }}"
    platform="{{ platform }}"
    binary="{{ binary }}"
    [ -x "$binary" ] || { echo "Error: $binary is not an executable core" >&2; exit 1; }
    # THE MEMBER MUST BE THE VERSION THE BUNDLE CLAIMS, and the strong proof is
    # to ask it. That only works when this kernel can exec it: `just bundles`
    # cross-builds the Linux half on an Apple Silicon laptop, where the musl ELF
    # will not run. So the proof is asked of a NATIVE member, and degraded — out
    # loud, never silently — to a byte search for the version string in a
    # foreign one. Running the cross-built core is the Linux CI lane's job
    # (.github/workflows/rust.yml runs it, version-checks it, proves it static,
    # and resolves a real name through it); this recipe never pretends to have
    # done that here.
    case "$(uname -s):$(uname -m)" in
        Darwin:arm64) host_platform=darwin-arm64 ;;
        Linux:x86_64) host_platform=linux-x86_64-musl ;;
        *) host_platform= ;;
    esac
    want="ae $version"
    if [ "$platform" = "$host_platform" ]; then
        got="$("$binary" --version)"
        [ "$got" = "$want" ] || { echo "Error: --version printed '$got', want '$want'" >&2; exit 1; }
    else
        LC_ALL=C grep -qa -- "$version" "$binary" ||
            { echo "Error: $binary carries no '$version' string — it is not a $version core" >&2; exit 1; }
        echo "==> $platform is foreign to this host: version proven by byte search, not by running it"
    fi
    root="ae-$version-$platform"
    # C83: a bundle root is published 0555, so `rm -rf` on a previous run's
    # directory fails "Permission denied" — a 0555 directory refuses the unlink
    # of its own entries. Make it writable first, the same way the installer
    # does for its own private trees.
    [ ! -e "$root" ] || chmod -R u+w "$root" 2>/dev/null || true
    rm -rf "$root"
    mkdir "$root"
    cp "$binary" "$root/ae-core"
    cp install "$root/install"
    if command -v sha256sum >/dev/null 2>&1; then
        sums() { sha256sum "$@"; }
    else
        sums() { shasum -a 256 "$@"; }
    fi
    # Bare basenames: the manifest is read relative to the directory holding it,
    # in the version directory as well as here, so it must not carry a path.
    ( cd "$root" && sums ae-core install > SHA256SUMS )
    chmod 0555 "$root/ae-core" "$root/install"
    chmod 0444 "$root/SHA256SUMS"
    chmod 0555 "$root"
    tar -czf "$root.tar.gz" "$root"
    echo "==> $root.tar.gz"

# Both release halves, and the release-level SHA256SUMS over them, built HERE.
#
# THIS IS THE RECIPE THAT TAKES GITHUB ACTIONS OFF THE CRITICAL PATH. It emits
# exactly the three files .github/workflows/release.yml used to publish, so a
# release no longer waits on a runner and `install` verifies the same bytes it
# always did.
#
# It does NOT restate what a bundle is. `just bundle` remains the one
# definition, called once per platform — the same two calls the two workflow
# legs make — so a change to a bundle's shape still cannot land on one platform
# and miss the other.
#
# Apple Silicon only, and refused elsewhere rather than half-served: the
# darwin-arm64 half is the native build and the linux-x86_64-musl half is
# cross-linked against the Homebrew musl toolchain, and a Linux host has no way
# to produce the macOS half at all.
#
# Output lands in `dist/` (gitignored), wiped first — a stale tarball from a
# previous version must never reach a release.

# Build both platform bundles + the release SHA256SUMS into ./dist
bundles:
    #!/usr/bin/env bash
    set -euo pipefail

    case "$(uname -s):$(uname -m)" in
        Darwin:arm64) : ;;
        *)
            echo "Error: just bundles needs an Apple Silicon host — it builds darwin-arm64 natively and cross-links linux-x86_64-musl" >&2
            exit 1
            ;;
    esac

    command -v {{ RUST_MUSL_CC }} >/dev/null 2>&1 || {
        echo "Error: {{ RUST_MUSL_CC }} not found — the linux-x86_64-musl half cannot be linked." >&2
        echo "       brew install messense/macos-cross-toolchains/x86_64-unknown-linux-musl" >&2
        exit 1
    }

    # llvm-readobj comes from the PINNED toolchain's llvm-tools component
    # (rust-toolchain.toml), never from whatever binutils a laptop happens to
    # carry: macOS ships no readelf, and a static proof that depends on the
    # machine is a proof that silently stops running.
    readobj="$(rustc --print sysroot)/lib/rustlib/$(rustc -vV | sed -n 's/^host: //p')/bin/llvm-readobj"
    [ -x "$readobj" ] || { echo "Error: $readobj is missing — rustup component add llvm-tools" >&2; exit 1; }

    version="$(just version)"
    out="dist"
    native_bin="target/release/ae"
    musl_bin="target/{{ RUST_CROSS_TARGET }}/release/ae"

    cargo build --release --locked
    # ring's build script compiles C for the target, so the cross build needs a
    # musl C COMPILER as well as a musl linker. The linker is pinned in
    # .cargo/config.toml; CC_<target> is what the `cc` crate reads, and it is
    # set HERE rather than there because that name does not exist on the Linux
    # CI leg, which builds this same target against musl-tools.
    CC_x86_64_unknown_linux_musl={{ RUST_MUSL_CC }} \
        cargo build --release --locked --target {{ RUST_CROSS_TARGET }}

    # The native half is RUN, which is the whole reason it is the native half.
    echo "==> native: $("$native_bin" --version)"

    # The musl half is proven STATIC. It cannot be run here — that proof, and
    # the musl DNS/NSS proof beside it, stay on the Linux CI leg. What a laptop
    # can prove is the link, and the authoritative signal is the absence of a
    # PT_INTERP segment: no program interpreter means no dynamic loader.
    #
    # Capture FIRST. Piping llvm-readobj straight into grep would turn a broken
    # readobj into "no PT_INTERP found", which is to say into a pass.
    headers="$("$readobj" --program-headers "$musl_bin")"
    if printf '%s\n' "$headers" | grep -q 'PT_INTERP'; then
        echo "Error: the musl binary has a PT_INTERP segment — it is dynamically linked" >&2
        printf '%s\n' "$headers" >&2
        exit 1
    fi
    # Second, independent signal. Rust musl builds are static-pie, which `file`
    # spells "static-pie linked" or "statically linked" depending on version —
    # both contain "static"; a dynamic build says "dynamically".
    described="$(file -b "$musl_bin")"
    case "$described" in
        *dynamically*) echo "Error: file reports a dynamic binary: $described" >&2; exit 1 ;;
        *static*) : ;;
        *) echo "Error: file does not report a static binary: $described" >&2; exit 1 ;;
    esac
    echo "==> musl: static proof ok — $described"

    # C83 again: a bundle root is published 0555, and a 0555 directory refuses
    # the unlink of its own entries, so a previous run's tree is made writable
    # before it is removed.
    [ ! -e "$out" ] || chmod -R u+w "$out" 2>/dev/null || true
    rm -rf "$out"
    mkdir "$out"

    just bundle "$version" darwin-arm64 "$native_bin"
    just bundle "$version" linux-x86_64-musl "$musl_bin"
    for platform in darwin-arm64 linux-x86_64-musl; do
        root="ae-$version-$platform"
        mv "$root.tar.gz" "$out/"
        chmod -R u+w "$root" 2>/dev/null || true
        rm -rf "$root"
    done

    # THE RELEASE-LEVEL MANIFEST, and a different file from the SHA256SUMS
    # inside each bundle: this one covers the two TARBALLS. `install` reads it
    # to discover the latest version and to verify an archive before extracting
    # it, so its bytes are a contract — `<sha256><SP><SP><basename>`, bare
    # basenames, glob order — byte-identical to the `sha256sum -- ae-*.tar.gz`
    # the publish job ran.
    if command -v sha256sum >/dev/null 2>&1; then
        sums() { sha256sum "$@"; }
    else
        sums() { shasum -a 256 "$@"; }
    fi
    ( cd "$out" && sums -- ae-*.tar.gz > SHA256SUMS )

    echo "==> $out/"
    ls -l "$out"

# ── Install ──────────────────────────────────────────────────────────

# Checkout-mode installation through the same door a release install uses.
#
# NOT `./install`: since slice Z4 that script is a fetch-only bootstrap and
# would download a release over the developer's own build. Bundling the local
# binary first keeps `bundle` the ONE spelling of what a bundle is, and the
# checkout then publishes through `_install` exactly as a downloaded one does.
install:
    #!/usr/bin/env bash
    set -euo pipefail
    version="$(just version)"
    case "$(uname -s):$(uname -m)" in
        Darwin:arm64) platform=darwin-arm64 ;;
        Linux:x86_64) platform=linux-x86_64-musl ;;
        *) echo 'Error: unsupported platform' >&2; exit 1 ;;
    esac
    cargo build --release --locked
    just bundle "$version" "$platform" target/release/ae
    ./target/release/ae _install --from "ae-$version-$platform"

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

# ── Rust core lanes (epic #79 / #80) ─────────────────────────────────
# Bash recipes above remain for policy-frozen glue. Everything below is prefixed
# `rust-` and owns core/toolchain work.
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

# The foreign target. musl, not gnu — ae ships a STATIC binary with no host
# runtime dependency, and gnu is not that (see rust-toolchain.toml for the NSS caveat).
# It is LINKED here now, by `just bundles`, which builds the Linux release half
# on this laptop; it is RUN only on the Linux CI leg, which stays the proof of
# record for a musl binary that executes and resolves a real name.

RUST_CROSS_TARGET := "x86_64-unknown-linux-musl"

# The musl C compiler and linker driver, in the spelling the prebuilt Homebrew
# tap publishes (messense/macos-cross-toolchains). ONE name, three readers: the
# linker pin in `.cargo/config.toml`, the `CC_x86_64_unknown_linux_musl` the
# `cc` crate reads when ring compiles C for the target, and the advisory in
# `rust-setup`. `tests/it/gate.rs` refuses drift between this and the config.
#
# It is deliberately NOT installed by `rust-setup`: a cross toolchain is a
# machine dependency of RELEASING, and the bootstrap contract (rustup + just,
# nothing else) covers developing. Ubuntu names the same tool `musl-gcc`, which
# is why the workflow legs override the linker rather than sharing this pin.

RUST_MUSL_CC := "x86_64-unknown-linux-musl-gcc"

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

    # REPORTED, NOT PROVISIONED, and never fatal. The musl cross toolchain is
    # needed by `just bundles` (and so by `just release`) and by nothing else:
    # `just rust-check` and `just rust-build-release` are native lanes. A clone
    # that only develops is complete without it, so this prints the one line
    # that fixes it and moves on. Both spellings are accepted because the tap
    # publishes the triple-prefixed name and the keg carries the short one.
    echo "==> musl cross toolchain (optional — only 'just bundles' links it)"
    if command -v {{ RUST_MUSL_CC }} >/dev/null 2>&1 || command -v x86_64-linux-musl-gcc >/dev/null 2>&1; then
        printf '    ok   %-16s %s\n' "musl cc" "{{ RUST_MUSL_CC }}"
    else
        printf '    --   %-16s absent; just bundles cannot link the Linux half\n' "musl cc"
        printf '         brew install messense/macos-cross-toolchains/x86_64-unknown-linux-musl\n'
    fi

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
# target, so a musl `cargo check` needs a musl C toolchain. This recipe stays
# NATIVE-ONLY regardless: a bare clone must build with rustup and just and
# nothing else, and requiring a cross toolchain of every clone to run a release
# smoke is the wrong trade.
#
# Where the musl target IS linked is `just bundles`, which needs the prebuilt
# Homebrew toolchain ({{ RUST_MUSL_CC }}) and says so when it is missing. Where
# it is RUN stays the Linux CI leg (.github/workflows/rust.yml): a macOS kernel
# cannot exec an ELF binary, so "it links and carries no PT_INTERP" is the whole
# of what a laptop can prove, and the run plus the musl DNS/NSS proof remain
# CI's. {{ RUST_CROSS_TARGET }} is also deny.toml's second graph.

# Release build: native binary (the musl half is `just bundles`; its RUN proof is Linux CI)
rust-build-release:
    cargo build --release --locked
    @echo "==> native binary: target/release/ae"
    @./target/release/ae --version

# bacon is deliberately NOT installed by rust-setup: a personal dev loop is not
# part of the bootstrap contract.

# Optional watch loop (requires bacon, installed separately)
rust-watch:
    bacon clippy-all
