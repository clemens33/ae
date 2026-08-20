#!/opt/homebrew/bin/bash
# ARM GROUP A5 — SC-514, doctor exits under a CONTROLLED PATH / capability fixture.
#
# The controlled bin dir is a directory of SYMLINKS to every executable on the standard
# search path, and PATH is that directory ALONE. A planted arm removes exactly ONE symlink,
# and the arm proves it removed exactly one by diffing the bin listing against the clean
# arm's. The interpreter is invoked by ABSOLUTE path, so no removal can take it away —
# including the arm that deliberately removes `bash` from the controlled dir to show that.
source "$(dirname "$0")/armlib.sh"
ARMG=A5
mkdir -p "$ADEST/$ARMG"
printf 'case\trows\tgroup\tmember\n' >"$ADEST/$ARMG/ledger.tsv"
BINSRC=/tmp/aecx/a5bin
build_binsrc() {
    rm -rf "$BINSRC"; mkdir -p "$BINSRC"
    local d f n=0
    for d in /usr/bin /bin /usr/sbin /sbin /opt/homebrew/bin; do
        [[ -d "$d" ]] || continue
        for f in "$d"/*; do
            [[ -x "$f" && ! -d "$f" ]] || continue
            local b; b="$(basename "$f")"
            [[ -e "$BINSRC/$b" ]] && continue
            ln -s "$f" "$BINSRC/$b" && n=$((n+1))
        done
    done
    echo "$n"
}

a5_case() { # <case-id> <removed-tool-or-none> <capability: none|no-config>
    local cid="$1" removed="$2" cap="$3"
    local base="$AROOT/$ARMG/$cid"
    [[ -e "$base" ]] && chmod -R u+w "$base" 2>/dev/null; rm -rf "$base"; mkdir -p "$base/home"
    export ARM_TMUXTMP="$base/tmuxtmp"; mkdir -p "$ARM_TMUXTMP"
    case_open "$ARMG" "$cid" "controlled-path"
    led rows "rows=SC-514" "template=A5/doctor-fixture" "removed_tool=${removed}" "capability=${cap}"
    local aehome="$base/home/.ae"
    t_clone A5 doctor-fixture "$aehome" rw || { led CLONE-FAILED; return 1; }
    local cf exp
    cf="$(dir_fingerprint "$aehome")"; exp="$(grep '^fingerprint_pre_protection=' "$TSTORE/A5/_meta/doctor-fixture.txt" | cut -d= -f2-)"
    led clone-VERIFIED "clone_fingerprint=$cf" "expected=$exp" "matches=$( [[ "$cf" == "$exp" ]] && echo yes || echo no )"
    # the controlled bin dir for THIS arm
    local bin="$base/bin"
    cp -R "$BINSRC" "$bin"
    if [[ "$removed" != none ]]; then
        [[ -e "$bin/$removed" ]] || { led HARNESS-ABORT "reason=cannot plant: '$removed' is not in the controlled bin dir"; return 1; }
        rm -f "$bin/$removed"
    fi
    ( cd "$bin" && ls -1 ) >"$ACAP/bin-listing.txt"
    led bin-listing "entries=$(wc -l <"$ACAP/bin-listing.txt" | tr -d ' ')" \
        "sha256=$(sha "$ACAP/bin-listing.txt")" "removed=$removed"
    if [[ "$cap" == no-config ]]; then
        mv "$aehome/config" "$base/config.moved-aside"
        led capability-manipulation "moved the config file aside" "from=config to=../config.moved-aside"
    fi
    { echo "arm=$ARMG case=$cid rows=SC-514 template=A5/doctor-fixture clone_mode=controlled-path"
      echo "clone_fingerprint=$cf clone_fingerprint_matches_template=$( [[ "$cf" == "$exp" ]] && echo yes || echo no )"
      echo "controlled_bin_dir=$bin (symlinks only; PATH is this directory ALONE)"
      echo "removed_tool=$removed"
      echo "capability_manipulation=$cap"
      echo "interpreter=$HARNESS_BASH invoked by ABSOLUTE path, so no removal can take it away"
      echo "frozen_sha=$FROZEN_SHA frozen_ae_sha256=$(sha "$FROZEN_AE")"
      echo "utc_start=$(/bin/date -u +%Y-%m-%dT%H:%M:%SZ)"; } >"$ACAP/case.txt"
    case_env_record "$aehome" ""
    env_tab_selfcheck || { led HARNESS-ABORT "reason=tab self-check"; return 1; }
    dir_manifest "$aehome" >"$ACAP/manifest.before.tsv"
    led manifest-before "artifact_sha256=$(sha "$ACAP/manifest.before.tsv")"
    # the consumer, with PATH = the controlled dir ALONE
    local -a pre=(env -i "HOME=$base/home" "AE_HOME=$aehome" "PATH=$bin"
        "TZ=UTC" "LANG=en_US.UTF-8" "LC_ALL=en_US.UTF-8" "TERM=xterm-256color"
        "TMUX_TMPDIR=$ARM_TMUXTMP")
    printf '%s\n' "${pre[@]}" >"$ACAP/env.txt"
    mkdir -p "$ACAP/out"
    led consumer-START "label=doctor" "argv=$HARNESS_BASH $FROZEN_AE doctor" "PATH=$bin"
    local rc=0
    ( cd "$base" && "${pre[@]}" "$HARNESS_BASH" "$FROZEN_AE" doctor </dev/null ) \
        >"$ACAP/out/doctor.stdout" 2>"$ACAP/out/doctor.stderr" || rc=$?
    printf 'doctor\t%s\t%s\t%s\t%s\t%s\t-\t-\t-\t%s\n' "$rc" \
        "$(sha "$ACAP/out/doctor.stdout")" "$(stat -f %z "$ACAP/out/doctor.stdout")" \
        "$(sha "$ACAP/out/doctor.stderr")" "$(stat -f %z "$ACAP/out/doctor.stderr")" \
        "ae doctor under PATH=$bin" >>"$ACAP/consumers.tsv"
    led consumer-COMPLETE "label=doctor" "rc=$rc" "stdout_sha256=$(sha "$ACAP/out/doctor.stdout")"
    # the checklist as the product printed it, lifted verbatim
    # the product prints "OK   <name>  <detail>", NOT "[OK]" — a bracketed pattern silently
    # captured only the Summary line on the first run, which is the lossy-filter class again
    grep -E '^(OK|WARN|FAIL)[[:space:]]|^Summary:' "$ACAP/out/doctor.stdout" >"$ACAP/checklist.txt" 2>/dev/null || true
    led checklist "artifact_sha256=$(sha "$ACAP/checklist.txt")" \
        "lines=$(wc -l <"$ACAP/checklist.txt" | tr -d ' ')"
    dir_manifest "$aehome" >"$ACAP/manifest.after.tsv"
    led manifest-after "artifact_sha256=$(sha "$ACAP/manifest.after.tsv")"
    diff "$ACAP/manifest.before.tsv" "$ACAP/manifest.after.tsv" >"$ACAP/manifest.diff.txt" 2>&1
    { echo "manifest_diff_lines=$(wc -l <"$ACAP/manifest.diff.txt" | tr -d ' ')"
      echo "doctor_rc=$rc"
      echo "utc_end=$(/bin/date -u +%Y-%m-%dT%H:%M:%SZ)"; } >>"$ACAP/case.txt"
    led case-CLOSE "case_txt_sha256=$(sha "$ACAP/case.txt")" "consumers_tsv_sha256=$(sha "$ACAP/consumers.tsv")"
    echo "  $cid (removed=$removed cap=$cap): doctor rc=$rc"
}
N="$(build_binsrc)"
echo "controlled bin dir built: $N symlinks"
reg() { printf '%s\tSC-514\tA5\tdoctor-fixture\n' "$1" >>"$ADEST/$ARMG/ledger.tsv"; }
reg a5-clean;             a5_case a5-clean             none    none
reg a5-no-tmux;           a5_case a5-no-tmux           tmux    none
reg a5-no-git;            a5_case a5-no-git            git     none
reg a5-no-flock;          a5_case a5-no-flock          flock   none
reg a5-no-timeout;        a5_case a5-no-timeout        timeout none
reg a5-no-tail;           a5_case a5-no-tail           tail    none
reg a5-no-bash-on-path;   a5_case a5-no-bash-on-path   bash    none
reg a5-no-config;         a5_case a5-no-config         none    no-config
echo "A5 DONE"
