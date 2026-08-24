#!/usr/bin/env bash
# RED-PROOF for sweep-check.sh's batch-taxonomy derivation.
#
# It runs sweep-check END TO END against ISOLATED COPIES of crit-assign.md (arg 5),
# never the tracked file. The predicate can no longer be tested in isolation because
# it no longer carries its own answer: it reads the declaration, which is the repair.
# So the subject is now the derivation, and the only honest way to exercise it is
# through the program that performs it.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
SRC="$HERE/crit-assign.md"
TMP="$(mktemp -d "/tmp/rp-batch.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT
fail=0

malformed() { # seeded-file -> ASSIGN_MALFORMED count
  bash "$HERE/sweep-check.sh" "" "" "" "" "$1" 2>/dev/null \
    | grep -oE 'ASSIGN_MALFORMED=[0-9]+' | head -1 | cut -d= -f2
}
# sweep-check takes positional paths; empty strings must fall back to defaults, so
# pass the real ones explicitly rather than relying on that.
R="$HERE/../../.."
malformed() {
  bash "$HERE/sweep-check.sh" "$R/docs/migration/semantic-contract.md" \
    "$R/docs/migration/ownership.md" "$R/docs/migration/evidence/closure-map.md" \
    "$R/docs/migration/evidence/closure-map.md" "$1" 2>/dev/null \
    | grep -oE 'ASSIGN_MALFORMED=[0-9]+' | head -1 | cut -d= -f2
}

check() { # name seeded-file expected-comparison why
  local got; got="$(malformed "$2")"
  if [ "$got" $3 ]; then printf '  ok   %-22s ASSIGN_MALFORMED=%-4s %s\n' "$1" "$got" "$4"
  else printf '  FAIL %-22s ASSIGN_MALFORMED=%-4s want %s  %s\n' "$1" "$got" "$3" "$4"; fail=1; fi
}

cp "$SRC" "$TMP/neutral.md"
base="$(malformed "$TMP/neutral.md")"
if [ "$base" != "0" ]; then echo "ABORT: neutral is not clean (ASSIGN_MALFORMED=$base)"; exit 1; fi
echo "neutral                     ASSIGN_MALFORMED=0  (tracked file only read)"

# 1. a row whose tag is not in the declaration
awk -F'|' 'BEGIN{OFS="|"} /^CRIT-ASSIGN:/ && !done {$2=" NOT-A-BATCH "; done=1} {print}' "$SRC" > "$TMP/unknown.md"
cmp -s "$SRC" "$TMP/unknown.md" && { echo "  SEED-DID-NOT-LAND unknown-tag — invalid test"; fail=1; } \
  || check "unknown tag in a row" "$TMP/unknown.md" "-ge 1" "a tag outside the declaration must fail"

# 2. the declaration removed entirely -> FAIL CLOSED, every row malformed
grep -v '^BATCH-TAXONOMY:' "$SRC" > "$TMP/nodecl.md"
cmp -s "$SRC" "$TMP/nodecl.md" && { echo "  SEED-DID-NOT-LAND no-declaration — invalid test"; fail=1; } \
  || check "declaration removed" "$TMP/nodecl.md" "-ge 300" "no taxonomy must mean accept NOTHING, loudly"

# 3. one ratified tag dropped from the declaration -> only its rows fail
sed 's/^\(BATCH-TAXONOMY:.*\) S-GATE\( .*\)$/\1\2/' "$SRC" > "$TMP/drop.md"
cmp -s "$SRC" "$TMP/drop.md" && { echo "  SEED-DID-NOT-LAND drop-S-GATE — invalid test"; fail=1; } \
  || check "one tag dropped" "$TMP/drop.md" "-eq 14" "exactly the 14 S-GATE rows must fail, and no others"

echo
[ $fail -eq 0 ] && echo "ASSIGNMENT-BATCH RED-PROOF: PASS" || echo "ASSIGNMENT-BATCH RED-PROOF: FAIL"
exit $fail
