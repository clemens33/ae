#!/usr/bin/env bash
# l-roster-check.sh [l-design.md] [crit-assign.md]
#
# Two-way machine proof that l-design.md's per-section LROSTER declarations
# equal crit-assign.md's L-batch assignment sets (colead L-gate finding 2: a
# header grep is a lookup, not an assertion).  Failure classes, each its own
# counter and line report:
#   L-MISSING        id assigned to the batch in crit-assign, absent from LROSTER
#   L-EXTRA          id declared in LROSTER, not assigned to that batch
#   L-DUP            id declared twice in LROSTER (any section)
#   L-UNKNOWN-BATCH  LROSTER section name outside the six-batch allowlist
#   L-WRONG-SECTION  id declared in one section while crit-assign assigns another L batch
# Exit 0 only when every counter is zero.  Run from evidence/ (relative args).
set -euo pipefail

design="${1:-l-design.md}"
assign="${2:-crit-assign.md}"
[[ -f $design && -f $assign ]] || { echo "usage: $0 [l-design.md] [crit-assign.md]" >&2; exit 2; }

awk '
BEGIN {
    split("L-END L-PURGE L-STOP L-COMPACT L-FROM L-RENTRANS", a, " ")
    for (i in a) allow[a[i]] = 1
    missing = extra = dup = unknown = wrong = 0
}
FNR == 1 { fileno++ }
# --- l-design.md: LROSTER: <batch> | <id> [<id> ...]
fileno == 1 && /^LROSTER: / {
    line = $0
    sub(/^LROSTER: */, "", line)
    n = index(line, "|")
    batch = substr(line, 1, n - 1); gsub(/ /, "", batch)
    rest = substr(line, n + 1)
    if (!(batch in allow)) { print "L-UNKNOWN-BATCH: " batch; unknown++ }
    cnt = split(rest, ids, /[[:space:]]+/)
    for (i = 1; i <= cnt; i++) {
        id = ids[i]
        if (id == "") continue
        if (id in declared) { print "L-DUP: " id " (" declared[id] " and " batch ")"; dup++ }
        else declared[id] = batch
    }
    next
}
# --- crit-assign.md: CRIT-ASSIGN: <id> | <batch> | ...
fileno == 2 && /^CRIT-ASSIGN: / {
    line = $0
    sub(/^CRIT-ASSIGN: */, "", line)
    n = index(line, "|")
    id = substr(line, 1, n - 1); gsub(/ /, "", id)
    rest = substr(line, n + 1)
    m = index(rest, "|")
    batch = substr(rest, 1, m - 1); gsub(/ /, "", batch)
    if (batch in allow) assigned[id] = batch
    next
}
END {
    for (id in assigned) {
        if (!(id in declared)) { print "L-MISSING: " id " (" assigned[id] ")"; missing++ }
        else if (declared[id] != assigned[id]) {
            print "L-WRONG-SECTION: " id " declared " declared[id] ", assigned " assigned[id]; wrong++
        }
    }
    for (id in declared) {
        if (!(id in assigned)) { print "L-EXTRA: " id " (" declared[id] ")"; extra++ }
    }
    printf "L-SUMMARY: MISSING=%d EXTRA=%d DUP=%d UNKNOWN_BATCH=%d WRONG_SECTION=%d\n", \
        missing, extra, dup, unknown, wrong
    exit (missing + extra + dup + unknown + wrong) > 0 ? 1 : 0
}
' "$design" "$assign"
