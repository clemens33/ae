#!/opt/homebrew/bin/bash
# Publish ONE template member into the committed tree, then rebuild the fingerprint table
# and the checksums. Deliberately NOT publish-templates.sh, which starts with `rm -rf` on
# the whole published tree: a full republish of 598 files to add one member puts every
# other member's committed bytes at the mercy of whatever is in /tmp today.
set -uo pipefail
GRP="$1"; MEM="$2"
S="$(dirname "$0")"
TSTORE=/tmp/aecx/templates
DEST=/Users/ckriech/projects/clemens33/ae-rust/docs/migration/evidence/batch-c-artifacts/templates
m="$TSTORE/$GRP/$MEM"
[[ -d "$m" ]] || { echo "no such member: $GRP/$MEM"; exit 1; }
mkdir -p "$DEST/$GRP/_meta"
cp "$TSTORE/$GRP/_meta/$MEM."* "$DEST/$GRP/_meta/" 2>/dev/null
out="$DEST/$GRP/fixture-bytes/$MEM"; rm -rf "$out"; mkdir -p "$out"
cp "$m/config" "$out/config" 2>/dev/null || echo "UNREADABLE-OR-ABSENT" >"$out/config.NOTE"
for sess in $(ls "$m/sessions" 2>/dev/null); do
    mkdir -p "$out/sessions/$sess"
    cp "$m/sessions/$sess/meta" "$out/sessions/$sess/meta" 2>/dev/null \
      || echo "meta ABSENT in this member (named mutation) — see _meta/$MEM.mutation.txt and _meta/$MEM.removed-meta.bytes.txt" >"$out/sessions/$sess/meta.NOTE"
    cp "$m/sessions/$sess/events.jsonl" "$out/sessions/$sess/events.jsonl" 2>/dev/null \
      || echo "events.jsonl ABSENT in this member (named mutation)" >"$out/sessions/$sess/events.jsonl.NOTE"
    if [[ -d "$m/sessions/$sess/messages" ]]; then
        mkdir -p "$out/sessions/$sess/messages"; cp "$m/sessions/$sess/messages/"* "$out/sessions/$sess/messages/" 2>/dev/null
    fi
done
# fingerprint table rebuilt by READING every member; no member's published bytes are touched
{ printf 'group\tmember\tfingerprint_pre_protection\tfingerprint_protected\tsession\tfiles\n'
  for g in "$TSTORE"/*/; do
      grp="$(basename "$g")"
      for mm in "$g"*/; do
          mem="$(basename "$mm")"; [[ "$mem" == "_meta" ]] && continue
          sess="$(ls "$mm/sessions" 2>/dev/null | head -1)"
          pre="$(grep '^fingerprint_pre_protection=' "$g/_meta/$mem.txt" 2>/dev/null | cut -d= -f2-)"
          prot="$(grep '^fingerprint_protected=' "$g/_meta/$mem.txt" 2>/dev/null | cut -d= -f2-)"
          n="$(wc -l < "$g/_meta/$mem.modes.tsv" 2>/dev/null | tr -d ' ')"
          printf '%s\t%s\t%s\t%s\t%s\t%s\n' "$grp" "$mem" "${pre:--}" "${prot:--}" "${sess:--}" "${n:--}"
      done
  done; } >"$DEST/FINGERPRINTS.tsv.tmp" && mv "$DEST/FINGERPRINTS.tsv.tmp" "$DEST/FINGERPRINTS.tsv"
"$S/write-sums.sh" "$DEST"
echo "published $GRP/$MEM"
