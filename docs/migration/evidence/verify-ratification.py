#!/usr/bin/env python3
"""GATE — is ratification-critical.md still about the contract it classifies?

NO WRITE PATH: no open-for-write, no temp, no rename. It reports; something else
repairs. `--file PATH` points it at a COPY, so a red-proof never has to mutate the
tracked evidence file in a shared checkout to test its own checker.

IT PARSES THE PINNED OBJECT, NOT THE WORKTREE. An earlier version read the worktree
contract for rows while comparing the recorded blob to HEAD's — so it classified one
set of bytes and pinned another, and a normative change that kept every row heading
passed as fresh (red-proved by gpt56sol:colead in an isolated worktree). The subject
of this file is now the blob it names, by construction; worktree disagreement is its
own named finding rather than a silent substitution.

Checks:
  COVERAGE        every contract SC row has exactly one class; every class a real row
  DUPLICATE       one row classified twice — SC and D alike
  COUNTS          the machine-checked record equals the entries, every key exactly once
  FRESHNESS       a contract blob is recorded, is well formed, and equals HEAD's
  WORKTREE-DRIFT  the worktree contract differs from the blob being classified
"""
import argparse
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
CRIT = os.path.join(HERE, "ratification-critical.md")
CONTRACT_REL = "docs/migration/semantic-contract.md"
WORKTREE_CONTRACT = os.path.normpath(os.path.join(HERE, "..", "semantic-contract.md"))
CLASSES = ("CRITICAL", "DEFERRABLE", "OBSERVED", "ALREADY-OBSERVED")
COUNT_KEYS = ("CRITICAL", "DEFERRABLE", "OBSERVED")
LETTER_KEYS = ("A", "B", "C", "D")
ROW = re.compile(r"^\s*(?:- )?\*\*(SC-[0-9]+[a-z]*)\b")


def git(args, cwd=HERE):
    """Return stdout, or None when git itself failed — never an empty string that
    a caller could mistake for a legitimate answer."""
    r = subprocess.run(["git"] + args, cwd=cwd, capture_output=True, text=True)
    return r.stdout.strip() if r.returncode == 0 else None


