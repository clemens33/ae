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
import json
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
    # ---- THE TWO NON-STOPPED REASON SEEDS. Both sit on A2/c01-filters-ro
    # list_all_json, a digest carrying SEVEN legitimate SC-509c reasons, so the OLD
    # row-level any/none converse stays green for both: deleting one leaves six, and
    # fabricating one adds an eighth beside them. Only exact Counter equality over
    # the complete owed set can see either. tg2b is RUNNING, so these exercise the
    # non-stopped half of the grammar the stopped-only derivation could not reach.
    ("OWED-MISSING", OBL, "one of seven owed reason loci deleted, six left standing",
     lambda s: "\n".join(l for l in s.split("\n") if not l.startswith(
         "arms/A2/c01-filters-ro\tlist_all_json\tSC-509c\tdigest\t"
         "sessions[tg2b].agents[fake:lead].reason\t"))),
    ("OWED-EXTRA", OBL, "a reason fabricated for a live agent with no carrier at all",
     lambda s: s.rstrip("\n") + "\n" + "\t".join(
         ["arms/A2/c01-filters-ro", "list_all_json", "SC-509c", "digest",
          "sessions[tg2b].agents[fake:bravo].reason", "null", "blocked", "equals",
          "OBSERVED", "OBSERVED", "seeded"]) + "\n"),
    # ---- THE SEVEN POPULATION SEEDS, all measured rc=0 against b1aeaacf's verifier
    # by gpt56sol:colead before the repair. Each is an EXACT SHAPE that the old
    # per-fragment checks accepted because they proved required loci EXIST and never
    # that unrequired loci are ABSENT. They exercise the SHIPPED verifier text.
    ("OWED-EXTRA", OBL, "an owed attention row RELOCATED from a pending session to a quiet one",
     lambda s: s.replace("sessions[tg2un].attention\t", "sessions[tg6a].attention\t", 1)),
    ("OWED-EXTRA", OBL, "an INVENTED SC-509 state on an agent with no producer declaration",
     lambda s: s.rstrip("\n") + "\n" + "\t".join(
         ["arms/A2/c01-filters-ro", "list_all_json", "SC-509", "digest",
          "sessions[tg6a].agents[fake:worker].state", "null", "working", "equals",
          "OBSERVED", "OBSERVED", "seeded"]) + "\n"),
    ("OWED-EXTRA", OBL, "an INVENTED SC-509c reason on an address with no carrier",
     lambda s: s.rstrip("\n") + "\n" + "\t".join(
         ["arms/A2/c01-filters-ro", "list_all_json", "SC-509c", "digest",
          "sessions[tg6a].agents[fake:worker].reason", "null", "blocked", "equals",
          "OBSERVED", "OBSERVED", "seeded"]) + "\n"),
    ("OWED-EXTRA", OBL, "the MOVER relabelled SC-518a -> SC-518 (counts move, ownership lies)",
     lambda s: s.replace("a6-c02-m2-wrong-ref-ro\trequests-all\tSC-518a\t",
                         "a6-c02-m2-wrong-ref-ro\trequests-all\tSC-518\t", 1)),
    ("OWED-EXTRA", OBL, "a status target COLLAPSED to from==to, locus left in place",
     lambda s: s.replace("].status\treplied\tpending\t", "].status\treplied\treplied\t", 1)),
    ("OWED-EXTRA", OBL, "an empty-scope MANDATED DIVERGENCE turned into a match",
     lambda s: s.replace("\tsessions[] (set)\t3\tempty\t", "\tsessions[] (set)\t3\t3\t", 1)),
    ("OWED-EXTRA", OBL, "a clock-bound row's PREDICATE swapped under unchanged values",
     lambda s: s.replace("(set) @ now=1787243367\ttg1\tempty\tequals\t",
                         "(set) @ now=1787243367\ttg1\tempty\tpresent\t", 1)),
    # ---- THE STOPPED-SESSION NULLING DEFECT, one seed PER FIELD CLASS. Frozen ae
    # nulls needs_attention/attention/attention_rank and every agent state on a
    # stopped session; SC-521c changes SELECTION and never the facts of a row
    # already selected. Restoring the defect means DELETING the obligations that
    # carry those facts, so each class is deleted separately — a single seed that
    # removed them all would prove only that SOMETHING fired, and the requirement
    # is that every affected class catches its own loss.
    ("OWED-MISSING", OBL, "the stopped-session agent states nulled again",
     lambda s: "\n".join(l for l in s.split("\n")
                         if not (l.startswith("arms/A2/c01-filters-ro\tlist_all_json\tSC-509\t")
                                 and ".state\t" in l))),
    ("OWED-MISSING", OBL, "the stopped-session attention facts nulled again",
     lambda s: "\n".join(l for l in s.split("\n")
                         if not (l.startswith("arms/A2/c01-filters-ro\tlist_all_json\tSC-017g\t")
                                 and "sessions[tg6b]." in l))),
    ("OWED-MISSING", OBL, "the stopped-session reasons nulled again",
     lambda s: "\n".join(l for l in s.split("\n")
                         if not (l.startswith("arms/A2/c01-filters-ro\tlist_all_json\tSC-509c\t")
                                 and "sessions[tg6b]." in l))),
    # ---- SC-518 / SC-518a. The identity move and the ordering move are separate
    # rows and each must be caught on its own; a seed that deleted both would not
    # show which rule the gate can actually police.
    ("OWED-MISSING", OBL, "the SC-518a ordering move deleted from A6 m2",
     lambda s: "\n".join(l for l in s.split("\n")
                         if not l.startswith("arms/A6/a6-c02-m2-wrong-ref-ro\trequests-all\tSC-518a\t"))),
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
    ("OWED-EXTRA", OBL, "the successor set seeded from the frozen mtime-sourced document",
     lambda s: s.replace("@ now=1787243367\ttg1\tempty\t",
                         "@ now=1787243367\ttg1\ttg1\t", 1)),
    ("OWED-EXTRA", OBL, "the captured half misreported, erasing the divergence",
     lambda s: s.replace("@ now=1787243367\ttg1\tempty\t",
                         "@ now=1787243367\tempty\tempty\t", 1)),
    # ---- MEMBER 3: the clock binding. Aimed at the DEFECT (a window invocation
    # scored at a clock it was not captured at), not at the code that carries it.
    ("OWED-MISSING", OBL, "a clock-bound window invocation stripped of its set obligation",
     lambda s: "\n".join(l for l in s.split("\n") if not (
         l.startswith("arms/A2/c01-filters-ro\twin_inside_list_active_json\tSC-521c")))),
    ("OWED-EXTRA", OBL, "a set obligation addressed to a clock the capture did not run at",
     lambda s: s.replace("(set) @ now=1787243367\ttg1", "(set) @ now=1787243368\ttg1", 1)),
    ("OWED-EXTRA", OBL, "a set obligation on a digest with no recorded clock",
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
    ("OWED-MISSING", OBL, "an ALERT-derived reason move stripped (evidence class 2)",
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
    ("OWED-EXTRA", OBL, "an empty-set obligation on a document whose scope is not empty",
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


def family_set_guard():
    """SEED 51, and it is a seed rather than a note BECAUSE a guard whose red-proof
    lives only in a report regresses silently.

    FAMILY-SET asserts that every declared owed-multiset comparison actually RAN.
    It exists because an unbounded text slice of mine once removed a comparison
    block and the verifier then reported VERIFIED — a green verdict for checks that
    no longer existed. It cannot be seeded through --obl/--fresh/--inv, because the
    subject is the VERIFIER's own call graph rather than any data file, so it is
    exercised here against the shipped module: declare a family nothing runs, and
    require rc=1.
    """
    import importlib.util as iu
    import io
    import contextlib
    spec = iu.spec_from_file_location("g", os.path.join(HERE, "verify-obligations.py"))
    g = iu.module_from_spec(spec)
    spec.loader.exec_module(g)
    buf = io.StringIO()
    with contextlib.redirect_stdout(buf):
        rc_clean, ids_clean = g.main(quiet=True)
    g._families_seen = set(g._families_seen)
    g.EXPECTED_FAMILIES = g.EXPECTED_FAMILIES | {"a-family-nobody-runs"}
    with contextlib.redirect_stdout(buf):
        rc_seeded, ids = g.main(quiet=True)
    # The claim is that a declared-but-unrun family FAILS — not that the table is
    # fresh. Requiring rc_clean == 0 coupled this guard to an unrelated STALE state
    # and made it unrunnable exactly while the table awaits a contract identity.
    ok = ("FAMILY-SET" not in ids_clean) and rc_seeded != 0 and "FAMILY-SET" in ids
    print("FAMILY-SET     control rc=%d (no FAMILY-SET)  seeded rc=%d ids=%s  %s  "
          "(a declared comparison family that never runs)"
          % (rc_clean, rc_seeded, ",".join(sorted(ids))[:34] or "-",
             "caught" if ok else "<-- MISSED"))
    return 0 if ok else 1


def third_branch_control():
    """The reason grammar's THIRD branch — a state event naming an agent as actor —
    is measured NEVER TO FIRE on this corpus: 47 events.bytes.jsonl files carry 72
    state events, all `working` or `done`, and neither is an agent-owned class. So
    the corpus proves the first two branches and CANNOT prove this one.

    Saying so is the point. What follows is not a claim that the corpus exercises
    it; it is a synthetic check that the branch behaves when fed a qualifying event,
    so the gap is bounded rather than merely admitted. Full-grammar proof is NOT
    claimed and must not be read into a green run.
    """
    import importlib.util as iu
    spec = iu.spec_from_file_location("g", os.path.join(HERE, "verify-obligations.py"))
    g = iu.module_from_spec(spec)
    spec.loader.exec_module(g)
    case = "arms/A2/c01-filters-ro"
    doc = json.loads(g.body(case, "list_all_json"))
    live = [x for x in doc["sessions"]
            if x.get("status") != "stopped" and any(
                a.get("reason") is None and a.get("state") not in g.AGENT_OWNED
                for a in x.get("agents") or [])]
    if not live:
        print("third-branch   NO SYNTHETIC SUBJECT — cannot bound the gap"); return 1
    sess = live[0]
    ref = next(a["ref"] for a in sess["agents"]
               if a.get("reason") is None and a.get("state") not in g.AGENT_OWNED)
    real = g.gate_declared_contributions
    _base_set = set(g.owed_reason(case, doc))
    base = len(_base_set)
    g.gate_declared_contributions = lambda c: {ref: {"blocked"}}
    seeded = g.owed_reason(case, doc)
    g.gate_declared_contributions = real
    want = ("SC-509c", "digest", "sessions[%s].agents[%s].reason" % (sess["name"], ref),
            "null", "blocked", "equals", "OBSERVED", "OBSERVED")
    # A ref can appear in SEVERAL sessions, so the synthetic carrier legitimately
    # owes one row per session holding it — asserting base+1 was wrong arithmetic
    # about the right behaviour. What must hold: the named row appears, and every
    # row the patch ADDS names that same agent.
    added = {t for t in seeded if t not in _base_set}
    ok = want in seeded and added and all("agents[%s]" % ref in t[2] for t in added)
    print("third-branch   corpus fires it 0 times; synthetic state-event carrier for "
          "%s/%s -> %s  (%d -> %d owed)"
          % (sess["name"], ref, "owed, as required" if ok else "<-- MISSED",
             base, len(seeded)))
    return 0 if ok else 1


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
    bad0 += family_set_guard()
    bad0 += third_branch_control()
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
