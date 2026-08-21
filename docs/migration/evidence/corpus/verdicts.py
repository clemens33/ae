#!/usr/bin/env python3
"""GENERATOR — the PRE-REGISTERED parity verdict for every P1 corpus row.

WHY NOW, WITH NO SUCCESSOR CODE IN EXISTENCE. A verdict computed after the
implementation exists is not pre-registered: it had the opportunity to be shaped by
what the code turned out to do. This is the only moment at which the verdict is
honest, because nothing can currently produce a single one of these outputs.

DERIVED FROM CAPTURED BYTES, NOT FROM SURFACE NAMES. Whether a row is status-bearing
or digest-bearing is read out of the stdout the frozen product actually produced. An
earlier count of the same partition was made from the invocation's surface name; this
one is made from the artifact, and any disagreement between them is a finding rather
than a rounding difference.

NO ASSERTIONS ARE WRITTEN. There is nothing to assert against yet, and that is the
point of doing it now.
"""
import csv, glob, os, re, sys, collections

HERE = os.path.dirname(os.path.abspath(__file__))
SRC = os.path.normpath(os.path.join(HERE, "..", "batch-c-artifacts"))
INV = os.path.join(HERE, "INVOCATIONS.tsv")
OUT = os.path.join(HERE, "VERDICTS.tsv")

STATUS_HUMAN = re.compile(r"^SESSION\s+STATUS\b", re.M)
STATUS_JSON = re.compile(r'"status"\s*:')
DIGEST = re.compile(r'"schema_version"\s*:')

def case_unreachable(case_dir):
    """A CAPTURED fact, not an inference: the case's own tmux snapshot records that
    the server could not be reached."""
    tb = os.path.join(SRC, case_dir, "tmux.before.txt")
    if not os.path.exists(tb): return False
    head = open(tb, encoding="utf-8", errors="replace").read()
    return "error connecting" in head

def stdout_for(case_dir, consumer):
    p = os.path.join(SRC, case_dir, "out", consumer + ".stdout")
    if os.path.exists(p):
        return open(p, encoding="utf-8", errors="replace").read()
    return None

def main():
    rows, missing = [], 0
    unreachable = {}
    for r in csv.DictReader(open(INV, encoding="utf-8"), delimiter="\t"):
        if r["phase"] != "P1": continue
        case = os.path.dirname(r["case"])
        if case not in unreachable: unreachable[case] = case_unreachable(case)
        body = stdout_for(case, r["consumer"])
        listing = r["surface"] in ("ae list", "ae ls")
        if body is None:
            missing += 1
            shape, status_bearing, digest_bearing = "no-stdout-captured", False, False
        else:
            digest_bearing = bool(DIGEST.search(body))
            status_bearing = bool(STATUS_HUMAN.search(body) or STATUS_JSON.search(body))
            shape = ("digest" if digest_bearing else "human-status" if status_bearing else "neither")
        listing = r["surface"] in ("ae list", "ae ls")
        mandates = []
        if digest_bearing:
            mandates.append("SC-509d")                       # schema_version 1 -> 2, unconditional
        # SC-017l/m are NOT gated on the captured output carrying a status field.
        # CORRECTED after reading one row of this class, per the coverage rule:
        # `ae list` on an unreachable server printed "No running ae sessions." and a
        # digest with `"sessions":[]`. Both carry NO status, and the first version of
        # this generator therefore scored them EXPECTED-MATCH. That is wrong —
        # SC-017m changes the MEMBERSHIP of the view, not only the labels in it: the
        # default view shows `running` then `unknown`, so sessions that become
        # `unknown` APPEAR where the frozen product showed nothing. An empty listing
        # is exactly the output that diverges most visibly.
        # So: any session-listing row from an unreachable-server case diverges,
        # whatever its captured bytes happen to contain.
        if listing and unreachable[case]:
            mandates.append("SC-017l/m")
        verdict = "EXPECTED-DIVERGENCE" if mandates else "EXPECTED-MATCH"
        # PROVENANCE of the divergence baseline, which is what an assertion could check:
        #   OBSERVED  — an end-to-end capture exhibits the frozen behaviour being changed
        #   SOURCE    — the frozen behaviour is source-proven only
        if "SC-017l/m" in mandates:
            prov = "OBSERVED"        # colead relabelled SC-017l after independent reproduction
        elif mandates:
            prov = "SOURCE"          # SC-509d: frozen baseline source-proven, successor pending
        else:
            prov = "-"
        rows.append((r["case"], r["consumer"], r["surface"], shape,
                     "yes" if status_bearing else "no", "yes" if digest_bearing else "no",
                     "yes" if unreachable[case] else "no", verdict,
                     "+".join(mandates) or "-", prov))
    with open(OUT, "w", encoding="utf-8") as fh:
        fh.write("case\tconsumer\tsurface\toutput_shape\tstatus_bearing\tdigest_bearing\t"
                 "server_unreachable\tverdict\tmandated_by\tbaseline_provenance\n")
        for x in rows: fh.write("\t".join(x) + "\n")
    c = collections.Counter((x[7], x[8]) for x in rows)
    print("P1 rows: %d   (stdout not captured for %d)" % (len(rows), missing))
    for k in sorted(c): print("  %-20s %-18s %4d" % (k[0], k[1], c[k]))
    print("  %-39s %4d" % ("EXPECTED-DIVERGENCE total",
                           sum(v for k, v in c.items() if k[0] == "EXPECTED-DIVERGENCE")))
    print("  %-39s %4d" % ("EXPECTED-MATCH total",
                           sum(v for k, v in c.items() if k[0] == "EXPECTED-MATCH")))
    print("wrote %s" % OUT)

if __name__ == "__main__":
    main()
