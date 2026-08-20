#!/opt/homebrew/bin/bash
# L-HOOKS-v4 admissibility: per-fixture INACTIVE-hook equivalence on the RENAME
# fixture, with its own KNOWN-DIFFERENCE control.
set -uo pipefail
source /tmp/aelx/lib/sandbox.sh
NORM=/tmp/aelx/lib/norm.sed
norm() { sed -f "$NORM" "$1" 2>/dev/null; }

fixture_rename() { # <arm> <frozen|v4> <ok|badname>
    local arm="$1" which="$2" mode="$3"
    local R="/tmp/aelx/EQUIV/$arm"
    { l_mksandbox EQUIV "$arm" frozen; l_config "$R" claude; l_mkrepo "$R"; } >"/tmp/aelx/EQUIV.$arm.build.log" 2>&1
    [[ "$which" == v4 ]] && { cp /tmp/aelx/instr4/ae "$R/b/ae"; chmod 0755 "$R/b/ae"; }
    local -a e=(); mapfile -t e < <(l_env "$R")
    local sock; sock="$(l_sock "$R")"
    ( cd "$R/w" && env -i "${e[@]}" "$R/b/ae" --local rn1 ) >"$R/cap/1launch.out" 2>"$R/cap/1launch.err"; echo $? >"$R/cap/1launch.rc"
    sleep 2
    ( cd "$R/w" && env -i "${e[@]}" "$R/b/ae" --local rn1x ) >"$R/cap/1blaunch.out" 2>"$R/cap/1blaunch.err"; echo $? >"$R/cap/1blaunch.rc"
    sleep 2
    l_manifest "$R/h/.ae" "$R/cap/m.before.tsv"; l_tmuxsnap "$sock" "$R/cap/t.before.txt"
    if [[ "$mode" == ok ]]; then
        ( cd "$R/w" && env -i "${e[@]}" "$R/b/ae" rename rn1 rn2 ) >"$R/cap/2op.out" 2>"$R/cap/2op.err"; echo $? >"$R/cap/2op.rc"
    else
        ( cd "$R/w" && env -i "${e[@]}" "$R/b/ae" rename rn1 'not a valid name' ) >"$R/cap/2op.out" 2>"$R/cap/2op.err"; echo $? >"$R/cap/2op.rc"
    fi
    sleep 1
    ( cd "$R/w" && env -i "${e[@]}" "$R/b/ae" list --json ) >"$R/cap/3op.out" 2>"$R/cap/3op.err"; echo $? >"$R/cap/3op.rc"
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

fixture_rename rnm-frozen01 frozen ok      || exit 1
fixture_rename rnm-v4inact1 v4     ok      || exit 1
fixture_rename rnm-knowndf1 frozen badname || exit 1
mkdir -p /tmp/aelx/cap
compare "J: v4 INACTIVE-HOOK equivalence on the RENAME fixture (frozen vs v4, AE_L_HOOKS unset)" rnm-frozen01 rnm-v4inact1 /tmp/aelx/cap/equiv-J-v4-inactive-rename.txt
compare "J-control: KNOWN DIFFERENCE on the RENAME fixture (a target name outside the grammar)"  rnm-frozen01 rnm-knowndf1 /tmp/aelx/cap/equiv-J-known-difference.txt
for f in J-v4-inactive-rename J-known-difference; do
  printf '%-22s %s\n' "$f" "$(grep '## comparator verdict' /tmp/aelx/cap/equiv-$f.txt)"
done
