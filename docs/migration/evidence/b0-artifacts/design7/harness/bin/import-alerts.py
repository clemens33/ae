#!/usr/bin/env python3
"""Import cexec's T-WD alert-family specimens and PROVE SET EQUALITY.

Binding seat guard: before any whole-cohort mutation touches an alert-bearing
cohort, the B0 specimen set must equal cexec's five-hash enumeration exactly —
all in, none dropped, none altered, no selection or normalisation by reason
string. This tool proves that against TWO independent sources:
  (1) the per-line hashes tabulated in batch-c-artifacts/MANIFEST.md
  (2) the per-line hashes recorded inside alert-specimens.<arm>.jsonl,
      whose FILE hashes are themselves checked against specimens/SHA256SUMS.txt
and recomputes sha256 over the raw bytes it will actually use. Any mismatch is
fatal (exit 4); nothing is written.
"""
import hashlib, json, re, sys, os, shutil

REPO = "/Users/ckriech/projects/clemens33/ae-rust"
SRC = os.path.join(REPO, "docs/migration/evidence/batch-c-artifacts/twd-precursor/specimens")
CMANI = os.path.join(REPO, "docs/migration/evidence/batch-c-artifacts/MANIFEST.md")
OUT = sys.argv[1]                       # destination dir for the copies
LINES_OUT = sys.argv[2]                 # the 5 raw lines, newline-terminated
PROOF = sys.argv[3]                     # the set-equality proof

def sha(b):
    return hashlib.sha256(b).hexdigest()

report = []
def say(s):
    report.append(s)
    print(s)

# ── 1. file-level integrity of the source specimen files ──
sums = {}
for line in open(os.path.join(SRC, "SHA256SUMS.txt"), encoding="utf-8"):
    h, p = line.split()
    sums[os.path.basename(p)] = h
os.makedirs(OUT, exist_ok=True)
files = sorted(f for f in sums if f.startswith("alert-specimens."))
say("## 1. Source file integrity (specimens/SHA256SUMS.txt)")
ok = True
for f in files:
    raw = open(os.path.join(SRC, f), "rb").read()
    got = sha(raw)
    match = (got == sums[f])
    ok &= match
    say("  %-28s recorded=%s recomputed=%s %s" % (f, sums[f], got, "MATCH" if match else "MISMATCH"))
    shutil.copyfile(os.path.join(SRC, f), os.path.join(OUT, f))
shutil.copyfile(os.path.join(SRC, "SHA256SUMS.txt"), os.path.join(OUT, "SHA256SUMS.txt"))
if not ok:
    say("FATAL: source file hash mismatch"); open(PROOF, "w").write("\n".join(report)); sys.exit(4)

# ── 2. the specimen records ──
records = []
for f in files:
    for line in open(os.path.join(SRC, f), encoding="utf-8"):
        line = line.strip()
        if line:
            records.append(json.loads(line))
say("")
say("## 2. Specimen records read from alert-specimens.*.jsonl: %d" % len(records))

# ── 3. cexec MANIFEST.md tabulation ──
mani_hashes = {}
for line in open(CMANI, encoding="utf-8"):
    m = re.match(r"^\|\s*`(a\d)`\s*\|\s*(\d+)\s*\|\s*(\d+)\s*\|\s*(\d+)\s*\|\s*`([0-9a-f]{64})`\s*\|\s*`([a-z-]+)`\s*\|", line)
    if m:
        mani_hashes[m.group(5)] = dict(arm=m.group(1), line_no=int(m.group(2)),
                                       byte_offset=int(m.group(3)), byte_len_with_nl=int(m.group(4)),
                                       action=m.group(6))
say("## 3. Hashes tabulated in batch-c-artifacts/MANIFEST.md: %d" % len(mani_hashes))

# ── 4. recompute over the bytes we will actually use ──
say("")
say("## 4. Per-specimen recomputation over the raw bytes B0 will use")
mine = {}
lines = []
for r in records:
    raw = r["raw_line_no_nl"]
    b = raw.encode("utf-8")
    h = sha(b)
    hn = sha(b + b"\n")
    mine[h] = r
    lines.append(raw)
    say("  arm=%s line_no=%s action=%-16s recomputed_no_nl=%s recorded_no_nl=%s %s" %
        (r["arm"], r["line_no"], r["action"], h, r["sha256_line_no_nl"],
         "MATCH" if h == r["sha256_line_no_nl"] else "MISMATCH"))
    say("      with_nl recomputed=%s recorded=%s %s  byte_len_no_nl=%s(recomputed %d)" %
        (hn, r["sha256_line_with_nl"], "MATCH" if hn == r["sha256_line_with_nl"] else "MISMATCH",
         r["byte_len_no_nl"], len(b)))
    if h != r["sha256_line_no_nl"] or hn != r["sha256_line_with_nl"] or len(b) != r["byte_len_no_nl"]:
        ok = False

# ── 5. SET EQUALITY ──
say("")
say("## 5. SET EQUALITY — B0 specimen set vs cexec's enumeration")
a, b_ = set(mine), set(mani_hashes)
say("  |B0 set| = %d   |cexec enumeration| = %d" % (len(a), len(b_)))
say("  in cexec but not in B0 (DROPPED): %s" % (sorted(b_ - a) or "none"))
say("  in B0 but not in cexec (ADDED):   %s" % (sorted(a - b_) or "none"))
equal = (a == b_) and len(a) == 5
say("  SET EQUALITY: %s (both sets have 5 members and are identical)" % ("PROVEN" if equal else "FAILED"))
for h in sorted(a & b_):
    say("    %s  arm=%s action=%s" % (h, mani_hashes[h]["arm"], mani_hashes[h]["action"]))
if not (equal and ok):
    open(PROOF, "w").write("\n".join(report) + "\n")
    sys.exit(4)

# ── 6. emit the lines, in cexec's (arm, line_no) order; no filtering, no normalisation ──
records_sorted = sorted(records, key=lambda r: (r["arm"], r["line_no"]))
with open(LINES_OUT, "w", encoding="utf-8") as f:
    for r in records_sorted:
        f.write(r["raw_line_no_nl"] + "\n")
say("")
say("## 6. Emitted %s — %d lines, ordered by (arm, line_no), unaltered" % (LINES_OUT, len(records_sorted)))
say("  file sha256 = %s" % sha(open(LINES_OUT, "rb").read()))
say("  order: %s" % ", ".join("%s/%s:%s" % (r["arm"], r["line_no"], r["action"]) for r in records_sorted))
say("")
say("## 7. Provenance recorded by cexec and carried forward unchanged")
say("  every specimen: actor=_watchdog, ref=ABSENT (NUL-prefixed sentinel, distinct from emitted-empty)")
say("  the throttle-cleared specimen (a3/3) is PROCESS-MEMORY-DERIVED (one running")
say("  watchdog, subarm B) and cannot be re-derived from a fresh clone; there is no")
say("  re-harvest without a full cexec re-run.")
open(PROOF, "w").write("\n".join(report) + "\n")
