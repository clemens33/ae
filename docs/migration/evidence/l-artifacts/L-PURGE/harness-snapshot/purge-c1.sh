#!/opt/homebrew/bin/bash
# L-PURGE chunk 1: no-prior-archive, existing-archive (both readings), claim,
# empty-identity (purge + from), symlinked-root control.
set -uo pipefail
source /tmp/aelx/lib/purge-lib.sh

snap_pre()  { l_manifest "$R/h/.ae" "$R/cap/1pre.aehome.tsv"; l_manifest "$R/h/.ae/archive" "$R/cap/1pre.archive.tsv"; l_manifest "$R/h/.ae/sessions" "$R/cap/1pre.sessions.tsv"; }
snap_post() { l_manifest "$R/h/.ae" "$R/cap/3post.aehome.tsv"; l_manifest "$R/h/.ae/archive" "$R/cap/3post.archive.tsv"; l_manifest "$R/h/.ae/sessions" "$R/cap/3post.sessions.tsv"; l_tmuxsnap "$SOCK" "$R/cap/3post.tmux.txt"; }

# ── SC-810a: purge inversion with NO prior archive ────────────────────────
arm_no_prior() {
    l_arm_begin L-PURGE no-prior-archive frozen
    l_config "$R" claude
    { l_mkrepo "$R"; } >/dev/null 2>&1
    HOOKS=""; BLOCK=""; l_arm_env
    AE_CWD="$R/w"; WDIR="$R/w"
    l_ae 0launch --local np
    sleep 2
    l_arm_preflight np || { printf 'PREFLIGHT-FAILED\n' >"$R/cap/ARM-INVALID.txt"; l_arm_end; return 1; }
    PG_UUID="$(grep '^session_id=' "$R/h/.ae/sessions/np/meta" | head -1 | cut -d= -f2-)"
    cp "$R/h/.ae/sessions/np/meta" "$R/cap/session-meta.txt"
    snap_pre
    l_ae 2op end -f --purge-history np
    sleep 1
    snap_post
    parmtxt no-prior-archive SC-810a \
      "a real --local session with a session id and NO archive anywhere is ended with --purge-history; the archive root is captured before and after" \
      "op	ae end -f --purge-history np" "op_rc	$(cat "$R/cap/2op.rc")"
    l_arm_end
}

# ── SC-810b, reading (a): the specimen EXACTLY as produced (claim standing)
arm_existing_as_produced() {
    l_arm_begin L-PURGE existing-archive-as-produced instrumented
    purge_template b_post_rename pg || { l_arm_end; return 1; }
    snap_pre
    HOOKS=""; BLOCK=""; l_arm_env
    l_ae 2op end -f --purge-history pg
    sleep 1
    snap_post
    parmtxt existing-archive-as-produced SC-810b \
      "the session-to-archive pairing is the NATURAL output of a real end cut at b_post_rename in this same sandbox: session directory, published archive and the publisher's still-standing .publishing.<uuid> claim, with no synthetic binding of any kind. end --purge-history then runs on it UNMODIFIED" \
      "reading	(a) the specimen exactly as produced — the standing claim is part of what that cut produces, and the overlap with the claim arm is itself the observation" \
      "claim_standing_at_op	yes" "op	ae end -f --purge-history pg" "op_rc	$(cat "$R/cap/2op.rc")"
    l_arm_end
}

# ── SC-810b, reading (b): archive present, claim already released BY THE PRODUCT
arm_existing_claim_released() {
    l_arm_begin L-PURGE existing-archive-claim-released instrumented
    purge_template b_pre_cleanup pg || { l_arm_end; return 1; }
    snap_pre
    HOOKS=""; BLOCK=""; l_arm_env
    l_ae 2op end -f --purge-history pg
    sleep 1
    snap_post
    parmtxt existing-archive-claim-released SC-810b \
      "the same natural pairing, produced by cutting the real end one barrier later (b_pre_cleanup): the archive is published and the publisher has ALREADY RELEASED its own claim, and the session directory is still on disk. NO controller manipulation was needed to clear the claim — the product released it" \
      "reading	(b) archive present without a standing claim; reached by moving the CUT, not by a manipulation" \
      "claim_standing_at_op	no" "op	ae end -f --purge-history pg" "op_rc	$(cat "$R/cap/2op.rc")"
    l_arm_end
}

