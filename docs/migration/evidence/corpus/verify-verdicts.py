#!/usr/bin/env python3
"""GATE — re-derives every verdict from the captured bytes and reconciles the counts.
No write path.

THE CONVERSE CHECKS ARE THE POINT. Confirming that each EXPECTED-DIVERGENCE row
carries what it claims is the easy half; the error this file exists to catch is an
EXPECTED-MATCH row that should have diverged. The first generator scored an
unreachable-server listing as MATCH because its captured bytes carried no status
field — missing that SC-017m changes the MEMBERSHIP of the view, so an empty
listing is exactly the output that diverges most visibly.
"""
import csv, os, re, sys, collections

HERE = os.path.dirname(os.path.abspath(__file__))
SRC = os.path.normpath(os.path.join(HERE, "..", "batch-c-artifacts"))
INV = os.path.join(HERE, "INVOCATIONS.tsv")
VER = os.path.join(HERE, "VERDICTS.tsv")
LISTING = ("ae list", "ae ls")

def main():
    for p in (INV, VER):
        if not os.path.exists(p):
            print("FAIL  %s absent" % os.path.basename(p)); return 1
    p1 = {(r["case"], r["consumer"]) for r in csv.DictReader(open(INV, encoding="utf-8"), delimiter="\t")
          if r["phase"] == "P1"}
    rows = list(csv.DictReader(open(VER, encoding="utf-8"), delimiter="\t"))
    fails = []
    keys = [(r["case"], r["consumer"]) for r in rows]
    if len(keys) != len(set(keys)): fails.append("duplicate (case, consumer) keys in VERDICTS.tsv")
    missing = p1 - set(keys); extra = set(keys) - p1
    for k in sorted(missing): fails.append("P1 row with NO verdict: %s / %s" % k)
    for k in sorted(extra): fails.append("verdict for a row that is not P1: %s / %s" % k)

    for r in rows:
        p = os.path.join(SRC, os.path.dirname(r["case"]), "out", r["consumer"] + ".stdout")
        body = open(p, encoding="utf-8", errors="replace").read() if os.path.exists(p) else ""
        digest = '"schema_version"' in body
        unreach_listing = r["server_unreachable"] == "yes" and r["surface"] in LISTING
        m = r["mandated_by"]
        # forward: what it claims, it carries
        if "SC-509d" in m and not digest:
            fails.append("%s/%s claims SC-509d but carries no digest" % (r["case"], r["consumer"]))
        if "SC-017l/m" in m and not unreach_listing:
            fails.append("%s/%s claims SC-017l/m but is not an unreachable-server listing" % (r["case"], r["consumer"]))
        # CONVERSE: what it does not claim, it does not carry
        if "SC-509d" not in m and digest:
            fails.append("%s/%s carries a digest but does not claim SC-509d" % (r["case"], r["consumer"]))
        if "SC-017l/m" not in m and unreach_listing:
            fails.append("%s/%s is an unreachable-server listing but does not claim SC-017l/m" % (r["case"], r["consumer"]))
        if (r["verdict"] == "EXPECTED-DIVERGENCE") != (m != "-"):
            fails.append("%s/%s verdict and mandated_by disagree" % (r["case"], r["consumer"]))
        if r["baseline_provenance"] == "OBSERVED" and "SC-017l/m" not in m:
            fails.append("%s/%s claims an OBSERVED baseline without the observed row" % (r["case"], r["consumer"]))

    c = collections.Counter(r["verdict"] for r in rows)
    d, mt = c["EXPECTED-DIVERGENCE"], c["EXPECTED-MATCH"]
    if d + mt != len(rows): fails.append("verdict classes do not partition the rows")
    if len(rows) != len(p1): fails.append("row count %d != P1 count %d" % (len(rows), len(p1)))
    print("verdicts: %d rows = %d EXPECTED-DIVERGENCE + %d EXPECTED-MATCH" % (len(rows), d, mt))
    bym = collections.Counter(r["mandated_by"] for r in rows)
    for k in sorted(bym): print("  mandated_by %-20s %4d" % (k, bym[k]))
    for f in fails[:20]: print("FAIL  %s" % f)
    print("VERDICT COLUMN VERIFIED" if not fails else "VERDICT COLUMN NOT VERIFIED — %d finding(s)" % len(fails))
    return 1 if fails else 0

if __name__ == "__main__":
    sys.exit(main())
