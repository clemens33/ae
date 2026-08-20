#!/opt/homebrew/bin/bash
# Per-fixture inactive-equivalence proofs for the two remaining shims:
#   E: the flock delegate-and-log spy      (compact fixture)
#   F: the git delegate-log-fail shim in its DELEGATING mode (managed end fixture)
# Plus a KNOWN-DIFFERENCE control per fixture so the comparator is shown to
# discriminate before either "no differences" reading is trusted.
set -uo pipefail
source /tmp/aelx/lib/sandbox.sh
NORM=/tmp/aelx/lib/norm.sed
norm() { sed -f "$NORM" "$1" 2>/dev/null; }

fixture_compact() { # <arm> <flock|noflock> [extraflag]
    local arm="$1" fl="$2" extra="${3:-}"
    local R="/tmp/aelx/EQUIV/$arm"
    { l_mksandbox EQUIV "$arm" frozen; l_config "$R" claude; l_mkrepo "$R"
      [[ "$fl" == flock ]] && { cp /tmp/aelx/lib/flockshim.sh "$R/b/flock"; chmod 0755 "$R/b/flock"; }
    } >"/tmp/aelx/EQUIV.$arm.build.log" 2>&1
    local -a e=(); mapfile -t e < <(l_env "$R" "AE_L_FLOCK_LOG=$R/cap/flock.log")
    local sock; sock="$(l_sock "$R")"
    ( cd "$R/w" && env -i "${e[@]}" "$R/b/ae" --local eq1 ) >"$R/cap/1launch.out" 2>"$R/cap/1launch.err"; echo $? >"$R/cap/1launch.rc"
    sleep 3
    l_manifest "$R/h/.ae" "$R/cap/m.before.tsv"; l_tmuxsnap "$sock" "$R/cap/t.before.txt"
    ( cd "$R/w" && env -i "${e[@]}" "$R/b/ae" compact -f --digest-only $extra eq1 ) >"$R/cap/2op.out" 2>"$R/cap/2op.err"; echo $? >"$R/cap/2op.rc"
    sleep 2
    l_manifest "$R/h/.ae" "$R/cap/m.after.tsv"; l_tmuxsnap "$sock" "$R/cap/t.after.txt"
    l_teardown "$R"
    for p in "$R"/cap/{1launch.rc,2op.rc,m.before.tsv,m.after.tsv}; do [[ -s "$p" ]] || { echo "FIXTURE-INVALID $p" >&2; return 1; }; done
    return 0
}

# KNOWN-DIFFERENCE control for the compact fixture: the same fixture without
# --digest-only and under a short handover bound, so the operation demonstrably
# takes a different path. Its purpose is to prove the comparator discriminates.
fixture_compact_nodigest() { # <arm> <flock|noflock>
    local arm="$1" fl="$2"
    local R="/tmp/aelx/EQUIV/$arm"
    { l_mksandbox EQUIV "$arm" frozen; l_config "$R" claude; l_mkrepo "$R"
      [[ "$fl" == flock ]] && { cp /tmp/aelx/lib/flockshim.sh "$R/b/flock"; chmod 0755 "$R/b/flock"; }
    } >"/tmp/aelx/EQUIV.$arm.build.log" 2>&1
    local -a e=(); mapfile -t e < <(l_env "$R" "AE_L_FLOCK_LOG=$R/cap/flock.log" "AE_COMPACT_HANDOVER_SECS=3")
    local sock; sock="$(l_sock "$R")"
    ( cd "$R/w" && env -i "${e[@]}" "$R/b/ae" --local eq1 ) >"$R/cap/1launch.out" 2>"$R/cap/1launch.err"; echo $? >"$R/cap/1launch.rc"
    sleep 3
    l_manifest "$R/h/.ae" "$R/cap/m.before.tsv"; l_tmuxsnap "$sock" "$R/cap/t.before.txt"
    ( cd "$R/w" && env -i "${e[@]}" "$R/b/ae" compact -f eq1 ) >"$R/cap/2op.out" 2>"$R/cap/2op.err"; echo $? >"$R/cap/2op.rc"
    sleep 2
    l_manifest "$R/h/.ae" "$R/cap/m.after.tsv"; l_tmuxsnap "$sock" "$R/cap/t.after.txt"
    l_teardown "$R"
    return 0
}

