#!/usr/bin/env python3
"""RED-PROOF — every check path in verify-obligations.py, both directions.

IT NEVER MUTATES THE TRACKED EVIDENCE FILES. Seeds are written to an isolated
temp directory and the verifier is pointed at them; the shared checkout is only
ever READ. A red-proof that copies, overwrites and restores the subject exposes
seeded bytes to every concurrent reader for the length of the run, and a restore
on the happy path does not close that window.

Neutral must pass; each seeded mutation must be CAUGHT BY ITS OWN NAMED CHECK, with
the seed diffed first. A seed that does not land is an INVALID TEST, not a pass — a
mutation of an absent phrase produces silence indistinguishable from a working check.
"""
import difflib, os, re, subprocess, sys, tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
OBL = os.path.join(HERE, "OBLIGATIONS.tsv")
FRESH = os.path.join(HERE, "FRESHNESS.tsv")
INV = os.path.join(HERE, "INVOCATIONS.tsv")

def run(obl=None, fresh=None, inv=None):
    cmd = [sys.executable, os.path.join(HERE, "verify-obligations.py")]
    for flag, val in (("--obl", obl), ("--fresh", fresh), ("--inv", inv)):
        if val:
            cmd += [flag, val]
    r = subprocess.run(cmd, capture_output=True, text=True)
    ids = {l.split()[1] for l in r.stdout.splitlines() if l.startswith("FAIL")}
    return r.returncode, ids

