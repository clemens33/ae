#!/opt/homebrew/bin/bash
# L-END arms: identity (SC-806a/b), unreachable-server (SC-816),
# hostile symlinked archive root. Capture-only.
set -uo pipefail
source /tmp/aelx/lib/arm.sh

setup_local() { # <arm> <sess>
    local arm="$1" sess="$2"
    l_arm_begin L-END "$arm" frozen
    l_config "$R" claude
    { l_mkrepo "$R"; } >/dev/null 2>&1
    HOOKS=""; BLOCK=""; l_arm_env
    AE_CWD="$R/w"
    WDIR="$R/w"
    l_ae 1launch --local "$sess"
    sleep 2
    l_arm_preflight "$sess" || return 1
    SESS_UUID="$(grep '^session_id=' "$R/h/.ae/sessions/$sess/meta" | head -1 | cut -d= -f2-)"
    return 0
}

armtxt() { local n="$1"; shift; { printf 'arm\t%s\nsection\tL-END\n' "$n"; printf '%s\n' "$@"; } >"$R/cap/ARM.txt"; }

# ─────────────────────────────── identity: same name ended twice (SC-806a)
arm_identity_same_name() {
    setup_local identity-same-name-twice id1 || { l_arm_end; return 1; }
    cp "$R/h/.ae/sessions/id1/meta" "$R/cap/meta.run1.txt"
    local u1="$SESS_UUID"
    l_snap 0pre
    l_ae 2end1 end -f id1
    sleep 1
    l_manifest "$R/h/.ae/archive" "$R/cap/archive.after-end1.tsv"
    # recreate the SAME name
    l_ae 3launch2 --local id1
    sleep 2
    cp "$R/h/.ae/sessions/id1/meta" "$R/cap/meta.run2.txt"
    local u2; u2="$(grep '^session_id=' "$R/h/.ae/sessions/id1/meta" | head -1 | cut -d= -f2-)"
    l_ae 4end2 end -f id1
    sleep 1
    l_manifest "$R/h/.ae/archive" "$R/cap/archive.after-end2.tsv"
    { printf '# archive directory names, in on-disk order\n'
      ls -1 "$R/h/.ae/archive" 2>&1
      printf '\n# id fields by exact key, per archive meta\n'
      for m in "$R"/h/.ae/archive/*/meta; do
        [[ -e "$m" ]] || continue
        printf '=== %s ===\n' "$m"
        grep -n '^archive_id=\|^source_session\|^session_id\|^archive_id_origin=\|^session_id_origin=\|^parent_archive_id=' "$m"
      done
    } >"$R/cap/archive-identity.txt" 2>&1
    l_snap 5post
    armtxt identity-same-name-twice "roster_ids	SC-806a" \
      "fixture	--local family" \
      "construction	a session is ended, the SAME name is launched again, and that one is ended too — no mutation" \
      "session	id1" "uuid_run1	$u1" "uuid_run2	$u2" \
      "end1_rc	$(cat "$R/cap/2end1.rc")" "end2_rc	$(cat "$R/cap/4end2.rc")"
    l_arm_end
}

# ────────── identity: writer boundary — live meta uuid mutated to uppercase (SC-806b)
arm_identity_uppercase() {
    setup_local identity-uppercase-meta-uuid id2 || { l_arm_end; return 1; }
    local META="$R/h/.ae/sessions/id2/meta"
    cp "$META" "$R/cap/meta.before-mutation.txt"
    local lower="$SESS_UUID" upper
    upper="$(printf '%s' "$lower" | tr 'a-f' 'A-F')"
    # NAMED BYTE DIFF, writer-shaped (temp + rename), on a REAL LIVE session's meta
    MODE_BEFORE="$(stat -f '%Lp' "$META")"
    l_rewrite_preserving_mode "$META" "s/^session_id=${lower}\$/session_id=${upper}/"
    MODE_AFTER="$(stat -f '%Lp' "$META")"
    cp "$META" "$R/cap/meta.after-mutation.txt"
    diff -u "$R/cap/meta.before-mutation.txt" "$R/cap/meta.after-mutation.txt" >"$R/cap/meta.mutation.diff" 2>&1
    { printf 'mutation\tsession_id value case-folded a-f -> A-F\n'
      printf 'before\t%s\n' "$lower"; printf 'after\t%s\n' "$upper"
      printf 'writer_shape\ttemp file + chmod-to-original-mode + rename — only the NAMED bytes change\n'
      printf 'mode.before\t%s\n' "$MODE_BEFORE"
      printf 'mode.after\t%s\n' "$MODE_AFTER"; } >"$R/cap/mutation.txt"
    l_snap 0pre
    l_ae 2end end -f id2
    sleep 1
    { printf '# archive directory names (od of each name)\n'
      for d in "$R"/h/.ae/archive/*/; do [[ -d "$d" ]] || continue
        printf '%s\n' "$(basename "$d")"; printf '%s' "$(basename "$d")" | od -c | head -4; done
      printf '\n# archive meta id keys, exact bytes\n'
      for m in "$R"/h/.ae/archive/*/meta; do [[ -e "$m" ]] || continue
        printf '=== %s ===\n' "$m"
        grep -n '^archive_id=\|^session_id\|^source_session\|^archive_id_origin=' "$m"
        printf '%s\n' '--- od of the archive_id line ---'
        grep '^archive_id=' "$m" | od -c
      done
    } >"$R/cap/archive-identity.txt" 2>&1
    l_manifest "$R/h/.ae/archive" "$R/cap/final-archive.tsv"
    l_snap 3post
    armtxt identity-uppercase-meta-uuid "roster_ids	SC-806b" \
      "fixture	--local family, REAL LIVE session" \
      "construction	the live session meta's session_id value is case-folded a-f -> A-F by a temp+rename write, then end runs" \
      "session	id2" "uuid_before	$lower" "uuid_after	$upper" \
      "end_rc	$(cat "$R/cap/2end.rc")"
    l_arm_end
}

# ─────────────────────────────────── unreachable recorded server (SC-816)
arm_unreachable_server() {
    setup_local unreachable-server us1 || { l_arm_end; return 1; }
    # a second session on the same server, so 'per-target output' has more than one target
    l_ae 1blaunch2 --local us2
    sleep 2
    local SRVPID; SRVPID="$(/opt/homebrew/bin/tmux -S "$SOCK" display-message -p '#{pid}' 2>/dev/null)"
    cp "$R/h/.ae/sessions/us1/meta" "$R/cap/meta.us1.txt"
    l_snap 0pre
    { printf 'recorded_socket\t%s\n' "$(grep '^tmux_server=' "$R/h/.ae/sessions/us1/meta" | cut -d= -f2-)"
      printf 'socket_dir\t%s\n' "$(dirname "$SOCK")"
      printf 'server_pid_before\t%s\n' "${SRVPID:-<none>}"
      printf 'manipulation\tthe directory holding the recorded socket is removed (the socket path becomes unreachable; the server process is left running)\n'
    } >"$R/cap/manipulation.txt"
    l_manifest "$(dirname "$SOCK")" "$R/cap/socketdir.before.tsv"
    rm -rf "$(dirname "$SOCK")"
    l_manifest "$(dirname "$SOCK")" "$R/cap/socketdir.after.tsv"
    l_ae 2end end -f us1
    l_ae 3endall end -f all
    sleep 1
    { printf 'server_pid_after\t%s\n' "${SRVPID:-<none>}"
      printf 'server_alive_after\t%s\n' "$(kill -0 "${SRVPID:-0}" 2>/dev/null && echo yes || echo no)"; } >>"$R/cap/manipulation.txt"
    l_manifest "$R/h/.ae" "$R/cap/3post.aehome.tsv"
    l_manifest "$R/h/.ae/archive" "$R/cap/3post.archive.tsv"
    l_tmuxsnap "$SOCK" "$R/cap/3post.tmux.txt"
    armtxt unreachable-server "roster_ids	SC-816" \
      "fixture	--local family, two sessions on one recorded server" \
      "construction	the directory holding the recorded tmux socket is removed, then end runs for one target and then for 'all'" \
      "end_single_rc	$(cat "$R/cap/2end.rc")" "end_all_rc	$(cat "$R/cap/3endall.rc")" \
      "server_pid	${SRVPID:-<none>}"
    [[ -n "${SRVPID:-}" ]] && kill -9 "$SRVPID" 2>/dev/null
    l_arm_end
}

# ───────────────────────── hostile construction: symlinked archive ROOT
arm_symlink_root() {
    setup_local hostile-symlinked-archive-root sy1 || { l_arm_end; return 1; }
    mkdir -p "$R/elsewhere-archive"
    rm -rf "$R/h/.ae/archive"
    ln -s "$R/elsewhere-archive" "$R/h/.ae/archive"
    { printf 'manipulation\tthe archive ROOT ($AE_HOME/archive) is replaced by a symlink to a directory outside AE_HOME\n'
      printf 'link\t%s -> %s\n' "$R/h/.ae/archive" "$R/elsewhere-archive"; } >"$R/cap/manipulation.txt"
    l_manifest "$R/h/.ae" "$R/cap/0pre.aehome.tsv"
    l_manifest "$R/elsewhere-archive" "$R/cap/0pre.linktarget.tsv"
    l_tmuxsnap "$SOCK" "$R/cap/0pre.tmux.txt"
    l_ae 2end end -f sy1
    sleep 1
    l_manifest "$R/h/.ae" "$R/cap/3post.aehome.tsv"
    l_manifest "$R/elsewhere-archive" "$R/cap/3post.linktarget.tsv"
    l_tmuxsnap "$SOCK" "$R/cap/3post.tmux.txt"
    armtxt hostile-symlinked-archive-root "roster_ids	(none — hostile construction, captures only)" \
      "fixture	--local family" \
      "construction	the archive root is a symlink to a directory outside AE_HOME" \
      "end_rc	$(cat "$R/cap/2end.rc")"
    l_arm_end
}

arm_identity_uppercase
echo DONE
