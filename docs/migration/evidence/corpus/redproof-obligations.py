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
import re
import difflib, os, re, subprocess, sys, tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
OBL = os.path.join(HERE, "OBLIGATIONS.tsv")
FRESH = os.path.join(HERE, "FRESHNESS.tsv")
INV = os.path.join(HERE, "INVOCATIONS.tsv")
GAP = os.path.join(HERE, "UNOBSERVABLE-ADDED-ROSTER.tsv")
UNPROVED = os.path.join(HERE, "SC-509C-UNPROVED.tsv")

def run(obl=None, fresh=None, inv=None, gap=None, unproved=None):
    cmd = [sys.executable, os.path.join(HERE, "verify-obligations.py")]
    # FRESHNESS and its three data members are generated as one four-file snapshot.
    # Always bind the whole tuple, even when only one member is seeded, so a
    # temporary table can never fall through to a live gap or UNPROVED declaration
    # (or vice versa).
    artifacts = (("--obl", obl or OBL), ("--fresh", fresh or FRESH),
                 ("--gap", gap or GAP), ("--unproved", unproved or UNPROVED))
    for flag, val in artifacts + (("--inv", inv),):
        if val:
            cmd += [flag, val]
    r = subprocess.run(cmd, capture_output=True, text=True)
    ids = {l.split()[1] for l in r.stdout.splitlines() if l.startswith("FAIL")}
    return r.returncode, ids