MUTATIONS = [
    # ---- THE SOURCE-DISCRIMINATION SEEDS. These are the incident's permanent
    # witness against its own recurrence. The shipped defect derived the successor
    # --active set by seeding it from the FROZEN document — the mtime-sourced
    # artifact under test — so it could add SC-524 futures but never remove an
    # mtime false positive. tg1 is the discriminating session: its newest EVENT is
    # 990s before the inside clock (INACTIVE under SC-017e), while its events.jsonl
    # MTIME is 60s before it (ACTIVE under the frozen bash rule). Any predicate
    # that substitutes mtime, or that seeds from the frozen set, produces `tg1`
    # here. The earlier seeds drifted only the CLOCK and the ADDRESS and were
    # structurally blind to the source error, which is exactly why it shipped.
    ("SC-521C-VALUE", OBL, "the successor set seeded from the frozen mtime-sourced document",
     lambda s: s.replace("@ now=1787243367\ttg1\tempty\t",
                         "@ now=1787243367\ttg1\ttg1\t", 1)),
    ("SC-521C-FROM", OBL, "the captured half misreported, erasing the divergence",
     lambda s: s.replace("@ now=1787243367\ttg1\tempty\t",
                         "@ now=1787243367\tempty\tempty\t", 1)),
    # ---- MEMBER 3: the clock binding. Aimed at the DEFECT (a window invocation
    # scored at a clock it was not captured at), not at the code that carries it.
    ("SC-521C-CLOCK-ARITY", OBL, "a clock-bound window invocation stripped of its set obligation",
     lambda s: "\n".join(l for l in s.split("\n") if not (
         l.startswith("arms/A2/c01-filters-ro\twin_inside_list_active_json\tSC-521c")))),
    ("SC-521C-CLOCK", OBL, "a set obligation addressed to a clock the capture did not run at",
     lambda s: s.replace("(set) @ now=1787243367\ttg1", "(set) @ now=1787243368\ttg1", 1)),
    ("SC-521C-SURFACE", OBL, "a set obligation on a digest with no recorded clock",
     lambda s: s.rstrip("\n") + "\n" + "\t".join(
         ["arms/A1/c01-healthy-ro", "list-json", "SC-521c", "digest", "sessions[] (set)",
          "1", "empty", "equals", "OBSERVED", "OBSERVED", "seeded"]) + "\n"),
    ("CLOCK-UNMAPPED", INV, "a window consumer the pinned harness does not produce",
     lambda s: s.replace("\twin_inside_list_busy\t", "\twin_inside_list_idle\t", 1)),
    ("CLOCK-AMBIGUOUS", INV, "a window consumer recorded twice, so bound to no single clock",
     lambda s: s.rstrip("\n") + "\n" + "\t".join(
         ["arms/A2/c01-filters-ro/consumers.tsv", "win_inside_list_busy", "0", "P1",
          "ae list", "list --busy", "seeded", "seeded"]) + "\n"),
    ("FROM", OBL, "a captured value the table misreports",
     lambda s: s.replace("\tschema_version\t1\t2\t", "\tschema_version\t3\t2\t", 1)),
    # Re-aimed: the SC-017o human diagnostic was the table's only `at-least` and its
    # only `stderr` row, and the entitlement re-derivation removed it. The domain checks
    # for those two values are therefore no longer exercised BY DATA — stated rather
    # than papered over by picking a value that happens to exist.
    ("PREDICATE", OBL, "a predicate outside the closed set",
     lambda s: s.replace("\tpresent\t", "\tvibes\t", 1)),
    ("STREAM", OBL, "a stream outside the closed set",
     lambda s: s.replace("\tdigest\t", "\ttelepathy\t", 1)),
    ("WRONG-KIND", OBL, "a membership obligation where the capture shows a label move",
     lambda s: s.replace("\tSC-017l\tdigest\tsessions[].status\tstopped\tunknown\tall-of\t",
                         "\tSC-017m\tdigest\t(row set)\tempty\tunknown rows present\tpresent\t", 1)),
    ("MISSING-509d", OBL, "a digest row stripped of its schema obligation",
     lambda s: "\n".join(l for l in s.split("\n")
                         if not (l.startswith("arms/A1/c01-healthy-ro\tlist-json\tSC-509d")))),
    ("POPULATION", OBL, "an obligation for a row outside the P1 universe it claims to cover",
     lambda s: s.rstrip("\n") + "\n" + "\t".join(
         ["arms/ZZ/not-a-case", "list", "SC-509d", "digest", "schema_version", "1", "2",
          "equals", "OBSERVED", "OBSERVED", "seeded"]) + "\n"),
    # The label check, red-proved by the ONE mutation a substring matcher cannot fail:
    # relabel a carrying row P1 -> P1-ADJACENT. Exact matching drops it from the universe
    # and its obligations become stray; substring matching still admits it and stays green.
    ("POPULATION", INV, "a carrying row relabelled out of P1 (substring matching would not notice)",
     lambda s: s.replace("arms/A1/c01-healthy-ro/consumers.tsv\tlist-json\t0\tP1\t",
                         "arms/A1/c01-healthy-ro/consumers.tsv\tlist-json\t0\tP1-ADJACENT\t", 1)),
    ("SUPPORT", OBL, "an obligation with no support verdict",
     lambda s: s.replace("\tUNSCORABLE\t", "\tmaybe\t", 1)),
    ("MISSING-509e", OBL, "an unreachable digest stripped of its agent-liveness move",
     lambda s: "\n".join(l for l in s.split("\n")
                         if not (l.startswith("arms/A1/c01-healthy-ro") and "\tSC-509e\t" in l))),
    ("MISSING-509b", OBL, "a read-loss digest stripped of its degraded move",
     lambda s: "\n".join(l for l in s.split("\n")
                         if not (l.startswith("arms/A1/c02-meta-mode-000-ro\tlist-all-json")
                                 and "\tSC-509b\t" in l))),
    ("MISSING-509c", OBL, "a digest stripped of the reason move its own agent state proves",
     lambda s: "\n".join(l for l in s.split("\n")
                         if not (l.startswith("arms/A3/c07-competing-rw\t")
                                 and "\tSC-509c\t" in l))),
    ("MISSING-509c", OBL, "an ALERT-derived reason move stripped (evidence class 2)",
     lambda s: "\n".join(l for l in s.split("\n")
                         if not ("\tSC-509c\t" in l and "sessions[twda1].agents[fake:probe]" in l))),
    ("SURFACE", OBL, "a JSON-only obligation parked on a human row",
     lambda s: s.replace("arms/A1/c02-meta-mode-000-ro\tlist-all-json\tSC-509b\t",
                         "arms/A1/c02-meta-mode-000-ro\tlist\tSC-509b\t", 1)),
    # colead's B1: an UNRELATED id adopting the new predicate. A closed-set member is
    # open until something binds who may use it.
    ("UNDECIDABLE", OBL, "an unrelated obligation adopting `undecidable` to launder itself",
     lambda s: re.sub(r"^(arms/A1/c01-healthy-ro\tlist-json\tSC-509d\t[^\n]*?)"
                      r"\tequals\tSOURCE\tOBSERVED\t",
                      r"\1\tundecidable\tSOURCE\tUNSCORABLE\t", s, count=1, flags=re.M)),
    # colead's B2b: the value row's target drifting while its predicate stays.
    ("UNDECIDABLE", OBL, "the completeness-value row with a drifted `to` target",
     lambda s: s.replace("the enumeration's actual completeness", "GARBAGE", 1)),
    ("VALUE-SHAPE", OBL, "the completeness-value locus carrying a scorable predicate",
     lambda s: s.replace("\tinventory_complete (value)\tABSENT\tthe enumeration's actual "
                         "completeness\tundecidable\tOBSERVED\tUNSCORABLE\t",
                         "\tinventory_complete (value)\tABSENT\tthe enumeration's actual "
                         "completeness\tequals\tOBSERVED\tOBSERVED\t", 1)),
    # colead's B2a: set membership where exact arity is owed.
    ("DUPLICATE-017o", OBL, "a digest carrying two completeness-value loci",
     lambda s: re.sub(r"^([^\n]*inventory_complete \(value\)[^\n]*)$", r"\1\n\1",
                      s, count=1, flags=re.M)),
    # colead's seed, and the one that matters: a CONTRADICTORY duplicate, not an
    # identical copy. The identical-copy seed is what let this escape — it tested the
    # easy duplicate while the payload-keyed address let the contradictory one buy
    # itself a new address by changing the very field that made it contradictory.
    ("DUPLICATE-ADDRESS", OBL, "a duplicated row whose `from` was altered to differ",
     lambda s: re.sub(r"^([^\n]*\tSC-509c\t[^\n]*)$",
                      lambda m: m.group(1) + "\n" + m.group(1).replace("\tnull\t", "\tGARBAGE\t", 1),
                      s, count=1, flags=re.M)),
    ("DUPLICATE-ADDRESS", OBL, "an identical duplicate — the easy case, kept as the control",
     lambda s: re.sub(r"^([^\n]*\tSC-509d\t[^\n]*)$", r"\1\n\1", s, count=1, flags=re.M)),
    # colead's converse seed: the required rows still there, an INVENTED third beside
    # them. Proving the owed rows exist never proved nothing else does.
    # colead's population seed, plus the sixth-ring sweep the lead asked me to run on
    # myself: a fabricated obligation on a row class its id may not appear on. Two of
    # the four ids I tested were previously caught only by the FROM check — payload,
    # not population — so they were caught by luck rather than by design.
    # colead's seed one level up: an id the declaration never heard of. Declaring who
    # may use each KNOWN member does not close the set unless an UNDECLARED member fails.
    ("UNKNOWN-ID", OBL, "an obligation id absent from the population declaration",
     lambda s: s.rstrip("\n") + "\narms/A1/c01-healthy-ro\tlist\tSC-BOGUS\tstdout"
               "\tfabricated locus\tABSENT\tsomething\tequals\tOBSERVED\tOBSERVED\tseeded\n"),
    ("POPULATION-ID", OBL, "an SC-017o diagnostic fabricated on a HUMAN invocation",
     lambda s: s.rstrip("\n") + "\narms/A1/c01-healthy-ro\tlist\tSC-017o\tstderr\tdiagnostic"
               "\tABSENT\tGARBAGE\tequals\tOBSERVED\tOBSERVED\tseeded\n"),
    ("POPULATION-ID", OBL, "a digest-only SC-509d fabricated on a human row",
     lambda s: s.rstrip("\n") + "\narms/A1/c01-healthy-ro\tlist\tSC-509d\tdigest\tschema_version"
               "\t1\t2\tequals\tSOURCE\tOBSERVED\tseeded\n"),
    ("EXTRA-017o", OBL, "an invented third SC-017o locus inflating the denominator",
     lambda s: re.sub(
         r"^([^\n]*\tSC-017o\tdigest\tinventory_complete\tABSENT\tpresent\tpresent\t[^\n]*)$",
         lambda m: m.group(1) + "\n" + m.group(1)
             .replace("\tinventory_complete\t", "\tGARBAGE-THIRD-LOCUS\t", 1)
             .replace("\tpresent\tpresent\t", "\tGARBAGE\tequals\t", 1),
         s, count=1, flags=re.M)),
    ("PRESENCE-SHAPE", OBL, "the presence locus existing BY NAME while asserting nothing",
     lambda s: re.sub(r"^([^\n]*\tSC-017o\tdigest\tinventory_complete\tABSENT\t)present\tpresent\t",
                      r"\1GARBAGE\tequals\t", s, count=1, flags=re.M)),
    # Fires UNDECIDABLE, not VALUE-SHAPE, and that is the gate being right rather than
    # the seed: any `undecidable` row whose WHOLE shape is wrong is reported by the
    # predicate branch, which prints the full shape including the drifted column. The
    # VALUE-SHAPE id is for a value-locus row that does NOT claim `undecidable`.
    ("UNDECIDABLE", OBL, "the value row with a drifted baseline_provenance — outside the old tuple",
     lambda s: re.sub(r"^([^\n]*inventory_complete \(value\)\tABSENT\tthe enumeration's actual "
                      r"completeness\tundecidable\t)OBSERVED\t", r"\1GARBAGE\t", s, count=1, flags=re.M)),
    ("MISSING-017o-SHAPE", OBL, "a digest stripped of its completeness VALUE locus",
     lambda s: "\n".join(l for l in s.split("\n")
                         if not (l.startswith("arms/A1/c01-healthy-ro\tlist-json\tSC-017o")
                                 and "inventory_complete (value)" in l))),
    # Member 1: the action-supplied carrier. Its contribution comes from action=throttled,
    # which no summary prefix matches — a summary-only reading dropped it entirely.
    ("MISSING-509c", OBL, "an ACTION-supplied throttled carrier stripped of its reason move",
     lambda s: "\n".join(l for l in s.split("\n")
                         if "sessions[tpairthrottledoverunanswered].agents[fake:high]" not in l)),
    # Member 2, both directions.
    ("SC-521C-ARITY", OBL, "an empty-scope digest stripped of its empty-set obligation",
     lambda s: "\n".join(l for l in s.split("\n")
                         if not (l.startswith("arms/A2/c01-filters-ro\tinter_needsattnstopped_json")
                                 and "\tSC-521c\t" in l))),
    # ADDS a row rather than MOVING one: moving it also emptied the source document and
    # fired ARITY, and it landed on a consumer name that does not exist in A2 (whose
    # consumers use underscores), so POPULATION fired too. A seed that trips three
    # checks proves none of them.
    ("SC-521C-SURFACE", OBL, "an empty-set obligation on a document whose scope is not empty",
     lambda s: s.rstrip("\n") + "\narms/A2/c01-filters-ro\tlist_json\tSC-521c\tdigest"
               "\tsessions[] (set)\t3\tempty\tequals\tOBSERVED\tOBSERVED\tseeded\n"),
    ("STALE", FRESH, "the contract having moved since derivation",
     lambda s: s.replace("contract_blob\t", "contract_blob\tdeadbeef", 1)),
]

