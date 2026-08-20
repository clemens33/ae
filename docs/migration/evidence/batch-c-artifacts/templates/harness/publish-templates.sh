#!/opt/homebrew/bin/bash
set -uo pipefail
TSTORE=/tmp/aecx/templates
DEST=/Users/ckriech/projects/clemens33/ae-rust/docs/migration/evidence/batch-c-artifacts/templates
rm -rf "$DEST"; mkdir -p "$DEST"
: >"$DEST/FINGERPRINTS.tsv"
printf 'group\tmember\tfingerprint_pre_protection\tfingerprint_protected\tsession\tfiles\n' >>"$DEST/FINGERPRINTS.tsv"
for g in "$TSTORE"/*/; do
    grp="$(basename "$g")"
    mkdir -p "$DEST/$grp/_meta"
    cp "$g/_meta/"* "$DEST/$grp/_meta/" 2>/dev/null
    for m in "$g"*/; do
        mem="$(basename "$m")"; [[ "$mem" == "_meta" ]] && continue
        # ALL session dirs, not just the first: a composite member holds many.
        mapfile -t SESSLIST < <(ls "$m/sessions" 2>/dev/null || true)
        sess="${SESSLIST[0]:-}"
        out="$DEST/$grp/fixture-bytes/$mem"
        mkdir -p "$out"
        cp "$m/config" "$out/config" 2>/dev/null || echo "UNREADABLE-OR-ABSENT" >"$out/config.NOTE"
        for sess in ${SESSLIST[@]+"${SESSLIST[@]}"}; do
            mkdir -p "$out/sessions/$sess"
            cp "$m/sessions/$sess/meta" "$out/sessions/$sess/meta" 2>/dev/null \
              || echo "meta present in the template but UNREADABLE at its stored mode (see _meta/$mem.modes.tsv and _meta/$mem.mutation.txt)" >"$out/sessions/$sess/meta.NOTE"
            cp "$m/sessions/$sess/events.jsonl" "$out/sessions/$sess/events.jsonl" 2>/dev/null \
              || echo "events.jsonl ABSENT in this member (named mutation)" >"$out/sessions/$sess/events.jsonl.NOTE"
            if [[ -d "$m/sessions/$sess/messages" ]]; then
                mkdir -p "$out/sessions/$sess/messages"; cp "$m/sessions/$sess/messages/"* "$out/sessions/$sess/messages/" 2>/dev/null
            fi
        done
        [[ -d "$m/_g11-producer-inputs" ]] && { mkdir -p "$out/_g11-producer-inputs"; cp "$m/_g11-producer-inputs/"* "$out/_g11-producer-inputs/" 2>/dev/null; }
        [[ -f "$m/_a1-510c-planted-producer-input.jsonl" ]] && cp "$m/_a1-510c-planted-producer-input.jsonl" "$out/" 2>/dev/null
        sess="${SESSLIST[0]:-}"
        pre="$(grep '^fingerprint_pre_protection=' "$g/_meta/$mem.txt" 2>/dev/null | cut -d= -f2-)"
        prot="$(grep '^fingerprint_protected=' "$g/_meta/$mem.txt" 2>/dev/null | cut -d= -f2-)"
        n="$(wc -l < "$g/_meta/$mem.modes.tsv" 2>/dev/null | tr -d ' ')"
        printf '%s\t%s\t%s\t%s\t%s\t%s\n' "$grp" "$mem" "${pre:--}" "${prot:--}" "${sess:--}" "${n:--}" >>"$DEST/FINGERPRINTS.tsv"
    done
done
( cd "$DEST" && find . -type f ! -name SHA256SUMS.txt -print0 | sort -z | xargs -0 shasum -a 256 ) > /tmp/aecx/tsums && mv /tmp/aecx/tsums "$DEST/SHA256SUMS.txt"
echo "PUBLISHED templates -> $DEST"
column -t -s $'\t' "$DEST/FINGERPRINTS.tsv"
