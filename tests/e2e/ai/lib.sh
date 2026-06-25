#!/usr/bin/env bash
# Shared library for ae's AI-driven e2e scenarios.
#
# These tests run REAL agent CLIs (claude/codex) as the SUBJECTS inside ae
# sessions and assert on what ae observably did (events.jsonl, session liveness,
# meta). The driver is this plain bash — not an AI. An optional AI "judge" is used
# ONLY for soft semantic checks. See AGENTS.md for the philosophy.
#
# Isolation (built on the AE_HOME foundation): we keep the REAL $HOME so the agent
# CLIs use your real ~/.claude / ~/.codex auth, and relocate ALL ae state via
# AE_HOME + a private tmux server + a throwaway git repo. Nothing touches your live
# ~/.ae or live tmux. No container, no credential copying.
#
# Sourced by run_scenario.sh; its helpers are then available to a scenario's
# steps. Not executed directly.

# ── gating: these cost real tokens + run real agents; opt-in only ───────────
E2E_SKIP=77            # runner treats this as "skipped" (gate off / tool absent)
E2E_RC_INCONCLUSIVE=75 # could not evaluate (e.g. launch/auth failed) — NOT success

e2e_require_gate() {
    if [[ "${AE_E2E_AI:-}" != "1" ]]; then
        echo "SKIP: AI e2e is opt-in. Set AE_E2E_AI=1 (runs REAL agents against your"
        echo "      real subscription — costs tokens, uses your live rate budget)."
        exit "$E2E_SKIP"
    fi
}

# Skip (not fail) when a required binary is missing — keeps CI/dev machines green.
e2e_require_tools() {
    local t
    for t in "$@"; do
        if ! command -v "$t" >/dev/null 2>&1; then
            echo "SKIP: required tool '$t' not on PATH"
            exit "$E2E_SKIP"
        fi
    done
}

# ── isolated workspace (real $HOME, relocated ae state) ─────────────────────
# Sets + exports: AE_HOME, AE_TMUX_SERVER, CONFIG_FILE. Sets: E2E_ROOT, E2E_REPO,
# E2E_ARTIFACTS, E2E_NAME. Installs an EXIT trap that captures artifacts on
# failure and tears the workspace down.
e2e_setup() {
    E2E_NAME="${1:?scenario name required}"
    E2E_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/ae-e2e-${E2E_NAME}.XXXXXX")"
    export AE_HOME="$E2E_ROOT/ae"
    export AE_TMUX_SERVER="ae-e2e-$$-${E2E_NAME}"
    export CONFIG_FILE="$AE_HOME/config"
    E2E_REPO="$E2E_ROOT/repo"
    mkdir -p "$AE_HOME" "$E2E_REPO"
    git -C "$E2E_REPO" init -q
    git -C "$E2E_REPO" config user.email e2e@ae.test
    git -C "$E2E_REPO" config user.name "ae e2e"
    git -C "$E2E_REPO" commit -q --allow-empty -m "e2e fixture"
    local artifact_base="${AE_E2E_ARTIFACTS:-$PWD/.local/e2e-ai}"
    E2E_ARTIFACTS="$artifact_base/${E2E_NAME}-$$"
    mkdir -p "$E2E_ARTIFACTS"
    chmod 700 "$artifact_base" "$E2E_ARTIFACTS" 2>/dev/null || true
    E2E_PASS=0
    E2E_FAIL=0
    E2E_JUDGE_PASS=0
    E2E_JUDGE_FAIL=0
    E2E_INCONCLUSIVE=0
    trap e2e_teardown EXIT
    echo "e2e: $E2E_NAME"
    echo "  AE_HOME=$AE_HOME  server=$AE_TMUX_SERVER  repo=$E2E_REPO"
    echo "  artifacts=$E2E_ARTIFACTS"
}

# tmux scoped to this scenario's private server.
e2e_tmux() { command tmux -L "$AE_TMUX_SERVER" "$@"; }

# THE way scenarios invoke ae. Always the repo copy under test, always with the
# isolated AE_HOME/AE_TMUX_SERVER/CONFIG_FILE (all exported by e2e_setup +
# run_scenario). Scenarios MUST use this, never bare `ae`, so isolation can't be
# bypassed by accident.
ae_e2e() { "${AE_E2E_BIN:?AE_E2E_BIN unset — run via run_scenario.sh}" "$@"; }

# Write the per-scenario ae config (verbatim INI) to the isolated CONFIG_FILE.
e2e_write_config() { printf '%s\n' "$1" >"$CONFIG_FILE"; }

e2e_events_file() { printf '%s\n' "$AE_HOME/sessions/$1/events.jsonl"; }

