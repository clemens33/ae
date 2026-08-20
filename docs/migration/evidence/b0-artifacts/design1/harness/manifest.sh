#!/opt/homebrew/bin/bash
# Recursive manifest: relpath TAB type TAB mode TAB symlink-target TAB sha256
set -euo pipefail
root="$1"
cd "$root"
find . \( -type f -o -type d -o -type l \) -print0 | sort -z | while IFS= read -r -d '' p; do
  if [[ -L "$p" ]]; then
    printf '%s\tsymlink\t%s\t%s\t-\n' "$p" "$(stat -f '%Lp' "$p")" "$(readlink "$p")"
  elif [[ -d "$p" ]]; then
    printf '%s\tdir\t%s\t-\t-\n' "$p" "$(stat -f '%Lp' "$p")"
  else
    printf '%s\tfile\t%s\t-\t%s\n' "$p" "$(stat -f '%Lp' "$p")" "$(shasum -a 256 "$p" | awk '{print $1}')"
  fi
done
