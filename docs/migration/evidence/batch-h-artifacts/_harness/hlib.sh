#!/opt/homebrew/bin/bash
# Batch H harness library. Pre-registered: this file is committed and hash-recorded in
# RUN-MANIFEST.md before any arm using it runs, and any amendment reopens the arms that
# ran under the previous hash.
#
# The instrument contract this implements, from the approved design:
#   - every measured invocation is BOUNDED; a timeout is its own artifact, never a rc
#   - controller CANARIES fire BEFORE and AFTER every measured invocation, through the
#     exact capture wrapper, and both must pass for the observation to be admissible
#   - surface-state.txt records the SOURCE state of the helper under test
#   - the xtrace twin is controller-only and admissible per helper only where proven inert
set -uo pipefail

SCRATCH=/private/tmp/claude-501/-Users-ckriech-projects-clemens33-ae-rust/347d2089-7268-421d-8188-8924e246bbf0/scratchpad
HARNESS_BASH=/opt/homebrew/bin/bash
FROZEN_SHA=72c7293
FROZEN_AE="$SCRATCH/frozen/ae"
FAKE_BIN=/tmp/aecx/bin/aefake
ADEST=/Users/ckriech/projects/clemens33/ae-rust/docs/migration/evidence/batch-h-artifacts
XFD=250            # BASH_XTRACEFD, clear of every measured fd in the frozen script

sha() { shasum -a 256 "$1" 2>/dev/null | cut -d' ' -f1; }

led() { # <event> [k=v ...]
    LED_SEQ=$((${LED_SEQ:-0} + 1))
    printf 'seq=%03d\tutc=%s\tepoch=%s\tevent=%s' "$LED_SEQ" \
        "$(/bin/date -u +%Y-%m-%dT%H:%M:%SZ)" "$(/bin/date -u +%s)" "$1" >>"$LEDGER"
    shift; for kv in "$@"; do printf '\t%s' "$kv" >>"$LEDGER"; done
    printf '\n' >>"$LEDGER"
}

case_open() { # <arm> <case-id>
    CASE_DIR="$ADEST/$1/$2"; mkdir -p "$CASE_DIR/out"
    LEDGER="$CASE_DIR/admissibility-ledger.txt"; : >"$LEDGER"; LED_SEQ=0
    led case-OPEN "arm=$1" "case=$2" "frozen_sha=$FROZEN_SHA" "frozen_ae_sha256=$(sha "$FROZEN_AE")"
    printf 'label\trc\tstdout_sha256\tstdout_bytes\tstderr_sha256\tstderr_bytes\tbound_s\ttimed_out\targv\n' \
        >"$CASE_DIR/invocations.tsv"
}

# The SOURCE state of the surface under test, read from the filesystem as the invoking uid.
# ABSENT and PRESENT-BUT-UNREADABLE are different rows here even where a consumer that
# renders neither cannot tell them apart.
surface_state() { # <path-to-surface> [more paths...]
    local out="$CASE_DIR/surface-state.txt" p rc err
    { echo "## source state of the surfaces under test, read from the filesystem"
      for p in "$@"; do
          echo "### $p"
          echo "  exists=$( [[ -e "$p" ]] && echo yes || echo no )"
          if [[ -e "$p" ]]; then
              echo "  type=$( [[ -L "$p" ]] && echo symlink || { [[ -d "$p" ]] && echo dir || echo file; } )"
              echo "  mode=$(stat -f %Lp "$p" 2>/dev/null || echo -)"
              echo "  size=$(stat -f %z "$p" 2>/dev/null || echo -)"
              echo "  interpreter=$(head -1 "$p" 2>/dev/null | grep '^#!' || echo '<none>')"
              echo "  sources_lib=$(grep -c 'source .*_lib' "$p" 2>/dev/null | tr -d ' ')"
          else
              echo "  type=ABSENT"; echo "  mode=-"; echo "  size=-"
          fi
          err="$(cat "$p" 2>&1 >/dev/null)"; rc=$?
          echo "  read_attempt_rc=$rc"
          echo "  read_attempt_stderr=${err:-<none>}"
          [[ $rc -eq 0 && -e "$p" ]] && echo "  sha256=$(sha "$p")" || echo "  sha256=- (not readable by this uid)"
      done; } >"$out"
    led source-state-captured "artifact_sha256=$(sha "$out")" "paths=$*"
}

