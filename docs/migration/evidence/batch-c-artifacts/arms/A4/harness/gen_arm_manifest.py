#!/usr/bin/env python3
"""Emit an arm group's MANIFEST section from its COMMITTED artifacts. Facts only."""
import csv, os, sys
ARM=sys.argv[1]
DEST=f"/Users/ckriech/projects/clemens33/ae-rust/docs/migration/evidence/batch-c-artifacts/arms/{ARM}"
def kv(p):
    d={}
    for line in open(p, encoding="utf-8", errors="replace"):
        if "=" in line:
            k,v=line.rstrip("\n").split("=",1); d.setdefault(k,v)
    return d
led={}
with open(os.path.join(DEST,"ledger.tsv")) as fh:
    for r in csv.DictReader(fh, delimiter="\t"): led[r["case"]]=r
rows=[]
for c in sorted(os.listdir(DEST)):
    p=os.path.join(DEST,c)
    if not os.path.isdir(p) or not os.path.exists(os.path.join(p,"case.txt")): continue
    d=kv(os.path.join(p,"case.txt"))
    parts=c.rsplit("-",1); base,mode=(parts[0],parts[1]) if len(parts)==2 else (c,"?")
    l=led.get(base,{})
    ncons=sum(1 for _ in open(os.path.join(p,"consumers.tsv")))-1
    lg=open(os.path.join(p,"admissibility-ledger.txt")).read().splitlines()
    def seq_of(ev):
        for x in lg:
            if f"event={ev}" in x: return int(x.split("\t")[0].split("=")[1])
        return None
    tab=seq_of("env-tab-selfcheck-COMPLETE"); eq=seq_of("tmux-shim-equivalence-COMPLETE")
    # "first consumer activity" is the earliest of a normal consumer START and a hooked
    # barrier being ARMED — a barrier case launches its consumer through the hook, so it
    # has no consumer-START line and must not read as unordered.
    cands=[x for x in (seq_of("consumer-START"), seq_of("barrier-ARMED")) if x is not None]
    first=min(cands) if cands else None
    order="yes" if first is not None and tab is not None and tab < first and (eq is None or eq < first) else "NO"
    rows.append((c,base,mode,l.get("rows","-"),l.get("group","-"),l.get("member","-"),
                 d.get("clone_fingerprint_matches_template","-"), d.get("manifest_diff_lines","-"),
                 d.get("tmux_snapshot_identical","-"), ncons,
                 f"{tab if tab is not None else '-'}/{eq if eq is not None else '-'}/{first if first is not None else '-'}", order))
o=[]
o.append(f"### {ARM} case table\n")
o.append("`checks<first consumer` names the ledger sequence numbers of the TAB round-trip")
o.append("COMPLETE, the tmux-shim equivalence COMPLETE (`-` where the case starts no server),")
o.append("and the first `consumer-START`. The ledger is append-only and written by the checks")
o.append("themselves, so the ordering is established by the original durable content — not by")
o.append("file mtimes and not by a hash list added afterwards. For a barrier case the first")
o.append("consumer activity is `barrier-ARMED` (the hooked run has no `consumer-START` line).\n")
o.append("A case whose design includes a CONTROLLER MUTATION necessarily shows a tmux delta;")
o.append("what the controller did, when, and from where is in `controller-mutation.txt` and in")
o.append("the ledger, and the before/at-barrier/after tmux snapshots bracket it.\n")
o.append("| case | clone | rows | template | clone fp = template fp | manifest diff | tmux snapshot identical | consumers | checks<first consumer | ordered |")
o.append("|---|---|---|---|---|---|---|---|---|---|")
for r in rows:
    o.append("| `%s` | %s | %s | `%s/%s` | %s | %s | %s | %d | %s | %s |" % (
        r[0],r[2],r[3],r[4],r[5],r[6],r[7],r[8],r[9],r[10],r[11]))
o.append("")
o.append(f"Artifact paths — `docs/migration/evidence/batch-c-artifacts/arms/{ARM}/<case>/`:\n")
o.append("- `admissibility-ledger.txt` — append-only, monotonic `seq` + UTC + epoch per event:")
o.append("  case open, rows, clone verification (clone vs expected fingerprint), the TAB")
o.append("  round-trip START/COMPLETE, the tmux-shim equivalence START/COMPLETE, the")
o.append("  before/after manifests, and every consumer START/COMPLETE with its rc and its")
o.append("  stdout / stderr / tmuxtrace sha256")
o.append("- `env-tab-selfcheck.txt` — the TAB round-trip in this case's own scrubbed")
o.append("  environment, plus the paired `LANG=LC_ALL=C` probe on the same throwaway server")
o.append("- `tmux-shim-equivalence.txt` — live cases only: the delegate-and-log shim proven")
o.append("  byte-identical to the real binary on this arm's own stable topology")
o.append("- `case.txt`, `env.txt`, `consumers.tsv` (label, rc, stdout/stderr sha256 + bytes,")
o.append("  tmuxtrace sha256 + line count, bounded flag, exact argv)")
o.append("- `out/<label>.stdout`, `out/<label>.stderr` (present only when non-empty),")
o.append("  `out/<label>.tmuxtrace` — per invocation: the effective `AE_TMUX_SERVER` and kind,")
o.append("  the effective locale, and the DELEGATED tmux argv")
o.append("- `manifest.before.tsv` / `manifest.after.tsv` / `manifest.diff.txt` — recursive:")
o.append("  type, mode, content hash, symlink target, path across the cloned AE_HOME")
o.append("- `tmux.before.txt` / `tmux.after.txt`")
o.append(f"- `{ARM}/ledger.tsv` (case -> row ids), `{ARM}/harness/` (the exact scripts and the")
o.append("  tmux shim), `SHA256SUMS.txt` (every file above)\n")
sys.stdout.write("\n".join(o)+"\n")
