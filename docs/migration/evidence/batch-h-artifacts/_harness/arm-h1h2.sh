#!/opt/homebrew/bin/bash
# A-H1 — SC-012b (help spellings) and SC-014 (version spellings).
# A-H2 — SC-013, the `steward` help and detach spellings.
#
# Each spelling is invoked SEPARATELY into its own capture: one shared capture cannot show
# a divergence between spellings, which is what these rows are about. The unknown-option
# and non-option classes are OUT-OF-BATCH (SC-022 and the launch path) and are not here.
# For SC-013 only the help and detach spellings are in scope; --init, --attach, a bare
# steward and the `hub` alias belong to SC-932/931/930/939f.
set -uo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
source "$HERE/hlib.sh"
source "$HERE/hfix.sh"
mkdir -p "$ADEST/A-H1" "$ADEST/A-H2"

h_sandbox h1 "cl:lead" "" || exit 1
BASE_HOME="$AE_HOME"; BASE_WORK="$ROOT/work"

dir_bytes() { ( cd "$1" 2>/dev/null && find . -type f -print0 | sort -z | while IFS= read -r -d '' f; do
    printf '%s\t%s\n' "$f" "$(shasum -a 256 "$f" 2>/dev/null | cut -d' ' -f1)"; done ); }

dcase() { # <arm> <case-id> <bound> [argv...]
    local arm="$1" cid="$2" bound="$3"; shift 3
    ARM="$arm"
    case_open "$arm" "$cid"
    led rows "rows=$( [[ $arm == A-H1 ]] && echo 'SC-012b/SC-014' || echo 'SC-013' )" \
        "surface=frozen ae dispatcher" "spelling_invoked_separately=yes"
    surface_state "$FROZEN_AE"
    dir_bytes "$BASE_HOME" >"$CASE_DIR/home-bytes.before.tsv"
    led measured-input "argv=ae $*" "bound_s=$bound"
    run_in "$BASE_WORK" measured "$cid" "ae" "$bound" -- "$HARNESS_BASH" "$FROZEN_AE" "$@"
    dir_bytes "$BASE_HOME" >"$CASE_DIR/home-bytes.after.tsv"
    diff "$CASE_DIR/home-bytes.before.tsv" "$CASE_DIR/home-bytes.after.tsv" \
        >"$CASE_DIR/home-bytes.diff.txt" 2>&1
    led write-record "changed_lines=$(wc -l <"$CASE_DIR/home-bytes.diff.txt" | tr -d ' ')"
    led case-CLOSE "invocations_sha256=$(sha "$CASE_DIR/invocations.tsv")"
    echo "  $cid"
}

########## A-H1 — help and version spellings, each on its own ##########
dcase A-H1 h1-c01-dash-h        20 -h
dcase A-H1 h1-c02-help-long     20 --help
dcase A-H1 h1-c03-help-word     20 help
dcase A-H1 h1-c04-version-word  20 version
dcase A-H1 h1-c05-version-long  20 --version
dcase A-H1 h1-c06-version-short 20 -V

########## A-H2 — steward: the help and detach spellings SC-013 owns ##########
dcase A-H2 h2-c01-dash-h            20 steward -h
dcase A-H2 h2-c02-help-long         20 steward --help
dcase A-H2 h2-c03-help-word         20 steward help
dcase A-H2 h2-c04-help-trailing     20 steward --help extra args
dcase A-H2 h2-c05-detach            30 steward --detach
dcase A-H2 h2-c06-no-attach         30 steward --no-attach
dcase A-H2 h2-c07-detach-then-attach 30 steward --detach --attach
dcase A-H2 h2-c08-attach-then-detach 30 steward --attach --detach
dcase A-H2 h2-c09-detach-repeated   30 steward --detach --detach
# teardown: anything the detach cases started is reaped, and what was reaped is recorded
{ echo "## processes under this fixture's AE_HOME at teardown"
  ps -eo pid=,ppid=,command= | grep -F "$BASE_HOME" | grep -v grep || echo "(none)"; } \
  >"$ADEST/A-H2/teardown-processes.txt"
pkill -f "$BASE_HOME" >/dev/null 2>&1
echo "A-H1/A-H2 DONE"
