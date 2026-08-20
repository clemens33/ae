#!/opt/homebrew/bin/bash
# Generate the per-arm summary table from the ARTIFACTS. Nothing here is typed by hand:
# every number is read out of the committed tree at the moment this runs.
set -uo pipefail
A="${1:-/Users/ckriech/projects/clemens33/ae-rust/docs/migration/evidence/batch-c-artifacts}"
printf '%-6s %6s %6s %7s %8s %9s %8s %8s  %s\n' ARM CASES FILES LEDGERS SELFCHKS SHIMEQUIV HOOKEQV TRACES ROWS
for d in "$A"/arms/*/; do
    arm="$(basename "$d")"
    cases=$(find "$d" -mindepth 1 -maxdepth 1 -type d ! -name harness | wc -l | tr -d ' ')
    files=$(find "$d" -type f | wc -l | tr -d ' ')
    led=$(find "$d" -name admissibility-ledger.txt | wc -l | tr -d ' ')
    stc=$(find "$d" -name env-tab-selfcheck.txt | wc -l | tr -d ' ')
    shm=$(find "$d" -name tmux-shim-equivalence.txt | wc -l | tr -d ' ')
    hke=$(find "$d" -name hook-inactive-equivalence.txt | wc -l | tr -d ' ')
    trc=$(find "$d" -name '*.tmuxtrace' | wc -l | tr -d ' ')
    rows=$(tail -n +2 "$d/ledger.tsv" 2>/dev/null | cut -f2 | tr ',' '\n' | sed 's/ //g' | sort -u | tr '\n' ',' | sed 's/,$//')
    printf '%-6s %6s %6s %7s %8s %9s %8s %8s  %s\n' "$arm" "$cases" "$files" "$led" "$stc" "$shm" "$hke" "$trc" "$rows"
done
printf '%-6s %6s %6s\n' TOTAL "$(find "$A"/arms/*/ -mindepth 1 -maxdepth 1 -type d ! -name harness | wc -l | tr -d ' ')" "$(find "$A" -type f | wc -l | tr -d ' ')"