# THE CAPTURE WRAPPER. Everything measured goes through this one function, so the canaries
# that test it test the same path the product runs through. Bounded; a timeout writes its
# own artifact and is never reported as a product rc.
capture() { # <label> <bound-seconds> -- <argv...>
    local label="$1" bound="$2"; shift 3
    local o="$CASE_DIR/out/$label.stdout" e="$CASE_DIR/out/$label.stderr"
    local rc timed_out=no t0 t1
    t0=$(/bin/date -u +%s)
    ( "$@" >"$o" 2>"$e" ) & local pid=$!
    local waited=0
    while kill -0 "$pid" 2>/dev/null && (( waited < bound )); do sleep 0.5; waited=$((waited + 1)); done
    if kill -0 "$pid" 2>/dev/null; then
        timed_out=yes; kill -TERM "$pid" 2>/dev/null; sleep 0.5; kill -KILL "$pid" 2>/dev/null
        rc="-"
        { echo "## BOUND EXPIRED — this is an artifact of the bound, not a product rc"
          echo "label=$label"; echo "bound_seconds=$bound"
          echo "elapsed_seconds=$(( $(/bin/date -u +%s) - t0 ))"
          echo "argv=$*"
          echo "outcome=INCONCLUSIVE for this invocation"; } >"$CASE_DIR/out/$label.BOUND-EXPIRED.txt"
        led BOUND-EXPIRED "label=$label" "bound_s=$bound" "outcome=INCONCLUSIVE"
    else
        wait "$pid"; rc=$?
    fi
    t1=$(/bin/date -u +%s)
    printf '%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\t%s\n' "$label" "$rc" \
        "$(sha "$o")" "$(stat -f %z "$o" 2>/dev/null || echo 0)" \
        "$(sha "$e")" "$(stat -f %z "$e" 2>/dev/null || echo 0)" \
        "$bound" "$timed_out" "$*" >>"$CASE_DIR/invocations.tsv"
    led invocation "label=$label" "rc=$rc" "elapsed_s=$((t1 - t0))" "timed_out=$timed_out" \
        "stdout_sha256=$(sha "$o")" "stderr_sha256=$(sha "$e")"
    return 0
}

# CONTROLLER CANARY — known stdout bytes, known stderr bytes, known rc, pushed through the
# EXACT wrapper above. It tests the equipment and says nothing about the product. Fired
# BEFORE and AFTER every measured invocation: a post-only canary cannot show the capture
# path was live while the product ran.
canary() { # <when: pre|post> <tag>
    # Split assignments deliberately: bash expands EVERY word of a single `local`
    # statement before any of its assignments take effect, so `local when="$1"
    # label="...$when..."` reads the OLD (here: unset) `when` and aborts under set -u.
    # This is the hazard AGENTS.md documents for `export HOME=... AE_HOME="$HOME/..."`,
    # and it cost this arm its first run.
    local when="$1"
    local tag="$2"
    local label="canary-$when-$tag"
    local token="AE-H-CANARY-$when-$tag-9f3c"
    capture "$label" 10 -- "$HARNESS_BASH" -c \
        "printf '%s\n' '$token'; printf '%s\n' '$token-stderr' >&2; exit 7"
    local o="$CASE_DIR/out/$label.stdout" e="$CASE_DIR/out/$label.stderr"
    local rc_seen; rc_seen="$(awk -F'\t' -v l="$label" '$1==l{print $2}' "$CASE_DIR/invocations.tsv" | tail -1)"
    local ok=yes
    grep -q "^$token\$" "$o" 2>/dev/null || ok=no
    grep -q "^$token-stderr\$" "$e" 2>/dev/null || ok=no
    [[ "$rc_seen" == 7 ]] || ok=no
    led canary "when=$when" "tag=$tag" "stdout_carried=$(grep -c "^$token\$" "$o" 2>/dev/null | tr -d ' ')" \
        "stderr_carried=$(grep -c "^$token-stderr\$" "$e" 2>/dev/null | tr -d ' ')" \
        "rc_seen=$rc_seen" "rc_expected=7" "pass=$ok"
    [[ "$ok" == yes ]] || { led HARNESS-ABORT "reason=capture canary failed $when $tag"; return 1; }
    return 0
}

# Bracket a measured invocation with both canaries. The PRE canary completes before
# PRODUCT-START and the POST canary begins after PRODUCT-COMPLETE, both content-bound to
# this case in the append-only ledger.
measured() { # <tag> <label> <bound> -- <argv...>
    local tag="$1"; shift
    canary pre "$tag" || return 1
    led PRODUCT-START "label=$2"
    capture "$@"
    led PRODUCT-COMPLETE "label=$2"
    canary post "$tag" || return 1
    return 0
}
