#!/opt/homebrew/bin/bash
# A2 composite: ONE AE_HOME holding session dirs harvested from G1, G2 (all six),
# G2b and G6, so the list filters have something to discriminate between. Composition
# is a COPY of whole producer-built session dirs — no byte inside any of them changes.
# events.jsonl MTIME is set deliberately per session and recorded: the frozen reader
# takes session activity from that mtime, so it is a load-bearing fixture property and
# must be chosen rather than inherited from whenever the copy happened.
source "$(dirname "$0")/tlib.sh"
GRP=A2; MEM=composite
mkdir -p "$TSTORE/$GRP/_meta"
DST="$TSTORE/$GRP/$MEM"
[[ -e "$DST" ]] && chmod -R u+w "$DST" 2>/dev/null; rm -rf "$DST"
mkdir -p "$DST/sessions"
cp -p "$TSTORE/G1/healthy/config" "$DST/config"
P="$TSTORE/$GRP/_meta/$MEM.txt"
{ echo "group=$GRP"; echo "member=$MEM"
  echo "construction=composition of whole producer-built session dirs, copied byte-for-byte"
  echo "config_from=G1/healthy"
  echo "frozen_sha=$FROZEN_SHA"; echo "built_utc=$(/bin/date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "--- members ---"
} >"$P"
add() { # <group> <member>
    local g="$1" m="$2" s
    s="$(ls "$TSTORE/$g/$m/sessions" | head -1)"
    cp -Rp "$TSTORE/$g/$m/sessions/$s" "$DST/sessions/$s"
    chmod -R u+w "$DST/sessions/$s"
    echo "  session=$s from=$g/$m source_fingerprint_protected=$(grep '^fingerprint_protected=' "$TSTORE/$g/_meta/$m.txt" | cut -d= -f2-)" >>"$P"
}
add G1 healthy
for m in dead stale throttled waiting-user blocked unanswered; do add G2 "$m"; done
add G2b competing
add G6 stopped-plain
add G6 stopped-attention

# Deliberate mtimes on events.jsonl. touch -t [[CC]YY]MMDDhhmm[.SS] — the portable form.
echo "--- events.jsonl mtimes (deliberate; the frozen reader takes activity from mtime) ---" >>"$P"
setm() { # <session> <touch-stamp> <why>
    local s="$1" t="$2" why="$3" f="$DST/sessions/$1/events.jsonl"
    if [[ -f "$f" ]]; then
        touch -t "$t" "$f"
        echo "  $s mtime=$t ($why) epoch=$(stat -f %m "$f")" >>"$P"
    else
        echo "  $s events.jsonl ABSENT — no mtime to set ($why)" >>"$P"
    fi
}
NOWSTAMP="$(/bin/date -v-1M '+%Y%m%d%H%M.%S' 2>/dev/null || /bin/date '+%Y%m%d%H%M.%S')"
OLDSTAMP="202601011200.00"
setm tg1     "$NOWSTAMP" "recent — one minute before the build"
for s in twda1 twda2 twda3 tg2wu tg2bl tg2un tg2b tg6a tg6b; do
    setm "$s" "$OLDSTAMP" "old — fixed 2026-01-01T12:00:00 local"
done
dir_manifest "$DST" >"$TSTORE/$GRP/_meta/$MEM.modes.tsv"
echo "fingerprint_pre_protection=$(dir_fingerprint "$DST")" >>"$P"
chmod -R a-w "$DST" 2>/dev/null || true
echo "fingerprint_protected=$(dir_fingerprint "$DST")" >>"$P"
echo "A2/composite built: $(grep '^fingerprint_protected=' "$P" | cut -d= -f2-)"
ls "$DST/sessions"
