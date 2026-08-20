#!/opt/homebrew/bin/bash
# ARM GROUP A1 — schema/document. Bash lane.
source "$(dirname "$0")/armlib.sh"
ARM=A1
[[ -e "$AROOT/$ARM" ]] && chmod -R u+w "$AROOT/$ARM" 2>/dev/null
rm -rf "$AROOT/$ARM"; mkdir -p "$AROOT/$ARM"
LEDGER="$AROOT/$ARM/ledger.tsv"
printf 'case\trows\tgroup\tmember\n' >"$LEDGER"
C() { # <case-id> <rows> <group> <member>
    printf '%s\t%s\t%s\t%s\n' "$1" "$2" "$3" "$4" >>"$LEDGER"
    echo "== case $1 ($2) <- $3/$4"
    arm_case "$ARM" "$1" "$3" "$4" ro
    arm_case "$ARM" "$1" "$3" "$4" rw
}
C c01-healthy              "SC-509,SC-509b,SC-510a"   G1  healthy
C c02-meta-mode-000        "SC-506"                   G3  meta-mode-000
C c03-malformed-line       "SC-506,SC-509b"           G3  malformed-complete-line
C c04-empty-vs-omitted     "SC-510b"                  A1  510b-empty-vs-omitted
C c05-recover-ref          "SC-510c"                  A1  510c-recover-ref
C c06-escapes              "SC-510d"                  G11 escapes
C c07-dupkey-known         "SC-510e"                  A1  510e-dupkey-known
C c08-dupkey-known-rev     "SC-510e"                  A1  510e-dupkey-known-reversed
C c09-dupkey-unknown       "SC-510f"                  A1  510f-dupkey-unknown
C c10-dupkey-unknown-rev   "SC-510f"                  A1  510f-dupkey-unknown-reversed
C c11-routing-known        "SC-511a,SC-511b"          G5  m1-control
C c12-routing-omitted      "SC-511a"                  A1  511a-omitted-routing
C c13-same-display-routing "SC-511b"                  G10 same-display-diff-routing
C c14-display-only-legacy  "SC-511b"                  G10 display-only-legacy
C c15-meta-unknown-keys    "SC-509,SC-509b"           G7  meta-unknown-keys
C c16-events-unknown-keys  "SC-509b,SC-510a"          G7  events-unknown-keys
C c17-unknown-action       "SC-509b,SC-510a"          G7  events-unknown-action
C c18-no-trailing-newline  "SC-509b"                  G8  no-trailing-newline
C c19-partial-tail         "SC-509b"                  G8  partial-trailing-record
echo "A1 DOCUMENT CASES DONE"
