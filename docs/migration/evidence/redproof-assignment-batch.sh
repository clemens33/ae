#!/usr/bin/env bash
# RED-PROOF for sweep-check.sh's known_assignment_batch().
#
# It exercises the SHIPPED function text, extracted from sweep-check.sh at run
# time rather than retyped here: a red-proof against a copy of the predicate
# proves the copy. It never reads or mutates crit-assign.md — the subject is the
# predicate, and the assignment file is not needed to test it.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
SRC="$HERE/sweep-check.sh"
fail=0

fn="$(awk '/^function known_assignment_batch\(batch\) \{/{f=1} f{print} f&&/^\}/{exit}' "$SRC")"
[[ -n "$fn" ]] || { echo "ABORT: could not extract known_assignment_batch from $SRC"; exit 2; }
grep -q 'S-GATE' <<<"$fn" || { echo "ABORT: extracted predicate does not mention S-GATE — wrong function or stale file"; exit 2; }

check() { # tag expected(accept|reject) why
  local got
  got="$(printf '%s\n' "$1" | awk "$fn"'{ print known_assignment_batch($0) ? "accept" : "reject" }')"
  if [[ "$got" == "$2" ]]; then printf '  ok   %-12s %-7s  %s\n' "[$1]" "$got" "$3"
  else printf '  FAIL %-12s got=%-7s want=%-7s  %s\n' "[$1]" "$got" "$2" "$3"; fail=1; fi
}

echo "=== the two ratified tags this change adds ==="
check 'S-GATE'    accept 'ratified successor observer-provenance tag'
check 'S-PENDING' accept 'ratified successor observer-provenance tag'

echo "=== an unknown S-* must STILL fail — the point is not to open the namespace ==="
check 'S-BOGUS'   reject 'unknown S-* tag'
check 'S-'        reject 'bare prefix'
check 'S'         reject 'prefix alone'

echo "=== anchors hold: no prefix/suffix/case slippage ==="
check 'S-GATEX'   reject 'suffix past a valid tag'
check 'XS-GATE'   reject 'prefix before a valid tag'
check 's-gate'    reject 'lowercase'
check ''          reject 'empty batch'

echo "=== pre-existing tags unchanged ==="
check 'B0'        accept 'existing'
check 'C'         accept 'existing'
check 'T-WD'      accept 'existing'
check 'F-CONTRIB' accept 'existing'
check 'L-STOP'    accept 'existing'
check 'NOPE'      reject 'never valid'

echo
if [[ $fail -eq 0 ]]; then echo "ASSIGNMENT-BATCH RED-PROOF: PASS"; else echo "ASSIGNMENT-BATCH RED-PROOF: FAIL"; fi
exit $fail
