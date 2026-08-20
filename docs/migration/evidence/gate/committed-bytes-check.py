#!/usr/bin/env python3
"""COMMITTED-BYTES check — the fourth gate guarantee: CLONE FIDELITY.

The other three checks all read the WORKING TREE, so none can see a tree that passes
locally and fails on clone. At ce8965e a text filter (autocrlf=input) rewrote four D04b pty
logs on the way into the object database: the working file hashed 8320a0a5, the stored blob
hashed 20fffc72, and all three working-tree checks passed while a fresh clone would have
failed verification on four evidence artifacts.

Two parts, both non-invasive — nothing is staged, committed, or written to the object DB:

A. WOULD-NORMALIZE (pre-commit, blocking). Attributes are read for every path in ONE
   `git check-attr --stdin` call. A path with `text` UNSET and no `filter`/`eol` attribute
   cannot be altered on the way in — that is proven from the attributes, not sampled. Any
   other path gets the expensive proof: the object id git would create WITH filters
   (`git hash-object --path`) versus the id of the RAW bytes (`--no-filters`).

B. HEAD FIDELITY (post-commit). Blob ids come from ONE `git ls-tree -r HEAD`, contents from
   ONE `git cat-file --batch`, so the whole tree costs two git processes rather than one per
   file. Files modified since HEAD are counted and named, not failed.

usage: committed-bytes-check.py [<tree>] [<repo-root>] [<tree-prefix-in-repo>]
"""
import hashlib, os, subprocess, sys

TREE = sys.argv[1] if len(sys.argv) > 1 else os.environ.get(
    "BATCH_C_ARTIFACTS",
    "/Users/ckriech/projects/clemens33/ae-rust/docs/migration/evidence/batch-c-artifacts")
REPO = sys.argv[2] if len(sys.argv) > 2 else os.environ.get(
    "REPO_ROOT", "/Users/ckriech/projects/clemens33/ae-rust")
PREFIX = sys.argv[3] if len(sys.argv) > 3 else os.environ.get(
    "TREE_PREFIX", os.path.relpath(os.path.abspath(TREE), os.path.abspath(REPO)))
# The prefix DERIVES from the tree under audit. It used to default to batch-c-artifacts
# whatever tree was passed, so auditing a second tree printed — and compared against — the
# first tree's path: a check answering confidently about something other than its argument,
# the same class as the gate that was hardcoded to the live tree.

def git(*a, data=None, text=False):
    return subprocess.run(["git", "-C", REPO, *a], capture_output=True, input=data)

# FAIL LOUDLY, never attribute-free: `git hash-object` works without a repository and would
# then answer WITHOUT the repo's .gitattributes, turning every legitimately-CR file into a
# false positive. An attribute-free answer looks like evidence and is not one.
_wt = git("rev-parse", "--is-inside-work-tree")
if _wt.returncode != 0 or _wt.stdout.strip() != b"true":
    print(f"repo={REPO}")
    print("COMMITTED-BYTES-UNEVALUABLE: that path is not a git work tree, so the repository's")
    print("  .gitattributes cannot be consulted and any answer would be attribute-free.")
    print("  Pass the REAL repository as the repo root (the audited TREE may be a copy).")
    sys.exit(2)

# --- collect what SHA256SUMS claims, once -----------------------------------
claims = []            # (repo_path, local_path, recorded_sha256)
for root, _dirs, files in os.walk(TREE):
    if "SHA256SUMS.txt" not in files:
        continue
    rel_d = os.path.relpath(root, TREE)
    for line in open(os.path.join(root, "SHA256SUMS.txt"), encoding="utf-8", errors="replace"):
        line = line.rstrip("\n")
        if "  " not in line or line.startswith("#"):
            continue
        recorded, rel = line.split("  ", 1)
        rel = rel.lstrip("./")
        local = os.path.join(root, rel)
        if not os.path.exists(local):
            continue
        claims.append((os.path.normpath(os.path.join(PREFIX, "" if rel_d == "." else rel_d, rel)),
                       local, recorded))

