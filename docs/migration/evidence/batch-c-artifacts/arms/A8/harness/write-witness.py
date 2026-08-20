#!/usr/bin/env python3
"""WRITE WITNESS for a mutating arm — which paths were REWRITTEN, byte-identically or not.

A content-hash manifest is blind to a rewrite that produces identical bytes, and that is
exactly what regenerating a session's helper set does. This reads the inode/mtime/size
witness pair instead, so "nothing changed" and "everything was rewritten with the same
bytes" stop looking alike.

Every case plants a control: a harness-owned path the controller rewrites byte-identically
AFTER the measured invocation. If the control does not show up in the rewritten set, this
instrument is blind in that case and a rewritten_paths=0 reading means nothing.

usage: write-witness.py <case-dir> [harness-touched-path ...]
"""
import os, sys
c = sys.argv[1]
touched = set(sys.argv[2:])

def who(path):
    q = path.lstrip("./")
    return "harness" if any(q == t.lstrip("./") or q.startswith(t.lstrip("./") + "/")
                            for t in touched) else "PRODUCT"
def load(p):
    d = {}
    for line in open(p, encoding="utf-8", errors="replace"):
        f = line.rstrip("\n").split("\t")
        if len(f) == 4: d[f[3]] = (f[0], f[1], f[2])
    return d
b, a = load(os.path.join(c, "witness.before.tsv")), load(os.path.join(c, "witness.after.tsv"))
rew = sorted(p for p in set(a) & set(b) if a[p] != b[p])
L = ["## write witness — paths REWRITTEN, including byte-identical rewrites",
     "inode/mtime/size per path; the content manifest cannot see a rewrite that produces",
     "identical bytes, and regenerating the helper set does exactly that.", ""]
for p in rew:
    L.append(f"  [{who(p)}] {p}")
    L.append(f"      before inode={b[p][0]} mtime={b[p][1]} size={b[p][2]}")
    L.append(f"      after  inode={a[p][0]} mtime={a[p][1]} size={a[p][2]}")
    L.append(f"      inode_changed={b[p][0] != a[p][0]}  mtime_changed={b[p][1] != a[p][1]}  size_changed={b[p][2] != a[p][2]}")
if not rew: L.append("  (no path was rewritten)")
L.append("")
ctl = "./.a8-witness-probe-rewrite"
L.append("## control — the controller rewrote this path byte-identically AFTER the measured")
L.append("## invocation. An instrument that cannot report it here cannot report a product")
L.append("## rewrite either, so a bare rewritten_paths=0 above would be uninterpretable.")
L.append(f"witness_control_path={ctl}")
L.append(f"witness_control_rewrite_seen={'yes' if ctl in rew else 'no'}")
L.append("")
prod = [p for p in rew if who(p) == "PRODUCT"]
L.append(f"rewritten_paths={len(rew)}")
L.append(f"rewritten_paths_harness={len(rew) - len(prod)}   (the controller's own probes)")
L.append(f"rewritten_paths_PRODUCT={len(prod)}")
L.append(f"added_paths={len(set(a)-set(b))}  removed_paths={len(set(b)-set(a))}")
open(os.path.join(c, "write-witness.txt"), "w").write("\n".join(L) + "\n")
print(f"rewritten_paths={len(rew)} product={len(prod)}")
