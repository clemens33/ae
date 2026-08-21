#!/usr/bin/env python3
"""GATE — checks the obligation table against the captured bytes, the verdict column,
and the contract's CURRENT hash. No write path: no open-for-write, no temp, no rename.

Four classes, and the freshness one is the reason this file exists. A derived artifact
goes stale the moment its source grows and nothing re-runs to say so; the previous
column was found stale by a human noticing. This makes staleness a gate result.
"""
import csv, os, re, subprocess, sys, collections

HERE = os.path.dirname(os.path.abspath(__file__))
SRC = os.path.normpath(os.path.join(HERE, "..", "batch-c-artifacts"))
OBL = os.path.join(HERE, "OBLIGATIONS.tsv")
FRESH = os.path.join(HERE, "FRESHNESS.tsv")
VERD = os.path.join(HERE, "VERDICTS.tsv")
INV = os.path.join(HERE, "INVOCATIONS.tsv")
STREAMS = {"digest", "stdout", "stderr"}
PREDICATES = {"equals", "at-least", "all-of", "present"}
LISTING = ("ae list", "ae ls")

def fail(out, cid, msg): out.append((cid, msg))

def head_blob():
    root = subprocess.run(["git", "rev-parse", "--show-toplevel"], cwd=HERE,
                          capture_output=True, text=True).stdout.strip()
    return subprocess.run(["git", "rev-parse", "HEAD:docs/migration/semantic-contract.md"],
                          cwd=root, capture_output=True, text=True).stdout.strip()

def body(case, consumer):
    p = os.path.join(SRC, case, "out", consumer + ".stdout")
    return open(p, encoding="utf-8", errors="replace").read() if os.path.exists(p) else ""

def unreachable(case):
    p = os.path.join(SRC, case, "tmux.before.txt")
    return os.path.exists(p) and "error connecting" in open(p, encoding="utf-8", errors="replace").read()

