#!/usr/bin/env bash
# AI-driven e2e scenario runner for ae.
#
#   tests/e2e/ai/run_scenario.sh scenarios/smoke/<name>/   # one scenario
#   tests/e2e/ai/run_scenario.sh scenarios/smoke/          # every scenario under
#
# A scenario is a DIRECTORY containing:
#   - scenario.md : flat key:value frontmatter (name, timeout, requires, config)
#                   + exactly one fenced ```ini block — the ae config, written
#                   VERBATIM to an isolated CONFIG_FILE (this is how you set the ae
#                   configuration PER scenario). `config: default` opts out of the
#                   block and uses ae's default config instead.
#   - steps.sh    : the executable driver — a REAL, lintable bash file (not a
#                   markdown code block) using lib.sh helpers. Run with lib.sh
#                   sourced and the isolated env exported; cwd is the throwaway repo.
#
# The driver is plain bash (NOT an AI): it starts real agents as SUBJECTS and
# asserts on ae's observable state (events.jsonl, session liveness). An optional AI
# judge rules only on soft semantics, reported as advisory. See AGENTS.md.
#
# Opt-in: needs AE_E2E_AI=1 (these run real agents against your real subscription).
set -uo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")" && pwd)"
REPO_ROOT="$(cd "$SELF_DIR/../../.." && pwd)"
LIB="$SELF_DIR/lib.sh"
export AE_E2E_BIN="$REPO_ROOT/ae" # the ae under test; ae_e2e() always uses this

# shellcheck source=tests/e2e/ai/lib.sh
. "$LIB"

# Body of the FIRST fenced ```<lang> ... ``` block (used only for the ini config).
_extract_block() {
    awk -v lang="$1" '
        !done && $0 == "```" lang { infence = 1; next }
        infence && $0 == "```"    { infence = 0; done = 1; next }
        infence                   { print }
    ' "$2"
}

# Count opening fences of a given language (to reject ambiguous multi-config files).
_count_blocks() { grep -cx "\`\`\`$1" "$2" 2>/dev/null || true; }

# Flat frontmatter key (between the first pair of --- lines).
_frontmatter_value() {
    awk 'NR==1 && $0=="---"{f=1; next} f && $0=="---"{exit} f{print}' "$2" |
        sed -n "s/^$1:[[:space:]]*//p" | head -1
}

