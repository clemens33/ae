#!/opt/homebrew/bin/bash
# L-HOOKS-v2 admissibility: per-fixture INACTIVE-hook equivalence for the second
# instrumented copy, on BOTH fixtures it will be used on, each with its own
# KNOWN-DIFFERENCE control.
set -uo pipefail
source /tmp/aelx/lib/sandbox.sh
NORM=/tmp/aelx/lib/norm.sed
norm() { sed -f "$NORM" "$1" 2>/dev/null; }

mk() { # <arm> <frozen|v1|v2>
    local arm="$1" which="$2"
    local R="/tmp/aelx/EQUIV/$arm"
    { l_mksandbox EQUIV "$arm" frozen; l_config "$R" claude; l_mkrepo "$R"; } >"/tmp/aelx/EQUIV.$arm.build.log" 2>&1
    case "$which" in
        v1) cp /tmp/aelx/instr/ae  "$R/b/ae" ;;
        v2) cp /tmp/aelx/instr2/ae "$R/b/ae" ;;
        v3) cp /tmp/aelx/instr3/ae "$R/b/ae" ;;
        v5) cp /tmp/aelx/instr5/ae "$R/b/ae" ;;
    esac
    chmod 0755 "$R/b/ae"
    printf '%s\n' "$R"
}

fixture_stop() { # <arm> <frozen|v2> <-y|NOYES>
    local arm="$1" which="$2"
    local yes="$3"
    [[ "$yes" == NOYES ]] && yes=""
    local R; R="$(mk "$arm" "$which")"
    local -a e=(); mapfile -t e < <(l_env "$R")
    local sock; sock="$(l_sock "$R")"
    ( cd "$R/w" && env -i "${e[@]}" "$R/b/ae" --local st1 ) >"$R/cap/1launch.out" 2>"$R/cap/1launch.err"; echo $? >"$R/cap/1launch.rc"
    sleep 2
    ( cd "$R/w" && env -i "${e[@]}" "$R/b/ae" --local st2 ) >"$R/cap/1blaunch.out" 2>"$R/cap/1blaunch.err"; echo $? >"$R/cap/1blaunch.rc"
    sleep 2
    l_manifest "$R/h/.ae" "$R/cap/m.before.tsv"; l_tmuxsnap "$sock" "$R/cap/t.before.txt"
    ( cd "$R/w" && env -i "${e[@]}" "$R/b/ae" stop $yes st1 ) >"$R/cap/2op.out" 2>"$R/cap/2op.err"; echo $? >"$R/cap/2op.rc"
    sleep 2
    ( cd "$R/w" && env -i "${e[@]}" "$R/b/ae" stop all $yes ) >"$R/cap/3op.out" 2>"$R/cap/3op.err"; echo $? >"$R/cap/3op.rc"
    sleep 5
    l_manifest "$R/h/.ae" "$R/cap/m.after.tsv"; l_tmuxsnap "$sock" "$R/cap/t.after.txt"
    l_teardown "$R"
    local p
    for p in 1launch.rc 2op.rc 3op.rc m.before.tsv m.after.tsv; do
        [[ -s "$R/cap/$p" ]] || { echo "FIXTURE-INVALID $R/cap/$p" >&2; return 1; }
    done
    return 0
}