def main(quiet=False):
    out = []
    for p in (OBL, FRESH, VERD, INV):
        if not os.path.exists(p):
            print("FAIL  MISSING  %s" % os.path.basename(p)); return 1
    rec = {}
    for line in open(FRESH, encoding="utf-8"):
        if line.startswith("#") or not line.strip(): continue
        k, _, v = line.rstrip("\n").partition("\t"); rec[k] = v

    # ---- 1. FRESHNESS: has the source moved since this was derived? ----
    # THE RELATION IS HEAD-RELATIVE, DELIBERATELY. It answers "is the COMMITTED table
    # fresh against the COMMITTED contract", which is the question a reviewer or CI
    # asks, and it means one agent's in-flight edit cannot fail everyone's gate. The
    # cost is that someone editing the contract LOCALLY and running this gets a pass
    # that says nothing about their own edit — so the success line names HEAD too,
    # and not only the failure line.
    now = head_blob()
    if rec.get("contract_blob") != now:
        fail(out, "STALE", "derived against contract blob %s; HEAD is %s — re-derive"
             % (rec.get("contract_blob", "?")[:12], now[:12]))

    obls = list(csv.DictReader(open(OBL, encoding="utf-8"), delimiter="\t"))
    carriers = collections.defaultdict(list)
    for o in obls: carriers[(o["case"], o["consumer"])].append(o)

    # ---- 2. TYPES: closed sets ----
    for o in obls:
        if o["stream"] not in STREAMS: fail(out, "STREAM", "unknown stream %r" % o["stream"])
        if o["predicate"] not in PREDICATES: fail(out, "PREDICATE", "unknown predicate %r" % o["predicate"])

    # ---- 3. FROM matches the captured bytes (re-read, never trusted) ----
    for o in obls:
        text = body(o["case"], o["consumer"])
        if o["locus"] == "schema_version":
            m = re.search(r'"schema_version"\s*:\s*(\d+)', text)
            got = m.group(1) if m else "ABSENT"
            if got != o["from"]:
                fail(out, "FROM", "%s/%s schema_version captured %s, table says %s"
                     % (o["case"], o["consumer"], got, o["from"]))
        elif o["locus"] == "inventory_complete":
            got = "present" if '"inventory_complete"' in text else "ABSENT"
            if got != o["from"]:
                fail(out, "FROM", "%s/%s inventory_complete captured %s, table says %s"
                     % (o["case"], o["consumer"], got, o["from"]))
        elif o["from"] == "stopped":
            n = len(re.findall(r'"status"\s*:\s*"stopped"', text)) if o["stream"] == "digest" \
                else len(re.findall(r"^\S+\s+stopped\b", text, re.M))
            if n == 0:
                fail(out, "FROM", "%s/%s claims a stopped->unknown move with no captured `stopped`"
                     % (o["case"], o["consumer"]))

    # ---- 4. CONVERSE: what carries an obligation must, and what does not must not ----
    p1 = [r for r in csv.DictReader(open(INV, encoding="utf-8"), delimiter="\t") if r["phase"] == "P1"]
    for r in p1:
        case, consumer = os.path.dirname(r["case"]), r["consumer"]
        text = body(case, consumer)
        ids = {o["obligation_id"] for o in carriers.get((case, consumer), [])}
        if '"schema_version"' in text and "SC-509d" not in ids:
            fail(out, "MISSING-509d", "%s/%s carries a digest and owes no SC-509d" % (case, consumer))
        if '"schema_version"' in text and "SC-017o" not in ids:
            fail(out, "MISSING-017o", "%s/%s carries a digest and owes no inventory_complete" % (case, consumer))
        listish = r["surface"] in LISTING
        if listish and unreachable(case):
            # WHICH obligation applies is re-derived here from the bytes, not merely
            # accepted from the table: a capture containing `stopped` owes a LABEL
            # move (SC-017l), one showing no sessions owes a MEMBERSHIP change
            # (SC-017m). Accepting "either" would let the generator pick wrongly.
            n = len(re.findall(r'"status"\s*:\s*"stopped"', text)) if '"schema_version"' in text \
                else len(re.findall(r"^\S+\s+stopped\b", text, re.M))
            want = "SC-017l" if n else "SC-017m"
            other = "SC-017m" if n else "SC-017l"
            if want not in ids:
                fail(out, "MISSING-" + want[3:], "%s/%s is an unreachable listing with %d captured `stopped` and owes no %s"
                     % (case, consumer, n, want))
            if other in ids:
                fail(out, "WRONG-KIND", "%s/%s owes %s but its capture has %d `stopped`"
                     % (case, consumer, other, n))
        if not listish and ("SC-017l" in ids or "SC-017m" in ids):
            fail(out, "SURFACE", "%s/%s is not a listing yet owes a listing obligation" % (case, consumer))

    # ---- 5. VERDICT IS DERIVED, and must agree with the stored column ----
    # VERDICTS.tsv keys on the consumers.tsv PATH; OBLIGATIONS.tsv keys on the case
    # DIRECTORY. Normalise, rather than letting a key mismatch read as 573 disagreements.
    stored = {(os.path.dirname(r["case"]), r["consumer"]): r["verdict"]
              for r in csv.DictReader(open(VERD, encoding="utf-8"), delimiter="\t")}
    disagree = 0
    for key, v in stored.items():
        derived = "EXPECTED-DIVERGENCE" if carriers.get(key) else "EXPECTED-MATCH"
        if derived != v:
            disagree += 1
            if disagree <= 5:
                fail(out, "VERDICT", "%s/%s stored %s, obligations derive %s" % (key[0], key[1], v, derived))
    if disagree > 5: fail(out, "VERDICT", "...and %d more" % (disagree - 5))

    if not quiet:
        per = collections.Counter(o["obligation_id"] for o in obls)
        print("obligations %d over %d carrying rows; contract blob %s"
              % (len(obls), len(carriers), rec.get("contract_blob", "?")[:12]))
        for k in sorted(per): print("  %-10s %4d" % (k, per[k]))
        for cid, msg in out[:20]: print("FAIL  %-14s %s" % (cid, msg))
        if not out:
            print("OBLIGATIONS VERIFIED — fresh against COMMITTED contract %s at HEAD"
                  % now[:12])
            print("  (HEAD-relative: an uncommitted local edit to the contract is NOT assessed)")
        else:
            print("NOT VERIFIED — %d finding(s)" % len(out))
    return (1 if out else 0), {c for c, _ in out}

if __name__ == "__main__":
    sys.exit(main()[0])
