#!/opt/homebrew/bin/bash
# A7, SC-405j rebuilt on the ask->REPLY pair. Same case runner as the rest of A7.
source "$(dirname "$0")/arm-a7-lib.sh"
C a7-c12-405j-pair-full-fresh  "SC-405j" A7 pair-405j-full-fresh  yes
C a7-c13-405j-pair-stale-keys  "SC-405j" A7 pair-405j-stale-keys  yes
C a7-c14-405j-pair-slot-only   "SC-405j" A7 pair-405j-slot-only   yes
C a7-c15-405j-pair-session-only "SC-405j" A7 pair-405j-session-only yes
C a7-c16-405j-pair-keyless     "SC-405j" A7 pair-405j-keyless     yes
C a7-c17-405j-pair-one-empty   "SC-405j" A7 pair-405j-one-empty   yes
C a7-c18-405j-pair-all-empty   "SC-405j" A7 pair-405j-all-empty   yes
echo "A7-405J DONE"
