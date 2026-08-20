#!/opt/homebrew/bin/bash
# L-FROM shared helpers.
set -uo pipefail
source /tmp/aelx/lib/arm.sh

l_use_v3() { cp /tmp/aelx/instr3/ae "$R/b/ae"; chmod 0755 "$R/b/ae"; }

fsnap() { # <label>
    local l="$1"
    l_manifest "$R/h/.ae" "$R/cap/$l.aehome.tsv"
    l_manifest "$R/h/.ae/sessions" "$R/cap/$l.sessions.tsv"
    l_manifest "$R/h/.ae/archive" "$R/cap/$l.archive.tsv"
    l_manifest "$R/h/.ae/worktrees" "$R/cap/$l.worktrees.tsv"
    l_tmuxsnap "$SOCK" "$R/cap/$l.tmux.txt"
    return 0
}

# A FULL sweep: session dirs, worktrees, tmux sessions, archive — everything a
# launch could have created, listed whether or not it exists.
full_sweep() { # <label>
    local l="$1"
    { printf '## session dirs\n'; ls -1a "$R/h/.ae/sessions" 2>&1
      printf '\n## worktree dirs\n'; ls -1a "$R/h/.ae/worktrees" 2>&1
      printf '\n## archive dirs\n'; ls -1a "$R/h/.ae/archive" 2>&1
      printf '\n## tmux sessions\n'; /opt/homebrew/bin/tmux -S "$SOCK" list-sessions -F '#{session_id}|#{session_name}' 2>&1
      printf '\n## AE_HOME top level\n'; ls -1a "$R/h/.ae" 2>&1
      printf '\n## work dir\n'; ls -1a "$R/w" 2>&1
    } >"$R/cap/$l.full-sweep.txt" 2>&1
    return 0
}

lineage_fields() { # <session> <out>
    local s="$1" out="$2"
    local m="$R/h/.ae/sessions/$s/meta"
    { printf '# lineage fields BY EXACT KEY from %s\n' "$m"
      if [[ -f "$m" ]]; then
        grep -n '^session_id=\|^session_id_origin=\|^parent_archive_id=\|^parent_archive_handover_count=\|^parent_archive_pending_count=\|^session=\|^origin=' "$m" 2>&1
        printf '\n# the whole meta, verbatim\n'; cat "$m"
      else
        printf '(no meta at that path)\n'
      fi
      printf '\n# workspace.md lineage lines\n'
      local w="$R/h/.ae/sessions/$s/workspace.md"
      if [[ -f "$w" ]]; then grep -n -i 'archive\|lineage\|--from\|continues\|parent' "$w" 2>&1 || printf '(no matching line)\n'
      else printf '(no workspace.md)\n'; fi
    } >"$out" 2>&1
    return 0
}

farmtxt() { # <arm> <ids> <construction> [extra...]
    local arm="$1" ids="$2" con="$3"; shift 3
    { printf 'arm\t%s\nsection\tL-FROM\n' "$arm"
      printf 'roster_ids\t%s\n' "$ids"
      printf 'construction\t%s\n' "$con"
      printf 'hook_patch_version\t%s\n' "${PATCHV:-none (frozen, unmodified)}"
      printf 'binary.sha256\t%s\n' "$(l_sha "$R/b/ae")"
      local x; for x in "$@"; do printf '%s\n' "$x"; done
    } >"$R/cap/ARM.txt"
}

# Produce a real parent archive by a real end. Sets PARENT_UUID, PARENT_NAME.
make_parent() { # <name>
    PARENT_NAME="$1"
    l_config "$R" claude
    { l_mkrepo "$R"; } >/dev/null 2>&1
    HOOKS=""; BLOCK=""; l_arm_env
    AE_CWD="$R/w"; WDIR="$R/w"
    l_ae 0launch-parent --local "$PARENT_NAME"
    sleep 3
    l_arm_preflight "$PARENT_NAME" || return 1
    PARENT_UUID="$(grep '^session_id=' "$R/h/.ae/sessions/$PARENT_NAME/meta" | head -1 | cut -d= -f2-)"
    l_ae 0end-parent end -f "$PARENT_NAME"
    sleep 1
    [[ -d "$R/h/.ae/archive/$PARENT_UUID" ]] || return 1
    cp -p "$R/h/.ae/archive/$PARENT_UUID/meta" "$R/cap/parent-archive-meta.txt"
    printf 'parent.name\t%s\nparent.uuid\t%s\n' "$PARENT_NAME" "$PARENT_UUID" >"$R/cap/parent.txt"
    return 0
}
