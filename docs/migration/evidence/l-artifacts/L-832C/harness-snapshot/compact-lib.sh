#!/opt/homebrew/bin/bash
# L-COMPACT shared helpers.
set -uo pipefail
source /tmp/aelx/lib/arm.sh

CPCHAN=b_cp_resolver_entry:b_cp_reval_after_confirmation:b_cp_reval_after_wait:b_cp_after_answer:b_cp_after_handover:b_cp_pre_relaunch

l_use_v3() { cp /tmp/aelx/instr3/ae "$R/b/ae"; chmod 0755 "$R/b/ae"; }

cp_setup() { # <session>
    local sess="$1"
    l_config "$R" claude
    { l_mkrepo "$R"; } >/dev/null 2>&1
    HOOKS=""; BLOCK=""; l_arm_env
    AE_CWD="$R/w"; WDIR="$R/w"
    l_ae 0launch --local "$sess"
    sleep 3
    l_arm_preflight "$sess" || return 1
    CP_UUID="$(grep '^session_id=' "$R/h/.ae/sessions/$sess/meta" | head -1 | cut -d= -f2-)"
    return 0
}

carmtxt() { # <arm> <ids> <construction> [extra...]
    local arm="$1" ids="$2" con="$3"; shift 3
    { printf 'arm\t%s\nsection\tL-COMPACT\n' "$arm"
      printf 'roster_ids\t%s\n' "$ids"
      printf 'construction\t%s\n' "$con"
      printf 'hook_patch_version\t%s\n' "${PATCHV:-none (frozen, unmodified)}"
      printf 'binary.sha256\t%s\n' "$(l_sha "$R/b/ae")"
      printf 'session_uuid\t%s\n' "${CP_UUID:-<none>}"
      local x; for x in "$@"; do printf '%s\n' "$x"; done
    } >"$R/cap/ARM.txt"
}

csnap() { # <label>
    local l="$1"
    l_manifest "$R/h/.ae" "$R/cap/$l.aehome.tsv"
    l_manifest "$R/h/.ae/sessions" "$R/cap/$l.sessions.tsv"
    l_manifest "$R/h/.ae/archive" "$R/cap/$l.archive.tsv"
    l_tmuxsnap "$SOCK" "$R/cap/$l.tmux.txt"
    local f
    for f in "$R"/h/.ae/sessions/*/events.jsonl; do
        [[ -e "$f" ]] || continue
        cp "$f" "$R/cap/$l.events.$(basename "$(dirname "$f")").jsonl"
    done
    return 0
}

# Byte-exact stream captures, both streams separately, plus od dumps.
cbytes() { # <prefix>
    local p="$1"
    od -c "$R/cap/$p.stdout" >"$R/cap/$p.stdout.od" 2>&1
    od -c "$R/cap/$p.stderr" >"$R/cap/$p.stderr.od" 2>&1
    { printf 'stdout.bytes\t%s\n' "$(stat -f '%z' "$R/cap/$p.stdout" 2>/dev/null || echo -)"
      printf 'stderr.bytes\t%s\n' "$(stat -f '%z' "$R/cap/$p.stderr" 2>/dev/null || echo -)"
      printf 'stdout.sha256\t%s\n' "$(l_sha "$R/cap/$p.stdout")"
      printf 'stderr.sha256\t%s\n' "$(l_sha "$R/cap/$p.stderr")"; } >"$R/cap/$p.stream-sizes.txt"
    return 0
}

# Barrier callback: snapshot BOTH streams as they stand at this cut, plus state.
CP_OBSERVE=""
cp_cb() { # <barrier-key> <tag>
    local k="$1" tag="$2"
    cp "$R/cap/2op.stdout" "$R/cap/$tag.stdout-at-cut" 2>/dev/null || : >"$R/cap/$tag.stdout-at-cut"
    cp "$R/cap/2op.stderr" "$R/cap/$tag.stderr-at-cut" 2>/dev/null || : >"$R/cap/$tag.stderr-at-cut"
    od -c "$R/cap/$tag.stdout-at-cut" >"$R/cap/$tag.stdout-at-cut.od" 2>&1
    l_manifest "$R/h/.ae/sessions" "$R/cap/$tag.sessions.tsv"
    l_manifest "$R/h/.ae/archive" "$R/cap/$tag.archive.tsv"
    if [[ -n "$CP_OBSERVE" ]]; then "$CP_OBSERVE" "$k" "$tag"; fi
    return 0
}
