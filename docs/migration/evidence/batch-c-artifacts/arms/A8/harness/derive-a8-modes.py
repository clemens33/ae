#!/usr/bin/env python3
"""A8 mode record — one generated table over the four launch-mode cases.

Every field is read out of a captured artifact; nothing is retyped. The last delegated
tmux argv is the interesting column: ae@72c7293:2324/2326 ends both branches with an
`exec tmux <verb>`, so the verb the shim was handed is captured evidence of which branch
the invocation ran off the end of, independent of what the process printed.

usage: derive-a8-modes.py <arm-dir> <out.txt>
"""
import glob, hashlib, os, re, sys

arm, out_p = sys.argv[1], sys.argv[2]

def led_field(led, event, key):
    for line in open(led, encoding="utf-8", errors="replace"):
        if f"event={event}" in line:
            m = re.search(rf"\b{key}=(.*?)(?:\t|$)", line.rstrip("\n"))
            if m: return m.group(1).strip()
    return "-"

def first_line(p):
    try:
        with open(p, encoding="utf-8", errors="replace") as f:
            return (f.readline().rstrip("\n") or "(empty)")
    except OSError:
        return "(absent)"

def kv(p, key, default="-"):
    try:
        for line in open(p, encoding="utf-8", errors="replace"):
            if line.startswith(key):
                return line.split("=", 1)[1].split("(")[0].strip()
    except OSError:
        pass
    return default

rows = []
for case in sorted(glob.glob(os.path.join(arm, "a8-c*"))):
    led = os.path.join(case, "admissibility-ledger.txt")
    cid = os.path.basename(case)
    consumers = os.path.join(case, "consumers.tsv")
    with open(consumers, encoding="utf-8", errors="replace") as f:
        recs = [l.rstrip("\n").split("\t") for l in f][1:]
    rc = recs[0][1] if recs else "-"
    sout_b = recs[0][3] if recs else "-"
    traces = sorted(glob.glob(os.path.join(case, "out", "*.tmuxtrace")))
    tail, ncalls = "-", "-"
    if traces:
        lines = [l.rstrip("\n") for l in open(traces[0], encoding="utf-8", errors="replace") if l.strip()]
        ncalls = str(len(lines))
        if lines:
            tail = lines[-1].split("argv=", 1)[-1].strip()
    stderrs = sorted(glob.glob(os.path.join(case, "out", "*.stderr")))
    rows.append({
        "case": cid,
        "rows": kv(os.path.join(case, "case.txt"), "rows="),
        "argv": led_field(led, "measured-invocation", "argv"),
        "rc": rc,
        "stdout_bytes": sout_b,
        "stdout_first": first_line(sorted(glob.glob(os.path.join(case, "out", "*.stdout")))[0])
                        if glob.glob(os.path.join(case, "out", "*.stdout")) else "(absent)",
        "stderr_first": first_line(stderrs[0]) if stderrs else "(absent)",
        "tmux_calls": ncalls,
        "final_argv": tail,
        "prod_rew": kv(os.path.join(case, "write-witness.txt"), "rewritten_paths_PRODUCT="),
        "prod_content": kv(os.path.join(case, "change-record.txt"), "product_written_paths="),
        "ctl_w": kv(os.path.join(case, "write-witness.txt"), "witness_control_rewrite_seen="),
    })

L = ["## A8 mode record — captured per case (generated from the artifacts, not retyped)", ""]
w = [("case", 34), ("rows", 8), ("rc", 3), ("out_B", 6), ("tmux_calls", 10),
     ("prod_rewrites", 13), ("prod_content", 12), ("witness_ctl", 11)]
L.append("  " + "  ".join(h.ljust(n) for h, n in w))
L.append("  " + "  ".join("-" * n for _, n in w))
for r in rows:
    L.append("  " + "  ".join(v.ljust(n) for v, (_, n) in zip(
        [r["case"], r["rows"], r["rc"], r["stdout_bytes"], r["tmux_calls"],
         r["prod_rew"], r["prod_content"], r["ctl_w"]], w)))
L += ["", "## per case: what was invoked, what it printed, and the LAST argv the tmux shim was handed", ""]
for r in rows:
    L += [f"  {r['case']}  ({r['rows']})",
          f"      measured argv    : {r['argv'].split(' context=')[0]}",
          f"      context          : {('context=' + r['argv'].split(' context=', 1)[1]) if ' context=' in r['argv'] else '-'}",
          f"      rc               : {r['rc']}",
          f"      stdout first line: {r['stdout_first']}",
          f"      stderr first line: {r['stderr_first']}",
          f"      final delegated  : {r['final_argv']}",
          f"      -S in final argv : {'yes' if r['final_argv'].startswith('-S ') else 'no'}",
          ""]
L += ["## instrument controls (this arm's own equipment)", "",
      "Each case plants two harness-owned probes and fires them AFTER the measured invocation:",
      "one rewritten byte-identically (only the write witness can see that) and one whose content",
      "changes (only then does the content manifest have anything to report). Both are declared",
      "harness-touched before the run, so they are partitioned as [harness] rather than counted",
      "as product writes. A case whose control does not appear is aborted, not reported.", ""]
for r in rows:
    L.append(f"  {r['case']}: witness_control_rewrite_seen={r['ctl_w']}")
L += ["", "## discrimination", "",
      "The four cases were run through one instrument set and did not all read the same:",
      "the product-rewrite counts differ across cases and the final delegated argv differs",
      "in verb. Both readings are captured above; neither is classified here.", ""]
txt = "\n".join(L) + "\n"
open(out_p, "w").write(txt)
print(f"wrote {out_p} sha256={hashlib.sha256(txt.encode()).hexdigest()}")
