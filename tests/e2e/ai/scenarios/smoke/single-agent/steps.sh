#!/usr/bin/env bash
# Driver for the single-agent smoke scenario.
#
# Sourced by run_scenario.sh with lib.sh helpers in scope and the isolated env
# (AE_HOME / AE_TMUX_SERVER / CONFIG_FILE) exported; cwd is the throwaway repo.
# Use ae_e2e (never bare `ae`) so the isolated workspace can't be bypassed.
#
# shellcheck shell=bash
# shellcheck disable=SC2154  # E2E_* / AE_* provided by lib.sh + run_scenario.sh

# Sourced by run_scenario.sh (it provides the helpers + isolated env). Running it
# directly does nothing useful and would have no isolation — refuse.
[[ "${BASH_SOURCE[0]}" != "${0}" ]] || {
    echo "run via tests/e2e/ai/run_scenario.sh, not directly" >&2
    exit 2
}

session="smoke1"

# Launch a real agent in the background (it blocks attaching; we observe out-of-band).
ae_e2e --local "$session" >/dev/null 2>&1 &

if ! e2e_wait_session "$session" 20; then
    e2e_inconclusive "session '$session' never started (launch/auth problem?)"
    return 0 # sourced by run_scenario.sh
fi
e2e_assert "session is alive" e2e_session_alive "$session"

# ae captures the agent's real session id shortly after launch; poll the meta.
meta="$AE_HOME/sessions/$session/meta"
captured=1
for _ in $(seq 1 30); do
    if grep -Eq '^agent\.main=claude:main:[0-9a-fA-F-]{8,}' "$meta" 2>/dev/null; then
        captured=0
        break
    fi
    sleep 2
done
e2e_assert "real Claude session id captured in meta (a real process launched)" \
    test "$captured" -eq 0

# Advisory semantic check: does the agent answer a trivial prompt? Pure judge —
# never gates the run. We give it a prompt via the agent's own send helper.
"$AE_HOME/sessions/$session/send" "@${session}:claude:main" \
    "Reply with exactly the word: READY" >/dev/null 2>&1 || true
sleep 25
pane="$(e2e_tmux capture-pane -p -t "$session" 2>/dev/null || true)"
e2e_judge "agent responded to a trivial prompt" \
    "Below is a terminal pane of a coding agent that was asked to reply with the word READY. Did it respond (any acknowledgement counts)?" \
    "$pane"
