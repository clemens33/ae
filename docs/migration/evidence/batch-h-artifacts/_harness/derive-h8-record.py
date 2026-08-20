#!/usr/bin/env python3
"""A-H8 record — generated from the captures. Facts only.

usage: derive-h8-record.py <arm-dir> <out.txt>
"""
import glob, hashlib, os, re, sys
arm, out = sys.argv[1], sys.argv[2]
L = ["## A-H8 — SC-211n, the long-lived query (generated from the captures)", "",
     "This surface never exits on its own: every case is closed by a NAMED barrier and then",
     "terminated by the controller. A termination here is a controller action and is never",
     "reported as a product rc; a barrier not seen within its bound is INCONCLUSIVE.", "",
     "## replay cohorts — planted against rendered", ""]
for case in sorted(glob.glob(os.path.join(arm, "h8-c0[234]*"))):
    rec = open(os.path.join(case, "replay-record.txt"), encoding="utf-8").read()
    def g(k):
        m = re.search(rf"{k}=(\S+)", rec); return m.group(1) if m else "-"
    L.append(f"  {os.path.basename(case):24s} planted_events={g('planted_events'):3s} "
             f"planted_lines={g('planted_lines'):3s} rendered_lines={g('rendered_lines'):3s} "
             f"first={g('first_rendered'):18s} last={g('last_rendered')}")
L += ["", "## the other cases", ""]
for case in sorted(glob.glob(os.path.join(arm, "h8-c0[1567]*"))):
    led = open(os.path.join(case, "admissibility-ledger.txt"), encoding="utf-8").read()
    note = (re.search(r"event=measured-input\tnote=(.*?)(?:\t|\n)", led) or [None, ""])[1]
    L.append(f"  {os.path.basename(case)}")
    L.append(f"      input            : {note}")
    L.append(f"      barriers seen    : {len(re.findall(r'event=follow-barrier.*seen=yes', led))}")
    L.append(f"      inconclusive     : {led.count('OUTCOME-INCONCLUSIVE')}")
    L.append(f"      canaries passing : {led.count('pass=yes')}")
    for m in re.finditer(r"event=(follow-barrier|transition|partial-step-\d)\t(.*)", led):
        L.append(f"      {m.group(1):16s} : {m.group(2)[:96]}")
L += ["", "## distinct readings", ""]
cohort = {}
for case in sorted(glob.glob(os.path.join(arm, "h8-c0[234]*"))):
    rec = open(os.path.join(case, "replay-record.txt"), encoding="utf-8").read()
    p = re.search(r"planted_events=(\d+)", rec).group(1)
    r = re.search(r"rendered_lines=(\d+)", rec).group(1)
    f = re.search(r"first_rendered=(\S*)", rec).group(1)
    cohort[p] = (r, f)
L.append(f"  cohorts (planted -> rendered, first rendered): " +
         ", ".join(f"{k} -> {v[0]} ({v[1]})" for k, v in sorted(cohort.items(), key=lambda x: int(x[0]))))
txt = "\n".join(L) + "\n"
open(out, "w").write(txt)
print(f"wrote {out} sha256={hashlib.sha256(txt.encode()).hexdigest()}")
