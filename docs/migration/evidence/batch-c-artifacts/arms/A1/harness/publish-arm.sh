#!/opt/homebrew/bin/bash
# Publish one arm group's captures into the repo artifacts dir, folding the
# per-consumer env/hash sidecars into one table per case (the bytes themselves are
# published verbatim; nothing is summarised away).
set -uo pipefail
ARM="$1"
SRC="/tmp/aecx/arms/$ARM"
DEST="/Users/ckriech/projects/clemens33/ae-rust/docs/migration/evidence/batch-c-artifacts/arms/$ARM"
[[ -d "$SRC" ]] || { echo "no captures at $SRC"; exit 1; }
[[ -e "$DEST" ]] && chmod -R u+w "$DEST" 2>/dev/null
rm -rf "$DEST"; mkdir -p "$DEST"
cp "$SRC/ledger.tsv" "$DEST/" 2>/dev/null
for d in "$SRC"/*/; do
    c="$(basename "$d")"; [[ -d "$d/cap" ]] || continue
    out="$DEST/$c"; mkdir -p "$out/out"
    cp "$d/cap/case.txt" "$out/" 2>/dev/null
    cp "$d/cap"/manifest.*.tsv "$d/cap"/manifest.diff.txt "$out/" 2>/dev/null
    cp "$d/cap"/tmux.*.txt "$out/" 2>/dev/null
    cp "$d/cap"/roster.*.txt "$out/" 2>/dev/null
    first=""
    printf 'consumer\trc\tstdout_sha256\tstdout_bytes\tstderr_sha256\tstderr_bytes\tbounded\targv\n' >"$out/consumers.tsv"
    # nested consumer dirs (live sub-arms) or flat
    for capdir in "$d/cap" "$d/cap"/consumers.*; do
        [[ -d "$capdir" ]] || continue
        pfx=""; [[ "$capdir" != "$d/cap" ]] && pfx="$(basename "$capdir" | sed 's/^consumers\.//')/"
        [[ -n "$pfx" ]] && mkdir -p "$out/out/${pfx%/}"
        for r in "$capdir"/*.rc; do
            [[ -f "$r" ]] || continue
            lbl="$(basename "$r" .rc)"
            rc="$(cat "$r")"
            so="$capdir/$lbl.stdout"; se="$capdir/$lbl.stderr"
            [[ -z "$first" && -f "$capdir/$lbl.env.txt" ]] && { first="$capdir/$lbl.env.txt"; sed -n '1,/^argv:/p' "$first" | sed '$d' >"$out/env.txt"; }
            argv="$(sed -n '/^argv:/,$p' "$capdir/$lbl.env.txt" 2>/dev/null | tail -n +2 | tr '\n' ' ' | sed 's/  */ /g')"
            bnd="$(grep -h '^stopped_by_harness_after=' "$capdir/$lbl.hash.txt" 2>/dev/null | cut -d= -f2- || true)"
            printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$pfx$lbl" "$rc" \
                "$(shasum -a 256 "$so" | cut -d' ' -f1)" "$(stat -f %z "$so")" \
                "$(shasum -a 256 "$se" | cut -d' ' -f1)" "$(stat -f %z "$se")" \
                "${bnd:--}" "$argv" >>"$out/consumers.tsv"
            cp "$so" "$out/out/$pfx$lbl.stdout"
            [[ -s "$se" ]] && cp "$se" "$out/out/$pfx$lbl.stderr"
        done
    done
done
( cd "$DEST" && find . -type f ! -name SHA256SUMS.txt -print0 | sort -z | xargs -0 shasum -a 256 ) > /tmp/aecx/as && mv /tmp/aecx/as "$DEST/SHA256SUMS.txt"
echo "PUBLISHED $ARM -> $DEST  ($(find "$DEST" -type f | wc -l | tr -d ' ') files, $(du -sh "$DEST" | cut -f1))"
