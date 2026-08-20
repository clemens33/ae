#!/usr/bin/env python3
"""A-H1301 record — generated from the captures. Facts only; no relation stated.

usage: derive-h1301-record.py <arm-dir> <out.txt>
"""
import glob, hashlib, os, re, sys
arm, out = sys.argv[1], sys.argv[2]
def led_of(c): return open(os.path.join(c, "admissibility-ledger.txt"), encoding="utf-8").read()
def f(c, n):
    p = os.path.join(c, n)
    return open(p, encoding="utf-8", errors="replace").read().strip() if os.path.exists(p) else ""
L = ["## A-H1301 — SC-1301, three writer-shaped cuts (generated from the captures)", "",
     "Three cuts, THREE DIFFERENT evidence claims, because the three writers do not share a",
     "boundary. Nothing here says what any reading means.", ""]
eq = os.path.join(arm, "h1301-c00-inactive-equivalence")
if os.path.isdir(eq):
    l = led_of(eq)
    m = re.search(r"event=hash-triple\t(.*)", l)
    e = re.search(r"event=inactive-equivalence\t(.*)", l)
    L += ["## admissibility", "", f"  {m.group(1) if m else '-'}", "", f"  {e.group(1) if e else '-'}", "",
          "  differing files (the hook rides the _lib emission list, so _lib and the files that",
          "  quote it necessarily differ):"]
    for line in f(eq, "equiv.differing-files.txt").split("\n"):
        if line.strip(): L.append(f"      {line}")
    L.append("")
for case, title in (("h1301-c01-atomic-temp-rename", "cut 1 — ae_meta_set, between the temp write and the rename"),
                    ("h1301-c02-spawn-partial-generation", "cut 2 — _cmd_spawn, between two of its own appends"),
                    ("h1301-c03-reader-fault-response", "cut 3 — start_capture_session_id: READER-FAULT RESPONSE, not a writer tear")):
    c = os.path.join(arm, case)
    if not os.path.isdir(c): continue
    l = led_of(c)
    L += [f"## {title}", ""]
    claim = re.search(r"event=rows\t.*?claim=(.*)", l)
    L.append(f"  claim        : {claim.group(1) if claim else '-'}")
    arm_ev = re.search(r"event=barrier-ARMED\t(.*)", l)
    if arm_ev: L.append(f"  barrier      : {arm_ev.group(1)}")
    for ev in ("reader-at-barrier", "controller-action", "controller-mutation"):
        m = re.search(rf"event={ev}\t(.*)", l)
        if m: L.append(f"  {ev:12s} : {m.group(1)[:110]}")
    if case.endswith("temp-rename"):
        L.append(f"  temp files at the barrier : {f(c, 'temp-files-at-barrier.txt')}")
        L.append(f"  reader at the barrier     : {f(c, 'out/reader-at-barrier.stdout')}")
        L.append(f"  meta before vs at barrier : {len(f(c, 'meta.before-vs-barrier.diff').split(chr(10))) if f(c,'meta.before-vs-barrier.diff') else 0} diff line(s)")
        L.append(f"  meta at barrier vs after  : {len(f(c, 'meta.barrier-vs-after.diff').split(chr(10))) if f(c,'meta.barrier-vs-after.diff') else 0} diff line(s)")
    if "spawn" in case:
        L.append("  meta after the kill, agent rows:")
        for line in f(c, "partial-generation.txt").split("\n"):
            if line.startswith("agent"): L.append(f"      {line}")
    if "reader-fault" in case:
        for line in f(c, "controller-mutation.txt").split("\n")[:6]:
            L.append(f"      {line}")
        L.append(f"  reader stdout first line : {f(c, 'out/reader.stdout').split(chr(10))[0][:80]}")
        L.append(f"  goal reader              : {f(c, 'out/reader-goal.stdout')[:60]}")
    L.append(f"  canaries passing          : {l.count('pass=yes')} of {l.count('event=canary')}")
    L.append(f"  inconclusive outcomes     : {l.count('OUTCOME-INCONCLUSIVE')}")
    L.append("")
txt = "\n".join(L) + "\n"
open(out, "w").write(txt)
print(f"wrote {out} sha256={hashlib.sha256(txt.encode()).hexdigest()}")
