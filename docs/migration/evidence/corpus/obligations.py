#!/usr/bin/env python3
"""GENERATOR — one row per (invocation, OBLIGATION), replacing a column that could
only say THAT a row differs with one that says HOW.

WHY THE UNIT CHANGED. A single unreachable-server human row owes three simultaneous
obligations — status moves `stopped`->`unknown`, stderr gains a diagnostic carrying a
loss count, and `inventory_complete` goes `false` on the paired digest. A row owing
three obligations cannot be represented by one record however many columns it gains.

WHY `verdict` IS NO LONGER STORED. It is DERIVED: a row is EXPECTED-DIVERGENCE iff it
carries at least one obligation. The old column stated it as a fact beside its reasons,
which is exactly how it came to disagree with them when SC-017o landed. This removes the
possibility rather than repairing the symptom.

`from` IS READ FROM THE CAPTURED BYTES, never assumed — the gate re-reads and compares.
"""
import csv, json, os, re, subprocess, sys, collections

HERE = os.path.dirname(os.path.abspath(__file__))
SRC = os.path.normpath(os.path.join(HERE, "..", "batch-c-artifacts"))
INV = os.path.join(HERE, "INVOCATIONS.tsv")
OUT = os.path.join(HERE, "OBLIGATIONS.tsv")
FRESH = os.path.join(HERE, "FRESHNESS.tsv")
CONTRACT = "docs/migration/semantic-contract.md"
LISTING = ("ae list", "ae ls")

HDR = ["case", "consumer", "obligation_id", "stream", "locus", "from", "to",
       "predicate", "baseline_provenance", "authority"]

def contract_blob():
    """The blob hash of the contract this derivation was made against. A lineage
    stamp says where an artifact came from; only this says whether the source has
    moved since."""
    root = subprocess.run(["git", "rev-parse", "--show-toplevel"], cwd=HERE,
                          capture_output=True, text=True).stdout.strip()
    return subprocess.run(["git", "rev-parse", f"HEAD:{CONTRACT}"], cwd=root,
                          capture_output=True, text=True).stdout.strip()

def body(case, consumer):
    p = os.path.join(SRC, case, "out", consumer + ".stdout")
    return open(p, encoding="utf-8", errors="replace").read() if os.path.exists(p) else ""

def unreachable(case):
    p = os.path.join(SRC, case, "tmux.before.txt")
    return os.path.exists(p) and "error connecting" in open(p, encoding="utf-8", errors="replace").read()

def main():
    rows, seen = [], set()
    for r in csv.DictReader(open(INV, encoding="utf-8"), delimiter="\t"):
        if r["phase"] != "P1":
            continue
        case, consumer = os.path.dirname(r["case"]), r["consumer"]
        seen.add((case, consumer))
        text = body(case, consumer)
        digest = '"schema_version"' in text
        listish = r["surface"] in LISTING
        incomplete = unreachable(case)

        if digest:
            # SC-509d: version 2, unconditionally, on every successor digest.
            m = re.search(r'"schema_version"\s*:\s*(\d+)', text)
            rows.append((case, consumer, "SC-509d", "digest", "schema_version",
                         m.group(1) if m else "ABSENT", "2", "equals", "SOURCE",
                         "successor digest is schema version 2"))
            # SC-017o: inventory_complete on EVERY successor digest, present even
            # for an empty inventory. Absent from every version-1 capture.
            rows.append((case, consumer, "SC-017o", "digest", "inventory_complete",
                         "present" if '"inventory_complete"' in text else "ABSENT",
                         "false" if incomplete else "true", "equals",
                         "OBSERVED" if incomplete else "SOURCE",
                         "every successor digest carries the boolean"))

        if listish and incomplete:
            # TWO DISTINCT OBLIGATIONS, and which one applies is READ FROM THE BYTES
            # rather than assumed. The first version of this generator emitted a
            # `stopped`->`unknown` move for every unreachable listing; the gate
            # rejected 140 of them because their capture contains no `stopped` at
            # all. That is the label-versus-MEMBERSHIP distinction again: a default
            # view on an unreachable server shows NOTHING, so nothing is relabelled —
            # sessions that were absent become present as `unknown` (SC-017m).
            n = len(re.findall(r'"status"\s*:\s*"stopped"', text)) if digest \
                else len(re.findall(r"^\S+\s+stopped\b", text, re.M))
            stream = "digest" if digest else "stdout"
            if n:
                rows.append((case, consumer, "SC-017l", stream,
                             "sessions[].status" if digest else "status cell",
                             "stopped", "unknown", "all-of", "OBSERVED",
                             f"{n} captured occurrence(s) must all move"))
            else:
                rows.append((case, consumer, "SC-017m", stream, "(row set)",
                             "empty", "unknown rows present", "present", "OBSERVED",
                             "default view shows running then unknown; absent becomes present"))
            if not digest:
                # SC-017o human half: stderr diagnostic carrying the NUMBER of failed
                # logical sources. at-least, not equals — a gate pinning the count
                # would fail a correct implementation that lost two sources.
                rows.append((case, consumer, "SC-017o", "stderr", "(whole stream)",
                             "ABSENT", "1", "at-least", "OBSERVED",
                             "explicit diagnostic naming the loss count"))

    rows.sort(key=lambda x: (x[0], x[1], x[2], x[4]))
    with open(OUT, "w", encoding="utf-8") as fh:
        fh.write("\t".join(HDR) + "\n")
        for x in rows:
            fh.write("\t".join(str(v) for v in x) + "\n")

    with open(FRESH, "w", encoding="utf-8") as fh:
        fh.write("# Freshness relation — the SOURCE this derivation was made against.\n")
        fh.write("# A lineage stamp says where an artifact came from; only a hash\n")
        fh.write("# comparison says whether the source has MOVED since.\n")
        fh.write("field\tvalue\n")
        fh.write(f"contract_path\t{CONTRACT}\n")
        fh.write(f"contract_blob\t{contract_blob()}\n")
        fh.write(f"p1_rows\t{len(seen)}\n")
        fh.write(f"obligation_rows\t{len(rows)}\n")

    per = collections.Counter(x[2] for x in rows)
    carriers = len({(x[0], x[1]) for x in rows})
    print(f"P1 rows {len(seen)}   obligations {len(rows)}   rows carrying >=1: {carriers}")
    for k in sorted(per):
        print(f"  {k:<10} {per[k]:4d}")
    print(f"derived EXPECTED-DIVERGENCE {carriers}   EXPECTED-MATCH {len(seen) - carriers}")

if __name__ == "__main__":
    main()
