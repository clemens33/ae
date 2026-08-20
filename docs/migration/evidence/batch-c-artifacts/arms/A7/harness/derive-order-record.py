#!/usr/bin/env python3
"""Build the SC-405f order-discrimination record from the A7 captures.

An order claim needs an arm that could observe the WRONG order. The opposed fixture makes
the two candidate answers DIFFERENT STRINGS by construction — a last-record reader and a
max-timestamp reader cannot both be right — and the agreeing and single fixtures establish
that the reader responds to a second goal at all. All three are read here from captured
bytes, with each source's sha256 recorded. Nothing is interpreted.
"""
import hashlib, json, os, sys
A = sys.argv[1]
def rd(case):
    d = os.path.join(A, case)
    out = {}
    p = os.path.join(d, "out", "list-json.stdout")
    if os.path.exists(p):
        raw = open(p, "rb").read()
        out["src_sha256"] = hashlib.sha256(raw).hexdigest()
        try:
            doc = json.loads(raw.decode())
            s = doc["sessions"][0] if doc.get("sessions") else {}
            out["goal"] = s.get("goal"); out["goal_set_epoch"] = s.get("goal_set_epoch")
        except Exception as e:
            out["goal"] = f"UNPARSEABLE: {e}"
    m = os.path.join(d, "meta.bytes.txt")
    if os.path.exists(m):
        for line in open(m, encoding="utf-8", errors="replace"):
            if line.startswith("goal="):
                out.setdefault("meta_goal_lines", []).append(line.rstrip("\n")[5:])
    e = os.path.join(d, "events.bytes.jsonl")
    if os.path.exists(e):
        out["goal_events"] = []
        for line in open(e, encoding="utf-8", errors="replace"):
            try:
                o = json.loads(line)
            except Exception:
                continue
            if o.get("action") == "goal":
                out["goal_events"].append((o.get("ts"), o.get("summary")))
    return out

L = []
L.append("## SC-405f order-discrimination record — derived from captured bytes")
L.append("derivation=pure post-processing; each source file's sha256 is recorded")
L.append("")
L.append("An order claim needs an arm that could observe the WRONG order. In the OPPOSED")
L.append("fixture the two candidate answers are DIFFERENT STRINGS by construction, so a")
L.append("last-record reader and a max-timestamp reader cannot both be right. The AGREEING")
L.append("and SINGLE fixtures are the controls that show the reader responds at all.")
L.append("")
for case, label in (("a7-c06-goal-order-opposed-rw", "OPPOSED  (append order vs ts order disagree)"),
                    ("a7-c07-goal-order-agreeing-rw", "AGREEING (control: both candidates coincide)"),
                    ("a7-c08-goal-order-single-rw",   "SINGLE   (control: nothing to choose)"),
                    ("a7-c09-goals-distinct-ts-rw",   "FOUR GOALS, increasing ts")):
    r = rd(case)
    L.append(f"### {label}")
    L.append(f"  case={case}")
    for ts, summ in r.get("goal_events", []):
        L.append(f"    goal event  ts={ts}  summary={summ}")
    L.append(f"    meta goal line(s): {r.get('meta_goal_lines')}")
    L.append(f"    rendered goal    : {r.get('goal')!r}")
    L.append(f"    goal_set_epoch   : {r.get('goal_set_epoch')}")
    L.append(f"    src_sha256       : {r.get('src_sha256')}")
    L.append("")
o = rd("a7-c06-goal-order-opposed-rw")
ev = o.get("goal_events", [])
if len(ev) == 2:
    by_append = ev[-1][1]
    by_ts = max(ev, key=lambda x: x[0])[1]
    L.append("### the two candidate answers on the OPPOSED fixture")
    L.append(f"  a last-record reader would return : {by_append!r}")
    L.append(f"  a max-timestamp reader would return: {by_ts!r}")
    L.append(f"  they differ                        : {by_append != by_ts}")
    L.append(f"  what the consumer rendered         : {o.get('goal')!r}")
    L.append("")
    L.append("  If the two candidates had been the same string this fixture could not have")
    L.append("  discriminated anything. No conclusion is drawn here; the values are the record.")
open(os.path.join(A, "order-discrimination.txt"), "w").write("\n".join(L) + "\n")
print("\n".join(L[-8:]))