# Copy whatever survived into the artifact dir, then destroy the workspace.
e2e_teardown() {
    local rc=$?
    if [[ -n "${AE_HOME:-}" && -d "$AE_HOME/sessions" ]]; then
        cp -r "$AE_HOME/sessions" "$E2E_ARTIFACTS/sessions" 2>/dev/null || true
    fi
    if [[ -n "${AE_TMUX_SERVER:-}" ]]; then
        command tmux -L "$AE_TMUX_SERVER" list-panes -a -F '#{session_name}:#{pane_id}' 2>/dev/null |
            while IFS= read -r p; do
                local sess="${p%%:*}" pid="${p##*:}"
                command tmux -L "$AE_TMUX_SERVER" capture-pane -p -t "$pid" \
                    >"$E2E_ARTIFACTS/pane-${sess}-${pid#%}.txt" 2>/dev/null || true
            done
        command tmux -L "$AE_TMUX_SERVER" kill-server 2>/dev/null || true
    fi
    [[ -n "${E2E_ROOT:-}" ]] && rm -rf "$E2E_ROOT"
    return "$rc"
}

# ── observation predicates (return 0/1) ─────────────────────────────────────
e2e_session_alive() { e2e_tmux has-session -t "$1" 2>/dev/null; }

# True once the session's event log has a line matching the grep -E pattern.
e2e_event_present() {
    local ef
    ef="$(e2e_events_file "$1")"
    [[ -f "$ef" ]] && grep -Eq "$2" "$ef"
}

# Poll until the event appears, or timeout (seconds, default 60). Returns 0/1.
e2e_wait_event() {
    local session="$1" pat="$2" timeout="${3:-60}" waited=0
    while ((waited < timeout)); do
        e2e_event_present "$session" "$pat" && return 0
        sleep 2
        waited=$((waited + 2))
    done
    return 1
}

# Wait for the tmux session to exist (ae launches it asynchronously).
e2e_wait_session() {
    local session="$1" timeout="${2:-15}" waited=0
    while ((waited < timeout)); do
        e2e_session_alive "$session" && return 0
        sleep 1
        waited=$((waited + 1))
    done
    return 1
}

# ── assertions (record pass/fail; never abort so all checks run) ────────────
# Usage: e2e_assert "<label>" <predicate-cmd> [args...]   (pass iff cmd exits 0)
e2e_assert() {
    local label="$1"
    shift
    if "$@"; then
        E2E_PASS=$((E2E_PASS + 1))
        echo "  PASS: $label"
    else
        E2E_FAIL=$((E2E_FAIL + 1))
        echo "  FAIL: $label"
    fi
}
# Negated form: pass iff the predicate exits NON-zero.
e2e_refute() {
    local label="$1"
    shift
    if "$@"; then
        E2E_FAIL=$((E2E_FAIL + 1))
        echo "  FAIL: $label"
    else
        E2E_PASS=$((E2E_PASS + 1))
        echo "  PASS: $label"
    fi
}

# Record a check that could not be evaluated (e.g. the session never started) —
# distinct from a real FAIL so flaky-setup doesn't masquerade as a regression.
e2e_inconclusive() {
    E2E_INCONCLUSIVE=$((E2E_INCONCLUSIVE + 1))
    echo "  INCONCLUSIVE: $1"
}

# ── optional soft AI judge (SEMANTIC, advisory — NOT the gate) ───────────────
# e2e_judge "<label>" "<question>" "<text>" — a one-shot headless agent rules on a
# soft semantic question. Tracked SEPARATELY from mechanics: deterministic asserts
# decide the scenario's exit code; the judge is reported as advisory. Skipped
# (no effect) when no judge CLI is available.
e2e_judge() {
    local label="$1" question="$2" text="$3" out
    if ! command -v claude >/dev/null 2>&1; then
        echo "  SKIP judge ($label): no claude CLI"
        return 0
    fi
    out="$(printf '%s' "$text" | claude -p "$question Answer ONLY compact JSON: {\"pass\":true|false,\"why\":\"...\"}" 2>/dev/null)"
    if printf '%s' "$out" | grep -Eq '"pass"[[:space:]]*:[[:space:]]*true'; then
        E2E_JUDGE_PASS=$((E2E_JUDGE_PASS + 1))
        echo "  judge PASS (semantic): $label"
    else
        E2E_JUDGE_FAIL=$((E2E_JUDGE_FAIL + 1))
        echo "  judge FAIL (semantic, advisory): $label — $out"
    fi
}

# ── final tally ─────────────────────────────────────────────────────────────
# The GATE is deterministic mechanics. The (advisory) judge never flips a green
# mechanics run to red. But an INCONCLUSIVE check (e.g. a session that never
# launched) is NOT success — it returns a distinct non-zero code so a flaky setup
# can't masquerade as a pass.
e2e_summary() {
    echo "  mechanics: ${E2E_PASS} passed, ${E2E_FAIL} failed" \
        "| judge: ${E2E_JUDGE_PASS}/${E2E_JUDGE_FAIL} (advisory)" \
        "| inconclusive: ${E2E_INCONCLUSIVE}"
    if ((E2E_FAIL > 0)); then
        return 1
    elif ((E2E_INCONCLUSIVE > 0)); then
        return "$E2E_RC_INCONCLUSIVE"
    fi
    return 0
}
