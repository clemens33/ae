#!/opt/homebrew/bin/bash
# Admissibility proof runner: (A) inactive-hook equivalence, (B) tmux-shim
# equivalence, (D) KNOWN-DIFFERENCE control that proves the comparator
# discriminates at all (AGENTS.md: verify the instrument on a known failure
# before trusting it about an unknown one).
set -uo pipefail
source /tmp/aelx/lib/sandbox.sh
NORM=/tmp/aelx/lib/norm.sed
norm() { sed -f "$NORM" "$1" 2>/dev/null; }

run_fixture() { # <arm> <frozen|instrumented> <shim|noshim> <endflag>
    local arm="$1" which="$2" shim="$3" endflag="${4:-}"
    local R="/tmp/aelx/EQUIV/$arm"
    {
        l_mksandbox EQUIV "$arm" "$which"
        l_config "$R" claude
        l_mkrepo "$R"
        [[ "$shim" == shim ]] && l_install_tmux_shim "$R"
    } >"/tmp/aelx/EQUIV.$arm.build.log" 2>&1
    local -a extra=()
    [[ "$shim" == shim ]] && extra=("AE_L_TMUX_LOG=$R/cap/tmux-argv.log")
    local -a e=(); mapfile -t e < <(l_env "$R" ${extra[@]+"${extra[@]}"})
    local sock; sock="$(l_sock "$R")"
    ( cd "$R/w" && env -i "${e[@]}" "$R/b/ae" --local eq1 ) >"$R/cap/1launch.out" 2>"$R/cap/1launch.err"; echo $? >"$R/cap/1launch.rc"
    sleep 2
    l_manifest "$R/h/.ae" "$R/cap/m.before.tsv"
    l_tmuxsnap "$sock" "$R/cap/t.before.txt"
    ( cd "$R/w" && env -i "${e[@]}" "$R/b/ae" list --json ) >"$R/cap/2list.out" 2>"$R/cap/2list.err"; echo $? >"$R/cap/2list.rc"
    if [[ -n "$endflag" ]]; then
        ( cd "$R/w" && env -i "${e[@]}" "$R/b/ae" end -f "$endflag" eq1 ) >"$R/cap/3end.out" 2>"$R/cap/3end.err"; echo $? >"$R/cap/3end.rc"
    else
        ( cd "$R/w" && env -i "${e[@]}" "$R/b/ae" end -f eq1 ) >"$R/cap/3end.out" 2>"$R/cap/3end.err"; echo $? >"$R/cap/3end.rc"
    fi
    sleep 1
    l_manifest "$R/h/.ae" "$R/cap/m.after.tsv"
    l_tmuxsnap "$sock" "$R/cap/t.after.txt"
    l_teardown "$R"
    # HARD ASSERTION: a missing capture must abort, not produce a vacuous diff.
    local f
    for f in 1launch.rc 2list.rc 3end.rc m.before.tsv m.after.tsv t.before.txt t.after.txt; do
        [[ -s "$R/cap/$f" ]] || { echo "FIXTURE-INVALID: $R/cap/$f missing/empty" >&2; return 1; }
    done
    return 0
}

compare() { # <label> <armA> <armB> <out>
    local label="$1" A="/tmp/aelx/EQUIV/$2" B="/tmp/aelx/EQUIV/$3" out="$4"
    local ndiff=0
    { printf '# equivalence compare: %s\n' "$label"
      printf '# A=%s\n# B=%s\n' "$A" "$B"
      local f
      for f in 1launch 2list 3end; do
        printf '\n## %s.rc  A=%s B=%s  %s\n' "$f" "$(cat "$A/cap/$f.rc")" "$(cat "$B/cap/$f.rc")" \
          "$( [[ "$(cat "$A/cap/$f.rc")" == "$(cat "$B/cap/$f.rc")" ]] && echo SAME || echo DIFF )"
        printf '## %s.stdout normalized diff\n' "$f"
        diff <(norm "$A/cap/$f.out") <(norm "$B/cap/$f.out") || ndiff=1
        printf '## %s.stderr normalized diff\n' "$f"
        diff <(norm "$A/cap/$f.err") <(norm "$B/cap/$f.err") || ndiff=1
      done
      for f in m.before m.after; do
        printf '\n## %s STRUCTURAL diff (path/type/mode/link; hashes dropped)\n' "$f"
        diff <(norm "$A/cap/$f.tsv" | cut -f1-6) <(norm "$B/cap/$f.tsv" | cut -f1-6) || ndiff=1
      done
      for f in t.before t.after; do
        printf '\n## %s normalized diff\n' "$f"
        diff <(norm "$A/cap/$f.txt") <(norm "$B/cap/$f.txt") || ndiff=1
      done
      printf '\n## comparator verdict: %s\n' "$( ((ndiff)) && echo DIFFERENCES_PRESENT || echo NO_DIFFERENCES )"
    } >"$out" 2>&1
}

for spec in "f-noshim01 frozen noshim" "i-noshim01 instrumented noshim" "f-shimon01 frozen shim" "f-purge001 frozen noshim --purge-history"; do
    # shellcheck disable=SC2086
    run_fixture $spec || { echo "ABORT: fixture $spec invalid" >&2; exit 1; }
done
mkdir -p /tmp/aelx/cap
compare "A: INACTIVE-HOOK equivalence (frozen vs instrumented, AE_L_HOOKS unset, no shim)" f-noshim01 i-noshim01 /tmp/aelx/cap/equiv-A-inactive-hook.txt
compare "B: TMUX-SHIM equivalence (frozen no-shim vs frozen + delegate-and-log shim)"      f-noshim01 f-shimon01   /tmp/aelx/cap/equiv-B-tmux-shim.txt
compare "D: KNOWN-DIFFERENCE control (same binary, end -f vs end -f --purge-history)"      f-noshim01 f-purge001  /tmp/aelx/cap/equiv-D-known-difference.txt
for f in A-inactive-hook B-tmux-shim D-known-difference; do
  printf '%-22s %s\n' "$f" "$(grep '## comparator verdict' /tmp/aelx/cap/equiv-$f.txt)"
done
