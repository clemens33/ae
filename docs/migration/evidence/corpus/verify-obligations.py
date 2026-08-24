#!/usr/bin/env python3
"""GATE — checks the obligation table against the captured bytes, the P1 population,
and the contract's CURRENT hash. No write path: no open-for-write, no temp, no rename.

Four classes, and the freshness one is the reason this file exists. A derived artifact
goes stale the moment its source grows and nothing re-runs to say so; the previous
column was found stale by a human noticing. This makes staleness a gate result.
"""
import csv, json, os, re, subprocess, sys, collections

HERE = os.path.dirname(os.path.abspath(__file__))
SRC = os.path.normpath(os.path.join(HERE, "..", "batch-c-artifacts"))
OBL = os.path.join(HERE, "OBLIGATIONS.tsv")
FRESH = os.path.join(HERE, "FRESHNESS.tsv")
INV = os.path.join(HERE, "INVOCATIONS.tsv")
STREAMS = {"digest", "stdout", "stderr"}
PREDICATES = {"equals", "at-least", "all-of", "present"}
SUPPORT = {"OBSERVED", "UNSCORABLE"}
LISTING = ("ae list", "ae ls")
AGENT_OWNED = ("dead", "stale", "waiting-user", "blocked", "throttled")


def digest_text(text):
    """The captured document, or None when this row is not a digest at all."""
    if '"schema_version"' not in text:
        return None
    try:
        return json.loads(text)
    except ValueError:
        return None

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
    for p in (OBL, FRESH, INV):
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
        if o.get("support") not in SUPPORT:
            fail(out, "SUPPORT", "unknown support value %r — every obligation must say whether "
                 "THIS CORPUS can score it" % o.get("support"))

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
        # New rows, same converse discipline: what must owe, owes.
        if unreachable(case) and '"schema_version"' in text and '"alive"' in text \
           and "SC-509e" not in ids:
            fail(out, "MISSING-509e", "%s/%s is an unreachable digest carrying agents[] and owes no alive->null"
                 % (case, consumer))
        if unreachable(case) and '"alive"' not in text and "SC-509e" in ids:
            fail(out, "SURFACE", "%s/%s owes an agents[].alive move with no captured alive field"
                 % (case, consumer))
        # ---- SC-509b / SC-509c converse, both directions -------------------
        # Their evidence is in the captured JSON itself, so the gate re-derives the
        # trigger from those bytes rather than trusting the generator's word.
        if digest_text(text) is not None:
            doc = digest_text(text)
            loss = set()
            mp = os.path.join(SRC, case, "manifest.before.tsv")
            if os.path.exists(mp):
                rowsm = [l.rstrip("\n").split("\t") for l in open(mp, encoding="utf-8")]
                sess = {r[-1].split("/")[2] for r in rowsm
                        if r and r[0] == "dir" and re.match(r"\./sessions/[^./][^/]*$", r[-1])}
                metas = {r[-1].split("/")[2]: r[2] for r in rowsm
                         if r and r[0] == "file"
                         and re.match(r"\./sessions/[^./][^/]*/meta$", r[-1])}
                # The manifest's OWN marker, not a mode enumeration — see the
                # generator's loss_sessions for why, and for the measurement that
                # the two agree on this corpus.
                loss = {n for n in sess if n not in metas or metas[n] == "UNREADABLE"}
            want_b = any(x.get("name") in loss for x in doc.get("sessions", []) or [])
            want_c = any(a.get("reason") is None and a.get("state") in AGENT_OWNED
                         for x in doc.get("sessions", []) or []
                         for a in (x.get("agents") or []))
            if want_b and "SC-509b" not in ids:
                fail(out, "MISSING-509b", "%s/%s has a session with unreadable meta and owes "
                     "no degraded move" % (case, consumer))
            if not want_b and "SC-509b" in ids:
                fail(out, "SURFACE", "%s/%s owes a degraded move with no read-loss session"
                     % (case, consumer))
            if want_c and "SC-509c" not in ids:
                fail(out, "MISSING-509c", "%s/%s has a null-reason agent whose own state names "
                     "an agent-owned contribution and owes no reason move" % (case, consumer))
            if not want_c and "SC-509c" in ids:
                fail(out, "SURFACE", "%s/%s owes a reason move with no such agent"
                     % (case, consumer))
        elif "SC-509b" in ids or "SC-509c" in ids:
            fail(out, "SURFACE", "%s/%s is not a digest yet owes a JSON-only obligation"
                 % (case, consumer))

        if not listish and ("SC-017l" in ids or "SC-017m" in ids):
            fail(out, "SURFACE", "%s/%s is not a listing yet owes a listing obligation" % (case, consumer))

    # ---- 5. VERDICT IS DERIVED — so the check is COVERAGE, not agreement ----
    # The stored VERDICTS.tsv column is RETIRED (superseded by this table). A stored
    # verdict beside a derived one is EXACTLY the shape that went stale, and we did not
    # repair that staleness, we removed the possibility of it. Keeping a copy alive would
    # reintroduce the possibility together with a checker to manage it — and a check that
    # exists only to police a redundancy is a reason to delete the redundancy.
    #
    # What replaces it is the check the stored column could never perform: that the
    # derivation covers the WHOLE population it claims to speak for. Rows with zero
    # obligations do not appear in OBLIGATIONS.tsv at all, so EXPECTED-MATCH is the
    # COMPLEMENT of the carrying set — and a complement is only meaningful against a
    # denominator that is itself verified.
    #
    # `phase` is matched EXACTLY, never by substring: `P1` as a substring also admits
    # `P1-ADJACENT` and silently inflates the universe 1065 -> 1414. That is not
    # hypothetical — it happened in a hand-written probe, and the arithmetic on top of it
    # was correct. Red-proved by a mutation a substring matcher structurally cannot fail.
    universe = {(os.path.dirname(r["case"]), r["consumer"]) for r in p1}
    if len(universe) != len(p1):
        fail(out, "DENOMINATOR", "%d P1 rows collapse to %d keys — the denominator is a bag, not a population"
             % (len(p1), len(universe)))
    stray = sorted(set(carriers) - universe)
    for k in stray[:5]:
        fail(out, "POPULATION", "%s/%s carries obligations but is not a P1 row" % k)
    if len(stray) > 5:
        fail(out, "POPULATION", "...and %d more" % (len(stray) - 5))
    divergence = len(set(carriers) & universe)

    if not quiet:
        per = collections.Counter(o["obligation_id"] for o in obls)
        print("obligations %d over %d carrying rows; contract blob %s"
              % (len(obls), len(carriers), rec.get("contract_blob", "?")[:12]))
        print("verdict (DERIVED, no stored column): %d EXPECTED-DIVERGENCE + %d EXPECTED-MATCH "
              "= %d P1 rows" % (divergence, len(universe) - divergence, len(universe)))
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
