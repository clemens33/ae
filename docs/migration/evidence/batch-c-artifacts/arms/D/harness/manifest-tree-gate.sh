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
CB_CHK="$(dirname "${BASH_SOURCE[0]}")/committed-bytes-check.py"
[[ -f "$CB_CHK" ]] || CB_CHK="$A/arms/A1/harness/committed-bytes-check.py"
if [[ -f "$CB_CHK" ]]; then
    python3 "$CB_CHK" "$A" "${REPO_ROOT:-/Users/ckriech/projects/clemens33/ae-rust}"         "${TREE_PREFIX:-docs/migration/evidence/batch-c-artifacts}" || rc=1
else
    echo "  COMMITTED-BYTES CHECK MISSING — clone reproducibility cannot be checked"; rc=1
fi

echo "## result: $( ((rc==0)) && echo PASS || echo FAIL )"
exit $rc