fixture_end() { # <arm> <gitshim|nogitshim> [extraflag]
    local arm="$1" gs="$2" extra="${3:-}"
    local R="/tmp/aelx/EQUIV/$arm"
    { l_mksandbox EQUIV "$arm" frozen; l_config "$R" claude; l_mkrepo "$R"
      [[ "$gs" == gitshim ]] && { cp /tmp/aelx/lib/gitshim.sh "$R/b/git"; chmod 0755 "$R/b/git"; }
    } >"/tmp/aelx/EQUIV.$arm.build.log" 2>&1
    local -a e=(); mapfile -t e < <(l_env "$R" "AE_L_GIT_LOG=$R/cap/git.log")
    local sock; sock="$(l_sock "$R")"
    ( cd "$R/w" && env -i "${e[@]}" "$R/b/ae" --worktree eq1 ) >"$R/cap/1launch.out" 2>"$R/cap/1launch.err"; echo $? >"$R/cap/1launch.rc"
    sleep 3
    printf 'work\n' >"$R/h/.ae/worktrees/eq1/wip.txt" 2>/dev/null
    l_manifest "$R/h/.ae" "$R/cap/m.before.tsv"; l_tmuxsnap "$sock" "$R/cap/t.before.txt"
    ( cd "$R/w" && env -i "${e[@]}" "$R/b/ae" end -f $extra eq1 ) >"$R/cap/2op.out" 2>"$R/cap/2op.err"; echo $? >"$R/cap/2op.rc"
    sleep 1
    l_manifest "$R/h/.ae" "$R/cap/m.after.tsv"; l_tmuxsnap "$sock" "$R/cap/t.after.txt"
    l_teardown "$R"
    for p in "$R"/cap/{1launch.rc,2op.rc,m.before.tsv,m.after.tsv}; do [[ -s "$p" ]] || { echo "FIXTURE-INVALID $p" >&2; return 1; }; done
    return 0
}

compare() { # <label> <armA> <armB> <out>
    local label="$1" A="/tmp/aelx/EQUIV/$2" B="/tmp/aelx/EQUIV/$3" out="$4" ndiff=0
    { printf '# equivalence compare: %s\n# A=%s\n# B=%s\n' "$label" "$A" "$B"
      local f
      for f in 1launch 2op; do
        printf '\n## %s.rc  A=%s B=%s  %s\n' "$f" "$(cat "$A/cap/$f.rc")" "$(cat "$B/cap/$f.rc")" \
          "$( [[ "$(cat "$A/cap/$f.rc")" == "$(cat "$B/cap/$f.rc")" ]] && echo SAME || echo DIFF )"
        printf '## %s.stdout normalized diff\n' "$f"; diff <(norm "$A/cap/$f.out") <(norm "$B/cap/$f.out") || ndiff=1
        printf '## %s.stderr normalized diff\n' "$f"; diff <(norm "$A/cap/$f.err") <(norm "$B/cap/$f.err") || ndiff=1
      done
      for f in m.before m.after; do
        printf '\n## %s STRUCTURAL diff (path/type/mode/link)\n' "$f"
        diff <(norm "$A/cap/$f.tsv" | cut -f1-6) <(norm "$B/cap/$f.tsv" | cut -f1-6) || ndiff=1
      done
      for f in t.before t.after; do
        printf '\n## %s normalized diff\n' "$f"; diff <(norm "$A/cap/$f.txt") <(norm "$B/cap/$f.txt") || ndiff=1
      done
      printf '\n## comparator verdict: %s\n' "$( ((ndiff)) && echo DIFFERENCES_PRESENT || echo NO_DIFFERENCES )"
    } >"$out" 2>&1
}

fixture_compact cp-noflock1 noflock || exit 1
fixture_compact cp-withflk1 flock   || exit 1
fixture_compact_nodigest cp-knowndf1 noflock || exit 1
fixture_end     en-nogitsh1 nogitshim || exit 1
fixture_end     en-withgit1 gitshim   || exit 1
fixture_end     en-knowndf1 nogitshim --purge-history || exit 1
mkdir -p /tmp/aelx/cap
compare "E: FLOCK-SPY equivalence (compact fixture, frozen, shim vs no shim)"                 cp-noflock1 cp-withflk1 /tmp/aelx/cap/equiv-E-flock-spy.txt
compare "E-control: KNOWN DIFFERENCE on the same fixture (no --digest-only, short handover bound)"              cp-noflock1 cp-knowndf1 /tmp/aelx/cap/equiv-E-known-difference.txt
compare "F: GIT-SHIM equivalence in DELEGATING mode (managed end fixture, shim vs no shim)"   en-nogitsh1 en-withgit1 /tmp/aelx/cap/equiv-F-git-shim.txt
compare "F-control: KNOWN DIFFERENCE on the same fixture (--purge-history added)"             en-nogitsh1 en-knowndf1 /tmp/aelx/cap/equiv-F-known-difference.txt
for f in E-flock-spy E-known-difference F-git-shim F-known-difference; do
  printf '%-22s %s\n' "$f" "$(grep '## comparator verdict' /tmp/aelx/cap/equiv-$f.txt)"
done
