#!/opt/homebrew/bin/bash
# L-PURGE fixture production. Every fixture is a REAL archive produced by a REAL
# frozen end; the arm's named mutation is applied to it afterwards and diffed.
set -uo pipefail
source /tmp/aelx/lib/arm.sh

PT_CUT=""
pt_cut_cb() { # <barrier-key> <tag>
    local k="$1" tag="$2"
    case "$k" in
      "$PT_CUT".*)
        l_manifest "$R/h/.ae" "$R/cap/template-cut.aehome.tsv"
        l_manifest "$R/h/.ae/archive" "$R/cap/template-cut.archive.tsv"
        { printf 'template.cut.barrier\t%s\n' "$k"
          printf 'template.cut.method\tSIGKILL to the whole end process tree\n'; } >"$R/cap/template-cut.txt"
        l_killtree "$AE_BG_PID"
        ;;
    esac
    return 0
}

# Produce, IN THIS ARM'S OWN SANDBOX, a real archive whose source session is still on
# disk: a real `ae end` cut at <barrier>. Cutting at b_pre_cleanup leaves the archive
# published, the publisher's claim already released and the session directory intact;
# cutting at b_post_rename leaves the claim standing as well.
# Sets: PG_UUID. Requires l_arm_begin to have run with the instrumented binary.
purge_template() { # <barrier> <session>
    local barrier="$1" sess="$2"
    PT_CUT="$barrier"
    l_config "$R" claude
    { l_mkrepo "$R"; } >/dev/null 2>&1
    HOOKS=""; BLOCK=""; l_arm_env
    AE_CWD="$R/w"; WDIR="$R/w"
    l_ae 0launch --local "$sess"
    sleep 2
    l_arm_preflight "$sess" || { printf 'PREFLIGHT-FAILED\n' >"$R/cap/ARM-INVALID.txt"; return 1; }
    PG_UUID="$(grep '^session_id=' "$R/h/.ae/sessions/$sess/meta" | head -1 | cut -d= -f2-)"
    cp "$R/h/.ae/sessions/$sess/meta" "$R/cap/template.session-meta.txt"
    HOOKS=b_stop_local:b_stop_git:b_git_fixed:b_stage_mid:b_pre_rename:b_post_rename:b_pre_cleanup
    BLOCK=1; BLOCK_MAX=1800; l_arm_env
    l_ae_bg 0template-end end -f "$sess"
    l_barriers "$sess" 240 pt_cut_cb || printf 'NOTE: template controller ended by subject death or bound\n' >>"$R/cap/barrier-order.tsv"
    wait "$AE_BG_PID" 2>/dev/null; printf '%s\n' "$?" >"$R/cap/0template-end.rc"
    sleep 1
    l_manifest "$R/h/.ae" "$R/cap/template.aehome.tsv"
    l_manifest "$R/h/.ae/archive" "$R/cap/template.archive.tsv"
    cp "$R/h/.ae/archive/$PG_UUID/meta" "$R/cap/template.archive-meta.txt" 2>/dev/null
    { printf 'template.session\t%s\n' "$sess"
      printf 'template.uuid\t%s\n' "$PG_UUID"
      printf 'template.cut_barrier\t%s\n' "$barrier"
      printf 'template.archive_present\t%s\n' "$( [[ -d "$R/h/.ae/archive/$PG_UUID" ]] && echo yes || echo no )"
      printf 'template.claim_present\t%s\n' "$( [[ -d "$R/h/.ae/archive/.publishing.$PG_UUID" ]] && echo yes || echo no )"
      printf 'template.session_dir_present\t%s\n' "$( [[ -d "$R/h/.ae/sessions/$sess" ]] && echo yes || echo no )"
      printf 'template.end_rc\t%s\n' "$(cat "$R/cap/0template-end.rc")"
    } >"$R/cap/template.txt"
    [[ -d "$R/h/.ae/archive/$PG_UUID" ]] || { printf 'TEMPLATE-INVALID: no archive at %s\n' "$PG_UUID" >"$R/cap/ARM-INVALID.txt"; return 1; }
    [[ -d "$R/h/.ae/sessions/$sess" ]] || { printf 'TEMPLATE-INVALID: session dir gone\n' >"$R/cap/ARM-INVALID.txt"; return 1; }
    return 0
}

# Record a named mutation with its byte diff.
mutate_record() { # <label> <path> <description> <mutator-fn...>
    local lbl="$1" path="$2" desc="$3"; shift 3
    if [[ -f "$path" ]]; then cp -p "$path" "$R/cap/$lbl.before.txt"; fi
    l_manifest "$(dirname "$path")" "$R/cap/$lbl.dir.before.tsv"
    "$@"
    if [[ -f "$path" ]]; then cp -p "$path" "$R/cap/$lbl.after.txt"; fi
    l_manifest "$(dirname "$path")" "$R/cap/$lbl.dir.after.tsv"
    { [[ -f "$R/cap/$lbl.before.txt" && -f "$R/cap/$lbl.after.txt" ]] && diff -u "$R/cap/$lbl.before.txt" "$R/cap/$lbl.after.txt"
      printf '%s\n' '--- directory manifest diff ---'
      diff -u "$R/cap/$lbl.dir.before.tsv" "$R/cap/$lbl.dir.after.tsv"; } >"$R/cap/$lbl.diff" 2>&1
    { printf 'mutation.label\t%s\n' "$lbl"
      printf 'mutation.target\t%s\n' "$path"
      printf 'mutation.description\t%s\n' "$desc"; } >"$R/cap/$lbl.mutation.txt"
    return 0
}

parmtxt() { # <arm> <ids> <construction> [extra...]
    local arm="$1" ids="$2" con="$3"; shift 3
    { printf 'arm\t%s\nsection\tL-PURGE\n' "$arm"
      printf 'roster_ids\t%s\n' "$ids"
      printf 'fixture\tREAL archive produced in THIS sandbox by a real frozen end cut at a barrier; the arm mutation is applied afterwards and diffed\n'
      printf 'construction\t%s\n' "$con"
      printf 'template_uuid\t%s\n' "${PG_UUID:-<none>}"
      printf 'binary\t%s\n' "$(l_sha "$R/b/ae")"
      local x; for x in "$@"; do printf '%s\n' "$x"; done
    } >"$R/cap/ARM.txt"
}
