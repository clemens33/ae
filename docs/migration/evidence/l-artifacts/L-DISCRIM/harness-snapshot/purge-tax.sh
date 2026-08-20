#!/opt/homebrew/bin/bash
# L-PURGE: validator taxonomy (SC-804a-f, SC-818c). ONE named mutation per case.
# Each mutation drives the purge path on ONE clone and a --from attempt on a
# SEPARATE clone — per-consumer separate clones, mandatory here.
set -uo pipefail
source /tmp/aelx/lib/purge-lib.sh

# ── mutators (each takes the archive directory) ───────────────────────────
mut_unexpected_entry() { local d="$1"; printf 'controller-planted unexpected entry\n' >"$d/EXTRA.txt"; chmod 0600 "$d/EXTRA.txt"; }
mut_symlink_inside()   { local d="$1"; rm -f "$d/memo.tsv"; ln -s /etc/hosts "$d/memo.tsv"; }
mut_fifo_inside()      { local d="$1"; rm -f "$d/memo.tsv"; mkfifo -m 0600 "$d/memo.tsv"; }
mut_messages_0755()    { local d="$1"; chmod 0755 "$d/messages"; }
mut_archivedir_0755()  { local d="$1"; chmod 0755 "$d"; }
mut_file_0644()        { local d="$1"; chmod 0644 "$d/meta"; }
mut_exec_user()        { local d="$1"; chmod 0700 "$d/digest.md"; }
mut_exec_group()       { local d="$1"; chmod 0610 "$d/digest.md"; }
mut_exec_other()       { local d="$1"; chmod 0601 "$d/digest.md"; }
mut_id_mismatch()      { local d="$1"; l_rewrite_preserving_mode "$d/meta" 's/^archive_id=.*$/archive_id=00000000-0000-4000-8000-000000000000/'; }
mut_count_mismatch()   { local d="$1"; l_rewrite_preserving_mode "$d/meta" 's/^handover_count=.*$/handover_count=42/'; }

tax_case() { # <case-id> <ids> <target-rel> <description> <mutator> <side>
    local cid="$1" ids="$2" target="$3" desc="$4" mut="$5" side="$6"
    local arm="validator-taxonomy-${cid}-${side}"
    l_arm_begin L-PURGE "$arm" instrumented
    purge_template b_pre_cleanup pg || { l_arm_end; return 1; }
    local ARCH="$R/h/.ae/archive/$PG_UUID"
    cp -p "$ARCH/meta" "$R/cap/archive-meta.pre-mutation.txt"
    l_manifest "$ARCH" "$R/cap/archive.pre-mutation.tsv"
    mutate_record mutation "$ARCH/$target" "$desc" "$mut" "$ARCH"
    l_manifest "$ARCH" "$R/cap/archive.post-mutation.tsv"
    diff -u "$R/cap/archive.pre-mutation.tsv" "$R/cap/archive.post-mutation.tsv" >"$R/cap/archive.mutation.diff" 2>&1
    l_manifest "$R/h/.ae" "$R/cap/1pre.aehome.tsv"
    l_manifest "$R/h/.ae/archive" "$R/cap/1pre.archive.tsv"
    l_manifest "$R/h/.ae/sessions" "$R/cap/1pre.sessions.tsv"
    HOOKS=""; BLOCK=""; l_arm_env
    if [[ "$side" == purge ]]; then l_ae 2op end -f --purge-history pg
    else l_ae 2op --local pgchild --from "$PG_UUID"; fi
    sleep 2
    l_manifest "$R/h/.ae" "$R/cap/3post.aehome.tsv"
    l_manifest "$R/h/.ae/archive" "$R/cap/3post.archive.tsv"
    l_manifest "$R/h/.ae/sessions" "$R/cap/3post.sessions.tsv"
    l_tmuxsnap "$SOCK" "$R/cap/3post.tmux.txt"
    parmtxt "$arm" "$ids" "$desc; this clone drives the $side path ONLY" \
      "case	$cid" "side	$side" "mutation_target	$target" \
      "op	$( [[ "$side" == purge ]] && echo 'ae end -f --purge-history pg' || echo "ae --local pgchild --from $PG_UUID" )" \
      "op_rc	$(cat "$R/cap/2op.rc")" \
      "also_covers	SC-818c (post-state manifests of the archive root and the session state after the attempt)"
    l_arm_end
    return 0
}

run_case() { # <case-id> <ids> <target> <desc> <mutator>
    tax_case "$1" "$2" "$3" "$4" "$5" purge
    tax_case "$1" "$2" "$3" "$4" "$5" from
}

case "${1:-all}" in
  chunkA)
    run_case a-unexpected-entry "SC-804a SC-818c" EXTRA.txt \
      "an unexpected extra entry (a regular file mode 0600 named EXTRA.txt) is created inside the archive directory" mut_unexpected_entry
    run_case b1-symlink-inside "SC-804b SC-818c" memo.tsv \
      "the archive member memo.tsv is replaced by a SYMLINK pointing outside the archive (/etc/hosts)" mut_symlink_inside
    run_case b2-fifo-inside "SC-804b SC-818c" memo.tsv \
      "sibling of b1: the archive member memo.tsv is replaced by a FIFO of mode 0600" mut_fifo_inside
    ;;
  chunkB)
    run_case c1-messages-dir-0755 "SC-804c SC-818c" messages \
      "the archive's messages/ directory mode is changed 0700 -> 0755" mut_messages_0755
    run_case c2-archive-dir-0755 "SC-804c SC-818c" . \
      "the archive directory's own mode is changed 0700 -> 0755" mut_archivedir_0755
    run_case d-file-0644 "SC-804f SC-818c" meta \
      "the archive member meta's mode is changed 0600 -> 0644 (a NAMED mode mutation, deliberate here)" mut_file_0644
    ;;
  chunkC)
    run_case e1-exec-user "SC-804d SC-818c" digest.md \
      "the archive member digest.md gains the USER executable bit (0600 -> 0700)" mut_exec_user
    run_case e2-exec-group "SC-804d SC-818c" digest.md \
      "the archive member digest.md gains the GROUP executable bit (0600 -> 0610)" mut_exec_group
    run_case e3-exec-other "SC-804d SC-818c" digest.md \
      "the archive member digest.md gains the OTHER executable bit (0600 -> 0601)" mut_exec_other
    ;;
  chunkD)
    run_case f1-id-mismatch "SC-804e SC-818c" meta \
      "the archive meta's archive_id value is replaced by a different well-formed uuid, mode preserved" mut_id_mismatch
    run_case f2-count-mismatch "SC-804e SC-818c" meta \
      "on a SECOND independent clone: the archive meta's handover_count value is replaced by 42 so meta and digest disagree, mode preserved" mut_count_mismatch
    ;;
esac
echo DONE