def main(path=None, quiet=False, worktree=None):
    out = []
    crit_path = path or CRIT
    wt_path = worktree or WORKTREE_CONTRACT
    crit = open(crit_path, encoding="utf-8").read()

    root = git(["rev-parse", "--show-toplevel"])
    head = git(["rev-parse", "HEAD:" + CONTRACT_REL], cwd=root or HERE) if root else None

    # ---- FRESHNESS, three outcomes: absent / malformed / stale ---------------
    line = re.search(r"^contract_blob:\s*(\S*)\s*$", crit, re.M)
    pinned = None
    if line is None:
        out.append(("FRESHNESS", "no contract_blob is recorded — the file asserts no relation"))
    elif not re.fullmatch(r"[0-9a-f]{40}", line.group(1)):
        out.append(("MALFORMED", "contract_blob is not a 40-hex blob id: %r" % line.group(1)[:24]))
    elif head is None:
        out.append(("FRESHNESS", "git could not resolve HEAD:%s" % CONTRACT_REL))
    elif line.group(1) != head:
        out.append(("STALE", "recorded contract blob %s but HEAD carries %s — the contract "
                             "moved and this classification has not been re-derived"
                    % (line.group(1)[:12], head[:12])))
    else:
        pinned = line.group(1)

    # ---- the subject is the PINNED blob -------------------------------------
    blob = pinned or head
    contract = git(["cat-file", "blob", blob], cwd=root or HERE) if blob else None
    if contract is None:
        out.append(("SUBJECT", "cannot read the contract blob being classified"))
        contract = ""
    if pinned:
        wt = git(["hash-object", wt_path])
        if wt is None:
            out.append(("WORKTREE-DRIFT", "cannot hash the worktree contract"))
        elif wt != pinned:
            out.append(("WORKTREE-DRIFT",
                        "the worktree contract is %s but this file classifies %s — the bytes "
                        "on disk are not the bytes being classified" % (wt[:12], pinned[:12])))

    rowset = {m.group(1) for m in (ROW.match(l) for l in contract.split("\n")) if m}

    classified, dupes = {}, []
    for m in re.finditer(r"^- (SC-[0-9]+[a-z]*) — ([A-Z-]+)", crit, re.M):
        (dupes if m.group(1) in classified else []).append(m.group(1))
        classified[m.group(1)] = m.group(2)
    drec, ddupes = {}, []
    for m in re.finditer(r"^- (D[0-9]+[a-z]*) — ([A-Z-]+)", crit, re.M):
        (ddupes if m.group(1) in drec else []).append(m.group(1))
        drec[m.group(1)] = m.group(2)

    # ---- COVERAGE ----------------------------------------------------------
    for rid in sorted(rowset - set(classified)):
        out.append(("COVERAGE", "%s is a contract row with no classification entry" % rid))
    for cid in sorted(set(classified) - rowset):
        out.append(("ORPHAN", "%s is classified but is not a contract row heading" % cid))
    for did in sorted(set(dupes) | set(ddupes)):
        out.append(("DUPLICATE", "%s has more than one classification entry" % did))
    for cid, cls in sorted(classified.items()) + sorted(drec.items()):
        if cls not in CLASSES:
            out.append(("CLASS", "%s carries an unknown class %r" % (cid, cls)))

    # ---- COUNTS: one record, every key EXACTLY ONCE -------------------------
    def record(name, keys, pattern):
        m = re.search(r"^%s:\s*(.+)$" % name, crit, re.M)
        if not m:
            out.append(("COUNTS", "no %s record" % name))
            return None
        seen = re.findall(pattern, m.group(1))
        got = {}
        for k, v in seen:
            if k in got:
                out.append(("COUNTS", "%s names %s more than once" % (name, k)))
            got[k] = int(v)
        for k in keys:
            if k not in got:
                out.append(("COUNTS", "%s does not name %s" % (name, k)))
        for k in got:
            if k not in keys:
                out.append(("COUNTS", "%s names unknown key %s" % (name, k)))
        return got

    actual = {}
    for cls in list(classified.values()) + list(drec.values()):
        actual[cls] = actual.get(cls, 0) + 1
    actual["OBSERVED"] = actual.get("OBSERVED", 0) + actual.get("ALREADY-OBSERVED", 0)
    stated = record("class_counts", COUNT_KEYS, r"\b(CRITICAL|DEFERRABLE|OBSERVED)=([0-9]+)\b")
    if stated:
        for k in COUNT_KEYS:
            if k in stated and stated[k] != actual.get(k, 0):
                out.append(("COUNTS", "class_counts says %s=%d, entries give %d"
                            % (k, stated[k], actual.get(k, 0))))

    letters = {}
    for m in re.finditer(r"^- (?:SC|D)[0-9a-zA-Z-]* — CRITICAL\(([A-D,]+)\)", crit, re.M):
        for L in m.group(1).split(","):
            letters[L.strip()] = letters.get(L.strip(), 0) + 1
    stated_l = record("letter_counts", LETTER_KEYS, r"\b([A-D])=([0-9]+)\b")
    if stated_l:
        for k in LETTER_KEYS:
            if k in stated_l and stated_l[k] != letters.get(k, 0):
                out.append(("COUNTS", "letter_counts says %s=%d, entries give %d"
                            % (k, stated_l[k], letters.get(k, 0))))

    if not quiet:
        print("contract rows %d (from blob %s); classified %d SC + %d D"
              % (len(rowset), (blob or "?")[:12], len(classified), len(drec)))
        for cid, msg in out[:25]:
            print("FAIL  %-15s %s" % (cid, msg))
        if len(out) > 25:
            print("      ...and %d more" % (len(out) - 25))
        print("RATIFICATION CLASSIFICATION VERIFIED — every row classified, both count "
              "records agree, fresh against HEAD, worktree matches"
              if not out else "NOT VERIFIED — %d finding(s)" % len(out))
    return (1 if out else 0), {c for c, _ in out}


if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    ap.add_argument("--file", default=None, help="verify a COPY instead of the tracked file")
    ap.add_argument("--worktree-contract", default=None,
                    help="hash this instead of the real worktree contract (red-proof only)")
    a = ap.parse_args()
    sys.exit(main(a.file, worktree=a.worktree_contract)[0])
