#!/usr/bin/env python3
"""A-H1 / A-H2 record — generated from the captures. Facts only.

Spellings are compared by the HASH of what they produced, so "these three coincide" is a
measured statement about bytes rather than an impression from reading three tables.

usage: derive-h1h2-record.py <arm-dir> <out.txt>
"""
import glob, hashlib, os, re, sys
arm, out = sys.argv[1], sys.argv[2]
rows = []
for case in sorted(glob.glob(os.path.join(arm, "h?-c*"))):
    cid = os.path.basename(case)
    led = open(os.path.join(case, "admissibility-ledger.txt"), encoding="utf-8").read()
    argv = (re.search(r"event=measured-input\targv=(.*?)\t", led) or [None, "-"])[1]
    inv = [l.rstrip("\n").split("\t") for l in open(os.path.join(case, "invocations.tsv"))][1:]
    r = next((x for x in inv if x[0] == "ae"), None)
    wrote = sum(1 for _ in open(os.path.join(case, "home-bytes.diff.txt"), encoding="utf-8"))
    e = os.path.join(case, "out", "ae.stderr")
    err1 = (open(e, encoding="utf-8", errors="replace").readline().strip() if os.path.exists(e) else "")
    rows.append((cid, argv, r[1], r[2], r[3], r[4], r[5], wrote, err1,
                 led.count("pass=yes"), led.count("event=canary")))
L = [f"## {os.path.basename(arm)} — generated from the captures", "",
     "Each spelling was invoked separately. `stdout sha256` is what makes 'these coincide'",
     "a statement about bytes: two spellings agree only if their captured stdout hashes",
     "agree.", ""]
w = [("case", 30), ("argv", 26), ("rc", 3), ("out B", 6), ("stdout sha256", 14), ("wrote", 5), ("canary", 6)]
L.append("  " + "  ".join(h.ljust(n) for h, n in w))
L.append("  " + "  ".join("-" * n for _, n in w))
for r in rows:
    L.append("  " + "  ".join(str(v)[:n].ljust(n) for v, (_, n) in zip(
        [r[0], r[1], r[2], r[4], r[3][:12], r[7], f"{r[9]}/{r[10]}"], w)))
L += ["", "## stderr first line, per case that produced any", ""]
for r in rows:
    if r[8]:
        L.append(f"  {r[0]}"); L.append(f"      {r[8]}")
groups = {}
for r in rows:
    groups.setdefault(r[3], []).append(r[0])
L += ["", "## spelling families, by captured stdout hash", ""]
for h, cs in sorted(groups.items(), key=lambda kv: -len(kv[1])):
    L.append(f"  {h[:12]}  {len(cs)} case(s): {', '.join(cs)}")
L += ["", f"  distinct stdout hashes: {len(groups)} across {len(rows)} cases",
      f"  distinct rc values: {sorted({r[2] for r in rows})}",
      f"  cases whose AE_HOME changed: {sum(1 for r in rows if r[7])}"]
txt = "\n".join(L) + "\n"
open(out, "w").write(txt)
print(f"wrote {out} sha256={hashlib.sha256(txt.encode()).hexdigest()}")