# The two purpose-built source-discrimination fixtures, asserted on every run.
# c09 has a FUTURE event ts with an ordinary mtime; c10 has an ordinary event ts
# with a future mtime. A predicate reading events answers active/inactive; one
# reading mtime answers the opposite on both. Neither case carries a member-3 row
# (neither records a capture clock), so these prove the PREDICATE, not the table.
#
# NOTE ON c10, measured rather than assumed: its "future mtime" is set by the
# capture harness at run time and is NOT recoverable from the tracked bytes — git
# does not preserve mtimes, so the checked-out file shows an ordinary one. That is
# itself the argument for an event-sourced predicate: the mtime a reviewer can see
# is not the mtime the capture saw, while the event ts is the same bytes forever.
CONTROLS = [
    ("c09 future EVENT ts -> active", "arms/A3/c09-524a-future-ts-ordinary-mtime-ro",
     lambda ev, now: ev is not None and ev > now),
    ("c10 ordinary EVENT ts -> inactive", "arms/A3/c10-524b-ordinary-ts-future-mtime-ro",
     lambda ev, now: ev is not None and now - ev > 300),
]


def controls():
    """Prove the predicate discriminates its source before trusting any seed."""
    sys.path.insert(0, HERE)
    import obligations as o
    now, bad = 1787243367, 0
    for label, case, ok in CONTROLS:
        ev = o.last_event_epoch(o.template_of(case), "tg1")
        good = ok(ev, now)
        bad += 0 if good else 1
        print("control  %-36s event_epoch=%-12s %s"
              % (label, ev, "holds" if good else "<-- BROKEN"))
    return bad


