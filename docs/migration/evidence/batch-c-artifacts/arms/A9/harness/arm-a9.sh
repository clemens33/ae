#!/opt/homebrew/bin/bash
# ARM GROUP A9 — quiet vs degraded (SC-519, SC-520) and META ABSENT (SC-405i).
source "$(dirname "$0")/arm-a9-lib.sh"

# quiet, both ways
C9 a9-c01-no-events        "SC-519,SC-520" G4 no-events
C9 a9-c02-zero-byte-events "SC-519,SC-520" G4 zero-byte-events
# degraded, both ways
C9 a9-c03-meta-mode-000    "SC-519,SC-520" G3 meta-mode-000
C9 a9-c04-malformed-event  "SC-519,SC-520" G3 malformed-complete-line
# the absence row
C9 a9-c05-meta-absent      "SC-405i"       A9 meta-absent
# the positive control: the same consumer set on an intact fixture, so an empty
# rendering elsewhere is known not to be a reader that never looked
C9 a9-c06-healthy-control  "control"       G1 healthy
echo "A9 DONE"
