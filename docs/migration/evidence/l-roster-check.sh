#!/usr/bin/env bash
# l-roster-check.sh [l-design.md] [crit-assign.md]
#
# Two-way machine proof that l-design.md's ARM declarations cover exactly the
# crit-assign L-batch assignment sets (colead L-gate v2 finding B1: a roster
# TABLE is a copied declaration; the checker must gate ARM COVERAGE, and a
# body whose ids are erased must go red).
#
# Declarations in l-design.md:
#   LROSTER: <section> | <id> ...            (the section's asserted roster)
#   LARM: <section> | <arm-label> | <primary-id> ... [| ref: <id> ...]
#     primary ids = coverage; ref ids = typed non-roster safety controls,
#     never counted toward coverage.
#
# Failure classes (each its own counter + line report; exit 1 on any):
#   L-ROSTER-VS-ASSIGN   LROSTER for a section != crit-assign L-<section> set
#   L-ARM-MISSING        a roster id is the primary of NO arm
#   L-ARM-EXTRA          an arm primary id is not in that section's roster
#   L-ARM-DUP            an id is primary of more than one arm (any section)
#   L-BODY-MISSING       a primary id never appears in the design BODY text
#                        (outside LROSTER/LARM lines) — the body-erasure catch
#   L-UNKNOWN-BATCH      a declared section name outside the six-batch allowlist
# Run from evidence/ (relative args).
set -euo pipefail

design="${1:-l-design.md}"
assign="${2:-crit-assign.md}"
[[ -f $design && -f $assign ]] || { echo "usage: $0 [l-design.md] [crit-assign.md]" >&2; exit 2; }

awk '
BEGIN {
    split("L-END L-PURGE L-STOP L-COMPACT L-FROM L-RENTRANS", a, " ")
    for (i in a) allow[a[i]] = 1
    rva = amiss = axtra = adup = bmiss = unknown = 0
}
FNR == 1 { fileno++ }

# ---- l-design.md ----
fileno == 1 && /^LROSTER: / {
    line = $0; sub(/^LROSTER: */, "", line)
    n = index(line, "|"); sec = substr(line, 1, n-1); gsub(/ /, "", sec)
    rest = substr(line, n+1)
    if (!(sec in allow)) { print "L-UNKNOWN-BATCH: " sec " (LROSTER)"; unknown++ }
    c = split(rest, ids, /[[:space:]]+/)
    for (i=1;i<=c;i++) if (ids[i] ~ /^SC-/) roster[sec, ids[i]] = 1
    next
}
fileno == 1 && /^LARM: / {
    line = $0; sub(/^LARM: */, "", line)
    n = index(line, "|"); sec = substr(line, 1, n-1); gsub(/ /, "", sec)
    rest = substr(line, n+1)
    m = index(rest, "|"); rest = substr(rest, m+1)          # drop arm-label
    if (!(sec in allow)) { print "L-UNKNOWN-BATCH: " sec " (LARM)"; unknown++ }
    # split off a trailing "| ref: ..." segment: ref ids are not coverage
    refpos = index(rest, "| ref:")
    if (refpos > 0) rest = substr(rest, 1, refpos-1)
    c = split(rest, ids, /[[:space:]]+/)
    for (i=1;i<=c;i++) {
        id = ids[i]; if (id !~ /^SC-/) continue
        if (id in armof) { print "L-ARM-DUP: " id " (" armof[id] " and " sec ")"; adup++ }
        else armof[id] = sec
        armsec[sec, id] = 1
        primary[id] = 1
    }
    next
}
# body lines: everything in the design that is not a declaration line
fileno == 1 {
    b = $0
    while (match(b, /SC-[0-9]+[a-z]?/)) {
        bodyseen[substr(b, RSTART, RLENGTH)] = 1
        b = substr(b, RSTART + RLENGTH)
    }
    next
}

# ---- crit-assign.md ----
fileno == 2 && /^CRIT-ASSIGN: / {
    line = $0; sub(/^CRIT-ASSIGN: */, "", line)
    n = index(line, "|"); id = substr(line, 1, n-1); gsub(/ /, "", id)
    rest = substr(line, n+1); m = index(rest, "|")
    batch = substr(rest, 1, m-1); gsub(/ /, "", batch)
    if (batch in allow) { assign[batch, id] = 1; assignids[id] = batch }
    next
}

END {
    # roster vs crit-assign, both directions
    for (k in roster) { split(k, p, SUBSEP); if (!((p[1] SUBSEP p[2]) in assign)) { print "L-ROSTER-VS-ASSIGN: " p[2] " in LROSTER " p[1] ", not assigned there"; rva++ } }
    for (k in assign) { split(k, p, SUBSEP); if (!((p[1] SUBSEP p[2]) in roster)) { print "L-ROSTER-VS-ASSIGN: " p[2] " assigned " p[1] ", not in LROSTER"; rva++ } }
    # arm coverage vs roster
    for (k in roster) { split(k, p, SUBSEP); if (!((p[1] SUBSEP p[2]) in armsec)) { print "L-ARM-MISSING: " p[2] " (roster " p[1] ", no primary arm)"; amiss++ } }
    for (k in armsec) { split(k, p, SUBSEP); if (!((p[1] SUBSEP p[2]) in roster)) { print "L-ARM-EXTRA: " p[2] " (arm " p[1] ", not in roster)"; axtra++ } }
    # body presence of every primary id
    for (id in primary) if (!(id in bodyseen)) { print "L-BODY-MISSING: " id " (declared as an arm primary, absent from body text)"; bmiss++ }
    printf "L-SUMMARY: ROSTER_VS_ASSIGN=%d ARM_MISSING=%d ARM_EXTRA=%d ARM_DUP=%d BODY_MISSING=%d UNKNOWN_BATCH=%d\n", rva, amiss, axtra, adup, bmiss, unknown
    exit (rva+amiss+axtra+adup+bmiss+unknown) > 0 ? 1 : 0
}
' "$design" "$assign"
