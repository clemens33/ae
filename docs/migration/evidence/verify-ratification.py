#!/usr/bin/env python3
"""GATE — is ratification-critical.md still about the contract it classifies?

NO WRITE PATH: no open-for-write, no temp, no rename. It reports; something else
repairs.

It exists because the classification document had NO RELATION to the contract.
`sweep-check.sh` checks a great deal — field presence, closure-map membership,
CRIT-ASSIGN coupling, an expected-id-set diff — but it never asks whether every
contract row HAS a class. Fifteen rows had landed without one, and they were not
random: the entire P1 list/ls liveness and selector family, the rows P1 is gated
on. Nothing would have said so.

Three checks, and the freshness one is why this file exists:
  COVERAGE   every contract SC row heading has exactly one classification entry,
             and every classified id is a real contract row.
  COUNTS     the header's stated class counts equal the entries actually present.
  FRESHNESS  the contract blob recorded here equals the contract blob at HEAD.
             HEAD-relative by design: a date says when someone last ran the
             generator, a blob says whether the thing it describes has moved.
"""
import os
import re
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
CRIT = os.path.join(HERE, "ratification-critical.md")
CONTRACT_REL = "docs/migration/semantic-contract.md"
CONTRACT = os.path.normpath(os.path.join(HERE, "..", "semantic-contract.md"))
CLASSES = ("CRITICAL", "DEFERRABLE", "OBSERVED", "ALREADY-OBSERVED")


def head_blob():
    root = subprocess.run(
        ["git", "rev-parse", "--show-toplevel"], cwd=HERE, capture_output=True, text=True
    ).stdout.strip()
    return subprocess.run(
        ["git", "rev-parse", "HEAD:" + CONTRACT_REL], cwd=root, capture_output=True, text=True
    ).stdout.strip()


def main(quiet=False):
    out = []
    crit = open(CRIT, encoding="utf-8").read()
    contract = open(CONTRACT, encoding="utf-8").read()

    rows = []
    for line in contract.split("\n"):
        m = re.match(r"^\s*(?:- )?\*\*(SC-[0-9]+[a-z]*)\b", line)
        if m:
            rows.append(m.group(1))
    rowset = set(rows)

    classified = {}
    dupes = []
    for m in re.finditer(r"^- (SC-[0-9]+[a-z]*) — ([A-Z-]+)", crit, re.M):
        if m.group(1) in classified:
            dupes.append(m.group(1))
        classified[m.group(1)] = m.group(2)

    # ---- COVERAGE ----------------------------------------------------------
    for rid in sorted(rowset - set(classified)):
        out.append(("COVERAGE", "%s is a contract row with no classification entry" % rid))
    for cid in sorted(set(classified) - rowset):
        out.append(("ORPHAN", "%s is classified but is not a contract row heading" % cid))
    for did in sorted(set(dupes)):
        out.append(("DUPLICATE", "%s has more than one classification entry" % did))
    for cid, cls in sorted(classified.items()):
        if cls not in CLASSES:
            out.append(("CLASS", "%s carries an unknown class %r" % (cid, cls)))

    # ---- COUNTS ------------------------------------------------------------
    # The header states combined SC+D counts, so D records are counted too.
    drec = dict(re.findall(r"^- (D[0-9]+[a-z]*) — ([A-Z-]+)", crit, re.M))
    actual = {}
    for cls in list(classified.values()) + list(drec.values()):
        actual[cls] = actual.get(cls, 0) + 1
    stated = dict(
        (k, int(v))
        for k, v in re.findall(r"\b(CRITICAL|DEFERRABLE|OBSERVED)=([0-9]+)", crit)[:3]
    )
    obs = actual.get("OBSERVED", 0) + actual.get("ALREADY-OBSERVED", 0)
    for name, got in (
        ("CRITICAL", actual.get("CRITICAL", 0)),
        ("DEFERRABLE", actual.get("DEFERRABLE", 0)),
        ("OBSERVED", obs),
    ):
        if name in stated and stated[name] != got:
            out.append(
                ("COUNT", "header states %s=%d, entries give %d" % (name, stated[name], got))
            )

    # ---- FRESHNESS ---------------------------------------------------------
    # Three outcomes, not two. A blob that is PRESENT BUT MALFORMED is a different
    # defect from one that is absent, and a single regex that requires 40 hex digits
    # collapses them: the red-proof caught exactly that — a corrupted blob reported
    # "no relation asserted" instead of "the contract moved".
    line = re.search(r"^contract_blob:\s*(\S*)\s*$", crit, re.M)
    rec = line if (line and re.fullmatch(r"[0-9a-f]{40}", line.group(1))) else None
    now = head_blob()
    if line is None:
        out.append(("FRESHNESS", "no contract_blob is recorded — the file asserts no relation"))
    elif rec is None:
        out.append(("MALFORMED", "contract_blob is recorded but is not a 40-hex blob id: %r"
                    % line.group(1)[:24]))
    elif not now:
        out.append(("FRESHNESS", "cannot resolve HEAD:%s" % CONTRACT_REL))
    elif rec.group(1) != now:
        out.append(
            (
                "STALE",
                "recorded contract blob %s but HEAD carries %s — the contract moved and "
                "this classification has not been re-derived" % (rec.group(1)[:12], now[:12]),
            )
        )

    if not quiet:
        print(
            "contract rows %d; classified %d (SC) + %d (D); contract blob %s"
            % (len(rowset), len(classified), len(drec), (now or "?")[:12])
        )
        for cid, msg in out[:25]:
            print("FAIL  %-10s %s" % (cid, msg))
        if len(out) > 25:
            print("      ...and %d more" % (len(out) - 25))
        if not out:
            print("RATIFICATION CLASSIFICATION VERIFIED — every contract row has a class, "
                  "counts agree, fresh against the COMMITTED contract at HEAD")
        else:
            print("NOT VERIFIED — %d finding(s)" % len(out))
    return (1 if out else 0), {c for c, _ in out}


if __name__ == "__main__":
    sys.exit(main()[0])
