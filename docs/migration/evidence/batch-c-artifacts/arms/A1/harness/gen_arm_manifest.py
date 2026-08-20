#!/usr/bin/env python3
"""Emit an arm group's MANIFEST section from its published artifacts. Facts only."""
import csv, os, sys
ARM=sys.argv[1]
DEST=f"/Users/ckriech/projects/clemens33/ae-rust/docs/migration/evidence/batch-c-artifacts/arms/{ARM}"
def kv(p):
    d={}
    for line in open(p, encoding="utf-8", errors="replace"):
        if "=" in line:
            k,v=line.rstrip("\n").split("=",1)
            d.setdefault(k,v)
    return d
led={}
with open(os.path.join(DEST,"ledger.tsv")) as fh:
    for r in csv.DictReader(fh, delimiter="\t"):
        led[r["case"]]=r
rows=[]
for c in sorted(os.listdir(DEST)):
    p=os.path.join(DEST,c)
    if not os.path.isdir(p): continue
    ct=os.path.join(p,"case.txt")
    if not os.path.exists(ct): continue
    d=kv(ct)
    base=c.rsplit("-",1)[0] if c.endswith(("-ro","-rw")) else c
    mode=c.rsplit("-",1)[1] if c.endswith(("-ro","-rw")) else "live"
    l=led.get(base,{})
    ncons=sum(1 for _ in open(os.path.join(p,"consumers.tsv")))-1
    rows.append((c,base,mode,l.get("rows","-"),l.get("group","-"),l.get("member","-"),
                 d.get("clone_fingerprint_matches_template","-"),
                 d.get("manifest_diff_lines","-"), d.get("tmux_snapshot_identical","-"), ncons))
out=[]
out.append(f"## Arm group {ARM} — schema/document (bash lane)\n")
out.append("Rows executed: SC-509, SC-509b, SC-506, SC-510a, SC-510b, SC-510c, SC-510d, SC-510e,")
out.append("SC-510f, SC-511a, SC-511b, SC-405k. (SC-511c is B0's and is not run here.)\n")
out.append("Each document case is run TWICE from the same template member: once on a **protected**")
out.append("clone (the design's read-only vehicle) and once on a **writable** clone whose modes are")
out.append("restored to what the producer wrote. Both are published. The pair exists because a")
out.append("protected clone can turn a write into a refusal rather than revealing it, while the")
out.append("writable clone lets the manifest diff be the proof the design asks for; reporting only")
out.append("one of the two would hide which of those two things happened.\n")
out.append("Consumer families per case, each captured as stdout + stderr + rc + byte counts +")
out.append("sha256: `ae list`, `ae list --json`, `ae list --all`, `ae list --all --json`, `ae ls`,")
out.append("`ae ls --all`, `ae status <session>`, `ae next`, the session's `requests all`, its")
out.append("`agents`, and its `events-tail`. `events-tail` is a streaming consumer with no one-shot")
out.append("mode, so it is bounded by the harness and the stop is recorded beside its bytes.")
out.append("Every consumer runs under `env -i` plus the documented minimum")
out.append("(`HOME`, `AE_HOME`, `PATH`, `TZ=UTC`, `LANG=LC_ALL=en_US.UTF-8`, `TERM`, `TMUX_TMPDIR`,")
out.append("`AE_TMUX_SERVER`+kind) — never inherited shell state. The exact env is published per")
out.append("case as `env.txt` and the exact argv per consumer in `consumers.tsv`.\n")
out.append("| case | clone | rows | template | clone fp = template fp | manifest diff (lines) | tmux snapshot identical | consumers |")
out.append("|---|---|---|---|---|---|---|---|")
for r in rows:
    out.append("| `%s` | %s | %s | `%s/%s` | %s | %s | %s | %d |" % (r[0],r[2],r[3],r[4],r[5],r[6],r[7],r[8],r[9]))
out.append("")
out.append(f"Artifact paths: `docs/migration/evidence/batch-c-artifacts/arms/{ARM}/<case>/` —")
out.append("`case.txt` (template + both fingerprints + clone fingerprint + verification result +")
out.append("manifest-diff line count + timestamps), `env.txt`, `consumers.tsv` (label, rc, stdout")
out.append("and stderr sha256 + byte counts, bounded flag, exact argv), `out/<label>.stdout` and")
out.append("`out/<label>.stderr` verbatim, `manifest.before.tsv` / `manifest.after.tsv` /")
out.append("`manifest.diff.txt` (recursive: type, mode, content hash, symlink target, path across")
out.append(f"the cloned AE_HOME), `tmux.before.txt` / `tmux.after.txt`, and `{ARM}/ledger.tsv`")
out.append("mapping every case to its row ids. `SHA256SUMS.txt` hashes every published file.\n")
print("\n".join(out))
