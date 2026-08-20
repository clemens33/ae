#!/opt/homebrew/bin/bash
# L-END arm: history-policy (managed). CLI flag x [workspace]
# purge_agent_history x unset — one CELL per cross, each on its own clone.
set -uo pipefail
source /tmp/aelx/lib/arm.sh

cell() { # <cell-id> <cli: none|purge|keep> <cfg: unset|true|false>
    local cid="$1" cli="$2" cfg="$3"
    local arm="history-policy-$cid"
    l_arm_begin L-END "$arm" frozen
    local -a cfgextra=()
    case "$cfg" in
        true)  cfgextra=("purge_agent_history = true") ;;
        false) cfgextra=("purge_agent_history = false") ;;
    esac
    l_config "$R" claude ${cfgextra[@]+"${cfgextra[@]}"}
    { l_mkrepo "$R"; } >/dev/null 2>&1
    HOOKS=""; BLOCK=""; l_arm_env
    AE_CWD="$R/w"; WDIR="$R/w"
    l_ae 1launch1 --local "hp1"
    sleep 2
    l_ae 1launch2 --local "hp2"
    sleep 2
    l_arm_preflight hp1 || { printf 'PREFLIGHT-FAILED\n' >"$R/cap/ARM-INVALID.txt"; l_arm_end; return 1; }
    # Controller-planted conversation files at the EXACT path the frozen locator
    # globs ($HOME/.claude/projects/*/<uuid>.jsonl). The frozen locator matches on
    # PATH only and never reads the content, so nothing is fabricated about content.
    local projd="$R/h/.claude/projects/-tmp-aelx-w"
    mkdir -p "$projd"
    local s u
    { printf 'planted conversation files (path-only marker; content never read by the frozen locator)\n'
      for s in hp1 hp2; do
        while IFS= read -r line; do
            case "$line" in agent.*) u="${line##*:}"; [[ "$u" == pending ]] && continue
                printf '{"planted":"batch-l","session":"%s","uuid":"%s"}\n' "$s" "$u" >"$projd/$u.jsonl"
                printf '  %s\t%s\t%s\n' "$s" "$u" "$projd/$u.jsonl" ;;
            esac
        done <"$R/h/.ae/sessions/$s/meta"
      done
    } >"$R/cap/planted-conversations.txt"
    l_manifest "$R/h/.claude" "$R/cap/conversations.before.tsv"
    l_snap 0pre
    local -a flag=()
    case "$cli" in purge) flag=(--purge-history) ;; keep) flag=(--keep-history) ;; esac
    # end all WITHOUT -f, on a real terminal, so the per-target confirmation body
    # and the per-session decision lines are both captured.
    l_pane_start 2endall "$R/b/ae" end ${flag[@]+"${flag[@]}"} all
    if l_pane_wait 60 "Continue?"; then
        l_pane_capture at-prompt
        l_pane_send y; l_pane_enter
    else
        l_pane_capture prompt-not-observed
        printf 'INCONCLUSIVE: the confirmation prompt was not observed within the 60s bound\n' >"$R/cap/INCONCLUSIVE.txt"
    fi
    if ! l_pane_wait_rc 120; then
        l_pane_capture rc-not-observed
        printf 'INCONCLUSIVE: no exit status observed within the 120s bound\n' >>"$R/cap/INCONCLUSIVE.txt"
    fi
    sleep 1
    l_pane_capture final
    l_pane_stop
    l_manifest "$R/h/.claude" "$R/cap/conversations.after.tsv"
    diff -u "$R/cap/conversations.before.tsv" "$R/cap/conversations.after.tsv" >"$R/cap/conversations.diff" 2>&1
    l_snap 3post
    { printf 'arm\t%s\nsection\tL-END\n' "$arm"
      printf 'roster_ids\tSC-838a SC-838b\n'
      printf 'fixture\tmanaged-family clone, two --local sessions, end all under a real terminal\n'
      printf 'cell\tcli=%s config.purge_agent_history=%s\n' "$cli" "$cfg"
      printf 'construction\tCLI history flag x [workspace] purge_agent_history, one cell per cross, own clone\n'
      printf 'end_all_rc\t%s\n' "$(cat "$R/cap/2endall.rc" 2>/dev/null || echo '<none>')"
      printf 'prompt_bound_sec\t60\n'; printf 'rc_bound_sec\t120\n'
    } >"$R/cap/ARM.txt"
    l_arm_end
    return 0
}

cell c9-keep-false  keep  false
echo DONE
