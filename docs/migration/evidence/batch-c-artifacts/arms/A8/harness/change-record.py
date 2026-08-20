#!/usr/bin/env python3
"""Per-path CHANGE RECORD for a mutating arm.

Until A8 a non-empty manifest diff meant something was WRONG. From A8 it means something
was MEASURED, and that inverts the safety property: an unpredicted mutation would otherwise
read as ordinary. So the record does not summarise the diff into a count — it ENUMERATES
every changed path and partitions it by WHO WROTE IT.

"Harness-touched" is a statement about the controller's own actions, declared by the arm
before the run: the paths the harness itself created, moved or removed. It is not a claim
about what the product should do. Everything else in the diff is product-written and is
listed path by path with its before/after content hash, so a change nobody declared is
visible as such rather than absorbed into a number.

usage: change-record.py <before.tsv> <after.tsv> <out.txt> [harness-touched-path ...]
"""
import hashlib, os, sys

before_p, after_p, out_p = sys.argv[1], sys.argv[2], sys.argv[3]
touched = set(sys.argv[4:])

def load(p):
    d = {}
    for line in open(p, encoding="utf-8", errors="replace"):
        parts = line.rstrip("\n").split("\t")
        if len(parts) >= 5:
            typ, mode, h, lnk, path = parts[0], parts[1], parts[2], parts[3], parts[4]
            d[path] = (typ, mode, h, lnk)
    return d

b, a = load(before_p), load(after_p)
added   = sorted(set(a) - set(b))
removed = sorted(set(b) - set(a))
changed = sorted(p for p in set(a) & set(b) if a[p] != b[p])

def who(p):
    q = p.lstrip("./")
    return "harness" if any(q == t.lstrip("./") or q.startswith(t.lstrip("./") + "/") for t in touched) else "PRODUCT"

L = ["## change record — every changed path, partitioned by who wrote it",
     f"before={os.path.basename(before_p)} sha256={hashlib.sha256(open(before_p,'rb').read()).hexdigest()}",
     f"after ={os.path.basename(after_p)} sha256={hashlib.sha256(open(after_p,'rb').read()).hexdigest()}",
     "",
     "harness-touched paths DECLARED by this arm before the run "
     "(a statement about the controller's own actions, not about what the product should do):"]
L += [f"  {t}" for t in sorted(touched)] or ["  (none — the harness wrote nothing)"]
L.append("")
for title, items in (("ADDED", added), ("REMOVED", removed), ("MODIFIED", changed)):
    L.append(f"### {title} ({len(items)})")
    for p in items:
        w = who(p)
        if title == "MODIFIED":
            L.append(f"  [{w}] {p}")
            L.append(f"        before type={b[p][0]} mode={b[p][1]} hash={b[p][2]}")
            L.append(f"        after  type={a[p][0]} mode={a[p][1]} hash={a[p][2]}")
        else:
            r = (a if title == "ADDED" else b)[p]
            L.append(f"  [{w}] {p}   type={r[0]} mode={r[1]} hash={r[2]}")
    if not items:
        L.append("  (none)")
    L.append("")
prod = [p for p in added + removed + changed if who(p) == "PRODUCT"]
L.append(f"product_written_paths={len(prod)}")
L.append(f"harness_written_paths={len(added)+len(removed)+len(changed)-len(prod)}")
L.append("Every product-written path is named above. No path is summarised away, and no")
L.append("statement is made here about whether any of them should have changed.")
open(out_p, "w").write("\n".join(L) + "\n")
print(f"product_written_paths={len(prod)} harness_written_paths={len(added)+len(removed)+len(changed)-len(prod)}")
