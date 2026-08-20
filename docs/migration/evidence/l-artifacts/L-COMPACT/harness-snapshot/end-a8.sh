#!/opt/homebrew/bin/bash
# L-END arm: launch-rerun (SC-811a, SC-811b, SC-812).
set -uo pipefail
source /tmp/aelx/lib/arm.sh

l_arm_begin L-END launch-rerun frozen
l_config "$R" claude
{ l_mkrepo "$R"; } >/dev/null 2>&1
HOOKS=""; BLOCK=""; l_arm_env
AE_CWD="$R/w"; WDIR="$R/w"
l_ae 1launch --local lr1
sleep 2
l_arm_preflight lr1 || { printf 'PREFLIGHT-FAILED\n' >"$R/cap/ARM-INVALID.txt"; l_arm_end; exit 1; }
META="$R/h/.ae/sessions/lr1"
SCRIPT="$META/launch.main.sh"
MARKER="$META/launch.main.started"

snapmark() { # <label>
    { printf 'label\t%s\n' "$1"
      printf 'script.exists\t%s\n' "$( [[ -e "$SCRIPT" ]] && echo yes || echo no )"
      printf 'script.mode\t%s\n' "$(stat -f '%Lp' "$SCRIPT" 2>/dev/null || echo '-')"
      printf 'script.sha256\t%s\n' "$(l_sha "$SCRIPT")"
      printf 'marker.exists\t%s\n' "$( [[ -e "$MARKER" ]] && echo yes || echo no )"
      printf 'marker.size\t%s\n' "$(stat -f '%z' "$MARKER" 2>/dev/null || echo '-')"
      printf 'marker.mtime\t%s\n' "$(stat -f '%m' "$MARKER" 2>/dev/null || echo '-')"
      printf 'fake.invocations.so_far\t%s\n' "$(ls "$R/cap/fake"/*.argv.nul 2>/dev/null | wc -l | tr -d ' ')"
    } >>"$R/cap/marker-timeline.txt"
    cp "$SCRIPT" "$R/cap/launch.main.sh.$1" 2>/dev/null
    return 0
}

: >"$R/cap/marker-timeline.txt"
snapmark 0after-launch
FAKELOG="$R/cap/fake"

# ── run the launch script in a control pane, twice ────────────────────────
run_script() { # <label>
    local lbl="$1"
    local before; before="$(ls "$FAKELOG" 2>/dev/null | wc -l | tr -d ' ')"
    local envq="" v; for v in "${AE_ENV[@]}"; do envq+=" $(printf '%q' "$v")"; done
    l_ctl kill-server 2>/dev/null
    l_ctl new-session -d -s drv -x 200 -y 50 "$L_BASH" -c \
        "cd $(printf '%q' "$R/w"); env -i${envq} $(printf '%q' "$SCRIPT") 2> $(printf '%q' "$R/cap/$lbl.script.stderr"); echo \$? > $(printf '%q' "$R/cap/$lbl.script.rc"); sleep 3600"
    # bounded POSITIVE barrier: the fake logs one artifact set per invocation
    local i=0 ok=0
    while (( i < 400 )); do
        local now; now="$(ls "$FAKELOG" 2>/dev/null | wc -l | tr -d ' ')"
        (( now > before )) && { ok=1; break; }
        sleep 0.1; i=$((i+1))
    done
    sleep 1
    l_ctl list-panes -a -F '#{pane_id}|#{pane_pid}|#{pane_current_command}|#{pane_dead}' >"$R/cap/$lbl.pane_current_command.txt" 2>&1
    l_ctl capture-pane -p -t drv >"$R/cap/$lbl.pane.txt" 2>&1
    ps -ax -o pid=,ppid=,command= | grep -F "$R" | grep -v '[g]rep' >"$R/cap/$lbl.ps.txt" 2>&1
    (( ok == 0 )) && printf 'INCONCLUSIVE: no new fake-tool invocation artifact within the 40s bound (%s)\n' "$lbl" >>"$R/cap/INCONCLUSIVE.txt"
    snapmark "$lbl"
    return 0
}

run_script 1first-run
l_ctl kill-server 2>/dev/null
sleep 1
snapmark 2after-first-run-pane-killed
run_script 3second-run
l_ctl kill-server 2>/dev/null
sleep 1
snapmark 4after-second-run-pane-killed

# argv of BOTH runs, verbatim (NUL-separated), from the fake's own log
{ for f in "$FAKELOG"/*.argv.nul; do
    [[ -e "$f" ]] || continue
    printf '=== %s ===\n' "$f"
    tr '\0' '\n' <"$f"
  done
  printf '\n=== index ===\n'; cat "$FAKELOG/index.txt" 2>/dev/null
} >"$R/cap/fake-argv-both-runs.txt" 2>&1

# ── ae rewrites the script: stop, then resume ─────────────────────────────
l_ae 5stop stop -y lr1
sleep 1
snapmark 5after-stop
l_ae 6resume --local lr1
sleep 3
snapmark 6after-resume-rewrite
diff -u "$R/cap/launch.main.sh.0after-launch" "$R/cap/launch.main.sh.6after-resume-rewrite" >"$R/cap/launch-script.rewrite.diff" 2>&1

l_snap 7post
{ printf 'arm\tlaunch-rerun\nsection\tL-END\n'
  printf 'roster_ids\tSC-811a SC-811b SC-812\n'
  printf 'fixture\t--local family, renamed-interpreter fake tool (a copy of bash named claude)\n'
  printf 'construction\tae'"'"'s OWN launch executes the generated launch.main.sh (frozen ae:12606), so execution 1 is ae'"'"'s; the controller then executes the SAME script directly in a control pane (execution 2), kills that pane, and executes it once more (execution 3); afterwards ae is made to rewrite the script by stop + resume (which executes it again, execution 4)\n'
  printf 'execution_ledger\tmarker-timeline.txt records fake.invocations.so_far at every label; fake-argv-both-runs.txt holds every execution'"'"'s argv verbatim (NUL-separated), index.txt maps pid->start time\n'
  printf 'script_path\t%s\n' "$SCRIPT"
  printf 'marker_path\t%s\n' "$MARKER"
  printf 'launch_rc\t%s\n' "$(cat "$R/cap/1launch.rc")"
  printf 'stop_rc\t%s\n' "$(cat "$R/cap/5stop.rc")"
  printf 'resume_rc\t%s\n' "$(cat "$R/cap/6resume.rc")"
  printf 'fake_invocation_bound_sec\t40\n'
} >"$R/cap/ARM.txt"
l_arm_end
echo DONE
