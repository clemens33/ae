#!/usr/bin/env python3
"""A-H5 record — generated from the captures. Facts only; no relation stated.

usage: derive-h5-record.py <arm-dir> <out.txt>
"""
import glob, hashlib, os, re, sys
arm, out = sys.argv[1], sys.argv[2]
rows = []
for case in sorted(glob.glob(os.path.join(arm, "h5-c*"))):
    cid = os.path.basename(case)
    led = open(os.path.join(case, "admissibility-ledger.txt"), encoding="utf-8").read()
    slot = next((l.split("slot_argv=", 1)[1].split("\t")[0] for l in led.split("\n")
                 if "event=measured-input" in l), "-")
    note = next((l.split("note=", 1)[1].split("\t")[0] for l in led.split("\n")
                 if "event=measured-input" in l), "")
    inv = [l.rstrip("\n").split("\t") for l in open(os.path.join(case, "invocations.tsv"))][1:]
    rc = next((r[1] for r in inv if r[0] == "register"), "-")
    sid = open(os.path.join(case, "sid-artifact.txt"), encoding="utf-8").read().strip()
    written = "no" if sid.startswith("no codex") else "yes"
    content = ""
    m = re.search(r"content: (.+)", sid)
    if m: content = m.group(1).strip()
    planted = open(os.path.join(case, "planted-inputs.txt"), encoding="utf-8").read()
    ncand = planted.count("planted ")
    mdiff = sum(1 for _ in open(os.path.join(case, "meta.diff.txt"), encoding="utf-8"))
    rows.append((cid, slot, note, rc, ncand, written, content, mdiff,
                 led.count("pass=yes"), led.count("event=canary")))
L = ["## A-H5 — SC-211o, `_register-sid` (generated from the captures)", "",
     "One fixture fact varies per case. `sid` is whether a `codex.<slot>.sid` artifact was",
     "written and what it contains; `meta diff` is the change to the session's own meta",
     "across the invocation. Candidate files and meta keys are controller-planted input",
     "data, listed per case in `planted-inputs.txt`. Nothing here says which candidate",
     "should win.", ""]
w = [("case", 26), ("slot", 9), ("rc", 3), ("cand", 5), ("sid", 4), ("meta diff", 9), ("canary", 7)]
L.append("  " + "  ".join(h.ljust(n) for h, n in w))
L.append("  " + "  ".join("-" * n for _, n in w))
for r in rows:
    L.append("  " + "  ".join(str(v)[:n].ljust(n) for v, (_, n) in zip(
        [r[0], r[1], r[3], r[4], r[5], r[7], f"{r[8]}/{r[9]}"], w)))
L += ["", "## what each case varied, and what the sid artifact carried", ""]
for r in rows:
    L.append(f"  {r[0]}")
    L.append(f"      varied : {r[2]}")
    L.append(f"      sid    : {r[6] or '(none written)'}")
L += ["", "## distinct readings", "",
      f"  cases writing a sid artifact: {sum(1 for r in rows if r[5]=='yes')} of {len(rows)}",
      f"  distinct sid contents: {len({r[6] for r in rows if r[6]})}",
      f"  distinct rc values: {sorted({r[3] for r in rows})}",
      f"  cases whose meta changed across the invocation: {sum(1 for r in rows if r[7])}"]
txt = "\n".join(L) + "\n"
open(out, "w").write(txt)
print(f"wrote {out} sha256={hashlib.sha256(txt.encode()).hexdigest()}")
