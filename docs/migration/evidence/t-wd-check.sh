#!/usr/bin/env bash
# T-WD design consistency checker.
#
# MECHANISM, NOT RESOLUTION. Every class below is a defect this document has
# ACTUALLY SHIPPED at least once — stale ordinals that silently retargeted, a
# spec count printed where a unit count was meant, a stale self-version, a
# barrier list that disagreed with the typed table. A rule stated in prose is
# broken by the commit that states it; this inspects instances instead.
#
# Terms are DERIVED from the document's own declarations wherever possible: a
# transcribed term list can only under-report (it once dropped `fails` and
# reported clean while five instances stood).
set -uo pipefail
DOC="${1:-t-wd-design.md}"
fail=0
say() { printf '%s\n' "$*"; }
bad() { fail=1; printf 'FAIL  %s\n' "$*"; }

# ---- 1. counts derived from the roster, compared to every printed count ----
roster="$(awk '/^\| # \| arm id \|/,/^$/' "$DOC" | grep '^| [0-9]')"
specs=$(printf '%s\n' "$roster" | wc -l | tr -d ' ')
gap=$(printf '%s\n' "$roster" | grep -c 'GAP' || true)
capture=$(printf '%s\n' "$roster" | grep -c 'CAPTURE-ONLY' || true)
red=$(printf '%s\n' "$roster" | grep -c '| RED |' || true)
twolane=$(printf '%s\n' "$roster" | grep -c 'bash+uv' || true)
runnable=$((specs - gap))
units=$((runnable + twolane))
m12=$(printf '%s\n' "$roster" | grep -v 'GAP' | grep -c '§4\.' || true)
say "derived: specs=$specs red=$red capture=$capture gap=$gap two-lane=$twolane runnable=$runnable units=$units m12=$m12"

# NOTE: a grep that matches NOTHING must not read as a failure. The first version
# piped grep into `while ... done || fail=1`, so "no printed count present" — the
# CORRECT state — exited 1 and reported a failure with no finding. An instrument
# that cannot distinguish "clean" from "broken" is not an instrument.
while IFS= read -r l; do
  [ -n "$l" ] || continue
  n="${l##*executed unit count is }"; n="${n%% *}"; n="${n%%.*}"
  [ "$n" = "$units" ] || bad "unit count says $n, roster yields $units (line ${l%%:*})"
done < <(grep -n 'executed unit count is [0-9]*' "$DOC" || true)
while IFS= read -r l; do
  [ -n "$l" ] || continue
  n=$(printf '%s' "$l" | sed -E 's/.*[^0-9]([0-9]+) arms \(rows.*/\1/')
  [ "$n" = "$m12" ] || bad "M12 count says $n, typed rows yield $m12 (line ${l%%:*})"
done < <(grep -nE '\b[0-9]+ arms \(rows SC-' "$DOC" || true)

# ---- 2. no self-version numeral (identity is the commit hash) ----
if grep -nE 'worker draft \*\*v[0-9]' "$DOC"; then
  bad "document carries its own version numeral — identity is the commit hash"
fi

# ---- 3. ordinal arm references outside permitted zones ----
# permitted: the roster table, the execution order, and change-log/history rows.
awk '
  /^\| # \| arm id \|/{roster=1} roster&&/^$/{roster=0}
  /^\*\*Execution order/{eo=1} eo&&/^$/{eo=0}
  /^## 6\./{hist=1}
  /^>/{next}
  !roster&&!eo&&!hist&&/arms? [0-9]+/{printf "%d: %s\n", NR, $0}
' "$DOC" > /tmp/twd_ord.$$ || true
if [ -s /tmp/twd_ord.$$ ]; then bad "ordinal arm references outside permitted zones:"; cat /tmp/twd_ord.$$; fi
rm -f /tmp/twd_ord.$$

# ---- 4. barrier lists must agree with the typed barrier table ----
tbl=$(awk '/^\| id \| site \| frozen anchor \|/{t=1} t&&/^\| `/{print} t&&/^$/{t=0}' "$DOC" \
      | sed -E 's/^\| `([^`]*)`.*/\1/' | sort -u)
oblig=$(grep -v '^>' "$DOC" | grep -o '`\(CUT\|BAR\)-[A-Z0-9]*-[A-Z0-9-]*`' | tr -d '`' | sort -u)
missing=$(comm -13 <(printf '%s\n' "$tbl") <(printf '%s\n' "$oblig"))
[ -n "$missing" ] && { bad "barrier ids used but absent from the typed table:"; printf '%s\n' "$missing"; }

# ---- 5. banned vocabulary, terms READ FROM the M1 declaration ----
terms=$(sed -n '/A committed linter rejects the design/,/It reads declared fields/p' "$DOC" \
        | tr -d '\n' | grep -o '`[^`]*`' | tr -d '`' | tr '|' '\n' | sed 's/^ *//;s/ *$//' | grep -v '^$' | grep -v 'CANDIDATE SPACE')
if [ -z "$terms" ]; then bad "could not derive lint terms from M1 — refusing to run a shorter list"; fi
pat=$(printf '%s' "$terms" | paste -sd'|' -)
say "derived lint terms: $pat"
awk -v pat="$pat" '
  /^## 3A/{z=1} /^## 5\. Fixture/{z=0}
  /^### 4\./{z=1} /^## 5\./{z=0}
  /^>/{next}
  z{ line=$0; gsub(/`[^`]*`/,"",line); if (line ~ "\\<(" pat ")\\>") printf "%d: %s\n", NR, $0 }
' "$DOC" > /tmp/twd_vocab.$$ || true
if [ -s /tmp/twd_vocab.$$ ]; then bad "banned vocabulary in arm fields:"; cat /tmp/twd_vocab.$$; fi
rm -f /tmp/twd_vocab.$$

# ---- 6. every RED arm names candidate A and candidate B ----
awk '
  /^#### /{h=$0; cs=0; a=0; b=0}
  /\*\*CANDIDATE SPACE\*\*/{cs=1}
  cs&&/\*\*A:\*\*/{a=1} cs&&/\*\*B:\*\*/{b=1} cs&&/`CS@/{a=1;b=1}
  /^- \*\*Fixture facts\*\*/{ if (h ~ /RED/ && cs && !(a&&b)) printf "FAIL  %s lacks a named candidate pair\n", h; cs=0 }
' "$DOC" > /tmp/twd_cs.$$ || true
if [ -s /tmp/twd_cs.$$ ]; then fail=1; cat /tmp/twd_cs.$$; fi
rm -f /tmp/twd_cs.$$

[ "$fail" -eq 0 ] && say "OK — all classes clean" || say "CHECKER REPORTED FAILURES"
exit "$fail"
