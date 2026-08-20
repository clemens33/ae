#!/opt/homebrew/bin/bash
# A9 fixture: SC-405i — a present session dir whose META IS ABSENT.
#
# The row is an ABSENCE claim, and three different states render the same emptiness:
# meta ABSENT (this member), meta PRESENT BUT UNREADABLE (G3/meta-mode-000, already
# built), and a reader that never looked at all (which is a property of the instrument,
# not the fixture — A9 carries G1/healthy through the same consumer set so a reader that
# looks is known to render something).
#
# Derived from G1/healthy by ONE named mutation, byte-recorded before removal.
source "$(dirname "$0")/tlib.sh"
mkdir -p "$TSTORE/A9/_meta"
grp=A9 mem=meta-absent sg=G1 sm=healthy
DST="$TSTORE/$grp/$mem"
[[ -e "$DST" ]] && chmod -R u+w "$DST" 2>/dev/null
rm -rf "$DST"; cp -R "$TSTORE/$sg/$sm" "$DST"; chmod -R u+w "$DST"
DIFF="$TSTORE/$grp/_meta/$mem.mutation.txt"; : >"$DIFF"
SESS="$(ls "$DST/sessions" | head -1)"
{ echo "group=$grp"; echo "member=$mem"
  echo "derived_from=$sg/$sm (byte copy)"
  echo "source_fingerprint_protected=$(grep '^fingerprint_protected=' "$TSTORE/$sg/_meta/$sm.txt" | cut -d= -f2-)"
  echo "session=$SESS"
  echo "frozen_sha=$FROZEN_SHA"; echo "built_utc=$(/bin/date -u +%Y-%m-%dT%H:%M:%SZ)"; } >"$TSTORE/$grp/_meta/$mem.txt"
B="$DST/sessions/$SESS/meta"
{ echo "## mutation: remove the session's meta file entirely"
  echo "file: sessions/$SESS/meta"
  echo "before: sha256=$(shasum -a 256 "$B" | cut -d' ' -f1) bytes=$(stat -f %z "$B") mode=$(stat -f %Lp "$B")"
  echo "before bytes, verbatim:"
  sed 's/^/    /' "$B"
  echo "after:  FILE ABSENT (the session DIRECTORY remains, with every other file untouched)"
  echo "distinct from G3/meta-mode-000, where the same bytes are present and unreadable,"
  echo "and from G4/no-events, where meta is intact and events.jsonl is the missing file."; } >>"$DIFF"
cp "$B" "$TSTORE/$grp/_meta/$mem.removed-meta.bytes.txt"
rm -f "$B"
dir_manifest "$DST" >"$TSTORE/$grp/_meta/$mem.modes.tsv"
echo "fingerprint_pre_protection=$(dir_fingerprint "$DST")" >>"$TSTORE/$grp/_meta/$mem.txt"
echo "named_mutations=see _meta/$mem.mutation.txt" >>"$TSTORE/$grp/_meta/$mem.txt"
chmod -R a-w "$DST" 2>/dev/null || true
echo "fingerprint_protected=$(dir_fingerprint "$DST")" >>"$TSTORE/$grp/_meta/$mem.txt"
echo "$grp/$mem sealed: $(grep '^fingerprint_protected=' "$TSTORE/$grp/_meta/$mem.txt" | cut -d= -f2-)"
echo "meta absent? $( [[ -e "$B" ]] && echo NO || echo yes ); session dir files: $(find "$DST/sessions/$SESS" -type f | wc -l | tr -d ' ')"
