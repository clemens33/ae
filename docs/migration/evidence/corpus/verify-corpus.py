#!/usr/bin/env python3
"""GATE — reads the corpus and the freeze record, and NEVER WRITES.

This program has no write path. Not "does not currently write" — there is no open()
in write mode, no temp file, no rename. That is deliberate: the failure this guards
against is a gate that regenerates the table it reports on, which overwrites the
drift it exists to detect. Its sibling `freeze-corpus.py` writes; this one cannot.

Exit 0 = the corpus on disk matches the freeze record.
Exit 1 = drift, and every difference is named.

Usage:  ./verify-corpus.py            # verify
        ./verify-corpus.py --roles    # also print the role census
"""
import hashlib, os, sys

HERE = os.path.dirname(os.path.abspath(__file__))
SRC = os.path.normpath(os.path.join(HERE, "..", "batch-c-artifacts"))
MANIFEST = os.path.join(HERE, "CORPUS-MANIFEST.tsv")
FREEZE = os.path.join(HERE, "FREEZE.txt")

def sha256(p):
    h = hashlib.sha256()
    with open(p, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 16), b""):
            h.update(chunk)
    return h.hexdigest()

def main():
    fails = []
    if not os.path.exists(MANIFEST) or not os.path.exists(FREEZE):
        print("FAIL  freeze record or manifest absent — nothing to verify against")
        return 1
    rows = []
    with open(MANIFEST, encoding="utf-8") as f:
        hdr = f.readline().rstrip("\n").split("\t")
        if hdr != ["role", "sha256", "bytes", "path"]:
            print("FAIL  manifest header is not the expected form: %r" % hdr); return 1
        for line in f:
            c = line.rstrip("\n").split("\t")
            if len(c) != 4:
                fails.append("manifest row has %d cells, expected 4: %r" % (len(c), c[:2])); continue
            rows.append(c)
    recorded = {}
    with open(FREEZE, encoding="utf-8") as f:
        for line in f:
            if line.startswith("#") or not line.strip(): continue
            k, _, v = line.rstrip("\n").partition("\t")
            recorded[k] = v

    # 1. every listed file present and unchanged
    listed = set()
    for role, want, nbytes, rel in rows:
        listed.add(rel)
        p = os.path.join(SRC, rel)
        if not os.path.exists(p):
            fails.append("MISSING  %s" % rel); continue
        got = sha256(p)
        if got != want:
            fails.append("CHANGED  %s\n         manifest %s\n         on disk  %s" % (rel, want, got))
        elif str(os.path.getsize(p)) != nbytes:
            fails.append("SIZE     %s (manifest %s)" % (rel, nbytes))

    # 2. nothing on disk that the manifest does not list
    on_disk = set()
    for dirpath, dirnames, filenames in os.walk(SRC):
        for fn in filenames:
            on_disk.add(os.path.relpath(os.path.join(dirpath, fn), SRC))
    for extra in sorted(on_disk - listed):
        fails.append("UNLISTED %s — present on disk, absent from the freeze" % extra)

    # 3. the root digest, which moves on any addition, removal or edit
    h = hashlib.sha256()
    for r in rows:
        h.update(("\t".join(r) + "\n").encode())
    if h.hexdigest() != recorded.get("root_digest"):
        fails.append("ROOT DIGEST  manifest yields %s, freeze records %s"
                     % (h.hexdigest(), recorded.get("root_digest")))
    if recorded.get("files") != str(len(rows)):
        fails.append("FILE COUNT   manifest has %d rows, freeze records %s" % (len(rows), recorded.get("files")))

    if "--roles" in sys.argv:
        counts = {}
        for r in rows: counts[r[0]] = counts.get(r[0], 0) + 1
        for k in sorted(counts): print("role %-14s %d" % (k, counts[k]))

    for m in fails: print("FAIL  %s" % m)
    print("CORPUS VERIFIED — %d files, root %s" % (len(rows), recorded.get("root_digest", "")[:16])
          if not fails else "CORPUS DRIFT — %d finding(s)" % len(fails))
    return 1 if fails else 0

if __name__ == "__main__":
    sys.exit(main())
