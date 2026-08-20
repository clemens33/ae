#!/usr/bin/env python3
"""A-H4 resolution record — generated from the captures. Facts only; no relation stated.

usage: derive-h4-record.py <arm-dir> <out.txt>
"""
import glob, hashlib, os, sys
arm, out = sys.argv[1], sys.argv[2]
rows = []
for case in sorted(glob.glob(os.path.join(arm, "h4-c*"))):
    cid = os.path.basename(case)
    o = os.path.join(case, "out", "resolve.stdout"); e = os.path.join(case, "out", "resolve.stderr")
    kv = {}
    for line in open(o, encoding="utf-8", errors="replace") if os.path.exists(o) else []:
        if "=" in line: k, v = line.rstrip("\n").split("=", 1); kv[k] = v
    err = open(e, encoding="utf-8", errors="replace").read().strip() if os.path.exists(e) else ""
    led = open(os.path.join(case, "admissibility-ledger.txt"), encoding="utf-8").read()
    inp = next((l.split("input=", 1)[1].split("\t")[0] for l in led.split("\n") if "event=measured-input" in l), "")
    rows.append({"case": cid, "input": inp, "rc": kv.get("rc", "-"),
                 "pane": kv.get("AE_RESOLVED_PANE", ""), "agent": kv.get("AE_RESOLVED_AGENT", ""),
                 "slot": kv.get("AE_RESOLVED_SLOT", ""), "sess": kv.get("AE_RESOLVED_SESSION", ""),
                 "err": err.split("\n")[0] if err else "",
                 "canaries": led.count("event=canary") , "canary_pass": led.count("pass=yes")})
L = ["## A-H4 — SC-211p, `_lib` name resolution (generated from the captures)", "",
     "Each row is one input class from the executor list, invoked through the generated",
     "`_lib`'s own `ae_resolve` and captured with the resolver's output contract",
     "(ae@72c7293:12983-12989). Nothing here states what any reading means.", "",
     "`canary` is this case's capture-path control: known stdout, known stderr, known rc",
     "through the exact wrapper, fired before AND after the measured invocation.", ""]
w = [("case", 32), ("input", 18), ("rc", 3), ("pane", 6), ("agent", 16), ("canary", 7)]
L.append("  " + "  ".join(h.ljust(n) for h, n in w))
L.append("  " + "  ".join("-" * n for _, n in w))
for r in rows:
    L.append("  " + "  ".join(str(v)[:n].ljust(n) for v, (_, n) in zip(
        [r["case"], r["input"] or "(empty)", r["rc"], r["pane"], r["agent"],
         f"{r['canary_pass']}/{r['canaries']}"], w)))
L += ["", "## stderr, verbatim, per case that produced any", ""]
for r in rows:
    if r["err"]:
        L.append(f"  {r['case']}")
        L.append(f"      {r['err']}")
L += ["", "## distinct readings", ""]
L.append(f"  distinct rc values: {sorted({r['rc'] for r in rows})}")
L.append(f"  cases resolving to a pane: {sum(1 for r in rows if r['pane'])} of {len(rows)}")
L.append(f"  distinct stderr first lines: {len({r['err'] for r in rows if r['err']})}")
txt = "\n".join(L) + "\n"
open(out, "w").write(txt)
print(f"wrote {out} sha256={hashlib.sha256(txt.encode()).hexdigest()}")
