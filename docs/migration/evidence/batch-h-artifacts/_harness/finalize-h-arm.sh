#!/opt/homebrew/bin/bash
# Finalize one Batch H arm: write the content-bound case index, then the checksums.
# Post-capture only — it reads what the arm already wrote and records no observation.
set -uo pipefail
[[ $# -ge 1 ]] || { echo "usage: finalize-h-arm.sh <arm-dir>" >&2; exit 2; }
DEST="$1"
{ printf 'case_dir\tledger_sha256\tledger_lines\tfiles\n'
  for d in "$DEST"/*/; do
      c="$(basename "$d")"
      [[ -f "$d/admissibility-ledger.txt" ]] || continue
      printf '%s\t%s\t%s\t%s\n' "$c" \
        "$(shasum -a 256 "$d/admissibility-ledger.txt" | cut -d' ' -f1)" \
        "$(wc -l <"$d/admissibility-ledger.txt" | tr -d ' ')" \
        "$(find "$d" -type f | wc -l | tr -d ' ')"
  done; } >"$DEST/CASES.tsv"
echo "CASES.tsv: $(( $(wc -l <"$DEST/CASES.tsv") - 1 )) cases indexed"
