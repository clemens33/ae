#!/opt/homebrew/bin/bash
set -uo pipefail
ARM="$1"
S=/private/tmp/claude-501/-Users-ckriech-projects-clemens33-ae-rust/347d2089-7268-421d-8188-8924e246bbf0/scratchpad
DEST="/Users/ckriech/projects/clemens33/ae-rust/docs/migration/evidence/batch-c-artifacts/arms/$ARM"
mkdir -p "$DEST/harness"
cp "$S/harness/armlib.sh" "$S/harness/tlib.sh" "$DEST/harness/"
cp "$S/harness/arm-$(echo "$ARM" | tr 'A-Z' 'a-z')"*.sh "$DEST/harness/" 2>/dev/null
cp /tmp/aecx/shim-tmux/tmux "$DEST/harness/tmux-shim"
cp "$S/harness/finalize-arm.sh" "$DEST/harness/"
( cd "$DEST" && find . -type f ! -name SHA256SUMS.txt -print0 | sort -z | xargs -0 shasum -a 256 ) > /tmp/aecx/asums && mv /tmp/aecx/asums "$DEST/SHA256SUMS.txt"
echo "$ARM finalized: $(find "$DEST" -type f | wc -l | tr -d ' ') files, $(du -sh "$DEST" | cut -f1)"
echo "  admissibility artifacts: ledgers=$(find "$DEST" -name admissibility-ledger.txt | wc -l | tr -d ' ') tab-selfchecks=$(find "$DEST" -name env-tab-selfcheck.txt | wc -l | tr -d ' ') shim-equiv=$(find "$DEST" -name tmux-shim-equivalence.txt | wc -l | tr -d ' ') tmuxtraces=$(find "$DEST" -name '*.tmuxtrace' | wc -l | tr -d ' ')"
echo "  in SHA256SUMS: $(grep -c 'admissibility-ledger.txt' "$DEST/SHA256SUMS.txt") ledger lines, $(grep -c 'env-tab-selfcheck.txt' "$DEST/SHA256SUMS.txt") selfcheck lines, $(grep -c 'tmuxtrace' "$DEST/SHA256SUMS.txt") trace lines"
