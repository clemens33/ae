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
    m = index(rest, "|"); label = substr(rest, 1, m-1); gsub(/^ +| +$/, "", label)
    rest = substr(rest, m+1)
    if (!(sec in allow)) { print "L-UNKNOWN-BATCH: " sec " (LARM)"; unknown++ }
    refpos = index(rest, "| ref:")                          # ref ids not coverage
    if (refpos > 0) rest = substr(rest, 1, refpos-1)
    c = split(rest, ids, /[[:space:]]+/)
    for (i=1;i<=c;i++) {
        id = ids[i]; if (id !~ /^SC-/) continue
        if (id in armof) { print "L-ARM-DUP: " id " (" armof[id] " and " sec ") "; adup++ }
        else armof[id] = sec
        armsec[sec, id] = 1
        armlabelof[sec, id] = label                          # the declared owning arm
        primary[id] = 1
    }
    next
}
# The seat annex is not worker-arm body: stop attributing ids once it starts.
fileno == 1 && /^## SEAT CLASSIFICATION ANNEX/ { in_annex = 1; next }
fileno == 1 && in_annex { next }
# Track the current section and the current worker-arm label from its bold head.
fileno == 1 && /^## Section (L-[A-Z]+)/ { cur_sec = $3; cur_arm = ""; next }
fileno == 1 && /^- \*\*/ {
    h = $0; sub(/^- \*\*/, "", h)
    cur_arm = h; sub(/[ (*].*$/, "", cur_arm)               # first token before space/paren/**
    # attribute the heading and prose ids to THIS arm; the check is per
    # (section,arm,id), so an id that vanishes from its declared arm or moves to
    # a different arm/the annex fires BODY-MISSING even if it survives elsewhere.
    while (match(h, /SC-[0-9]+[a-z]?/)) { armbody[cur_sec, cur_arm, substr(h, RSTART, RLENGTH)] = 1; h = substr(h, RSTART+RLENGTH) }
    next
}
# body/continuation lines: attribute ids to the current section+arm
fileno == 1 {
    if (cur_sec == "" || cur_arm == "") next
    b = $0
    while (match(b, /SC-[0-9]+[a-z]?/)) { armbody[cur_sec, cur_arm, substr(b, RSTART, RLENGTH)] = 1; b = substr(b, RSTART+RLENGTH) }
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
    # every primary id must appear in the body of ITS declared arm (not merely
    # somewhere in the file, and not in the annex): the wrong-arm/erasure catch
    for (k in armsec) {
        split(k, p, SUBSEP); sec = p[1]; id = p[2]; lbl = armlabelof[sec, id]
        if (!((sec SUBSEP lbl SUBSEP id) in armbody)) {
            print "L-BODY-MISSING: " id " (primary of " sec " arm [" lbl "], absent from that arm body)"; bmiss++
        }
    }
    printf "L-SUMMARY: ROSTER_VS_ASSIGN=%d ARM_MISSING=%d ARM_EXTRA=%d ARM_DUP=%d BODY_MISSING=%d UNKNOWN_BATCH=%d\n", rva, amiss, axtra, adup, bmiss, unknown
    exit (rva+amiss+axtra+adup+bmiss+unknown) > 0 ? 1 : 0
}
' "$design" "$assign"
