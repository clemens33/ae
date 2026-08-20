#!/opt/homebrew/bin/bash
# Write a SHA256SUMS.txt for ONE directory, with the two properties a checksum file needs
# and does not get by accident:
#   - it does NOT list itself. A file whose hash changes the moment it is written can never
#     verify from its own listing.
#   - every path is relative to THIS directory, and the header says so, so `shasum -c` is
#     run from the only place the paths resolve.
# The temp file is written OUTSIDE the directory being hashed: writing it inside makes the
# find that is building the listing enumerate the half-written listing itself (that is how
# three phantom ./.sums.tmp entries reached the committed T-WD archive).
set -uo pipefail
d="$1"
[[ -d "$d" ]] || { echo "write-sums: no such directory: $d" >&2; exit 2; }
_root="${BATCH_C_ARTIFACTS:-/Users/ckriech/projects/clemens33/ae-rust/docs/migration/evidence/batch-c-artifacts}"
_rel="${d#"$_root"/}"; [[ "$_rel" == "$d" ]] && _rel="$(basename "$d")"
tmp="$(mktemp /tmp/aecx/sums.XXXXXX)"
{
  printf '# sha256 checksums for the files in THIS directory, paths relative to it.\n'
  printf '# verify with:  cd %s && shasum -a 256 -c SHA256SUMS.txt\n' "$_rel"
  printf '# this file is deliberately NOT listed in itself.\n'
  ( cd "$d" && find . -type f ! -name SHA256SUMS.txt -print0 | sort -z | xargs -0 shasum -a 256 )
} >"$tmp"
mv "$tmp" "$d/SHA256SUMS.txt"
