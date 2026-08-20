#!/usr/bin/env python3
"""Rebuild an arm's ledger.tsv from its CASE DIRECTORIES, which are the authoritative
source: every case.txt names its own rows and template.

Written after truncating two ledgers with the same one-liner:
    open(p,'w').writelines([l for l in open(p) if ...])
Arguments evaluate left to right, so open(p,'w') TRUNCATES the file before the
comprehension reads it — the filter then reads an empty file and writes nothing back.
Read fully, then write. Never filter a file through a handle that has already truncated it.
"""
import os, re, sys
MODES = ("controlled-path","fixed-clock","ro","rw","live","twin","barrier","attach","follow")
d = sys.argv[1]
rows, seen = [], set()
for c in sorted(os.listdir(d)):
    cd = os.path.join(d, c); ct = os.path.join(cd, "case.txt")
    if not os.path.isdir(cd) or not os.path.exists(ct):
        continue
    txt = open(ct, encoding="utf-8", errors="replace").read()
    base = c
    for m in MODES:
        if c.endswith("-" + m):
            base = c[:-(len(m) + 1)]; break
    if base in seen:
        continue
    seen.add(base)
    m = re.search(r"rows=([A-Za-z0-9,\-]+)", txt)
    rowids = m.group(1) if m else "-"
    g, mem = "-", "-"
    m2 = re.search(r"template=([A-Za-z0-9]+)/([A-Za-z0-9_.-]+)", txt)
    if m2:
        g, mem = m2.group(1), m2.group(2)
    elif "template=none" in txt:
        g, mem = "live", "no-template (live launch)"
    rows.append((base, rowids, g, mem))
out = ["case\trows\tgroup\tmember"] + ["\t".join(r) for r in rows]
open(os.path.join(d, "ledger.tsv"), "w").write("\n".join(out) + "\n")
print(f"{os.path.basename(d)}: ledger.tsv rebuilt with {len(rows)} rows")
