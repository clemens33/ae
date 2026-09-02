#!/usr/bin/env bash
# Driver for the cross-agent-ask scenario.
#
# Sourced by run_scenario.sh with lib.sh helpers in scope and the isolated env
# exported; cwd is the throwaway repo. Use ae_e2e, never bare `ae`.
#
# shellcheck shell=bash
# shellcheck disable=SC2154  # E2E_* / AE_* provided by lib.sh + run_scenario.sh

# Sourced by run_scenario.sh (it provides the helpers + isolated env). Running it
# directly does nothing useful and would have no isolation — refuse.
[[ "${BASH_SOURCE[0]}" != "${0}" ]] || {
    echo "run via tests/e2e/ai/run_scenario.sh, not directly" >&2
    exit 2
}

session="smoke2"

ae_e2e --local "$session" >/dev/null 2>&1 &

if ! e2e_wait_session "$session" 25; then
    e2e_inconclusive "session '$session' never started"
    return 0 # sourced by run_scenario.sh
fi

# Both agents recorded in meta (main + the worker).
meta="$AE_HOME/sessions/$session/meta"
e2e_assert "lead agent present in meta" \
    grep -Eq '^seat\.main=lead$' "$meta"
e2e_assert "reviewer agent present in meta" \
    grep -Eq '^seat\.worker\.0=reviewer$' "$meta"

# Give the agents a moment to finish their first-turn setup before we poke them.
sleep 8

# Send a TRACKED ask to the reviewer. An ask from outside an agent pane has no
# caller identity (it would silently degrade to a plain `send`), so we attribute
# it to the lead via AE_SENDER_OVERRIDE — emitting a real ask the reviewer replies to.
AE_SENDER_OVERRIDE="lead" \
    "$AE_HOME/sessions/$session/ask" "reviewer" \
    "Reply with exactly: 2+2=4 — nothing else." >/dev/null 2>&1 || \
    e2e_inconclusive "ask helper invocation failed"

# Deterministic gate: a reply event must round-trip back within the timeout.
if e2e_wait_event "$session" '"action":"reply"' 180; then
    e2e_assert "a reply event round-tripped from the reviewer" true
else
    e2e_assert "a reply event round-tripped from the reviewer" false
fi

# Advisory: did the reply actually answer? Judge the reviewer's recent pane.
pane="$(e2e_tmux capture-pane -p -t "$session" 2>/dev/null || true)"
e2e_judge "the reply answers the question (≈ 2+2=4)" \
    "This is a coding-agent pane that was asked to reply '2+2=4'. Did it convey that the answer is 4?" \
    "$pane"
