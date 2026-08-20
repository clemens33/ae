#!/usr/bin/env python3
"""COMMITTED-BYTES check — the fourth gate guarantee: CLONE FIDELITY.

The other three checks all read the WORKING TREE, so none of them can see a tree that
passes locally and fails on clone. At commit ce8965e a text filter (autocrlf=input)
rewrote four pty logs on the way into the object database: the working file hashed
8320a0a5, the stored blob hashed 20fffc72, and all three working-tree checks passed while
a fresh clone would have failed hash verification on four evidence artifacts.

Two parts, both non-invasive — nothing is staged, committed, or written to the object
database:

A. WOULD-NORMALIZE (pre-commit, the blocking one). For each listed file, the object id git
   WOULD create from the working bytes WITH attributes and clean filters applied
   (`git hash-object --path <repo path>`) is compared against the id of the RAW bytes
   (`git hash-object --no-filters`). Any difference means the bytes that reach the
   repository are not the bytes that were hashed into SHA256SUMS, whatever the cause —
   autocrlf, a smudge/clean filter, a new path outside the -text globs.

B. HEAD FIDELITY (post-commit). For each listed file already present at HEAD, the sha256 of
   `git show HEAD:<path>` — what a fresh clone yields — is compared against the recorded
   hash. Files modified since HEAD are counted and named as MODIFIED, not failed: their
   divergence is ordinary uncommitted work, and part A is what covers them.

usage: committed-bytes-check.py [<tree>] [<repo-root>] [<tree-prefix-in-repo>]
"""
import hashlib, os, subprocess, sys

TREE = sys.argv[1] if len(sys.argv) > 1 else os.environ.get(
    "BATCH_C_ARTIFACTS",
    "/Users/ckriech/projects/clemens33/ae-rust/docs/migration/evidence/batch-c-artifacts")
REPO = sys.argv[2] if len(sys.argv) > 2 else os.environ.get(
    "REPO_ROOT", "/Users/ckriech/projects/clemens33/ae-rust")
PREFIX = sys.argv[3] if len(sys.argv) > 3 else os.environ.get(
    "TREE_PREFIX", "docs/migration/evidence/batch-c-artifacts")

def git(*a, data=None):
    return subprocess.run(["git", "-C", REPO, *a], capture_output=True, input=data)

# FAIL LOUDLY, never attribute-free. `git hash-object` does not need a repository, so with a
# REPO that is not a work tree it still returns oids — computed WITHOUT the repo's
# .gitattributes, which turns every legitimately-CR file into a false NORMALIZE-FAIL. An
# attribute-free answer looks like evidence and is not one.
_wt = subprocess.run(["git", "-C", REPO, "rev-parse", "--is-inside-work-tree"],
                     capture_output=True)
if _wt.returncode != 0 or _wt.stdout.strip() != b"true":
    print(f"repo={REPO}")
    print("COMMITTED-BYTES-UNEVALUABLE: that path is not a git work tree, so the repository's")
    print("  .gitattributes cannot be consulted and any answer would be attribute-free.")
    print("  Pass the REAL repository as the repo root (the audited TREE may be a copy).")
    sys.exit(2)

def oid(path_in_repo, local, filters):
    args = ["hash-object", "--path", path_in_repo] if filters else ["hash-object", "--no-filters"]
    r = git(*args, "--", local)
    return r.stdout.decode().strip() if r.returncode == 0 else None

norm, headfail, modified, at_head, untracked, checked = [], [], [], 0, 0, 0
for root, _dirs, files in os.walk(TREE):
    if "SHA256SUMS.txt" not in files:
        continue
    rel_d = os.path.relpath(root, TREE)
    for line in open(os.path.join(root, "SHA256SUMS.txt"), encoding="utf-8", errors="replace"):
        line = line.rstrip("\n")
        if "  " not in line:
            continue
        recorded, rel = line.split("  ", 1)
        rel = rel.lstrip("./")
        local = os.path.join(root, rel)
        in_repo = os.path.normpath(os.path.join(PREFIX, "" if rel_d == "." else rel_d, rel))
        if not os.path.exists(local):
            continue
        checked += 1
        # A. would the bytes change on the way in?
        a, b = oid(in_repo, local, True), oid(in_repo, local, False)
        if a and b and a != b:
            norm.append((in_repo, a, b))
        # B. do the bytes already at HEAD match what was recorded?
        r = git("show", f"HEAD:{in_repo}")
        if r.returncode != 0:
            untracked += 1
            continue
        at_head += 1
        hh = hashlib.sha256(r.stdout).hexdigest()
        if hh != recorded:
            wh = hashlib.sha256(open(local, "rb").read()).hexdigest()
            (modified if wh == recorded else headfail).append((in_repo, recorded, hh, wh))

print(f"tree={TREE}")
print(f"repo={REPO} prefix={PREFIX}")
print(f"files_checked={checked} present_at_HEAD={at_head} not_yet_at_HEAD={untracked}")
print(f"A_would_normalize={len(norm)}  B_head_mismatch={len(headfail)}  modified_since_HEAD={len(modified)}")
for p, a, b in norm[:40]:
    print(f"  NORMALIZE-FAIL {p}")
    print(f"    oid with filters ={a}")
    print(f"    oid raw bytes    ={b}   (they differ: the repo would not store what was hashed)")
for p, rec, hh, wh in headfail[:40]:
    print(f"  HEAD-BYTES-FAIL {p}")
    print(f"    recorded ={rec}")
    print(f"    HEAD blob={hh}   (what a fresh clone yields)")
    print(f"    working  ={wh}")
if modified:
    print(f"  ({len(modified)} file(s) modified since HEAD — recorded hash matches the WORKING bytes;")
    print("   ordinary uncommitted work, covered by part A, not a failure)")
sys.exit(1 if (norm or headfail) else 0)
