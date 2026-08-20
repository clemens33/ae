#!/opt/homebrew/bin/bash
# MANIFEST-vs-TREE gate. Fails if MANIFEST.md cites a path that is not in the tree, or if
# a published file is missing from its directory's SHA256SUMS.txt, or if any recorded hash
# no longer matches. Run before asking for a commit.
set -uo pipefail
A=/Users/ckriech/projects/clemens33/ae-rust/docs/migration/evidence/batch-c-artifacts
rc=0
echo "## cited paths"
while read -r p; do
    case "$p" in *'<'*|*'*'*|'') continue;; esac
    if [[ ! -e "$A/$p" ]]; then echo "  MISSING: $p"; rc=1; fi
done < <(grep -oE '`(templates|arms)/[A-Za-z0-9_./-]+`' "$A/MANIFEST.md" | tr -d '`' | sort -u)
echo "  cited-path check done"
echo "## SHA256SUMS coverage + verification"
for d in "$A"/templates "$A"/arms/*; do
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
