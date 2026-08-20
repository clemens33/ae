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
echo "## SHA256SUMS coverage + verification"
for d in "$A"/templates "$A"/hook-patch "$A"/arms/*; do
    [[ -d "$d" ]] || continue
    sums="$d/SHA256SUMS.txt"
    if [[ ! -f "$sums" ]]; then echo "  MISSING SHA256SUMS: ${d#$A/}"; rc=1; continue; fi
    n_files="$(find "$d" -type f ! -name SHA256SUMS.txt | wc -l | tr -d ' ')"
    n_sums="$(wc -l <"$sums" | tr -d ' ')"
    bad="$( ( cd "$d" && shasum -a 256 -c SHA256SUMS.txt 2>/dev/null | grep -cv 'OK$' ) || true )"
    echo "  ${d#$A/}: files=$n_files listed=$n_sums failed_verify=$bad"
    [[ "$n_files" == "$n_sums" ]] || { echo "    COVERAGE MISMATCH"; rc=1; }
    [[ "$bad" == 0 ]] || { echo "    HASH MISMATCH"; rc=1; }
done
echo "## result: $( ((rc==0)) && echo PASS || echo FAIL )"
exit $rc
