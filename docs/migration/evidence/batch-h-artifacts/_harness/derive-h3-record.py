#!/usr/bin/env python3
"""A-H3 record — generated from the captures. Facts only.

usage: derive-h3-record.py <arm-dir> <out.txt>
"""
import glob, hashlib, os, re, sys
arm, out = sys.argv[1], sys.argv[2]
rows = []
for case in sorted(glob.glob(os.path.join(arm, "h3-*"))):
    cid = os.path.basename(case)
    led = open(os.path.join(case, "admissibility-ledger.txt"), encoding="utf-8").read()
    mi = next((l for l in led.split("\n") if "event=measured-input" in l), "")
    helper = (re.search(r"helper=(\S+)", mi) or [None, "-"])[1]
    argv = (re.search(r"argv=(.*?)(?:\tinvocation_cwd|$)", mi) or [None, ""])[1].strip()
    inv = [l.rstrip("\n").split("\t") for l in open(os.path.join(case, "invocations.tsv"))][1:]
    row = next((r for r in inv if r[0] == helper), None)
    rc = row[1] if row else "-"
    ob = row[3] if row else "-"
    eb = row[5] if row else "-"
    timed = row[7] if row else "-"
    wrote = sum(1 for _ in open(os.path.join(case, "session-bytes.diff.txt"), encoding="utf-8"))
    e = os.path.join(case, "out", f"{helper}.stderr")
    first_err = ""
    if os.path.exists(e):
        first_err = (open(e, encoding="utf-8", errors="replace").readline() or "").strip()
    rows.append((cid, helper, argv, rc, ob, eb, wrote, timed,
                 led.count("pass=yes"), led.count("event=canary"), first_err))
L = ["## A-H3 — the argument surface (generated from the captures)", "",
     "One case per input class. `wrote` counts the lines by which the session directory's",
     "own byte listing changed across the invocation — several of these helpers write, and",
     "a refusal that wrote something is a different reading from one that did not.",
     "`canary` is the capture-path control, fired before and after each invocation.", ""]
w = [("case", 30), ("helper", 10), ("rc", 3), ("out B", 6), ("err B", 6), ("wrote", 5), ("canary", 6)]
L.append("  " + "  ".join(h.ljust(n) for h, n in w))
L.append("  " + "  ".join("-" * n for _, n in w))
for r in rows:
    L.append("  " + "  ".join(str(v)[:n].ljust(n) for v, (_, n) in zip(
        [r[0], r[1], r[3], r[4], r[5], r[6], f"{r[8]}/{r[9]}"], w)))
L += ["", "## argv and first stderr line, per case", ""]
for r in rows:
    L.append(f"  {r[0]}  ({r[1]})")
    L.append(f"      argv  : {r[2] or '(none)'}")
    if r[10]:
        L.append(f"      stderr: {r[10]}")
byh = {}
for r in rows:
    byh.setdefault(r[1], []).append(r)
L += ["", "## distinct readings per helper", ""]
for h, rs in sorted(byh.items()):
    L.append(f"  {h:10s} cases={len(rs):<3} distinct rc={sorted({x[3] for x in rs})} "
             f"distinct first-stderr={len({x[10] for x in rs if x[10]})} "
             f"cases that wrote={sum(1 for x in rs if x[6])}")
txt = "\n".join(L) + "\n"
open(out, "w").write(txt)
print(f"wrote {out} sha256={hashlib.sha256(txt.encode()).hexdigest()}")