MUTATIONS = [
    # ---- SURFACE-CROSSED, AND THE CROSSING IS THE POLICY RATHER THAN THE LUCK.
    # The first five l/m seeds all landed on HUMAN rows by accident, and the hole
    # they failed to find was DIGEST-side: the comparison sat inside a digest-only
    # block, so two of them went MISSED and that accident is the only reason the
    # scope hole was caught. A check hole inherited from its enclosing block is
    # provable ONLY by a seed living in the scope that lost coverage, so every shape
    # now exists on both surfaces.
    ("UNCONSUMED-ROW", OBL, "the legacy view-level m locus on the DIGEST surface",
     lambda s: s.replace("SC-017m\tdigest\tsessions[tg1]\t",
                         "SC-017m\tdigest\t(row set)\t", 1)),
    ("OWED-EXTRA", OBL, "the legacy NAMELESS l locus on the HUMAN surface",
     lambda s: s.replace("SC-017l\tstdout\tcandidate[tg1].status",
                         "SC-017l\tstdout\tstatus cell", 1)),
    ("OWED-EXTRA", OBL, "a HUMAN l row wearing the digest's stream and locus",
     lambda s: s.replace("SC-017l\tstdout\tcandidate[tg1].status",
                         "SC-017l\tdigest\tsessions[tg1].status", 1)),
    ("OWED-MISSING", OBL, "one derived l identity deleted on the HUMAN surface",
     lambda s: re.sub(r"(?m)^.*\tSC-017l\tstdout\tcandidate\[tg1\]\.status\t.*\n",
                      "", s, count=1)),
    ("OWED-MISSING", OBL, "one derived m identity deleted on the DIGEST surface",
     lambda s: re.sub(r"(?m)^.*\tSC-017m\tdigest\tsessions\[tg1\]\t.*\n",
                      "", s, count=1)),
    # ---- SC-017r's (session, rendered ref, short sid) class rows, one seed per
    # ruled failure.
    ("OWED-MISSING", OBL, "a class row rendered at the WRONG multiplicity",
     lambda s: s.replace("unambiguous unknown x2", "unambiguous unknown x3", 1)),
    ("OWED-MISSING", OBL, "a class row COLLAPSED to a single identity row",
     lambda s: s.replace("agents[tg10:fake:lead:-](class).health\tABSENT x2",
                         "agents[tg10:fake:lead:-].health\tABSENT", 1)),
    # ---- SC-017h shares r's fixed identity but owns the state carrier. These
    # mutations are aimed at the two wrong-column consequences: calling stopped
    # session_id a state, and losing exact collision multiplicity.
    ("OWED-MISSING", OBL, "a stopped h class relabelled from no-cell ABSENT to dash",
     lambda s: s.replace("agents[tg10:fake:lead:-](class).state\tABSENT x2",
                         "agents[tg10:fake:lead:-](class).state\t- x2", 1)),
    ("OWED-MISSING", OBL, "an h collision class carrying the wrong target multiplicity",
     lambda s: s.replace("agents[tg10:fake:lead:-](class).state\tABSENT x2\t- x2",
                         "agents[tg10:fake:lead:-](class).state\tABSENT x2\t- x1", 1)),
    ("OWED-MISSING", OBL, "one fixed-identity h obligation deleted",
     lambda s: re.sub(r"(?m)^.*\tSC-017h\tstdout\tagents\[[^\]]+\]\.state\t.*\n",
                      "", s, count=1)),
    # ---- p/q target legs. p/dead exists only on c20's exact missing pane; A4's
    # real six-field snapshot has matching panes but no pane_dead and therefore q.
    ("OWED-MISSING", OBL, "c20's exact missing-pane dead target collapsed to unknown",
     lambda s: s.replace("agents[ta1k:fake:worker:-].health\tblank\tdead",
                         "agents[ta1k:fake:worker:-].health\tblank\tunambiguous unknown", 1)),
    ("OWED-MISSING", OBL, "one c20 blank-to-literal-alive presentation row deleted",
     lambda s: re.sub(
         r"(?m)^arms/A1/c20-405k-live\ts0-baseline/list\tSC-017r\tstdout\t"
         r"agents\[ta1k:fake:lead:-\]\.health\tblank\talive\t.*\n", "", s, count=1)),
    ("OWED-MISSING", OBL, "a c20 blank-to-alive target collapsed to semantic equality",
     lambda s: s.replace("agents[ta1k:fake:lead:-].health\tblank\talive",
                         "agents[ta1k:fake:lead:-].health\tblank\tblank", 1)),
    ("OWED-MISSING", OBL, "A4's absent-pane_dead unknown target fabricated as dead",
     lambda s: s.replace("agents[ta4c01statuslive:fake:lead:-].health\tblank\t"
                         "unambiguous unknown",
                         "agents[ta4c01statuslive:fake:lead:-].health\tblank\tdead", 1)),
    # ---- THE OLD SHAPES MUST STAY DEAD. SC-017l/m were replaced wholesale: the
    # shipped rows were nameless and invocation-grained, and a table that quietly
    # re-acquires one of those shapes has un-done the replacement while every count
    # still looks plausible. Each legacy shape gets its own seed, because one seed
    # covering three shapes cannot say which came back.
    ("OWED-EXTRA", OBL, "the legacy view-level (row set) m locus, re-added",
     lambda s: s.replace("SC-017m\tstdout\tview.members[tg1]",
                         "SC-017m\tstdout\t(row set)", 1)),
    ("UNCONSUMED-ROW", OBL, "the legacy NAMELESS l locus, re-added",
     lambda s: s.replace("SC-017l\tdigest\tsessions[tg1].status",
                         "SC-017l\tdigest\tsessions[].status", 1)),
    ("UNCONSUMED-ROW", OBL, "an l row wearing the WRONG SURFACE's stream and locus",
     lambda s: s.replace("SC-017l\tdigest\tsessions[tg1].status",
                         "SC-017l\tstdout\tcandidate[tg1].status", 1)),
    # ---- AND THE REPLACEMENT MUST NOT BE SUPPRESSIBLE WITH THE PATH THAT MADE IT.
    # Deleting a derived identity has to fail as a MISSING obligation, not merely
    # stop being unconsumed -- otherwise deleting the emission and deleting the
    # expectation would cancel out and the gate would stay green over nothing.
    ("OWED-MISSING", OBL, "one derived l identity deleted from the table",
     lambda s: re.sub(r"(?m)^.*\tSC-017l\tdigest\tsessions\[tg1\]\.status\t.*\n", "", s, count=1)),
    ("OWED-MISSING", OBL, "one derived m identity deleted from the table",
     lambda s: re.sub(r"(?m)^.*\tSC-017m\tstdout\tview\.members\[tg1\]\t.*\n", "", s, count=1)),
    # ---- THE AUTHORITY SIGNATURE'S FIVE RULED SEEDS. The prefix is the byte-level
    # difference between an entitled row and an unreasoned coincidence, so each way
    # of corrupting it gets its own seed. The retired-term seed scans the WHOLE
    # field, prose included: a retired term in narrative still teaches the wrong
    # rule to the next reader, and these rows are read as the ruling.
    ("AUTHORITY-SIGNATURE", OBL, "an EMPTY signature prefix on a loss row",
     lambda s: re.sub(r"SIG owner=SC-509b [^\t]*? :: ", "", s, count=1)),
    ("AUTHORITY-SIGNATURE", OBL, "a signature naming the WRONG OWNER row",
     lambda s: s.replace("SIG owner=SC-509b", "SIG owner=SC-017g", 1)),
    ("AUTHORITY-SIGNATURE", OBL, "a signature naming a member that is not its locus",
     lambda s: s.replace("member=sessions[tg1].degraded", "member=sessions[tg1].goal", 1)),
    ("AUTHORITY-SIGNATURE", OBL, "an entitlement class outside the closed vocabulary",
     lambda s: s.replace("class=actual-loss-visible", "class=seems-about-right", 1)),
    ("AUTHORITY-RETIRED", OBL, "the retired lower-bound phrase anywhere in the authority",
     lambda s: s.replace("ALWAYS-PRESENT PARTIAL-EVIDENCE INDICATOR",
                         "degraded-context LOWER BOUND", 1)),
    # ---- B4 and B5, the two retained blockers colead reproduced at rc=0.
    ("OWED-MISSING", OBL, "a relational `from` that misreports the captured member",
     lambda s: s.replace("sessions[tg2un].needs_attention\tfalse\t",
                         "sessions[tg2un].needs_attention\ttrue\t", 1)),
    ("FRESHNESS-COUNT", FRESH, "the published P1 count stopped matching INVOCATIONS",
     lambda s: re.sub(r"(?m)^p1_rows\t\d+$", "p1_rows\t999", s, count=1)),
    ("FRESHNESS-COUNT", FRESH, "the published obligation count stopped matching the table",
     lambda s: re.sub(r"(?m)^obligation_rows\t\d+$",
                      "obligation_rows\t7", s, count=1)),
    ("FRESHNESS-SCHEMA", FRESH,
     "a conflicting duplicate whose correct contract blob remains last",
     lambda s: re.sub(r"(?m)^(contract_blob\t.+)$",
                      lambda m: "contract_blob\tdeadbeef\n" + m.group(1), s, count=1)),
    ("FRESHNESS-SCHEMA", FRESH, "a false contract_path in the tuple manifest",
     lambda s: s.replace("contract_path\tdocs/migration/semantic-contract.md",
                         "contract_path\tdocs/migration/not-the-contract.md", 1)),
    ("FRESHNESS-SCHEMA", FRESH, "an unknown tuple-manifest field",
     lambda s: s.rstrip("\n") + "\nfuture_unruled_field\tseems-fine\n"),
    ("FRESHNESS-SCHEMA", FRESH, "a drifted field/value header",
     lambda s: s.replace("field\tvalue", "name\tvalue", 1)),
    ("FRESHNESS-SCHEMA", FRESH, "the required p1_rows record omitted",
     lambda s: re.sub(r"(?m)^p1_rows\t\d+\n?", "", s, count=1)),
    # ---- THE GLOBAL CONSUMPTION SEEDS. Both were reproduced at rc=0 against the
    # pre-W3 verifier by gpt56sol:colead. They pass any check that filters the HELD
    # side, because a filter discards the addition before the comparison sees it —
    # which is why the held side is now compared in its entirety and every row must
    # be CONSUMED by some family's owed set.
    ("UNCONSUMED-ROW", OBL, "an invented SC-509d locus no derivation owes",
     lambda s: s.rstrip("\n") + "\n" + "\t".join(
         ["arms/A2/c01-filters-ro", "list_all_json", "SC-509d", "digest",
          "schema_version_v2", "1", "2", "equals", "SOURCE", "OBSERVED", "seeded"]) + "\n"),
    ("UNCONSUMED-ROW", OBL, "an SC-017g addition on a RUNNING session, outside every filter",
     lambda s: s.rstrip("\n") + "\n" + "\t".join(
         ["arms/A2/c01-filters-ro", "list_all_json", "SC-017g", "digest",
          "sessions[tg1].needs_attention", "false", "true", "equals", "OBSERVED",
          "OBSERVED", "seeded"]) + "\n"),
    # ---- THE OLD-ROW SEEDS. Six SC-509 state rows claimed the successor would render
    # `working`/`done` out of a ledger carrying a malformed COMPLETE record. The
    # source is unreadable, so SC-509b takes the locus and what is owed is ABSENCE.
    # These prove a table still carrying the old row cannot survive — in BOTH
    # directions, because a removal that is not also a re-target would leave the
    # address unowned.
    ("OWED-EXTRA", OBL, "an OLD SC-509 state row restored on the malformed-ledger case",
     lambda s: s.replace(
         "arms/A1/c03-malformed-line-ro\tlist-all-json\tSC-509b\tdigest\t"
         "sessions[tg1].agents[fake:lead].state\tnull\tABSENT\t",
         "arms/A1/c03-malformed-line-ro\tlist-all-json\tSC-509\tdigest\t"
         "sessions[tg1].agents[fake:lead].state\tnull\tworking\t", 1)),
    ("OWED-EXTRA", OBL, "the damaged-ledger state row re-targeted back to a rendered value",
     lambda s: s.replace(
         "sessions[tg1].agents[fake:worker].state\tnull\tABSENT\t",
         "sessions[tg1].agents[fake:worker].state\tnull\tdone\t", 1)),
    # ---- THE THREE-WAY REGRAIN PROOF. The qualifier and the qualified are one pair
    # per loss session, and the locus must NAME that session. Arm three is the one
    # that distinguishes a regrain ENFORCED from a regrain merely PERFORMED: a row
    # left at the old unqualified `sessions[].degraded` address must fail, or the
    # rename was cosmetic.
    ("OWED-MISSING", OBL, "the degraded qualifier dropped, its needs_attention half left standing",
     lambda s: "\n".join(l for l in s.split("\n") if not l.startswith(
         "arms/A1/c02-meta-mode-000-ro\tlist-all-json\tSC-509b\tdigest\tsessions[tg1].degraded\t"))),
    ("OWED-MISSING", OBL, "the needs_attention partial-evidence indicator dropped, its qualifier left standing",
     lambda s: "\n".join(l for l in s.split("\n") if not l.startswith(
         "arms/A1/c02-meta-mode-000-ro\tlist-all-json\tSC-509b\tdigest\tsessions[tg1].needs_attention\t"))),
    ("OWED-EXTRA", OBL, "a row left at the OLD unqualified locus — regrain performed, not enforced",
     lambda s: s.replace("\tsessions[tg1].degraded\t", "\tsessions[].degraded\t", 1)),
    ("OWED-MISSING", OBL, "the SC-405g branch move deleted from a degraded entry",
     lambda s: "\n".join(l for l in s.split("\n")
                         if "\tSC-405g\tdigest\tsessions[tg1].branch (presence)\t" not in l)),
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
     lambda s: s.replace("\tpresent\tOBSERVED\tOBSERVED\t",
                         "\tvibes\tOBSERVED\tOBSERVED\t", 1)),
    ("STREAM", OBL, "a stream outside the closed set",
     lambda s: s.replace("\tdigest\t", "\ttelepathy\t", 1)),
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
    ("OWED-MISSING", OBL, "a read-loss digest stripped of its degraded move",
     lambda s: "\n".join(l for l in s.split("\n")
                         if not (l.startswith("arms/A1/c02-meta-mode-000-ro\tlist-all-json")
                                 and "\tSC-509b\t" in l))),
    ("OWED-MISSING", OBL, "a digest stripped of the reason move its own agent state proves",
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
    ("OWED-MISSING", OBL, "an ACTION-supplied throttled carrier stripped of its reason move",
     lambda s: "\n".join(l for l in s.split("\n")
                         if "sessions[tpairthrottledoverunanswered].agents[fake:high]" not in l)),
    # Member 2, both directions.
    ("OWED-MISSING", OBL, "an empty-scope digest stripped of its empty-set obligation",
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
    # KEYED BY (session, actor). Keyed by actor alone this map added SIX rows for
    # one carrier — every same-ref agent across the composite digest — and my first
    # response was to relax the ASSERTION to match, which is repairing expectations
    # to fit output. base+1 was the right grain and the derivation was the wrong
    # thing; colead caught it.
    g.gate_declared_contributions = lambda c: {(sess["name"], ref): {"blocked"}}
    seeded = g.owed_reason(case, doc)
    g.gate_declared_contributions = real
    want = ("SC-509c", "digest", "sessions[%s].agents[%s].reason" % (sess["name"], ref),
            "null", "blocked", "equals", "OBSERVED", "OBSERVED")
    # EXACTLY ONE ROW, at exactly the selected session address. A carrier bound to
    # one session must not authorize its namesakes elsewhere.
    added = {t for t in seeded if t not in _base_set}
    ok = added == {want}
    print("third-branch   corpus fires it 0 times; synthetic (session, actor) carrier "
          "for %s/%s -> %s  (%d -> %d owed, %d added)"
          % (sess["name"], ref, "exactly its own address" if ok else "<-- MISSED",
             base, len(seeded), len(added)))
    return 0 if ok else 1


def loss_arity_control():
    """SEED 56. The 28 optional-member loci were once DROPPED from owed_loss and the
    only thing that would have caught it was a count living in a chat message. It is
    code now: every loss occurrence owes exactly four SC-509b loci plus at most one
    SC-405g branch move, asserted inside the derivation.

    Like FAMILY-SET this cannot be seeded through --obl/--fresh/--inv, because the
    subject is the DERIVATION's own completeness rather than any data file.
    """
    import importlib.util as iu
    import json
    spec = iu.spec_from_file_location("g", os.path.join(HERE, "verify-obligations.py"))
    g = iu.module_from_spec(spec)
    spec.loader.exec_module(g)
    case = "arms/A1/c02-meta-mode-000-ro"
    doc = json.loads(g.body(case, "list-all-json"))
    full = g.owed_loss(case, doc)
    occ = sum(1 for x in doc.get("sessions", []) or []
              if (x.get("name") or "") in g.gate_loss_sessions(case))
    branch = sum(1 for t in full if t[0] == "SC-405g")
    for member in ("attention", "attention_rank", "needs_attention", "degraded"):
        trimmed = {t for t in full if not t[2].endswith("." + member)}
        if len(trimmed) == occ * 4 + branch:
            print("loss-arity     dropping .%s left the arity intact <-- MISSED" % member)
            return 1
    print("loss-arity     %d occurrence(s) owe %d SC-509b loci + %d branch; dropping any "
          "one member breaks the arity" % (occ, occ * 4, branch))
    return 0


def plural_candidate_control():
    """SEED 62. The multi-candidate path runs 268 times over SINGLETONS.

    Measured: every one of the 268 unreachable listing cases carries EXACTLY ONE
    durable candidate, so 268 green passes say nothing about whether the loop
    discriminates the plural case. A corpus that proves one branch cannot prove the
    other, so the other is fed synthetically and the gap is BOUNDED rather than
    admitted — the same treatment the reason grammar's unexercised third branch got.
    """
    import importlib.util as iu
    import tempfile
    spec = iu.spec_from_file_location("g", os.path.join(HERE, "verify-obligations.py"))
    g = iu.module_from_spec(spec)
    spec.loader.exec_module(g)
    real = g.SRC
    try:
        with tempfile.TemporaryDirectory(prefix="plural-cand-") as td:
            d = os.path.join(td, "fake")
            os.makedirs(d)
            with open(os.path.join(d, "manifest.before.tsv"), "w", encoding="utf-8") as fh:
                fh.write("dir\t-\t-\t./sessions/alpha\n"
                         "dir\t-\t-\t./sessions/beta\n"
                         "file\t644\tabc\t./sessions/alpha/meta\n")
            g.SRC = td
            cands = g.durable_candidates("fake")
            rows = g.owed_candidate_rows("fake", cands, "OBSERVED")
        ok = sorted(cands) == ["alpha", "beta"] and len(rows) == 2
    finally:
        g.SRC = real
    print("plural-cand    corpus is 268 singletons; synthetic two-candidate view -> "
          "%d candidates, %d owed row(s)  %s"
          % (len(cands), len(rows), "caught" if ok else "<-- MISSED"))
    return 0 if ok else 1


def class_drop_control():
    """SEED 63. Remove a member CLASS from the declared mapping and the
    derivation must change — across EVERY declared class, not one family's.

    The old hardcoded 14x4 arity could not perform this mutation at all: it carried
    a COPY of the member list, so a class removed from the mapping left the copy
    intact and the check green. Iterating only the event-derived five would have
    proven the mechanism on the family that happened to be under repair, while the
    defects this slice actually shipped included an OMITTED META SCALAR set and a
    misfiled COMMON qualifier. So every declared class is dropped in turn, plus the
    branch-presence ownership that lives outside LOSS_MEMBERS.

    WHAT 14-OF-14 DOES NOT COVER, stated here rather than in a report because a gap
    that lives in a message evaporates at the next context boundary and what
    survives is the number: `meta-duplicate` contributes ZERO iterations, because
    its declared tuple is EMPTY BY DESIGN — its members are the keys actually
    duplicated, so the path is DATA-DEPENDENT rather than declared and there is no
    class here to drop. That path is covered by the arity control, by the
    loss-family Counter, and by meta_duplicate_control() below — NOT by class-drop.
    Read 14-of-14 as fourteen declared classes, never as complete coverage.
    """
    import importlib.util as iu
    import json
    spec = iu.spec_from_file_location("g", os.path.join(HERE, "verify-obligations.py"))
    g = iu.module_from_spec(spec)
    spec.loader.exec_module(g)
    # a case per kind, so every class has a population that can move
    subjects = {
        "common": ("arms/A1/c02-meta-mode-000-ro", "list-all-json"),
        "meta-absent": ("arms/A1/c02-meta-mode-000-ro", "list-all-json"),
        "meta-duplicate": ("arms/A7/a7-c02-meta-duplicate-key-ro", "list-json"),
        "events-skipped": ("arms/A1/c03-malformed-line-ro", "list-all-json"),
    }
    results = []
    for kind, spec_members in sorted(g.LOSS_MEMBERS.items()):
        case, cons = subjects.get(kind, subjects["events-skipped"])
        doc = json.loads(g.body(case, cons))
        before = g.owed_loss(case, doc)
        for scope in ("session", "agent"):
            real = spec_members[scope]
            for drop in real:
                g.LOSS_MEMBERS[kind][scope] = tuple(m for m in real if m != drop)
                try:
                    after = g.owed_loss(case, doc)
                    moved = after != before
                except RuntimeError:
                    moved = True            # the arity assertion itself fired
                finally:
                    g.LOSS_MEMBERS[kind][scope] = real
                results.append(("%s/%s" % (kind, drop), moved))
    # branch presence is owned outside LOSS_MEMBERS; drop it by name
    case, cons = subjects["events-skipped"]
    doc = json.loads(g.body(case, cons))
    base = g.owed_loss(case, doc)
    without_branch = {t for t in base if t[0] != "SC-405g"}
    results.append(("branch-presence", without_branch != base))
    ok = all(m for _, m in results)
    print("class-drop     %d declared class(es) dropped in turn: %s  %s"
          % (len(results), ", ".join("%s=%s" % (n, "yes" if m else "NO")
                                     for n, m in results),
             "caught" if ok else "<-- MISSED"))
    return 0 if ok else 1


def meta_duplicate_control():
    """SEED 64 — the data-dependent path class-drop structurally cannot reach.

    `meta-duplicate` declares no members, so the only way to break it is to make the
    DERIVATION return empty for a real occurrence. A7 a7-c02 duplicates `goal`, and
    emptying that path must produce a NAMED gate failure — the owed-set delta alone
    is not the requirement, because a derivation can shrink without anything saying
    so.
    """
    import importlib.util as iu
    import io
    import contextlib

    def load():
        spec = iu.spec_from_file_location("g", os.path.join(HERE, "verify-obligations.py"))
        m = iu.module_from_spec(spec)
        spec.loader.exec_module(m)
        return m

    buf = io.StringIO()
    g = load()
    with contextlib.redirect_stdout(buf):
        rc_clean, ids_clean = g.main(quiet=True)
    g = load()
    g.gate_duplicated_meta_keys = lambda c, s: ()
    with contextlib.redirect_stdout(buf):
        rc_seed, ids_seed = g.main(quiet=True)
    # Emptied derivation means the held duplicated-meta rows become OWED-EXTRA.
    # Accepting any nonempty failure set would let an unrelated check prove this
    # seed; bind the exact named discriminator and require it absent on neutral.
    target = "OWED-EXTRA"
    ok = (rc_clean == 0 and target not in ids_clean
          and rc_seed != 0 and target in ids_seed)
    print("meta-dup       duplicated-key derivation emptied for a real A7 occurrence: "
          "control rc=%d, mutant rc=%d ids=%s  %s"
          % (rc_clean, rc_seed, ",".join(sorted(ids_seed))[:34] or "-",
             "caught" if ok else "<-- MISSED"))
    return 0 if ok else 1


def name_consumption_control():
    """SEED 65, rebuilt twice — once because its seam shipped, once because its
    number was wrong.

    SC-017j keeps a live-only running row and a durable candidate of the same NAME
    as two identities. That resolution otherwise lives only in the shape of the
    code, and a refactor reintroducing name-consumption would be wrong BY AGREEMENT:
    nothing in the output would disagree, there would simply be less of it.

    The mutation SOURCE-PATCHES A SCRATCH COPY rather than flipping a flag — a
    runtime switch that can select known-wrong behaviour is an alternate gate path
    in shipped code, invocable by accident or by drift.

    And the expectation is the OBSERVED IDENTITY SET, not a literal: the namesake
    count moved 8 -> 16 when the selector predicate was repaired, so a hardcoded 8
    would have quietly become a wrong target instead of a failing assertion.
    """
    import importlib.util as iu
    import csv as _csv
    import shutil
    import tempfile

    def load(path):
        spec = iu.spec_from_file_location("gmut", path)
        m = iu.module_from_spec(spec)
        spec.loader.exec_module(m)
        return m

    sys.path.insert(0, HERE)
    import obligations as _o
    src = os.path.join(HERE, "verify-obligations.py")

    def namesakes(g):
        seen = set()
        with open(os.path.join(HERE, "INVOCATIONS.tsv"), encoding="utf-8") as fh:
            for r in _csv.DictReader(fh, delimiter="\t"):
                if r["phase"] != "P1":
                    continue
                case = os.path.dirname(r["case"])
                if not (_o.unreachable(case) or g.gate_missing_selector(case)):
                    continue
                text = _o.body(case, r["consumer"])
                for c, rep in g.candidate_representation(case, r["consumer"], text).items():
                    if rep == "live-only-namesake":
                        seen.add((case, r["consumer"], c))
        return seen

    before = namesakes(load(src))
    with tempfile.TemporaryDirectory(prefix="name-consume-") as td:
        patched = os.path.join(td, "verify-obligations.py")
        shutil.copy(src, patched)
        text = open(patched, encoding="utf-8").read()
        marker = '        elif shown[c] == "running":'
        if marker not in text:
            print("name-consume   cannot locate the identity rule to patch — INVALID TEST")
            return 1
        open(patched, "w", encoding="utf-8").write(
            text.replace(marker, '        elif False:', 1))
        after = namesakes(load(patched))
    ok = bool(before) and not after
    print("name-consume   scratch-copy patch removes the identity rule; the %d observed "
          "namesake identities all stop being pair cases  %s"
          % (len(before), "caught" if ok else "<-- MISSED"))
    return 0 if ok else 1


def selector_leg_control():
    """SEED 66 — each selector leg deleted SEPARATELY, requiring the EXACT missed
    identity set rather than a moved total.

    A selector goes missing two ways: the meta is ABSENT from the manifest, or it is
    PRESENT with an unusable mode. The first version of this predicate scanned
    manifest ROWS for a bad mode, so the absent leg had no row to carry a marker and
    could not be seen — it reported 10 invocations where there are 20. A total would
    have moved under either deletion and told you nothing about WHICH leg died, so
    each is deleted on its own and the exact identities it loses are required.
    """
    import importlib.util as iu
    import csv as _csv

    def load():
        spec = iu.spec_from_file_location("g", os.path.join(HERE, "verify-obligations.py"))
        m = iu.module_from_spec(spec)
        spec.loader.exec_module(m)
        return m

    def identities(g):
        seen = set()
        with open(os.path.join(HERE, "INVOCATIONS.tsv"), encoding="utf-8") as fh:
            for r in _csv.DictReader(fh, delimiter="\t"):
                if r["phase"] != "P1":
                    continue
                case = os.path.dirname(r["case"])
                for c in g.gate_missing_selector(case):
                    seen.add((case, c))
        return seen

    full = identities(load())
    results = []
    for leg, keep in (("meta-absent", 1), ("present-but-unusable", 0)):
        g = load()
        real = g.selector_legs
        g.selector_legs = lambda case, _r=real, _k=keep: (
            ((), _r(case)[1]) if _k == 1 else (_r(case)[0], ()))
        lost = full - identities(g)
        expected = {(c, n) for (c, n) in full
                    if n in (real(c)[0] if leg == "meta-absent" else real(c)[1])}
        results.append((leg, lost == expected and bool(lost), len(lost)))
    ok = all(good for _, good, _ in results)
    print("selector-legs  %s  %s"
          % ("; ".join("%s deletion loses exactly its %d identities: %s"
                       % (leg, n, "yes" if good else "NO")
                       for leg, good, n in results),
             "caught" if ok else "<-- MISSED"))
    return 0 if ok else 1


_SHADOW_BODY = (
    # BUILT FROM THE BYTES THAT PRODUCED THE JUNK KEY, not from an imagined table.
    # The first fixture invented a requests table keyed by ID and the collision never
    # happened -- the seed passed while proving nothing. The real requests view puts
    # STATUS in column 0, so `pending` and `replied` ARE its row keys. The tables are
    # butted together with no blank line, which is the variant that actually breaks a
    # blank-line boundary.
    "SESSION                   STATUS    MODE        ORIGIN\n"
    "pending                   stopped   local       /tmp/x\n"
    "  goal (1m ago): a session whose NAME is another table's column-0 key\n"
    "STATUS   TYPE     ID                           FROM        TO          SUMMARY\n"
    "replied  ask      ae-20260820T161302Z-c2c01848 fake:lead   fake:worker G5 mirror\n"
    "pending  ask      ae-20260820T161304Z-be61f143 fake:third  fake:lead   G5 third\n"
)


def shadow_control():
    """SEED 67 — a LEGAL session name that is also another table's key.

    colead's blocker, and it is the right one: binding the whole-body scrape to the
    candidate universe converts silent misclassification into a loud stop but leaves
    the parser structurally wrong. Three scraped keys are legal session names --
    STATUS, pending, replied -- so a durable candidate literally named `pending`
    lets an unrelated requests-table row PASS the universe filter, where it either
    shadows the real listing row or aborts a valid corpus. A filter cannot separate
    them, because both are keys by then; only the section they came from can.

    The real session must WIN and the unrelated row must be INERT. The old parser is
    run on the same bytes to prove the control discriminates: a control that only
    the fixed code can pass has not been shown to fail.
    """
    import importlib.util as iu

    def load(name, path):
        spec = iu.spec_from_file_location(name, os.path.join(HERE, path))
        m = iu.module_from_spec(spec)
        spec.loader.exec_module(m)
        return m

    def old_scrape(text):
        shown = {}
        for m in re.finditer(r"^(\S+)\s+(\S+)", text, re.M):
            if m.group(1) not in ("SESSION", "No"):
                shown[m.group(1)] = m.group(2)
        return shown

    g = load("g67", "obligations.py")
    v = load("v67", "verify-obligations.py")
    gen = g.candidate_shown(_SHADOW_BODY)

    v.durable_candidates = lambda case: ["pending"]
    ver = v.candidate_representation("synthetic", "list", _SHADOW_BODY)

    old = old_scrape(_SHADOW_BODY)
    gen_ok = gen == {"pending": "stopped"}
    ver_ok = ver == {"pending": "aligned:stopped"}
    old_wrong = old.get("pending") != "stopped"
    ok = gen_ok and ver_ok and old_wrong
    print("shadowed-name  generator keeps the real session row: %s; verifier classes "
          "it aligned:stopped: %s; old whole-body scrape gets it WRONG (%r): %s  %s"
          % (gen_ok, ver_ok, old.get("pending"), old_wrong,
             "caught" if ok else "MISSED"))
    return 0 if ok else 1


def off_status_control():
    """SEED 68 — an unanalysed status is fail-closed on BOTH sides, differently.

    The third branch used to be a bare `else`: anything not `running` became
    `aligned`. Every one of the 230 aligned rows in this corpus is literally
    `stopped`, so the branch has never been exercised by a second value and the class
    population is unbound. Kept separate from seed 67 deliberately -- one seed
    proving two properties cannot say which one failed.
    """
    import importlib.util as iu

    body = _SHADOW_BODY.replace("pending                   stopped",
                                "pending                   exited ")
    assert "exited" in body and "stopped" not in body.split("\n")[1]

    def load(name, path):
        spec = iu.spec_from_file_location(name, os.path.join(HERE, path))
        m = iu.module_from_spec(spec)
        spec.loader.exec_module(m)
        return m

    g = load("g68", "obligations.py")
    g.loss_candidates = lambda case: ["pending"]
    try:
        g.candidate_class("synthetic", body)
        gen_ok = False
    except AssertionError as exc:
        gen_ok = "exited" in str(exc)

    v = load("v68", "verify-obligations.py")
    v.durable_candidates = lambda case: ["pending"]
    ver = v.candidate_representation("synthetic", "list", body)
    ver_ok = ver == {"pending": "unanalysed-status:exited"}

    ok = gen_ok and ver_ok
    print("off-universe-status  generator asserts and names the value: %s; verifier "
          "reports it as unanalysed rather than aligned: %s  %s"
          % (gen_ok, ver_ok, "caught" if ok else "MISSED"))
    return 0 if ok else 1

def grain_control():
    """SEED 69 — the m grain, and RED ON BOTH ITS FAILURE MODES.

    Ownership at VIEW grain, carriage at (view, candidate identity) -- colead's
    refinement, which is what satisfies the pinned same-identity pairing without
    making status value m's subject. This corpus cannot check it: both readings emit
    exactly 240 m rows, because every m-owing view here omits exactly ONE candidate
    (identities-per-m-owing-view is {1: 240}). Two readings, one number, agreement
    manufactured by the corpus -- after by-agreement and by-framing.

    So the fixture is the check, and asserting today's output is not enough: a
    predicate is only shown to work when it goes RED on the ways it can fail. Both
    ruled modes are constructed and required to fail:
      COLLAPSE   -- one view-level opaque tuple in place of the identity rows
      CONSUMPTION -- one candidate's contribution swallowing the other's
    """
    import importlib.util as iu

    spec = iu.spec_from_file_location("g69", os.path.join(HERE, "obligations.py"))
    g = iu.module_from_spec(spec)
    spec.loader.exec_module(g)
    g.loss_candidates = lambda case: ["gone1", "gone2", "present1"]
    g.candidate_causes = lambda case, consumer: {
        candidate: ("selector-server-unattempted",)
        for candidate in ("gone1", "gone2", "present1")
    }

    human = ("SESSION                   STATUS    MODE        ORIGIN\n"
             "present1                  stopped   local       /tmp/x\n")
    digest = ('{"schema_version": 1, "sessions": '
              '[{"name": "present1", "status": "stopped"}]}')

    def check(rows, lpat, mpat):
        """TWO DISTINCT m identity contributions, TWO same-identity l partners, and
        the aligned candidate owing l alone."""
        mloc = [r[4] for r in rows if r[2] == "SC-017m"]
        lloc = {r[4] for r in rows if r[2] == "SC-017l"}
        if len(mloc) != 2 or len(set(mloc)) != 2:
            return False
        if not all((mpat % c) in mloc and (lpat % c) in lloc for c in ("gone1", "gone2")):
            return False
        return (lpat % "present1") in lloc and (mpat % "present1") not in mloc

    results = []
    for surface, text, lpat, mpat in (
            ("human", human, "candidate[%s].status", "view.members[%s]"),
            ("digest", digest, "sessions[%s].status", "sessions[%s]")):
        rows = g.emit_unknown_family("synthetic", "list", "ae list", text, "ae list")
        live = check(rows, lpat, mpat)

        # MODE 1: collapse to one view-level opaque tuple.
        keep = [r for r in rows if r[2] != "SC-017m"]
        collapsed = keep + [tuple(list(rows[0][:4]) + ["view.members",
                                                       "(set)", "(set)"] + list(rows[0][7:]))]
        # MODE 2: one candidate consumes the other -- a single m row for both.
        consumed = [r for r in rows if not (r[2] == "SC-017m" and "gone2" in r[4])]

        results.append((surface, live and not check(collapsed, lpat, mpat)
                        and not check(consumed, lpat, mpat)))

    ok = all(good for _, good in results)
    print("m-grain        %s  (red on collapse-to-view-tuple AND on one candidate "
          "consuming the other)  %s"
          % ("; ".join("%s: two identity contributions with same-identity l "
                       "partners: %s" % (s, good) for s, good in results),
             "caught" if ok else "MISSED"))
    return 0 if ok else 1


def surface_population_control():
    """SEED 70 — the l/m population is the LISTING surfaces, and nothing else.

    Unguarded, the emission fired on helper:requests and helper:events-tail: 90 pairs
    on documents that render NO session-selection view, where every durable candidate
    is trivially absent and the omission branch answers about a view that does not
    exist. The rows were well-formed, correctly paired, correctly signed -- wrong by
    AGREEMENT, with nothing in the output to disagree with.

    Required both ways: a non-listing surface owes NOTHING, and the same bytes on a
    listing surface still owe their rows -- so the guard cannot be satisfied by a
    predicate that simply emits less.
    """
    import importlib.util as iu

    spec = iu.spec_from_file_location("g70", os.path.join(HERE, "obligations.py"))
    g = iu.module_from_spec(spec)
    spec.loader.exec_module(g)
    g.loss_candidates = lambda case: ["gone1"]
    g.candidate_causes = lambda case, consumer: {
        "gone1": ("selector-server-unattempted",),
    }
    text = ("SESSION                   STATUS    MODE        ORIGIN\n"
            "other                     stopped   local       /tmp/x\n")

    silent = {s: len(g.emit_unknown_family("synthetic", "c", "ae list", text, s))
              for s in ("helper:requests", "helper:events-tail")}
    speaks = {s: len(g.emit_unknown_family("synthetic", "c", "ae list", text, s))
              for s in g.LISTING}
    # THIRD LEG: a surface outside the closed vocabulary must produce a NAMED
    # refusal, not silence. A filter that quietly drops an unclassified surface is
    # how the 90 spurious pairs would have come back as 90 missing ones.
    try:
        g.emit_unknown_family("synthetic", "c", "ae list", text, "ae summary")
        refused = False
    except AssertionError as exc:
        refused = "ae summary" in str(exc)

    ok = (all(n == 0 for n in silent.values()) and all(n == 2 for n in speaks.values())
          and refused)
    print("surface-population  non-listing owe nothing %s; listing still owe their "
          "pair %s; an unclassified surface is REFUSED by name: %s  %s"
          % (silent, speaks, refused, "caught" if ok else "MISSED"))
    return 0 if ok else 1


def human_agent_printer_shape_control():
    """SEED 71 — the frozen printers have two distinct agent-row schemas.

    These are valid byte shapes copied from the frozen running printer and c13's
    stopped capture.  A stopped row carries membership plus session_id and has NO
    state or health cell.  A running row carries session_id, state and a trailing
    health cell; that final cell may be empty.  Reading the last non-space token
    therefore turns stopped session_id and running state into fake health values.

    Both derivations must preserve the roster while reading only the real health
    cell.  The generator and verifier use different traversals, so agreement alone
    is not enough: each is checked against the literal expected shapes first.
    """
    import importlib.util as iu

    def load(name, path):
        spec = iu.spec_from_file_location(name, os.path.join(HERE, path))
        m = iu.module_from_spec(spec)
        spec.loader.exec_module(m)
        return m

    g = load("g71", "obligations.py")
    v = load("v71", "verify-obligations.py")
    g.candidate_causes = lambda case, consumer: {
        "tg10": ("selector-server-unattempted",),
        "tg1": ("selector-server-unattempted",),
    }
    v.gate_candidate_causes = lambda case, consumer: {
        "tg10": ("selector-server-unattempted",),
        "tg1": ("selector-server-unattempted",),
    }

    stopped = (
        "SESSION                   STATUS    MODE        ORIGIN\n"
        "tg10                      stopped   local       /tmp/aecx/tpl/g10/work\n"
        "  git:master · ae 0.2.1 · active 25m ago\n"
        "  fake:lead               -\n"
        "  fake:worker             -\n"
        "  fake:lead               -\n"
    )
    running = (
        "SESSION                   STATUS    MODE        ORIGIN\n"
        "tg1                       running   local       /tmp/aecx/tpl/g1/work\n"
        "  fake:lead               -         working       \n"
        "  fake:worker             -         done          !\n"
    )

    def gen(text):
        return {(x[4], x[5], x[6])
                for x in g.emit_agent_health("s", "list", text, "ae list")}

    def ver(text):
        return {(x[2], x[3], x[4])
                for x in v.owed_agent_health_v("s", "list", text, "ae list")}

    expected_stopped = {
        ("agents[tg10:fake:lead:-](class).health", "ABSENT x2",
         "unambiguous unknown x2"),
        ("agents[tg10:fake:worker:-].health", "ABSENT", "unambiguous unknown"),
    }
    expected_running = {
        ("agents[tg1:fake:lead:-].health", "blank", "unambiguous unknown"),
        ("agents[tg1:fake:worker:-].health", "!", "unambiguous unknown"),
    }
    gen_stopped_fields = g.parse_human_agent_row("  fake:lead               -", "stopped")
    gen_running_fields = g.parse_human_agent_row(
        "  fake:lead               -         working       ", "running")
    ver_stopped_fields = v.gate_human_agent_row("  fake:lead               -", "stopped")
    ver_running_fields = v.gate_human_agent_row(
        "  fake:lead               -         working       ", "running")
    stopped_outputs = (gen(stopped), ver(stopped))
    running_outputs = (gen(running), ver(running))
    results = {
        "generator-stopped": stopped_outputs[0] == expected_stopped,
        "generator-running": running_outputs[0] == expected_running,
        "verifier-stopped": stopped_outputs[1] == expected_stopped,
        "verifier-running": running_outputs[1] == expected_running,
        "generator-state-columns": gen_stopped_fields == ("fake:lead", "-", None, None)
        and gen_running_fields == ("fake:lead", "-", "working", ""),
        "verifier-state-columns": ver_stopped_fields == ("fake:lead", "-", None, None)
        and ver_running_fields == ("fake:lead", "-", "working", ""),
        "stopped-ABSENT-cannot-consume-blank": all(
            {row[1] for row in output} == {"ABSENT", "ABSENT x2"}
            for output in stopped_outputs
        ),
        "running-blank-cannot-consume-ABSENT": all(
            ("agents[tg1:fake:lead:-].health", "blank", "unambiguous unknown") in output
            and not any(row[1] == "ABSENT" for row in output)
            for output in running_outputs
        ),
        "running-bang-cannot-consume-blank": all(
            ("agents[tg1:fake:worker:-].health", "!", "unambiguous unknown") in output
            for output in running_outputs
        ),
    }
    # These literal source-value assertions prevent either empty running health
    # or stopped no-cell from consuming the other's semantic carrier.
    ok = all(results.values())
    print("human-agent-shape  %s  %s"
          % ("; ".join("%s=%s" % x for x in sorted(results.items())),
             "caught" if ok else "<-- MISSED"))
    return 0 if ok else 1


def r_grain_control():
    """SEED 72 — SC-017r's ruled grain, all four consequences, on values that DIFFER.

    c13's stopped rows render no health cell, so a swap there is literally identical
    health bytes and proves nothing. The fixture gives them DIFFERENT health values, which is
    the case the row rules on directly: two agents under one display name may carry
    different health and remain UNBOUND, because nothing in the human bytes associates
    either value with either roster slot.

    Required, from the row: DROP fails, WRONG MULTIPLICITY fails, EXCHANGE is NOT
    OBSERVED and therefore neutral, and neither a LIST nor a SET is permissible --
    both make their totals agree with something.
    """
    import importlib.util as iu

    def load(name, path):
        spec = iu.spec_from_file_location(name, os.path.join(HERE, path))
        m = iu.module_from_spec(spec)
        spec.loader.exec_module(m)
        return m

    g = load("g71", "obligations.py")
    v = load("v71", "verify-obligations.py")
    g.candidate_causes = lambda case, consumer: {
        "tgX": ("selector-server-unattempted",),
    }
    v.gate_candidate_causes = lambda case, consumer: {
        "tgX": ("selector-server-unattempted",),
    }

    head = ("SESSION                   STATUS    MODE        ORIGIN\n"
            "tgX                       running   local       /tmp/x\n")
    def body(rows):
        return head + "".join("  %-22s  %-8s  %-12s  %s\n" % row for row in rows)

    base = [("fake:lead", "11111111", "working", ""),
            ("fake:worker", "22222222", "done", ""),
            ("fake:lead", "11111111", "working", "!")]
    swapped = [("fake:lead", "11111111", "working", "!"),
               ("fake:worker", "22222222", "done", ""),
               ("fake:lead", "11111111", "working", "")]
    dropped = [("fake:lead", "11111111", "working", ""),
               ("fake:worker", "22222222", "done", "")]
    tripled = base + [("fake:lead", "11111111", "working", "")]

    def owed(rows, mod, fn):
        return set(fn(mod, body(rows)))

    def gen(text):
        return {(x[4], x[5], x[6]) for x in g.emit_agent_health("s", "list", text, "ae list")}

    def ver(text):
        return {(x[2], x[3], x[4]) for x in v.owed_agent_health_v("s", "list", text, "ae list")}

    results = {}
    for label, fn in (("generator", gen), ("verifier", ver)):
        b, sw, dr, tr = fn(body(base)), fn(body(swapped)), fn(body(dropped)), fn(body(tripled))
        results[label] = (
            b == sw,          # EXCHANGE neutral -- byte-identical owed set
            b != dr,          # DROP red
            b != tr,          # WRONG MULTIPLICITY red
            # MY FIRST PREDICATE HERE WAS WRONG, not the code: it looked for "x2"
            # in the from-value, but this class holds blank and `!`, so the
            # multiset renders "! x1 blank x1" and the x2 lives in the TO side's
            # cardinality. A control that asserts the wrong shape reports MISSED
            # for correct behaviour -- the direction that would have sent me to
            # "fix" a working derivation.
            any("(class)" in loc and to.endswith("x2")
                and frm == "! x1 blank x1" for loc, frm, to in b),
        )
    ok = all(all(t) for t in results.values()) and gen(body(base)) and \
        {(l, f, t) for l, f, t in gen(body(base))} == ver(body(base))
    print("r-grain        %s; both derivations agree on the base: %s  %s"
          % ("; ".join("%s swap-neutral=%s drop-red=%s multiplicity-red=%s "
                       "multiset-shaped=%s" % ((k,) + tuple(map(str, t)))
                       for k, t in sorted(results.items())),
             gen(body(base)) == ver(body(base)), "caught" if ok else "MISSED"))
    return 0 if ok else 1


def sid_identity_control():
    """SEED 73 — retained rendered short sid is part of human agent identity.

    Frozen `_parse_agent_entry` emits an eight-byte short sid, or dash for
    absent/empty/pending.  These two rows are valid running-printer shapes with the
    same rendered ref and different retained sids.  They must remain two bindable
    identities even though their health values differ; collapsing by ref alone
    manufactures a false class and lets one sid consume the other's value owner.
    """
    import importlib.util as iu

    def load(name, path):
        spec = iu.spec_from_file_location(name, os.path.join(HERE, path))
        m = iu.module_from_spec(spec)
        spec.loader.exec_module(m)
        return m

    g = load("g73", "obligations.py")
    v = load("v73", "verify-obligations.py")
    g.candidate_causes = lambda case, consumer: {
        "tgX": ("selector-server-unattempted",),
    }
    v.gate_candidate_causes = lambda case, consumer: {
        "tgX": ("selector-server-unattempted",),
    }
    text = (
        "SESSION                   STATUS    MODE        ORIGIN\n"
        "tgX                       running   local       /tmp/x\n"
        "  fake:lead               11111111  working       \n"
        "  fake:lead               22222222  working       !\n"
    )
    expected = {
        ("agents[tgX:fake:lead:11111111].health", "blank", "unambiguous unknown"),
        ("agents[tgX:fake:lead:22222222].health", "!", "unambiguous unknown"),
    }
    generated = {(x[4], x[5], x[6])
                 for x in g.emit_agent_health("s", "list", text, "ae list")}
    verified = {(x[2], x[3], x[4])
                for x in v.owed_agent_health_v("s", "list", text, "ae list")}
    results = {
        "generator-distinct-short-sid": generated == expected,
        "verifier-distinct-short-sid": verified == expected,
        "different-sid-cannot-form-class": all("(class)" not in x[0]
                                                 for x in generated | verified),
        "different-sid-cannot-consume-owner": generated == verified == expected,
    }
    ok = all(results.values())
    print("sid-identity    %s  %s"
          % ("; ".join("%s=%s" % x for x in sorted(results.items())),
             "caught" if ok else "<-- MISSED"))
    return 0 if ok else 1


def state_carrier_control():
    """SEED 77 — SC-017h uses the same fixed identity and printer grammar as r.

    Stopped rows have membership plus short sid but no state cell, so their source
    state is ABSENT. Running rows retain the emitted state even when health is empty.
    The duplicated stopped identity proves exact class multiplicity without inventing
    an occurrence order.
    """
    import importlib.util as iu

    def load(name, path):
        spec = iu.spec_from_file_location(name, os.path.join(HERE, path))
        mod = iu.module_from_spec(spec)
        spec.loader.exec_module(mod)
        return mod

    g = load("g74h", "obligations.py")
    v = load("v74h", "verify-obligations.py")
    targets = {("tgS", "fake:lead"): "working",
               ("tgS", "fake:worker"): "-",
               ("tgR", "fake:lead"): "unknown"}
    g.declared_state_for = lambda case, session, agent: (
        targets[(session, agent)], ("fixed-seed-ledger",))
    v.gate_state_target = lambda case, session, agent: targets[(session, agent)]
    text = (
        "SESSION                   STATUS    MODE        ORIGIN\n"
        "tgS                       stopped   local       /tmp/s\n"
        "  fake:lead               -\n"
        "  fake:worker             -\n"
        "  fake:lead               -\n"
        "tgR                       running   local       /tmp/r\n"
        "  fake:lead               11111111  working       \n"
    )
    expected = {
        ("agents[tgS:fake:lead:-](class).state", "ABSENT x2", "working x2"),
        ("agents[tgS:fake:worker:-].state", "ABSENT", "-"),
        ("agents[tgR:fake:lead:11111111].state", "working", "unknown"),
    }
    generated = {(row[4], row[5], row[6])
                 for row in g.emit_agent_state("s", "list", text, "ae list")}
    verified = {(row[2], row[3], row[4])
                for row in v.owed_agent_state_v("s", "list", text, "ae list")}
    results = {
        "generator-exact-state-carriers": generated == expected,
        "verifier-exact-state-carriers": verified == expected,
        "stopped-sid-not-state": all(frm.startswith("ABSENT")
                                      for locus, frm, _to in generated
                                      if locus.startswith("agents[tgS:")),
        "collision-multiplicity": any("(class).state" in locus and
                                      frm == "ABSENT x2" and to == "working x2"
                                      for locus, frm, to in generated | verified),
    }
    ok = all(results.values())
    print("state-carrier    %s  %s" % (
        "; ".join("%s=%s" % item for item in sorted(results.items())),
        "caught" if ok else "<-- MISSED"))
    return 0 if ok else 1


def sole_session_six_field_control():
    """SEED 78 — A4's real sole-session pane shape is not an empty enumeration.

    The fixed rows are `%pane|ref|slot|command|pid|geometry`; session is supplied by
    the snapshot's singleton `## sessions` section and pane_dead is genuinely absent.
    Both roster refs therefore match exact panes, but SC-017s cannot prove them alive:
    the result is q/unknown, never p/dead from a parser-created empty pane map.
    """
    import importlib.util as iu

    def load(name, path):
        spec = iu.spec_from_file_location(name, os.path.join(HERE, path))
        mod = iu.module_from_spec(spec)
        spec.loader.exec_module(mod)
        return mod

    case, consumer = "arms/A4/c01-status-live-live", "list"
    g, v = load("g78", "obligations.py"), load("v78", "verify-obligations.py")
    text = g.body(case, consumer)
    expected = {
        ("agents[ta4c01statuslive:fake:lead:-].health", "blank",
         "unambiguous unknown"),
        ("agents[ta4c01statuslive:fake:worker:-].health", "blank",
         "unambiguous unknown"),
    }
    generated = {(row[4], row[5], row[6])
                 for row in g.emit_agent_health(case, consumer, text, "ae list")}
    verified = {(row[2], row[3], row[4])
                for row in v.owed_agent_health_v(case, consumer, text, "ae list")}
    results = {
        "generator-six-field-pane-not-empty": generated == expected,
        "verifier-six-field-pane-not-empty": verified == expected,
        "six-field-no-false-dead": all(to != "dead"
                                        for _locus, _frm, to in generated | verified),
    }
    ok = all(results.values())
    print("six-field-pane  %s  %s" % (
        "; ".join("%s=%s" % item for item in sorted(results.items())),
        "caught" if ok else "<-- MISSED"))
    return 0 if ok else 1


def _source_case(root, recorded, query, panes, rows, rosters=None):
    """Build one fixed-source mixed-selector case for Seeds 74-76."""
    from pathlib import Path
    base = Path(root) / "arms" / "S" / "mixed"
    tpl = Path(root) / "templates" / "S" / "fixture-bytes" / "mixed" / "sessions"
    (base / "out").mkdir(parents=True)
    manifest = []
    for session, socket in recorded.items():
        meta = tpl / session / "meta"
        meta.parent.mkdir(parents=True)
        roster = ((rosters or {}).get(session)
                  or [("main", "fake:lead", "pending"),
                      ("worker.0", "fake:worker", "pending")])
        meta.write_text(
            "mode=local\ntmux_server=%s\ntmux_server_kind=socket\n%s"
            % (socket, "".join("agent.%s=%s:%s\n" % item for item in roster)),
            encoding="utf-8",
        )
        manifest.extend((
            "dir\t755\t-\t-\t./sessions/%s\n" % session,
            "file\t644\thash\t-\t./sessions/%s/meta\n" % session,
        ))
    (base / "manifest.before.tsv").write_text("".join(manifest), encoding="utf-8")
    (base / "case.txt").write_text("template=S/mixed\n", encoding="utf-8")
    (base / "out" / "list.tmuxtrace").write_text(
        "AE_TMUX_SERVER=%s\targv=-S %s list-sessions -F format\n"
        "AE_TMUX_SERVER=%s\targv=-S %s list-panes -s -t exact -F format\n"
        % (query, query, query, query),
        encoding="utf-8",
    )
    sessions = sorted({row[0] for row in rows})
    (base / "tmux.before.txt").write_text(
        # c20's frozen live snapshot grammar carries pane id, ref, slot, command,
        # pid, pane_dead, session.  Seed 75 needs the pane_dead=0 fact to prove the
        # surviving lead alive while independently proving the missing worker dead.
        "## panes\n" + "".join("%%%d|%s|%s|aefake|123|0|%s\n"
                                  % (n, ref, slot, session)
                                  for n, (session, ref, slot) in enumerate(panes))
        + "## sessions\n" + "".join("%s|1\n" % s for s in sessions),
        encoding="utf-8",
    )
    body = "SESSION                   STATUS    MODE        ORIGIN\n"
    for session in sessions:
        body += "%-26s%-10s%-12s%s\n" % (session, "running", "local", "/tmp/x")
        for row_session, ref, sid, state, health in rows:
            if row_session == session:
                body += "  %-22s  %-8s  %-12s  %s\n" % (ref, sid, state, health)
    (base / "out" / "list.stdout").write_text(body, encoding="utf-8")
    return "arms/S/mixed", body


def candidate_cause_grain_control():
    """SEED 74 — causes are per candidate, and attempt precedes outcome.

    Two durable candidates share one frozen query. Only one recorded selector equals
    that queried socket. The other is NEVER ATTEMPTED; it is not a failed query and
    cannot paint the matching candidate. Mutating only its recorded socket to equal
    the queried socket must remove only its unknown family.
    """
    import importlib.util as iu

    def load(name, path):
        spec = iu.spec_from_file_location(name, os.path.join(HERE, path))
        mod = iu.module_from_spec(spec)
        spec.loader.exec_module(mod)
        return mod

    results = {}
    with tempfile.TemporaryDirectory(prefix="rp-candidate-grain-") as td:
        case, text = _source_case(
            td,
            {"match": "/sock/q", "miss": "/sock/other"},
            "/sock/q",
            [("match", "fake:lead", "main"),
             ("miss", "fake:lead", "main")],
            [("match", "fake:lead", "-", "working", ""),
             ("miss", "fake:lead", "-", "working", "")],
        )
        for label, path, fn_name in (
            ("generator", "obligations.py", "candidate_causes"),
            ("verifier", "verify-obligations.py", "gate_candidate_causes"),
        ):
            mod = load("s74" + label, path)
            mod.SRC = td
            fn = getattr(mod, fn_name, None)
            causes = fn(case, "list") if fn else {}
            results[label + "-matching-clean"] = not causes.get("match")
            results[label + "-mismatch-unattempted"] = (
                causes.get("miss") == ("selector-server-unattempted",)
            )
            results[label + "-never-attempted-not-failed"] = all(
                "failed" not in c and "unreachable" not in c
                for c in causes.get("miss", ())
            )
        # Change only the distinguishing source fact: recorded == queried.
        meta = (os.path.join(td, "templates", "S", "fixture-bytes", "mixed",
                             "sessions", "miss", "meta"))
        source = open(meta, encoding="utf-8").read()
        open(meta, "w", encoding="utf-8").write(
            source.replace("tmux_server=/sock/other", "tmux_server=/sock/q")
        )
        for label, path, fn_name in (
            ("generator", "obligations.py", "candidate_causes"),
            ("verifier", "verify-obligations.py", "gate_candidate_causes"),
        ):
            mod = load("s74equal" + label, path)
            mod.SRC = td
            fn = getattr(mod, fn_name, None)
            causes = fn(case, "list") if fn else {"miss": ("missing-derivation",)}
            results[label + "-socket-equality-clears"] = not causes.get("miss")
    ok = all(results.values())
    print("candidate-grain  %s  %s" % (
        "; ".join("%s=%s" % x for x in sorted(results.items())),
        "caught" if ok else "<-- MISSED"))
    return 0 if ok else 1


def p_dead_leg_control():
    """SEED 75 — exact recorded-server pane absence is SC-017p dead."""
    import importlib.util as iu

    def load(name, path):
        spec = iu.spec_from_file_location(name, os.path.join(HERE, path))
        mod = iu.module_from_spec(spec)
        spec.loader.exec_module(mod)
        return mod

    results = {}
    with tempfile.TemporaryDirectory(prefix="rp-p-dead-") as td:
        case, text = _source_case(
            td, {"dead": "/sock/q"}, "/sock/q",
            [("dead", "fake:lead", "main")],
            [("dead", "fake:lead", "-", "working", ""),
             ("dead", "fake:worker", "-", "working", "")],
        )
        for label, path, fn_name in (
            ("generator", "obligations.py", "emit_agent_health"),
            ("verifier", "verify-obligations.py", "owed_agent_health_v"),
        ):
            mod = load("s75" + label, path)
            mod.SRC = td
            rows = getattr(mod, fn_name)(case, "list", text, "ae list")
            shaped = {(x[4], x[5], x[6]) for x in rows} if label == "generator" \
                else {(x[2], x[3], x[4]) for x in rows}
            worker = [row for row in shaped if "fake:worker" in row[0]]
            results[label + "-missing-exact-pane-dead"] = worker == [
                ("agents[dead:fake:worker:-].health", "blank", "dead")]
    ok = all(results.values())
    print("p-dead-leg      %s  %s" % (
        "; ".join("%s=%s" % x for x in sorted(results.items())),
        "caught" if ok else "<-- MISSED"))
    return 0 if ok else 1


def alive_presentation_leg_control():
    """SEED 82 — frozen blank/alive still diverges from literal successor alive.

    The exact recorded server, session and pane all succeed, and pane_dead=0 plus
    the non-shell command proves semantic alive. The source-shaped frozen running
    row carries an EMPTY health cell. Both derivations must retain blank -> alive as
    a presentation obligation instead of normalizing it away as semantic equality.
    """
    import importlib.util as iu

    def load(name, path):
        spec = iu.spec_from_file_location(name, os.path.join(HERE, path))
        mod = iu.module_from_spec(spec)
        spec.loader.exec_module(mod)
        return mod

    results = {}
    with tempfile.TemporaryDirectory(prefix="rp-alive-presentation-") as td:
        case, text = _source_case(
            td, {"alive": "/sock/q"}, "/sock/q",
            [("alive", "fake:lead", "main")],
            [("alive", "fake:lead", "-", "working", "")],
        )
        expected = {("agents[alive:fake:lead:-].health", "blank", "alive")}
        for label, path, fn_name in (
            ("generator", "obligations.py", "emit_agent_health"),
            ("verifier", "verify-obligations.py", "owed_agent_health_v"),
        ):
            mod = load("s82" + label, path)
            mod.SRC = td
            rows = getattr(mod, fn_name)(case, "list", text, "ae list")
            shaped = {(x[4], x[5], x[6]) for x in rows} if label == "generator" \
                else {(x[2], x[3], x[4]) for x in rows}
            results[label + "-blank-to-literal-alive"] = shaped == expected
    ok = all(results.values())
    print("alive-leg       %s  %s" % (
        "; ".join("%s=%s" % x for x in sorted(results.items())),
        "caught" if ok else "<-- MISSED"))
    return 0 if ok else 1


def slot_display_swap_control():
    """SEED 83 — pane display may swap; health follows stable ``@ae_slot``.

    Baseline and mutant keep two roster entries, two panes, two human rows and two
    obligations.  Only pane ``@ae_agent`` display values exchange.  Main stays dead
    and worker.0 stays unambiguously unknown, so a display-ref matcher swaps the
    answers and fails. These targets deliberately avoid Seed 82's blank/alive rule,
    keeping this discriminator specific to slot association.
    """
    import importlib.util as iu

    def load(name, path):
        spec = iu.spec_from_file_location(name, os.path.join(HERE, path))
        mod = iu.module_from_spec(spec)
        spec.loader.exec_module(mod)
        return mod

    expected = {
        ("agents[swap:fake:lead:-].health", "blank", "dead"),
        ("agents[swap:fake:worker:-].health", "blank",
         "unambiguous unknown"),
    }
    results = {}
    with tempfile.TemporaryDirectory(prefix="rp-slot-display-") as td:
        case, text = _source_case(
            td, {"swap": "/sock/q"}, "/sock/q", [],
            [("swap", "fake:lead", "-", "working", ""),
             ("swap", "fake:worker", "-", "working", "")],
        )
        topology = os.path.join(td, "arms", "S", "mixed", "tmux.before.txt")
        snapshots = {
            "baseline": (
                "## panes\n"
                "%0|fake:lead|main|aefake|123|1|swap\n"
                "%1|fake:worker|worker.0|bash|124|0|swap\n"
                "## sessions\nswap|1\n"),
            "display-swap": (
                "## panes\n"
                "%0|fake:worker|main|aefake|123|1|swap\n"
                "%1|fake:lead|worker.0|bash|124|0|swap\n"
                "## sessions\nswap|1\n"),
        }
        for label, path, fn_name in (
            ("generator", "obligations.py", "emit_agent_health"),
            ("verifier", "verify-obligations.py", "owed_agent_health_v"),
        ):
            mod = load("s83" + label, path)
            mod.SRC = td
            shaped = {}
            for variant, snapshot in snapshots.items():
                open(topology, "w", encoding="utf-8").write(snapshot)
                rows = getattr(mod, fn_name)(case, "list", text, "ae list")
                shaped[variant] = ({(x[4], x[5], x[6]) for x in rows}
                                   if label == "generator"
                                   else {(x[2], x[3], x[4]) for x in rows})
            results[label + "-baseline-follows-slot"] = shaped["baseline"] == expected
            results[label + "-display-swap-neutral"] = shaped["display-swap"] == expected
            results[label + "-equal-obligation-count"] = (
                len(shaped["baseline"]) == len(shaped["display-swap"]) == 2)
        results["source-pane-count-equal"] = all(
            snapshot.count("\n%") == 2 for snapshot in snapshots.values())
    ok = all(results.values())
    print("slot-display    %s  %s" % (
        "; ".join("%s=%s" % item for item in sorted(results.items())),
        "caught" if ok else "<-- MISSED"))
    return 0 if ok else 1


def heterogeneous_slot_class_control():
    """SEED 84 — one human ref+SID class retains per-slot target multiplicity.

    Slots main and worker.0 deliberately collide on the same human identity. Main is
    provably alive; worker.0 has a shell command and is unknown. The class target is
    the order-free mixed multiset, never one display-derived value repeated twice.
    """
    import importlib.util as iu

    def load(name, path):
        spec = iu.spec_from_file_location(name, os.path.join(HERE, path))
        mod = iu.module_from_spec(spec)
        spec.loader.exec_module(mod)
        return mod

    expected = {
        ("agents[collision:fake:twin:-](class).health", "blank x2",
         "alive x1 unambiguous unknown x1")
    }
    results = {}
    with tempfile.TemporaryDirectory(prefix="rp-slot-class-") as td:
        case, text = _source_case(
            td, {"collision": "/sock/q"}, "/sock/q", [],
            [("collision", "fake:twin", "-", "working", ""),
             ("collision", "fake:twin", "-", "working", "")],
            rosters={"collision": [
                ("main", "fake:twin", "pending"),
                ("worker.0", "fake:twin", "pending"),
            ]},
        )
        topology = os.path.join(td, "arms", "S", "mixed", "tmux.before.txt")
        open(topology, "w", encoding="utf-8").write(
            "## panes\n"
            "%0|fake:twin|main|aefake|123|0|collision\n"
            "%1|fake:twin|worker.0|bash|124|0|collision\n"
            "## sessions\ncollision|1\n")
        for label, path, fn_name in (
            ("generator", "obligations.py", "emit_agent_health"),
            ("verifier", "verify-obligations.py", "owed_agent_health_v"),
        ):
            mod = load("s84" + label, path)
            mod.SRC = td
            rows = getattr(mod, fn_name)(case, "list", text, "ae list")
            shaped = ({(x[4], x[5], x[6]) for x in rows}
                      if label == "generator"
                      else {(x[2], x[3], x[4]) for x in rows})
            results[label + "-mixed-target-multiset"] = shaped == expected
    ok = all(results.values())
    print("slot-class      %s  %s" % (
        "; ".join("%s=%s" % item for item in sorted(results.items())),
        "caught" if ok else "<-- MISSED"))
    return 0 if ok else 1


def missing_slot_spoof_control():
    """SEED 85 — a display names worker under main; worker.0 remains absent/dead."""
    import importlib.util as iu

    def load(name, path):
        spec = iu.spec_from_file_location(name, os.path.join(HERE, path))
        mod = iu.module_from_spec(spec)
        spec.loader.exec_module(mod)
        return mod

    expected = {
        ("agents[spoof:fake:lead:-].health", "blank", "alive"),
        ("agents[spoof:fake:worker:-].health", "blank", "dead"),
    }
    results = {}
    with tempfile.TemporaryDirectory(prefix="rp-slot-spoof-") as td:
        case, text = _source_case(
            td, {"spoof": "/sock/q"}, "/sock/q", [],
            [("spoof", "fake:lead", "-", "working", ""),
             ("spoof", "fake:worker", "-", "working", "")],
        )
        topology = os.path.join(td, "arms", "S", "mixed", "tmux.before.txt")
        open(topology, "w", encoding="utf-8").write(
            "## panes\n"
            "%0|fake:worker|main|aefake|123|0|spoof\n"
            "## sessions\nspoof|1\n")
        for label, path, fn_name in (
            ("generator", "obligations.py", "emit_agent_health"),
            ("verifier", "verify-obligations.py", "owed_agent_health_v"),
        ):
            mod = load("s85" + label, path)
            mod.SRC = td
            rows = getattr(mod, fn_name)(case, "list", text, "ae list")
            shaped = ({(x[4], x[5], x[6]) for x in rows}
                      if label == "generator"
                      else {(x[2], x[3], x[4]) for x in rows})
            results[label + "-worker-slot-stays-dead"] = shaped == expected
    ok = all(results.values())
    print("slot-spoof      %s  %s" % (
        "; ".join("%s=%s" % item for item in sorted(results.items())),
        "caught" if ok else "<-- MISSED"))
    return 0 if ok else 1


def q_unknown_leg_control():
    """SEED 76 — an unattempted recorded server is SC-017q unknown.

    The equality variant changes only the recorded socket and must remove the
    divergence. Thus the seed cannot pass by arm-name or expected-value tuning.
    """
    import importlib.util as iu

    def load(name, path):
        spec = iu.spec_from_file_location(name, os.path.join(HERE, path))
        mod = iu.module_from_spec(spec)
        spec.loader.exec_module(mod)
        return mod

    results = {}
    with tempfile.TemporaryDirectory(prefix="rp-q-unknown-") as td:
        case, text = _source_case(
            td, {"miss": "/sock/other"}, "/sock/q",
            [("miss", "fake:lead", "main")],
            [("miss", "fake:lead", "-", "working", "")],
        )
        for label, path, fn_name in (
            ("generator", "obligations.py", "emit_agent_health"),
            ("verifier", "verify-obligations.py", "owed_agent_health_v"),
        ):
            mod = load("s76" + label, path)
            mod.SRC = td
            rows = getattr(mod, fn_name)(case, "list", text, "ae list")
            shaped = {(x[4], x[5], x[6]) for x in rows} if label == "generator" \
                else {(x[2], x[3], x[4]) for x in rows}
            results[label + "-mismatch-unknown"] = shaped == {
                ("agents[miss:fake:lead:-].health", "blank", "unambiguous unknown")
            }
        meta = os.path.join(td, "templates", "S", "fixture-bytes", "mixed",
                            "sessions", "miss", "meta")
        source = open(meta, encoding="utf-8").read()
        open(meta, "w", encoding="utf-8").write(
            source.replace("tmux_server=/sock/other", "tmux_server=/sock/q")
        )
        for label, path, fn_name in (
            ("generator", "obligations.py", "emit_agent_health"),
            ("verifier", "verify-obligations.py", "owed_agent_health_v"),
        ):
            mod = load("s76equal" + label, path)
            mod.SRC = td
            rows = getattr(mod, fn_name)(case, "list", text, "ae list")
            shaped = {(x[4], x[5], x[6]) for x in rows} if label == "generator" \
                else {(x[2], x[3], x[4]) for x in rows}
            results[label + "-equality-moves-target-to-alive"] = shaped == {
                ("agents[miss:fake:lead:-].health", "blank", "alive")
            }
    ok = all(results.values())
    print("q-unknown-leg   %s  %s" % (
        "; ".join("%s=%s" % x for x in sorted(results.items())),
        "caught" if ok else "<-- MISSED"))
    return 0 if ok else 1


def artifact_tuple_control():
    """SEED 79 — generated-artifact overrides cannot cross temp and live sets.

    Each content mutation preserves parsed rows and counts. ARTIFACT-TUPLE must be
    the ONLY finding, proving exact identity rather than an incidental semantic gate.
    """
    import pathlib

    results = {}
    with tempfile.TemporaryDirectory(prefix="rp-artifact-tuple-") as td:
        root = pathlib.Path(td)
        copies = {}
        for source in (OBL, FRESH, GAP, UNPROVED):
            target = root / os.path.basename(source)
            target.write_text(open(source, encoding="utf-8").read(),
                              encoding="utf-8")
            copies[source] = str(target)
        def check():
            return run(obl=copies[OBL], fresh=copies[FRESH], gap=copies[GAP],
                       unproved=copies[UNPROVED])

        rc, ids = check()
        results["isolated-tuple-clean"] = rc == 0 and not ids

        gap_path = pathlib.Path(copies[GAP])
        gap_original = gap_path.read_text(encoding="utf-8")
        gap_mutant = gap_original.replace("# session unnoticed.\n",
                                          "# session still unnoticed.\n", 1)
        gap_path.write_text(gap_mutant, encoding="utf-8")
        rc, ids = check()
        results["equal-semantics-gap-hash-red"] = (
            gap_mutant != gap_original and rc != 0 and ids == {"ARTIFACT-TUPLE"})
        gap_path.write_text(gap_original, encoding="utf-8")

        obl_path = pathlib.Path(copies[OBL])
        obl_original = obl_path.read_text(encoding="utf-8")
        obl_mutant = obl_original.replace(":: cause=", ":: derivation-cause=", 1)
        obl_path.write_text(obl_mutant, encoding="utf-8")
        rc, ids = check()
        results["equal-semantics-obligation-hash-red"] = (
            obl_mutant != obl_original and rc != 0 and ids == {"ARTIFACT-TUPLE"})
        obl_path.write_text(obl_original, encoding="utf-8")

        unproved_path = pathlib.Path(copies[UNPROVED])
        unproved_original = unproved_path.read_text(encoding="utf-8")
        unproved_mutant = unproved_original.replace(
            "# NOT a claim of impossibility:", "# Still NOT a claim of impossibility:", 1)
        unproved_path.write_text(unproved_mutant, encoding="utf-8")
        rc, ids = check()
        results["equal-semantics-unproved-hash-red"] = (
            unproved_mutant != unproved_original and rc != 0
            and ids == {"ARTIFACT-TUPLE"})

        cmd = [sys.executable, os.path.join(HERE, "verify-obligations.py"),
               "--obl", copies[OBL]]
        partial = subprocess.run(cmd, capture_output=True, text=True)
        results["partial-tuple-refused"] = (
            partial.returncode != 0 and "ARTIFACT-TUPLE" in partial.stdout)
    ok = all(results.values())
    print("artifact-tuple  %s  %s" % (
        "; ".join("%s=%s" % item for item in sorted(results.items())),
        "caught" if ok else "<-- MISSED"))
    return 0 if ok else 1


def snapshot_read_control():
    """SEED 80 — data members are snapshotted once; manifest is opened last."""
    import builtins
    import importlib.util as iu
    import pathlib
    from unittest import mock

    spec = iu.spec_from_file_location("v80", os.path.join(HERE, "verify-obligations.py"))
    verifier = iu.module_from_spec(spec)
    spec.loader.exec_module(verifier)
    results = {}
    with tempfile.TemporaryDirectory(prefix="rp-snapshot-read-") as td:
        root = pathlib.Path(td)
        copies = {}
        for source in (OBL, GAP, UNPROVED, FRESH):
            target = root / os.path.basename(source)
            target.write_bytes(pathlib.Path(source).read_bytes())
            copies[source] = str(target)
        expected = [os.path.abspath(copies[x]) for x in (OBL, GAP, UNPROVED, FRESH)]
        seen = []
        real_open = builtins.open

        def tracked_open(path, *args, **kwargs):
            resolved = os.path.abspath(os.fspath(path))
            if resolved in expected:
                seen.append(resolved)
            return real_open(path, *args, **kwargs)

        with mock.patch("builtins.open", tracked_open):
            rc, ids = verifier.main(
                quiet=True, obl=copies[OBL], fresh=copies[FRESH], gap=copies[GAP],
                unproved=copies[UNPROVED])
        results["snapshotted-semantics-clean"] = rc == 0 and not ids
        results["each-generated-path-opened-once"] = (
            all(seen.count(path) == 1 for path in expected))
        results["data-before-manifest"] = seen == expected
    ok = all(results.values())
    print("snapshot-read   %s  %s" % (
        "; ".join("%s=%s" % item for item in sorted(results.items())),
        "caught" if ok else "<-- MISSED"))
    return 0 if ok else 1


def output_sink_control():
    """SEED 81 — registered tuple sinks exact; recognized bypass spellings absent.

    The tuple chokepoint is the CONTRACT; this AST scan is partial enforcement. It
    recognizes builtins/io open modes (including dynamic modes), os.open,
    os.rename/replace, method names write_text/write_bytes/rename, raw `.write`
    outside the chokepoint, shutil copy/move spellings and non-allowlisted subprocess
    calls. It cannot see aliases/wrappers, ctypes, a custom method under another name,
    monkeypatching, an imported helper's hidden side effect or an artifact that
    rewrites itself at runtime. `Path.open` and `Path.replace` are also blind when
    receiver type is not syntactically knowable; the guard therefore separately
    requires that this generator import neither `pathlib` nor `Path`. Those remain
    review properties; they are acceptable here because this generator imports only
    standard read/format modules and the four explicit calls below are the registered
    publication API. Never treat this scan as universal proof that Python has no
    other write mechanism.
    """
    import ast

    source = open(os.path.join(HERE, "obligations.py"), encoding="utf-8").read()
    tree = ast.parse(source)
    parents = {child: node for node in ast.walk(tree)
               for child in ast.iter_child_nodes(node)}

    def enclosing_function(node):
        while node in parents:
            node = parents[node]
            if isinstance(node, (ast.FunctionDef, ast.AsyncFunctionDef)):
                return node.name
        return "<module>"

    published = []
    recognized_bypasses = []
    registered_replaces = []
    for node in ast.walk(tree):
        if not isinstance(node, ast.Call):
            continue
        scope = enclosing_function(node)
        if isinstance(node.func, ast.Name) and node.func.id == "atomic_write_text":
            if node.args and isinstance(node.args[0], ast.Name):
                published.append((node.lineno, node.args[0].id))
            else:
                published.append((node.lineno, "<dynamic>"))
        if isinstance(node.func, ast.Name) and node.func.id == "open":
            mode_node = node.args[1] if len(node.args) > 1 else None
            for kw in node.keywords:
                if kw.arg == "mode":
                    mode_node = kw.value
            if mode_node is not None:
                mode = mode_node.value if isinstance(mode_node, ast.Constant) else None
                if not isinstance(mode, str) or any(ch in mode for ch in "wax+"):
                    recognized_bypasses.append((node.lineno, "builtins.open"))
        if isinstance(node.func, ast.Attribute):
            owner = node.func.value.id if isinstance(node.func.value, ast.Name) else None
            attr = node.func.attr
            if owner == "os" and attr == "replace":
                if scope == "atomic_write_text":
                    registered_replaces.append(node.lineno)
                else:
                    recognized_bypasses.append((node.lineno, "os.replace"))
            elif owner == "os" and attr in ("open", "rename"):
                recognized_bypasses.append((node.lineno, "os." + attr))
            elif owner == "io" and attr == "open":
                recognized_bypasses.append((node.lineno, "io.open"))
            elif owner == "shutil" and attr in (
                    "copy", "copy2", "copyfile", "copyfileobj", "move"):
                recognized_bypasses.append((node.lineno, "shutil." + attr))
            elif owner == "os" and attr == "fdopen":
                if scope != "atomic_write_text":
                    recognized_bypasses.append((node.lineno, "os.fdopen"))
            elif attr in ("write_text", "write_bytes", "rename"):
                recognized_bypasses.append((node.lineno, "." + attr))
            elif attr == "write" and scope != "atomic_write_text":
                recognized_bypasses.append((node.lineno, ".write"))
            elif owner == "subprocess" and attr in (
                    "run", "Popen", "call", "check_call", "check_output"):
                prefix = ()
                if node.args and isinstance(node.args[0], (ast.List, ast.Tuple)):
                    prefix = tuple(x.value for x in node.args[0].elts[:2]
                                   if isinstance(x, ast.Constant)
                                   and isinstance(x.value, str))
                capture = any(kw.arg == "capture_output"
                              and isinstance(kw.value, ast.Constant)
                              and kw.value.value is True for kw in node.keywords)
                allowed = (attr == "run" and capture
                           and prefix in (("git", "rev-parse"), ("git", "cat-file"))
                           and scope in ("contract_blob", "contract_anomaly_rows"))
                if not allowed:
                    recognized_bypasses.append((node.lineno, "subprocess." + attr))
    ordered = [name for _line, name in sorted(published)]
    governed = "\n".join(open(os.path.join(HERE, name), encoding="utf-8").read()
                           for name in ("obligations.py", "verify-obligations.py",
                                        "redproof-obligations.py", "FRESHNESS.tsv"))
    retired = (
        "These " + "three files are generated",
        "OBLIGATIONS, FRESHNESS and the " + "added-roster declaration",
        "after atomic OUT/" + "GAP replacement",
        "three-file " + "snapshot",
    )
    pathlib_imported = any(
        (isinstance(node, ast.Import)
         and any(alias.name == "pathlib" for alias in node.names))
        or (isinstance(node, ast.ImportFrom) and node.module == "pathlib")
        for node in ast.walk(tree))
    results = {
        "four-sinks-exact": ordered == ["UNPROVED", "OUT", "GAP", "FRESH"],
        "no-unregistered-recognized-sink": not recognized_bypasses,
        "one-registered-replace-chokepoint": len(registered_replaces) == 1,
        "fresh-published-last": bool(ordered) and ordered[-1] == "FRESH",
        "pathlib-open-replace-blind-class-absent": not pathlib_imported,
        "retired-three-member-prose-zero": not any(term in governed
                                                       for term in retired),
    }
    ok = all(results.values())
    print("output-sinks    %s paths=%s  %s" % (
        "; ".join("%s=%s" % item for item in sorted(results.items())),
        ",".join(ordered), "caught" if ok else "<-- MISSED"))
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
    bad0 += loss_arity_control()
    bad0 += plural_candidate_control()
    bad0 += class_drop_control()
    bad0 += meta_duplicate_control()
    bad0 += name_consumption_control()
    bad0 += selector_leg_control()
    bad0 += shadow_control()
    bad0 += off_status_control()
    bad0 += grain_control()
    bad0 += surface_population_control()
    bad0 += human_agent_printer_shape_control()
    bad0 += r_grain_control()
    bad0 += sid_identity_control()
    bad0 += state_carrier_control()
    bad0 += sole_session_six_field_control()
    bad0 += candidate_cause_grain_control()
    bad0 += alive_presentation_leg_control()
    bad0 += slot_display_swap_control()
    bad0 += heterogeneous_slot_class_control()
    bad0 += missing_slot_spoof_control()
    bad0 += p_dead_leg_control()
    bad0 += q_unknown_leg_control()
    bad0 += artifact_tuple_control()
    bad0 += snapshot_read_control()
    bad0 += output_sink_control()
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