run_one() {
    local dir="${1%/}"
    if [[ ! -d "$dir" ]]; then
        echo "FAIL: $1 — not a directory" >&2
        return 2
    fi
    # Resolve to an ABSOLUTE path: steps.sh is sourced AFTER cd'ing into the
    # throwaway repo, so a relative path would fail to source — and (counters at
    # zero) would otherwise look green.
    dir="$(cd "$dir" && pwd)" || {
        echo "FAIL: $1 — cannot resolve path" >&2
        return 2
    }
    local md="$dir/scenario.md" steps="$dir/steps.sh"
    [[ -f "$md" ]] || {
        echo "FAIL: $dir — no scenario.md" >&2
        return 2
    }
    [[ -f "$steps" ]] || {
        echo "FAIL: $dir — no steps.sh (the driver must be a real lintable file)" >&2
        return 2
    }

    local name timeout requires config_mode ini nblocks
    name="$(_frontmatter_value name "$md")"
    name="${name:-$(basename "$dir")}"
    timeout="$(_frontmatter_value timeout "$md")"
    timeout="${timeout:-180}"
    requires="$(_frontmatter_value requires "$md")"
    config_mode="$(_frontmatter_value config "$md")"

    nblocks="$(_count_blocks ini "$md")"
    if [[ "$config_mode" == "default" ]]; then
        ini="" # use ae's default config
    elif [[ "$nblocks" == "1" ]]; then
        ini="$(_extract_block ini "$md")"
    else
        echo "FAIL: $name — expected exactly one \`\`\`ini config block (found $nblocks);" >&2
        echo "      or set 'config: default' in the frontmatter to use the default config." >&2
        return 2
    fi

    e2e_require_gate
    # shellcheck disable=SC2086 # intentional word-split of a comma/space list
    [[ -n "$requires" ]] && e2e_require_tools ${requires//,/ }

    e2e_setup "$name"
    if [[ -n "$ini" ]]; then
        e2e_write_config "$ini"
        chmod 600 "$CONFIG_FILE" 2>/dev/null || true
    fi

    # Run the COMMITTED steps.sh in a child under a hard timeout: lib.sh sourced
    # for helpers, isolated env exported, counters fresh, cwd = throwaway repo. Its
    # e2e_summary exit code is the verdict. The parent's EXIT trap (from e2e_setup)
    # captures artifacts + tears the workspace down regardless.
    local rc=0
    # The single-quoted bash -c body is intentional: $AE_E2E_* / $E2E_REPO expand
    # in the CHILD from the exported env above, not here.
    # shellcheck disable=SC2016
    AE_HOME="$AE_HOME" AE_TMUX_SERVER="$AE_TMUX_SERVER" CONFIG_FILE="$CONFIG_FILE" \
        AE_E2E_BIN="$AE_E2E_BIN" AE_E2E_LIB="$LIB" AE_E2E_STEPS="$steps" \
        E2E_REPO="$E2E_REPO" E2E_ARTIFACTS="$E2E_ARTIFACTS" E2E_NAME="$E2E_NAME" \
        E2E_ROOT="$E2E_ROOT" \
        timeout --signal=TERM "$timeout" bash -c '
            set -uo pipefail
            . "$AE_E2E_LIB"
            E2E_PASS=0; E2E_FAIL=0; E2E_JUDGE_PASS=0; E2E_JUDGE_FAIL=0; E2E_INCONCLUSIVE=0
            cd "$E2E_REPO" || exit 3
            . "$AE_E2E_STEPS" || { echo "  FAIL: could not source steps.sh ($AE_E2E_STEPS)" >&2; exit 2; }
            e2e_summary
        ' || rc=$?
    ((rc == 124)) && echo "  FAIL: $name — TIMED OUT after ${timeout}s"
    return "$rc"
}

# A scenario dir has scenario.md; a parent dir holds scenario dirs.
_is_scenario() { [[ -f "$1/scenario.md" ]]; }

main() {
    if [[ $# -eq 0 ]]; then
        echo "usage: run_scenario.sh <scenario-dir | parent-dir> ..." >&2
        exit 2
    fi
    local target rc d
    local ran=0 ok=0 fails=0 incon=0 skipped=0
    _account() { # $1 = a scenario's exit code
        ran=$((ran + 1))
        case "$1" in
            0) ok=$((ok + 1)) ;;
            "$E2E_SKIP") skipped=$((skipped + 1)) ;;
            "$E2E_RC_INCONCLUSIVE") incon=$((incon + 1)) ;;
            *) fails=$((fails + 1)) ;;
        esac
    }
    for target in "$@"; do
        target="${target%/}"
        if _is_scenario "$target"; then
            rc=0
            (run_one "$target") || rc=$?
            _account "$rc"
        elif [[ -d "$target" ]]; then
            local found=0
            while IFS= read -r d; do
                found=1
                rc=0
                (run_one "$d") || rc=$?
                _account "$rc"
            done < <(find "$target" -mindepth 1 -name scenario.md -printf '%h\n' | sort -u)
            if ((found == 0)); then
                echo "FAIL: no scenario.md found under '$target'" >&2
                _account 2
            fi
        else
            echo "FAIL: no scenario at '$target'" >&2
            _account 2
        fi
    done
    echo "── $ran scenario(s): $ok ok, $fails failed, $incon inconclusive, $skipped skipped"
    # Exit-code contract: a real failure wins; an inconclusive (flaky setup) is a
    # distinct non-zero; an all-skipped run reports skipped (so an accidental
    # non-opt-in run is NOT mistaken for green); a mix of ok+skipped is success.
    if ((fails > 0)); then
        return 1
    elif ((incon > 0)); then
        return "$E2E_RC_INCONCLUSIVE"
    elif ((ran > 0 && skipped == ran)); then
        return "$E2E_SKIP"
    fi
    return 0
}

main "$@"
