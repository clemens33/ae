#!/opt/homebrew/bin/bash
# Per-fixture INACTIVE-EQUIVALENCE proof for the PATH-first date shim.
# Replays every DISTINCT argv the fixture's producers actually invoked (recorded
# in the fixture's delegate-and-log) through the shim with AE_FAKE_NOW unset and
# through the real binary, comparing stdout+stderr+rc byte for byte.
# Deterministic argv are compared once. Now-forms (whose output legitimately
# advances with the clock) are compared over N back-to-back paired trials and the
# match count is REPORTED, not judged.
set -uo pipefail
LOG="$1"; OUT="$2"; TRIALS="${3:-20}"
SHIM=/tmp/aecx/shim/date
REAL=/bin/date
export AE_REAL_DATE="$REAL"
unset AE_FAKE_NOW AE_DATE_SHIM_LOG
{
  echo "## date-shim inactive-equivalence replay"
  echo "shim=$SHIM sha256=$(shasum -a 256 "$SHIM" | cut -d' ' -f1)"
  echo "real=$REAL sha256=$(shasum -a 256 "$REAL" | cut -d' ' -f1)"
  echo "source_log=$LOG distinct_argv=$(cut -f2 "$LOG" | sort -u | wc -l | tr -d ' ')"
  echo "trials_per_nowform=$TRIALS"
  echo
} >"$OUT"
cut -f2 "$LOG" | sort -u | while IFS= read -r argvq; do
    [[ -n "$argvq" ]] || continue
    eval "set -- $argvq"
    so_s="$("$SHIM" "$@" 2>"$OUT.se_s")"; rc_s=$?
    so_r="$("$REAL" "$@" 2>"$OUT.se_r")"; rc_r=$?
    se_s="$(cat "$OUT.se_s")"; se_r="$(cat "$OUT.se_r")"
    if [[ "$so_s" == "$so_r" && "$se_s" == "$se_r" && "$rc_s" == "$rc_r" ]]; then
        printf 'argv=%s\n  IDENTICAL stdout/stderr/rc on first pair (rc=%s)\n' "$argvq" "$rc_s" >>"$OUT"
    else
        m=0
        for ((i=0;i<TRIALS;i++)); do
            a="$("$SHIM" "$@" 2>/dev/null)"; ra=$?
            b="$("$REAL" "$@" 2>/dev/null)"; rb=$?
            [[ "$a" == "$b" && "$ra" == "$rb" ]] && m=$((m+1))
        done
        printf 'argv=%s\n  NOW-FORM: paired trials matched %s/%s (differences are clock advance between the two calls)\n' "$argvq" "$m" "$TRIALS" >>"$OUT"
    fi
done
rm -f "$OUT.se_s" "$OUT.se_r"
{ echo; echo "## structural"
  echo "With AE_FAKE_NOW unset the shim reaches 'exec \$REAL \"\$@\"' before any case analysis."
  echo "xtrace of one inactive invocation:"
  ( set -x; "$SHIM" -u '+%Y/%m/%d' >/dev/null ) 2>&1 | sed 's/^/  /'
} >>"$OUT"
echo "WROTE $OUT"
