#!/usr/bin/env python3
"""GATE — does each registry still describe the thing it says it describes?

NO WRITE PATH. `--dir` points it at a COPY so a red-proof never seeds the tracked
files.

FRESHNESS FOLLOWS DIRECT PROVENANCE, NOT ONE BLOB EVERYWHERE. A classification can
move under a fixed contract, so crit-assign's source is ratification-critical.md and
NOT the contract; pinning everything to the contract would call crit-assign fresh
while the classification it assigns had changed underneath it.

THERE IS NO CENTRAL MANIFEST OF PINS, deliberately. A file listing which registry
points where would become one more authority that only checks itself, and the first
registry nobody adds to it is invisible. Instead each registry DECLARES ITS OWN
relation in its own bytes:

    SOURCE: <repo-relative path> blob <40 hex>

and this gate DISCOVERS those declarations. A registry that stops declaring one
stops being checked — which is why the discovered set is printed on every run, so a
disappearance is visible rather than silent.

Not covered here, and named so it does not read as unchecked: ratification-critical.md
declares `contract_blob:` and is checked by verify-ratification.py, which also owns
its coverage and count relations.
"""
import argparse
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
DECL = re.compile(r"^SOURCE:\s+(\S+)\s+blob\s+(\S+)\s*$", re.M)
ELSEWHERE = {"ratification-critical.md": "verify-ratification.py (contract_blob:)"}


def git(args, cwd):
    r = subprocess.run(["git"] + args, cwd=cwd, capture_output=True, text=True)
    return r.stdout.strip() if r.returncode == 0 else None


def main(directory=None, quiet=False):
    out, seen = [], []
    d = directory or HERE
    root = git(["rev-parse", "--show-toplevel"], HERE)
    if not root:
        return 1, {"REPO"}
    for name in sorted(os.listdir(d)):
        if not name.endswith(".md"):
            continue
        text = open(os.path.join(d, name), encoding="utf-8", errors="replace").read()
        decls = DECL.findall(text)
        if not decls:
            continue
        if len(decls) > 1:
            out.append(("MULTIPLE", "%s declares %d SOURCE relations; exactly one is "
                        "required" % (name, len(decls))))
            continue
        path, pinned = decls[0]
        seen.append((name, path, pinned))
        if not re.fullmatch(r"[0-9a-f]{40}", pinned):
            out.append(("MALFORMED", "%s pins %r, which is not a 40-hex blob id"
                        % (name, pinned[:24])))
            continue
        head = git(["rev-parse", "HEAD:" + path], root)
        if head is None:
            out.append(("SOURCE", "%s names %s, which git cannot resolve at HEAD"
                        % (name, path)))
        elif head != pinned:
            out.append(("STALE", "%s pins %s at %s but HEAD carries %s — its source moved "
                        "and it has not been re-derived"
                        % (name, path, pinned[:12], head[:12])))
    if not quiet:
        for name, path, pinned in seen:
            print("  %-24s -> %-46s %s" % (name, path, pinned[:12]))
        for name, who in sorted(ELSEWHERE.items()):
            print("  %-24s -> checked by %s" % (name, who))
        for cid, msg in out:
            print("FAIL  %-10s %s" % (cid, msg))
        print("REGISTRY FRESHNESS VERIFIED — %d declared relation(s), each against its own "
              "direct source" % len(seen) if not out
              else "NOT VERIFIED — %d finding(s)" % len(out))
    return (1 if out else 0), {c for c, _ in out}


if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    ap.add_argument("--dir", default=None, help="scan a COPY instead of the tracked dir")
    a = ap.parse_args()
    sys.exit(main(a.dir)[0])
