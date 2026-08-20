#!/opt/homebrew/bin/bash
# Per-fixture inactive-equivalence for the ssh and rsync delegate-and-log shims,
# on the TRANSFER fixture, with its own KNOWN-DIFFERENCE control.
set -uo pipefail
source /tmp/aelx/lib/sandbox.sh
NORM=/tmp/aelx/lib/norm.sed
norm() { sed -f "$NORM" "$1" 2>/dev/null; }

fixture_transfer() { # <arm> <shims|noshims> <validname|hostilename>
    local arm="$1" sh="$2" nm="$3"
    local R="/tmp/aelx/EQUIV/$arm"
    { l_mksandbox EQUIV "$arm" frozen; l_config "$R" claude; l_mkrepo "$R"; } >"/tmp/aelx/EQUIV.$arm.build.log" 2>&1
    if [[ "$sh" == shims ]]; then
        cp /tmp/aelx/lib/sshshim.sh "$R/b/ssh"; cp /tmp/aelx/lib/rsyncshim.sh "$R/b/rsync"
        chmod 0755 "$R/b/ssh" "$R/b/rsync"
    fi
    local -a e=(); mapfile -t e < <(l_env "$R" "AE_L_SSH_LOG=$R/cap/ssh.log" "AE_L_RSYNC_LOG=$R/cap/rsync.log")
    local sock; sock="$(l_sock "$R")"
    ( cd "$R/w" && env -i "${e[@]}" "$R/b/ae" --local tf1 ) >"$R/cap/1launch.out" 2>"$R/cap/1launch.err"; echo $? >"$R/cap/1launch.rc"
    sleep 2
    ( cd "$R/w" && env -i "${e[@]}" "$R/b/ae" stop -y tf1 ) >"$R/cap/1blaunch.out" 2>"$R/cap/1blaunch.err"; echo $? >"$R/cap/1blaunch.rc"
    sleep 2
    l_manifest "$R/h/.ae" "$R/cap/m.before.tsv"; l_tmuxsnap "$sock" "$R/cap/t.before.txt"
    local NAME=tf1
    [[ "$nm" == hostilename ]] && NAME='../victim'
    ( cd "$R/w" && env -i "${e[@]}" "$R/b/ae" transfer "$NAME" nosuchpeer.invalid -y ) >"$R/cap/2op.out" 2>"$R/cap/2op.err"; echo $? >"$R/cap/2op.rc"
    ( cd "$R/w" && env -i "${e[@]}" "$R/b/ae" transfer "$NAME" nosuchpeer.invalid --pull -y ) >"$R/cap/3op.out" 2>"$R/cap/3op.err"; echo $? >"$R/cap/3op.rc"
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
      printf '\n## comparator verdict: %s\n' "$( ((nd)) && echo DIFFERENCES_PRESENT || echo NO_DIFFERENCES )"
    } >"$out" 2>&1
}

fixture_transfer tfr-noshim01 noshims validname   || exit 1
fixture_transfer tfr-shimon01 shims   validname   || exit 1
fixture_transfer tfr-knowndf1 noshims hostilename || exit 1
mkdir -p /tmp/aelx/cap
compare "K: SSH+RSYNC delegate-and-log shim equivalence (transfer fixture, valid name, shims vs no shims)" tfr-noshim01 tfr-shimon01 /tmp/aelx/cap/equiv-K-ssh-rsync-shims.txt
compare "K-control: KNOWN DIFFERENCE on the same fixture (a hostile session name instead of a valid one)" tfr-noshim01 tfr-knowndf1 /tmp/aelx/cap/equiv-K-known-difference.txt
for f in K-ssh-rsync-shims K-known-difference; do
  printf '%-22s %s\n' "$f" "$(grep '## comparator verdict' /tmp/aelx/cap/equiv-$f.txt)"
done
echo "--- shim logs from the shims arm (proof the shims were REACHED at all) ---"
wc -l /tmp/aelx/EQUIV/tfr-shimon01/cap/ssh.log /tmp/aelx/EQUIV/tfr-shimon01/cap/rsync.log 2>&1