# --- A. attributes for every path in ONE call --------------------------------
attr_proven, needs_oid = 0, []
if claims:
    stdin = b"\0".join(c[0].encode() for c in claims) + b"\0"
    r = git("check-attr", "-z", "--stdin", "text", "filter", "eol", data=stdin)
    fields = r.stdout.split(b"\0")
    attrs = {}
    for i in range(0, len(fields) - 2, 3):
        p, a, v = fields[i].decode(), fields[i+1].decode(), fields[i+2].decode()
        attrs.setdefault(p, {})[a] = v
    for repo_path, local, rec in claims:
        a = attrs.get(repo_path, {})
        if a.get("text") == "unset" and a.get("filter") in (None, "unspecified") \
                and a.get("eol") in (None, "unspecified"):
            attr_proven += 1
        else:
            needs_oid.append((repo_path, local, rec))

norm = []
for repo_path, local, rec in needs_oid:
    a = git("hash-object", "--path", repo_path, "--", local).stdout.decode().strip()
    b = git("hash-object", "--no-filters", "--", local).stdout.decode().strip()
    if a and b and a != b:
        norm.append((repo_path, a, b))

# --- B. HEAD contents in two calls -------------------------------------------
head_blobs = {}
r = git("ls-tree", "-r", "-z", "HEAD", "--", PREFIX)
for ent in r.stdout.split(b"\0"):
    if not ent:
        continue
    meta, _, path = ent.partition(b"\t")
    parts = meta.split()
    if len(parts) >= 3 and parts[1] == b"blob":
        head_blobs[path.decode()] = parts[2].decode()

wanted = [(c[0], c[2], c[1]) for c in claims if c[0] in head_blobs]
oid_sha = {}
if wanted:
    batch_in = "\n".join(head_blobs[w[0]] for w in wanted).encode() + b"\n"
    proc = subprocess.run(["git", "-C", REPO, "cat-file", "--batch"],
                          input=batch_in, capture_output=True)
    out, pos = proc.stdout, 0
    for w in wanted:
        nl = out.find(b"\n", pos)
        hdr = out[pos:nl].split()
        size = int(hdr[2]); pos = nl + 1
        oid_sha[w[0]] = hashlib.sha256(out[pos:pos+size]).hexdigest()
        pos += size + 1

# Which paths does the REPOSITORY itself consider changed? A file git reports as clean
# whose committed bytes nevertheless disagree with the recorded hash is the ce8965e
# signature exactly: the repo believes nothing is pending, and a clone would still hand out
# different bytes. A file git reports as modified is ordinary uncommitted work.
dirty = set()
r = git("status", "--porcelain", "-z", "--", PREFIX)
for ent in r.stdout.split(b"\0"):
    if len(ent) > 3:
        dirty.add(ent[3:].decode())

headfail, modified = [], []
for repo_path, rec, local in wanted:
    hh = oid_sha.get(repo_path)
    if hh is None or hh == rec:
        continue
    wh = hashlib.sha256(open(local, "rb").read()).hexdigest()
    if repo_path in dirty:
        modified.append((repo_path, rec, hh, wh))
    else:
        headfail.append((repo_path, rec, hh, wh))

print(f"tree={TREE}")
print(f"repo={REPO} prefix={PREFIX}")
print(f"files_checked={len(claims)} present_at_HEAD={len(wanted)} not_yet_at_HEAD={len(claims)-len(wanted)}")
print(f"A_attribute_proven={attr_proven} A_oid_compared={len(needs_oid)} A_would_normalize={len(norm)}"
      f"  B_head_mismatch={len(headfail)}  modified_since_HEAD={len(modified)}")
for p, a, b in norm[:40]:
    print(f"  NORMALIZE-FAIL {p}")
    print(f"    oid with filters ={a}")
    print(f"    oid raw bytes    ={b}   (they differ: the repo would not store what was hashed)")
for p, rec, hh, wh in headfail[:40]:
    print(f"  HEAD-BYTES-FAIL {p}   (git reports this path CLEAN — a clone would differ)")
    print(f"    recorded ={rec}")
    print(f"    HEAD blob={hh}   (what a fresh clone yields)")
    print(f"    working  ={wh}")
if modified:
    print(f"  ({len(modified)} file(s) modified since HEAD — recorded hash matches the WORKING")
    print("   bytes; ordinary uncommitted work, covered by part A, not a failure)")
sys.exit(1 if (norm or headfail) else 0)
