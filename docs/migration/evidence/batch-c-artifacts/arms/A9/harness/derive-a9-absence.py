#!/usr/bin/env python3
"""A9 absence record — the source state beside the rendering, for every case.

SC-405i is an ABSENCE claim, and three states collapse to the same emptiness in a
rendering: meta ABSENT, meta PRESENT BUT UNREADABLE, and a reader that never looked. The
first two are separated HERE, from the filesystem (`meta-state.txt`), because a consumer
that renders neither cannot tell them apart. The third is excluded by carrying G1/healthy
through the identical consumer set: a reader that looks renders agents and a goal there.

Nothing is re-run and nothing is classified. Every value is read out of a captured file,
each source is hashed, and the pairwise comparison reports which renderings are identical
and on which keys the others differ.

usage: derive-a9-absence.py <arm-dir> <out.txt>
"""
import glob, hashlib, json, os, sys

arm, out_p = sys.argv[1], sys.argv[2]

def sha(p):
    try: return hashlib.sha256(open(p, "rb").read()).hexdigest()
    except OSError: return "-"

def kvblock(p):
    """meta-state.txt is sectioned by '### <file>'; return {section: {k: v}}."""
    cur, d = None, {}
    try: lines = open(p, encoding="utf-8", errors="replace").read().splitlines()
    except OSError: return d
    for line in lines:
        if line.startswith("### "): cur = line[4:].strip(); d[cur] = {}
        elif "=" in line and cur:
            k, v = line.strip().split("=", 1); d[cur][k] = v
    return d

def session_obj(p):
    try: doc = json.load(open(p, encoding="utf-8"))
    except Exception: return None
    ss = doc.get("sessions") or []
    return ss[0] if ss else None

rows = []
for case in sorted(glob.glob(os.path.join(arm, "a9-c*"))):
    cid = os.path.basename(case)
    st = kvblock(os.path.join(case, "meta-state.txt"))
    meta, ev = st.get("meta", {}), st.get("events.jsonl", {})
    lj = os.path.join(case, "out", "list-json.stdout")
    laj = os.path.join(case, "out", "list-all-json.stdout")
    s = session_obj(lj)
    sa = session_obj(laj)
    # the rendered object, canonically serialised, is what "identical rendering" means here
    canon = json.dumps(s, sort_keys=True, separators=(",", ":")) if s is not None else "<no session rendered>"
    canon_all = json.dumps(sa, sort_keys=True, separators=(",", ":")) if sa is not None else "<no session rendered>"
    rows.append({
        "case": cid,
        "meta_exists": meta.get("exists", "-"),
        "meta_mode": meta.get("mode", "-"),
        "meta_rc": meta.get("read_attempt_rc", "-"),
        "meta_sha": meta.get("sha256", "-"),
        "ev_exists": ev.get("exists", "-"),
        "ev_size": ev.get("size", "-"),
        "listjson_sha": sha(lj),
        "obj": s, "canon": canon,
        "obj_all": sa, "canon_all": canon_all,
        "obj_sha": hashlib.sha256(canon.encode()).hexdigest()[:12],
        "obj_all_sha": hashlib.sha256(canon_all.encode()).hexdigest()[:12],
        "agents": "-" if s is None else str(len(s.get("agents") or [])),
        "goal": "-" if s is None else ("null" if s.get("goal") is None else "set"),
        "goal_epoch": "-" if s is None else ("null" if s.get("goal_set_epoch") is None else str(s["goal_set_epoch"])),
        "last_active": "-" if s is None else str(s.get("last_active_epoch")),
        "status": "-" if s is None else str(s.get("status")),
    })

L = ["## A9 absence record — source state beside rendering (generated; nothing retyped)", "",
     "`meta rc` is the exit status of an actual read attempt as the consumer's own uid, so",
     "ABSENT (no such file) and PRESENT-BUT-UNREADABLE (mode 000) are different rows here",
     "even where the rendering cannot tell them apart.", ""]
w = [("case", 36), ("meta", 6), ("mode", 5), ("rc", 3), ("events", 7), ("ev_size", 8),
     ("status", 8), ("goal", 5), ("goal_epoch", 11), ("last_active", 12), ("agents", 6), ("obj#", 13)]
L.append("  " + "  ".join(h.ljust(n) for h, n in w))
L.append("  " + "  ".join("-" * n for _, n in w))
for r in rows:
    L.append("  " + "  ".join(str(v).ljust(n) for v, (_, n) in zip(
        [r["case"], r["meta_exists"], r["meta_mode"], r["meta_rc"], r["ev_exists"], r["ev_size"],
         r["status"], r["goal"], r["goal_epoch"], r["last_active"], r["agents"], r["obj_sha"]], w)))

L += ["", "## the three-way discriminator the row needs", "",
      "Rendered `list --json` session objects, compared canonically (sorted keys). Reported",
      "as identical / differing-on-these-keys. No reading is called right here.", ""]
live = {r["case"]: r for r in rows if r["case"].endswith("-ro")}
names = sorted(live)
for i in range(len(names)):
    for j in range(i + 1, len(names)):
        a, b = live[names[i]], live[names[j]]
        if a["obj"] is None or b["obj"] is None:
            verdict = "one of the two rendered no session object"
        elif a["canon"] == b["canon"]:
            verdict = "IDENTICAL rendering"
        else:
            keys = sorted(set(a["obj"]) | set(b["obj"]))
            diff = [k for k in keys if a["obj"].get(k) != b["obj"].get(k)]
            verdict = "differs on: " + ", ".join(diff)
        L.append(f"  {names[i]}")
        L.append(f"    vs {names[j]}: {verdict}")
L += ["", "## instrument control — did the reader look at all?", "",
      "`a9-c06-healthy-control` is the same consumer set on an intact fixture. Its readings:", ""]
for r in rows:
    if r["case"].startswith("a9-c06"):
        L.append(f"  {r['case']}: status={r['status']} goal={r['goal']} agents={r['agents']} "
                 f"last_active_epoch={r['last_active']}")
L += ["",
      "An empty rendering in another case is therefore a reading taken with a reader that is",
      "known to render agents and a goal when the fixture carries them.", "",
      "## distinct readings", ""]
for mode in ("-ro", "-rw", "-ro-noserver"):
    sel = [r for r in rows if r["case"].endswith(mode)]
    if mode == "-ro":
        sel = [r for r in sel if not r["case"].endswith("-ro-noserver")]
    d = len({r["canon"] for r in sel})
    da = len({r["canon_all"] for r in sel})
    L.append(f"  mode {mode.lstrip('-'):<12} cases={len(sel):<3} distinct list --json objects={d}  "
             f"distinct list --all --json objects={da}")
L += ["",
      "## sources", ""]
for r in rows:
    L.append(f"  {r['case']}: meta-state.txt sha256={sha(os.path.join(arm, r['case'], 'meta-state.txt'))}")
    L.append(f"      out/list-json.stdout sha256={r['listjson_sha']}")
txt = "\n".join(L) + "\n"
open(out_p, "w").write(txt)
print(f"wrote {out_p} sha256={hashlib.sha256(txt.encode()).hexdigest()}")
