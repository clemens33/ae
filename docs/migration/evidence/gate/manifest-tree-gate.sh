#!/opt/homebrew/bin/bash
# MANIFEST-vs-TREE gate. Fails if MANIFEST.md cites a path that is not in the tree, or if
# a published file is missing from its directory's SHA256SUMS.txt, or if any recorded hash
# no longer matches. Run before asking for a commit.
set -uo pipefail
# The tree under audit is an ARGUMENT, not a constant. Hardcoding it made the gate audit
# the live tree no matter which copy you pointed it at, so a red-proof in a scratch copy
# would report PASS with defects injected — the biased-probe class this workspace has a
# rule about. First arg wins, then $BATCH_C_ARTIFACTS, then the live tree.
A="${1:-${BATCH_C_ARTIFACTS:-/Users/ckriech/projects/clemens33/ae-rust/docs/migration/evidence/batch-c-artifacts}}"
[[ -d "$A" ]] || { echo "gate: no such tree: $A" >&2; exit 2; }
# THE GATE'S OWN COVERAGE, printed first and kept beside the checks it describes.
#
# The list of what a gate measures used to exist only as the order of its output sections,
# so nobody could look at the gate and see what it does NOT measure — and a fix that moved
# in an unmeasured dimension was invisible by construction. Reachability and chronology
# were both discovered that way, three arms apart. The second list is the more useful half:
# it is where those two would have been written down before they bit.
cat <<'DIMS'
## dimensions CHECKED by this gate
   citations       every path cited by the tree's MANIFEST resolves to something in it
   schema          every case carries the artifacts its kind requires, plus a case index
                   bound to each ledger's content
   sums            every file is listed in a SHA256SUMS and verifies against it
   committed-bytes what a fresh clone yields matches what is here, prefix derived from the
                   tree under audit
   chronology      ledger sequence identities are unique and monotonic
## dimensions KNOWN NOT TO BE CHECKED (and why)
   discrimination  whether a case's inputs could have produced a different reading. A pair
                   that cannot discriminate passes every check above. Seat review and the
                   arm's own opposed pairs carry this.
   reachability    whether the code path a case names was actually entered. The xtrace twin
                   and the guard witnesses carry it per case, not here.
   one-variable    whether a pair differs in exactly the field it claims. Held by design
                   review; two pairs have already failed it.
   semantic-fit    whether a citation points at the line that BEARS the claim beside it.
                   The pin file makes a seat's reading cheap; it does not perform it.
   liveness        whether a fixture presented the input class it names. fixture-validity
                   artifacts carry it per arm.
DIMS
echo "## tree under audit: $A"
rc=0
echo "## cited paths (multi-base resolver)"
# Every backticked path-shaped token — including slash-less file citations and wildcard
# patterns — resolved against the tree root, arms/, templates/, twd-precursor/, the repo
# root, the group/member -> fixture-bytes mapping, and the real context directories a
# relative citation can legitimately be written against. Emits PATH-CITES.tsv.
RESOLVER="$(dirname "${BASH_SOURCE[0]}")/path-cite-resolver.py"
[[ -f "$RESOLVER" ]] || RESOLVER="$A/arms/A1/harness/path-cite-resolver.py"
if [[ -f "$RESOLVER" ]]; then
    python3 "$RESOLVER" "$A" "${REPO_ROOT:-/Users/ckriech/projects/clemens33/ae-rust}" || rc=1
else
    echo "  RESOLVER MISSING — cannot check citations"; rc=1
fi
echo "## per-case artifact schema + case index"
# files==listed cannot see a file deleted TOGETHER with its SHA256SUMS line, and the
# citation resolver deduplicates relative tokens globally, so neither of the other halves
# checks per-case COMPLETENESS. This one does, from each case's own ledger-derived kind
# and from the arm's content-bound case index.
SCHEMA_CHK="$(dirname "${BASH_SOURCE[0]}")/case-schema-check.py"
SCHEMA_TSV="$(dirname "${BASH_SOURCE[0]}")/case-schema.tsv"
[[ -f "$SCHEMA_CHK" ]] || { SCHEMA_CHK="$A/arms/A1/harness/case-schema-check.py"; SCHEMA_TSV="$A/arms/A1/harness/case-schema.tsv"; }
if [[ -f "$SCHEMA_CHK" && -f "$SCHEMA_TSV" ]]; then
    python3 "$SCHEMA_CHK" "$A" "$SCHEMA_TSV" || rc=1
else
    echo "  SCHEMA CHECK MISSING — per-case completeness cannot be checked"; rc=1
fi

echo "## SHA256SUMS coverage + verification"
# EVERY directory that carries a SHA256SUMS.txt, discovered rather than listed: a
# hardcoded list silently skipped twd-precursor/, whose SUMS files went unverified for the
# whole run and were carrying three entries for files that do not exist.
while IFS= read -r sums; do
    d="$(dirname "$sums")"
    n_files="$(find "$d" -type f ! -name SHA256SUMS.txt | wc -l | tr -d ' ')"
    # count only checksum lines: the header comment lines are not entries
    n_sums="$(grep -c '^[0-9a-f]\{64\}  ' "$sums" | tr -d ' ')"
    bad="$( ( cd "$d" && shasum -a 256 -c SHA256SUMS.txt 2>/dev/null | grep -cv 'OK$' ) || true )"
    echo "  ${d#$A/}: files=$n_files listed=$n_sums failed_verify=$bad"
    [[ "$n_files" == "$n_sums" ]] || { echo "    COVERAGE MISMATCH"; rc=1; }
    [[ "$bad" == 0 ]] || { echo "    HASH MISMATCH"; rc=1; }
done < <(find "$A" -name SHA256SUMS.txt | sort)
echo "## committed bytes (what a fresh clone yields)"
# The three checks above all read the WORKING TREE, so none of them can see a tree that
# passes locally and fails on clone: a text filter can make the committed BLOB differ from
# both the working file and the recorded hash. Verified real at commit ce8965e.
echo "## ledger chronology (unique and monotonic sequence identities)"
LED_CHK="$(dirname "${BASH_SOURCE[0]}")/check-ledger-chronology.py"
if [[ -f "$LED_CHK" ]]; then
    python3 "$LED_CHK" "$A" || rc=1
else
    echo "  LEDGER CHRONOLOGY CHECK MISSING — order cannot be checked"; rc=1
fi

CB_CHK="$(dirname "${BASH_SOURCE[0]}")/committed-bytes-check.py"
[[ -f "$CB_CHK" ]] || CB_CHK="$A/arms/A1/harness/committed-bytes-check.py"
if [[ -f "$CB_CHK" ]]; then
    # The prefix is DERIVED from the tree under audit, not defaulted to batch C's. The old
    # default made this half answer about batch-c-artifacts while auditing a different
    # tree — every file read as "not at HEAD" because it was looked up under the wrong
    # path. Same class as the gate that was once hardcoded to the live tree.
    _repo="${REPO_ROOT:-/Users/ckriech/projects/clemens33/ae-rust}"
    _prefix="${TREE_PREFIX:-$(python3 -c 'import os,sys; print(os.path.relpath(os.path.abspath(sys.argv[1]), os.path.abspath(sys.argv[2])))' "$A" "$_repo")}"
    python3 "$CB_CHK" "$A" "$_repo" "$_prefix" || rc=1
else
    echo "  COMMITTED-BYTES CHECK MISSING — clone reproducibility cannot be checked"; rc=1
fi

echo "## result: $( ((rc==0)) && echo PASS || echo FAIL )"
exit $rc
