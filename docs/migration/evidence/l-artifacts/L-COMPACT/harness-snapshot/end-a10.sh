#!/opt/homebrew/bin/bash
# L-END arm: endall-freeze (SC-820a, SC-821a, SC-821b).
set -uo pipefail
source /tmp/aelx/lib/arm.sh

# ── SC-820a: a target's tmux session is renamed between the confirmation and the lock
RENAMED=""
rename_cb() { # <barrier-key> <tag>
    local k="$1" tag="$2"
    case "$k" in
      b_confirm_answered.*)
        [[ -n "$RENAMED" ]] && return 0
        RENAMED=yes
        l_tmuxsnap "$SOCK" "$R/cap/$tag.tmux.before-rename.txt"
        /opt/homebrew/bin/tmux -S "$SOCK" rename-session -t ef2 ef2-renamed >"$R/cap/$tag.rename.out" 2>&1
        printf 'rename.rc\t%s\n' "$?" >>"$R/cap/$tag.rename.out"
        l_tmuxsnap "$SOCK" "$R/cap/$tag.tmux.after-rename.txt"
        { printf 'controller.action\ttmux rename-session -t ef2 ef2-renamed\n'
          printf 'controller.barrier\t%s\n' "$k"
          printf 'note\tthe on-disk session directory and meta are NOT touched; only the tmux session name\n'
        } >"$R/cap/$tag.controller.txt"
        ;;
    esac
    return 0
}

arm_rename_between() {
    l_arm_begin L-END endall-rename-between-confirm-and-lock instrumented
    l_config "$R" claude
    { l_mkrepo "$R"; } >/dev/null 2>&1
    HOOKS=""; BLOCK=""; l_arm_env
    AE_CWD="$R/w"; WDIR="$R/w"
    local s
    for s in ef1 ef2 ef3; do l_ae "1launch-$s" --local "$s"; sleep 2; done
    l_arm_preflight ef2 || { printf 'PREFLIGHT-FAILED\n' >"$R/cap/ARM-INVALID.txt"; l_arm_end; return 1; }
    l_snap 0pre
    for s in ef1 ef2 ef3; do cp "$R/h/.ae/sessions/$s/meta" "$R/cap/meta.$s.before.txt" 2>/dev/null; done
    HOOKS=b_confirm_answered; BLOCK=1; BLOCK_MAX=1800; l_arm_env
    l_pane_start 2endall "$R/b/ae" end all
    if l_pane_wait 60 "Continue?"; then
        l_pane_capture at-prompt
        l_pane_send y; l_pane_enter
    else
        l_pane_capture prompt-not-observed
        printf 'INCONCLUSIVE: the confirmation prompt was not observed within the 60s bound\n' >>"$R/cap/INCONCLUSIVE.txt"
    fi
    l_barriers_pane 300 "$R/cap/2endall.rc" rename_cb || \
        printf 'INCONCLUSIVE: barrier controller expired (bound 300s)\n' >>"$R/cap/INCONCLUSIVE.txt"
    l_pane_wait_rc 120 || printf 'INCONCLUSIVE: no exit status within the 120s bound\n' >>"$R/cap/INCONCLUSIVE.txt"
    sleep 1
    l_pane_capture final
    l_pane_stop
    l_snap 3post
    od -c "$R/cap/2endall.stdout" >"$R/cap/2endall.stdout.od"
    od -c "$R/cap/2endall.stderr" >"$R/cap/2endall.stderr.od"
    { printf '# tmux state after\n'; l_tmuxsnap "$SOCK" /dev/stdout
      printf '\n# session dirs after\n'; ls -1 "$R/h/.ae/sessions" 2>&1
      printf '\n# archive dirs after\n'; ls -1 "$R/h/.ae/archive" 2>&1; } >"$R/cap/post-state.txt" 2>&1
    { printf 'arm\tendall-rename-between-confirm-and-lock\nsection\tL-END\n'
      printf 'roster_ids\tSC-820a\n'
      printf 'fixture\t--local family, THREE sessions on one server\n'
      printf 'construction\tae end all runs on a real terminal; after the answer is accepted and before the per-target lifecycle lock, the controller renames ONE target tmux session (ef2 -> ef2-renamed) and leaves its on-disk state untouched\n'
      printf 'barrier\tb_confirm_answered (after the confirmation phase, before target dispatch)\n'
      printf 'end_all_rc\t%s\n' "$(cat "$R/cap/2endall.rc" 2>/dev/null || echo '<none>')"
      printf 'prompt_bound_sec\t60\nbarrier_bound_sec\t300\nrc_bound_sec\t120\n'
    } >"$R/cap/ARM.txt"
    l_arm_end
}

# ── SC-821a/b: the frozen plan carries no targets
arm_empty_plan() {
    l_arm_begin L-END endall-empty-plan frozen
    l_config "$R" claude
    { l_mkrepo "$R"; } >/dev/null 2>&1
    HOOKS=""; BLOCK=""; l_arm_env
    AE_CWD="$R/w"; WDIR="$R/w"
    mkdir -p "$R/h/.ae/sessions"
    l_manifest "$R/h/.ae" "$R/cap/0pre.aehome.tsv"
    { printf '# sessions dir before\n'; ls -la "$R/h/.ae/sessions" 2>&1
      printf '\n# worktrees dir before\n'; ls -la "$R/h/.ae/worktrees" 2>&1; } >"$R/cap/0pre.dirs.txt"
    l_pane_start 2endall "$R/b/ae" end all
    if l_pane_wait 60 "Continue?"; then
        l_pane_capture at-prompt
        cp "$R/cap/2endall.pane.at-prompt.txt" "$R/cap/frozen-plan-as-rendered.txt"
        l_pane_send y; l_pane_enter
    else
        l_pane_capture prompt-not-observed
        printf 'INCONCLUSIVE: the confirmation prompt was not observed within the 60s bound\n' >>"$R/cap/INCONCLUSIVE.txt"
    fi
    l_pane_wait_rc 120 || printf 'INCONCLUSIVE: no exit status within the 120s bound\n' >>"$R/cap/INCONCLUSIVE.txt"
    sleep 1
    l_pane_capture final
    l_pane_stop
    od -c "$R/cap/2endall.stdout" >"$R/cap/2endall.stdout.od"
    od -c "$R/cap/2endall.stderr" >"$R/cap/2endall.stderr.od"
    l_manifest "$R/h/.ae" "$R/cap/3post.aehome.tsv"
    { printf 'arm\tendall-empty-plan\nsection\tL-END\n'
      printf 'roster_ids\tSC-821a SC-821b\n'
      printf 'fixture\tan AE_HOME with a config and no sessions at all\n'
      printf 'construction\tae end all runs on a real terminal against a state whose target enumeration yields nothing; the prompt transcript, the plan block as rendered, and the outcome record are captured\n'
      printf 'end_all_rc\t%s\n' "$(cat "$R/cap/2endall.rc" 2>/dev/null || echo '<none>')"
      printf 'prompt_bound_sec\t60\nrc_bound_sec\t120\n'
    } >"$R/cap/ARM.txt"
    l_arm_end
}

arm_rename_between
arm_empty_plan
echo DONE
