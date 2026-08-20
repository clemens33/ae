#!/usr/bin/env python3
"""Re-derive checklist.txt from each A5 case's captured doctor stdout.

Pure post-processing of an already-captured artifact — the source sha256 is recorded in the
derived file and nothing is re-run. The first extraction used a bracketed pattern the
product does not print, so it captured only the Summary line; a checklist that silently
drops the checklist is the same lossy-filter class as the attention-fields one.
"""
import hashlib, os, re, sys
A = sys.argv[1] if len(sys.argv) > 1 else \
    "/Users/ckriech/projects/clemens33/ae-rust/docs/migration/evidence/batch-c-artifacts/arms/A5"
pat = re.compile(r"^(OK|WARN|FAIL)\s|^Summary:")
n = 0
for c in sorted(os.listdir(A)):
    src = os.path.join(A, c, "out", "doctor.stdout")
    if not os.path.exists(src):
        continue
    raw = open(src, "rb").read()
    lines = [l for l in raw.decode("utf-8", "replace").splitlines() if pat.match(l)]
    out = [f"## doctor checklist, re-derived from out/doctor.stdout",
           f"source=out/doctor.stdout sha256={hashlib.sha256(raw).hexdigest()} bytes={len(raw)}",
           "derivation=pure post-processing of the captured bytes; nothing was re-run", ""]
    out += lines
    open(os.path.join(A, c, "checklist.txt"), "w").write("\n".join(out) + "\n")
    n += 1
print(f"rebuilt {n} checklist.txt")