fixture_end() { # <arm> <frozen|v2> [endflag]
    local arm="$1" which="$2" ef="${3:-}"
    local R; R="$(mk "$arm" "$which")"
    local -a e=(); mapfile -t e < <(l_env "$R")
    local sock; sock="$(l_sock "$R")"
    ( cd "$R/w" && env -i "${e[@]}" "$R/b/ae" --local st1 ) >"$R/cap/1launch.out" 2>"$R/cap/1launch.err"; echo $? >"$R/cap/1launch.rc"
    sleep 2
    : >"$R/cap/1blaunch.out"; : >"$R/cap/1blaunch.err"; echo 0 >"$R/cap/1blaunch.rc"
    l_manifest "$R/h/.ae" "$R/cap/m.before.tsv"; l_tmuxsnap "$sock" "$R/cap/t.before.txt"
    ( cd "$R/w" && env -i "${e[@]}" "$R/b/ae" list --json ) >"$R/cap/2op.out" 2>"$R/cap/2op.err"; echo $? >"$R/cap/2op.rc"
    ( cd "$R/w" && env -i "${e[@]}" "$R/b/ae" end -f $ef st1 ) >"$R/cap/3op.out" 2>"$R/cap/3op.err"; echo $? >"$R/cap/3op.rc"
    sleep 1
    l_manifest "$R/h/.ae" "$R/cap/m.after.tsv"; l_tmuxsnap "$sock" "$R/cap/t.after.txt"
    l_teardown "$R"
    local p
    for p in 1launch.rc 2op.rc 3op.rc m.before.tsv m.after.tsv; do
        [[ -s "$R/cap/$p" ]] || { echo "FIXTURE-INVALID $R/cap/$p" >&2; return 1; }
    done
    return 0
}

compare() { # <label> <armA> <armB> <out>
    local label="$1" A="/tmp/aelx/EQUIV/$2" B="/tmp/aelx/EQUIV/$3" out="$4" nd=0
    { printf '# equivalence compare: %s\n# A=%s\n# B=%s\n' "$label" "$A" "$B"
      local f
      for f in 1launch 1blaunch 2op 3op; do
        printf '\n## %s.rc  A=%s B=%s  %s\n' "$f" "$(cat "$A/cap/$f.rc" 2>/dev/null)" "$(cat "$B/cap/$f.rc" 2>/dev/null)" \
          "$( [[ "$(cat "$A/cap/$f.rc" 2>/dev/null)" == "$(cat "$B/cap/$f.rc" 2>/dev/null)" ]] && echo SAME || echo DIFF )"
        printf '## %s.stdout normalized diff\n' "$f"; diff <(norm "$A/cap/$f.out") <(norm "$B/cap/$f.out") || nd=1
        printf '## %s.stderr normalized diff\n' "$f"; diff <(norm "$A/cap/$f.err") <(norm "$B/cap/$f.err") || nd=1
      done
      for f in m.before m.after; do
        printf '\n## %s STRUCTURAL diff (path/type/mode/link)\n' "$f"
        diff <(norm "$A/cap/$f.tsv" | cut -f1-6) <(norm "$B/cap/$f.tsv" | cut -f1-6) || nd=1
      done
      for f in t.before t.after; do
        printf '\n## %s normalized diff\n' "$f"; diff <(norm "$A/cap/$f.txt") <(norm "$B/cap/$f.txt") || nd=1
      done
      printf '\n## comparator verdict: %s\n' "$( ((nd)) && echo DIFFERENCES_PRESENT || echo NO_DIFFERENCES )"
    } >"$out" 2>&1
}