# ── SC-818b: a claim planted by the controller ────────────────────────────
arm_claim() {
    l_arm_begin L-PURGE claim instrumented
    purge_template b_pre_cleanup pg || { l_arm_end; return 1; }
    local ROOT="$R/h/.ae/archive"
    mutate_record mutation "$ROOT/.publishing.$PG_UUID" \
      "a .publishing.<uuid> claim directory for this archive's own uuid is created mode 0700 under the archive root" \
      mkdir -m 0700 "$ROOT/.publishing.$PG_UUID"
    snap_pre
    HOOKS=""; BLOCK=""; l_arm_env
    l_ae 2op end -f --purge-history pg
    sleep 1
    snap_post
    parmtxt claim SC-818b \
      "a claim for the archive's own uuid is planted under the archive root, then end --purge-history runs" \
      "op	ae end -f --purge-history pg" "op_rc	$(cat "$R/cap/2op.rc")"
    l_arm_end
}

# ── SC-818d: source identity emptied — purge clone and --from clone ───────
empty_identity_mut() { # <archive-meta>
    local m="$1"
    l_rewrite_preserving_mode "$m" 's/^source_session=.*$/source_session=/'
}
arm_empty_identity() { # <purge|from>
    local side="$1"
    l_arm_begin L-PURGE "empty-identity-$side" instrumented
    purge_template b_pre_cleanup pg || { l_arm_end; return 1; }
    local ARCH="$R/h/.ae/archive/$PG_UUID"
    mutate_record mutation "$ARCH/meta" \
      "the archive meta's source_session value is emptied (key kept, value removed), temp+rename" \
      empty_identity_mut "$ARCH/meta"
    snap_pre
    HOOKS=""; BLOCK=""; l_arm_env
    if [[ "$side" == purge ]]; then l_ae 2op end -f --purge-history pg
    else l_ae 2op --local pgchild --from "$PG_UUID"; fi
    sleep 2
    snap_post
    parmtxt "empty-identity-$side" SC-818d \
      "the archive meta's source_session value is emptied on a fresh clone; this clone drives the $side path only" \
      "side	$side" "op_rc	$(cat "$R/cap/2op.rc")"
    l_arm_end
}

# ── SC-818a control (ALREADY-OBSERVED, non-roster): symlinked archive ROOT
arm_symlink_root_control() {
    l_arm_begin L-PURGE control-symlinked-archive-root instrumented
    purge_template b_pre_cleanup pg || { l_arm_end; return 1; }
    mkdir -p "$R/elsewhere-archive"
    cp -Rp "$R/h/.ae/archive/." "$R/elsewhere-archive/"
    rm -rf "$R/h/.ae/archive"
    ln -s "$R/elsewhere-archive" "$R/h/.ae/archive"
    { printf 'manipulation\tthe archive ROOT is replaced by a symlink to a directory outside AE_HOME holding the same real archive\n'
      printf 'link\t%s -> %s\n' "$R/h/.ae/archive" "$R/elsewhere-archive"; } >"$R/cap/manipulation.txt"
    l_manifest "$R/elsewhere-archive" "$R/cap/1pre.linktarget.tsv"
    l_manifest "$R/h/.ae" "$R/cap/1pre.aehome.tsv"
    HOOKS=""; BLOCK=""; l_arm_env
    l_ae 2op end -f --purge-history pg
    sleep 1
    l_manifest "$R/elsewhere-archive" "$R/cap/3post.linktarget.tsv"
    l_manifest "$R/h/.ae" "$R/cap/3post.aehome.tsv"
    l_manifest "$R/h/.ae/sessions" "$R/cap/3post.sessions.tsv"
    parmtxt control-symlinked-archive-root "(none — SC-818a is a non-roster safety control, ref only; this arm is NOT a coverage arm)" \
      "the archive root is a symlink to a directory outside AE_HOME holding the same real archive; end --purge-history then runs" \
      "op	ae end -f --purge-history pg" "op_rc	$(cat "$R/cap/2op.rc")"
    l_arm_end
}

arm_empty_identity purge
arm_empty_identity from
echo DONE