def main():
    rc, ids = run()
    if rc != 0:
        print("ABORT: neutral is not clean — %s" % sorted(ids)); return 1
    print("neutral            rc=0  clean")
    bad0 = controls()
    bad = 0
    # Every mutation target, read ONCE from the shared checkout and never written.
    originals = {t: open(t, encoding="utf-8").read()
                 for t in {m[1] for m in MUTATIONS}}
    with tempfile.TemporaryDirectory(prefix="rp-obligations-") as tmp:
        for want, target, why, fn in MUTATIONS:
            orig = originals[target]
            mutated = fn(orig)
            if mutated == orig:
                print("%-14s SEED-DID-NOT-LAND — invalid test, NOT a pass (%s)" % (want, why))
                bad += 1
                continue
            delta = sum(1 for l in difflib.unified_diff(orig.split("\n"), mutated.split("\n"), n=0)
                        if l[:1] in "+-" and l[:3] not in ("+++", "---"))
            seeded = os.path.join(tmp, os.path.basename(target))
            open(seeded, "w", encoding="utf-8").write(mutated)
            kw = {OBL: "obl", FRESH: "fresh", INV: "inv"}[target]
            kw = {kw: seeded}
            rc2, ids2 = run(**kw)
            ok = rc2 != 0 and want in ids2
            if not ok:
                bad += 1
            print("%-14s delta=%-3d rc=%d ids=%-28s %s  (%s)"
                  % (want, delta, rc2, ",".join(sorted(ids2))[:28] or "-",
                     "caught" if ok else "<-- MISSED", why))

    rc3, _ = run()
    print("restored           rc=%d  %s" % (rc3, "clean" if rc3 == 0 else "DIRTY"))
    if rc3 != 0: bad += 1
    bad += bad0
    print("RED-PROOF: %s" % ("ALL PATHS PROVEN BY NAMED CHECK" if bad == 0 else "%d FAILURE(S)" % bad))
    return 1 if bad else 0

if __name__ == "__main__":
    sys.exit(main())
