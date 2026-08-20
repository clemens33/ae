#!/usr/bin/env python3
"""A-H7L record — generated from the captures. Facts only.

usage: derive-h7l-record.py <arm-dir> <out.txt>
"""
import glob, hashlib, os, re, sys
arm, out = sys.argv[1], sys.argv[2]
rows = []
for case in sorted(glob.glob(os.path.join(arm, "h7l-c*"))):
    cid = os.path.basename(case)
    led = open(os.path.join(case, "admissibility-ledger.txt"), encoding="utf-8").read()
    note = (re.search(r"event=measured-input\tnote=(.*?)\t", led) or [None, ""])[1]
    inv = [l.rstrip("\n").split("\t") for l in open(os.path.join(case, "invocations.tsv"))][1:]
    r = next((x for x in inv if x[0] == "say"), None)
    rc = r[1] if r else "-"
    o = os.path.join(case, "out", "say.stdout")
    first = (open(o, encoding="utf-8", errors="replace").readline().strip() if os.path.exists(o) else "")
    ev = sum(1 for _ in open(os.path.join(case, "events.diff.txt"), encoding="utf-8"))
    def counts(f):
        m = re.search(r"in_range=(\d+) out_of_range=(\d+) unknown_reach=(\d+)", open(f, encoding="utf-8").read()) \
            if os.path.exists(f) else None
        return m.groups() if m else ("-", "-", "-")
    pre = re.search(r"event=census-pre.*?in_range=(\d+) out_of_range=(\d+) unknown_reach=(\d+)", led)
    post = re.search(r"event=census-post.*?in_range=(\d+) out_of_range=(\d+) unknown_reach=(\d+)", led)
    stub = (re.search(r"event=stub-demonstrated\tlog_lines=(\d+)", led) or [None, "-"])[1]
    ctl = (re.search(r"event=census-control.*?reported=(\d+)", led) or [None, "-"])[1]
    rows.append((cid, note, rc, first, ev, pre.groups() if pre else ("-",)*3,
                 post.groups() if post else ("-",)*3, stub, ctl))
L = ["## A-H7L — SC-211l, `say` under containment (generated from the captures)", "",
     "`stub` is the refusing curl stub's own log line count, fired deliberately in each case:",
     "a recorder nobody has seen fire is not evidence of silence. `ctl` is how many rows the",
     "census reported for its own deliberately in-range control; a case whose census cannot",
     "see its control is INCONCLUSIVE rather than contained.", ""]
w = [("case", 24), ("rc", 3), ("events", 6), ("stub", 5), ("ctl", 4), ("in-range pre/post", 18), ("unknown pre/post", 18)]
L.append("  " + "  ".join(h.ljust(n) for h, n in w))
L.append("  " + "  ".join("-" * n for _, n in w))
for r in rows:
    L.append("  " + "  ".join(str(v)[:n].ljust(n) for v, (_, n) in zip(
        [r[0], r[2], r[4], r[7], r[8], f"{r[5][0]}/{r[6][0]}", f"{r[5][2]}/{r[6][2]}"], w)))
L += ["", "## what each case invoked, and the surface's own first stdout line", ""]
for r in rows:
    L.append(f"  {r[0]}")
    L.append(f"      input : {r[1]}")
    L.append(f"      stdout: {r[3] or '(none)'}")
L += ["", "## distinct readings", "",
      f"  distinct rc values: {sorted({r[2] for r in rows})}",
      f"  cases whose events file grew: {sum(1 for r in rows if r[4])} of {len(rows)}",
      f"  cases whose stub log recorded an attempt: {sum(1 for r in rows if r[7] not in ('-', '0'))} of {len(rows)}"]
txt = "\n".join(L) + "\n"
open(out, "w").write(txt)
print(f"wrote {out} sha256={hashlib.sha256(txt.encode()).hexdigest()}")