fixture_stop stp-frozen01 frozen -y     || exit 1
fixture_stop stp-v5inact1 v5     -y     || exit 1
fixture_stop stp-v2inact1 v2     -y     || exit 1
fixture_stop stp-knowndf1 frozen NOYES  || exit 1
fixture_end  end-frozen01 frozen    || exit 1
fixture_end  end-v2inact1 v2        || exit 1
fixture_end  end-knowndf1 frozen --purge-history || exit 1
mkdir -p /tmp/aelx/cap
fixture_compact() { # <arm> <frozen|v3> <digest|nodigest>
    local arm="$1" which="$2" mode="$3"
    local R; R="$(mk "$arm" "$which")"
    local -a e=(); mapfile -t e < <(l_env "$R" "AE_COMPACT_HANDOVER_SECS=3")
    local sock; sock="$(l_sock "$R")"
    ( cd "$R/w" && env -i "${e[@]}" "$R/b/ae" --local st1 ) >"$R/cap/1launch.out" 2>"$R/cap/1launch.err"; echo $? >"$R/cap/1launch.rc"
    sleep 3
    : >"$R/cap/1blaunch.out"; : >"$R/cap/1blaunch.err"; echo 0 >"$R/cap/1blaunch.rc"
    l_manifest "$R/h/.ae" "$R/cap/m.before.tsv"; l_tmuxsnap "$sock" "$R/cap/t.before.txt"
    ( cd "$R/w" && env -i "${e[@]}" "$R/b/ae" archive preview st1 ) >"$R/cap/2op.out" 2>"$R/cap/2op.err"; echo $? >"$R/cap/2op.rc"
    if [[ "$mode" == digest ]]; then
        ( cd "$R/w" && env -i "${e[@]}" "$R/b/ae" compact -f --digest-only st1 ) >"$R/cap/3op.out" 2>"$R/cap/3op.err"; echo $? >"$R/cap/3op.rc"
    else
        ( cd "$R/w" && env -i "${e[@]}" "$R/b/ae" compact -f st1 ) >"$R/cap/3op.out" 2>"$R/cap/3op.err"; echo $? >"$R/cap/3op.rc"
    fi
    sleep 3
    l_manifest "$R/h/.ae" "$R/cap/m.after.tsv"; l_tmuxsnap "$sock" "$R/cap/t.after.txt"
    l_teardown "$R"
    local p
    for p in 1launch.rc 2op.rc 3op.rc m.before.tsv m.after.tsv; do
        [[ -s "$R/cap/$p" ]] || { echo "FIXTURE-INVALID $R/cap/$p" >&2; return 1; }
    done
    return 0
}

fixture_compact cpx-frozen01 frozen digest   || exit 1
fixture_compact cpx-v3inact1 v3     digest   || exit 1
fixture_compact cpx-knowndf1 frozen nodigest || exit 1
compare "I: v3 INACTIVE-HOOK equivalence on the COMPACT fixture (frozen vs v3, AE_L_HOOKS unset)" cpx-frozen01 cpx-v3inact1 /tmp/aelx/cap/equiv-I-v3-inactive-compact.txt
compare "I-control: KNOWN DIFFERENCE on the COMPACT fixture (--digest-only dropped, short handover bound)" cpx-frozen01 cpx-knowndf1 /tmp/aelx/cap/equiv-I-known-difference.txt
compare "L: v5 INACTIVE-HOOK equivalence on the STOP fixture (frozen vs v5, AE_L_HOOKS unset)" stp-frozen01 stp-v5inact1 /tmp/aelx/cap/equiv-L-v5-inactive-stop.txt
compare "G: v2 INACTIVE-HOOK equivalence on the STOP fixture (frozen vs v2, AE_L_HOOKS unset)" stp-frozen01 stp-v2inact1 /tmp/aelx/cap/equiv-G-v2-inactive-stop.txt
compare "G-control: KNOWN DIFFERENCE on the STOP fixture (-y dropped, no terminal)"            stp-frozen01 stp-knowndf1 /tmp/aelx/cap/equiv-G-known-difference.txt
compare "H: v2 INACTIVE-HOOK equivalence on the END fixture (frozen vs v2, AE_L_HOOKS unset)"  end-frozen01 end-v2inact1 /tmp/aelx/cap/equiv-H-v2-inactive-end.txt
compare "H-control: KNOWN DIFFERENCE on the END fixture (--purge-history added)"               end-frozen01 end-knowndf1 /tmp/aelx/cap/equiv-H-known-difference.txt
for f in L-v5-inactive-stop I-v3-inactive-compact I-known-difference G-v2-inactive-stop G-known-difference H-v2-inactive-end H-known-difference; do
  printf '%-24s %s\n' "$f" "$(grep '## comparator verdict' /tmp/aelx/cap/equiv-$f.txt)"
done
