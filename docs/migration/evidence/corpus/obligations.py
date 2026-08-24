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
import csv
import json, os, re, subprocess, sys, collections

HERE = os.path.dirname(os.path.abspath(__file__))
SRC = os.path.normpath(os.path.join(HERE, "..", "batch-c-artifacts"))
INV = os.path.join(HERE, "INVOCATIONS.tsv")
OUT = os.path.join(HERE, "OBLIGATIONS.tsv")
UNPROVED = os.path.join(HERE, "SC-509C-UNPROVED.tsv")
FRESH = os.path.join(HERE, "FRESHNESS.tsv")
CONTRACT = "docs/migration/semantic-contract.md"
LISTING = ("ae list", "ae ls")

HDR = ["case", "consumer", "obligation_id", "stream", "locus", "from", "to",
       "predicate", "baseline_provenance", "support", "authority"]

# SUPPORT — whether THIS CORPUS can score the obligation, distinct from whether the
# obligation holds. Added after colead refuted the partition: the captured query
# failures are against the CASE/ambient socket (env.txt AE_TMUX_SERVER), while
# SC-017k decides liveness from the candidate's RECORDED server — a different path
# the corpus never queried. Staleness is NOT itself a liveness result; the exact
# query outcome decides, and that outcome was never captured.
#   OBSERVED   — the deciding fact is in the artifacts
#   UNSCORABLE — the obligation stands normatively; this corpus cannot score it
def missing_selector_sessions(case):
    """The session names whose selector is missing by construction."""
    mb = os.path.join(SRC, case, "manifest.before.tsv")
    if not os.path.exists(mb): return []
    rows = [l.rstrip("\n").split("\t") for l in open(mb, encoding="utf-8", errors="replace")]
    sess = {r[-1].split("/")[2] for r in rows
            if r[0] == "dir" and re.match(r"\./sessions/[^./][^/]*$", r[-1])}
    metas = {r[-1].split("/")[2]: r[1] for r in rows
             if r[0] == "file" and re.match(r"\./sessions/[^./][^/]*/meta$", r[-1])}
    out = sorted(sess - set(metas))
    out += sorted(n for n, m in metas.items() if m in ("000", "0", "100", "200"))
    return out

def selector_missing(case):
    """A candidate whose meta is absent or unreadable has a `missing` selector by
    construction (SC-405l), which routes to `unknown` WITHOUT any server outcome.
    That is the only liveness route this corpus supports on its own."""
    mb = os.path.join(SRC, case, "manifest.before.tsv")
    if not os.path.exists(mb):
        return False
    rows = [l.rstrip("\n").split("\t") for l in open(mb, encoding="utf-8", errors="replace")]
    sess = {r[-1].split("/")[2] for r in rows
            if r[0] == "dir" and re.match(r"\./sessions/[^./][^/]*$", r[-1])}
    metas = {r[-1].split("/")[2]: r[1] for r in rows
             if r[0] == "file" and re.match(r"\./sessions/[^./][^/]*/meta$", r[-1])}
    return bool(sess - set(metas)) or any(m in ("000", "0", "100", "200") for m in metas.values())

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


# SC-509c's agent-owned contribution classes, verbatim from the row: "agent-owned
# active contributions (dead, stale, waiting-user, blocked, throttled)". `unanswered`
# is deliberately absent — SC-017g makes the aged-request rank SESSION-level, and the
# row forbids fabricating a per-agent reason from it.
AGENT_OWNED = ("dead", "stale", "waiting-user", "blocked", "throttled")


def declared_contributions(case):
    """owner -> {contribution}, from the case's FIXED event bytes.

    SECONDARY evidence only, and measured to be empty in this corpus: every `state`
    event here carries ref `working` or `done`, neither of which is an agent-owned
    ACTIVE contribution. Kept because an event names owner and class explicitly and
    would be decisive where one exists — but a predicate resting on it ALONE derives
    zero obligations, which is what the first version of this derivation did before
    the bytes were read."""
    out = {}
    p = os.path.join(SRC, case, "events.bytes.jsonl")
    if not os.path.exists(p):
        return out
    for line in open(p, encoding="utf-8", errors="replace"):
        line = line.strip()
        if not line:
            continue
        try:
            e = json.loads(line)
        except ValueError:
            continue
        if e.get("action") != "state":
            continue
        actor, cls = e.get("actor"), e.get("ref")
        if actor and cls in AGENT_OWNED:
            out.setdefault(actor, set()).add(cls)
    return out


# The producer-side carriers for the three DERIVED contributions. dead, stale and
# throttled are never self-declared, but the watchdog's own alert names the OWNER in
# `target` and the contribution in `summary` — so an exact per-agent locus exists
# where a session-attention-only reading could only guess.
# THE NAMED-LEDGER CARRIER GRAMMAR, DERIVED rather than enumerated as "the actions we
# searched". A ledger event is a carrier iff it NAMES A TARGET and supplies a
# contribution, and there are exactly two ways to supply one:
#   the ACTION IS the contribution      — action in AGENT_OWNED (e.g. `throttled`)
#   the action is `alert`               — the contribution is named in its summary
# The first branch is why enumerating searched actions was wrong: the four
# `action=throttled` events carry summary "upstream throttling detected — pausing
# nudge", which no summary prefix for `throttled` matches, so a summary-only reading
# dropped a carrier whose ACTION already said what it was. Summary parsing is neither
# needed nor permitted to narrow an action that is itself a contribution.
ALERT_SUMMARY = (("agent process dead", "dead"), ("max nudges reached", "stale"),
                 ("throttled for", "throttled"))
# A carrier stops being current when the newest DECISIVE event for that target is a
# clearance: the agent acting again (actor == target), or a target-named clear. The
# clear ACTIONS are derived from the same grammar — any action of the form
# <contribution>-cleared, or `alert-cleared` — and have NO specimen in this corpus, so
# that branch is asserted by construction and red-proved synthetically, never claimed
# as exercised.
CLEAR_ACTIONS = tuple("%s-cleared" % c for c in AGENT_OWNED) + ("alert-cleared",)


LIVE_SCOPE_FILTERS = ("--needs-attn", "--active")
SELECTORS = ("--running", "--stopped", "--all")


def empty_live_scope(argv):
    """True when the selector makes this document's session set EMPTY.

    SELECTOR-FIRST, and it is derived rather than pattern-matched on the consumer
    name: SC-521b makes same-dimension selectors ALTERNATIVES with the last distinct
    one winning, so the winning selector is the last of --running/--stopped/--all in
    argv order; SC-521c then says a stopped session never satisfies a live-scope
    predicate, so `--stopped` plus --needs-attn or --active is empty in BOTH
    orderings. Establishing the session set first is what makes a descendant
    obligation admissible at all — the previous derivation reached inside documents
    whose session set the contract forbids."""
    words = argv.split()
    winner = None
    for w in words:
        if w in SELECTORS:
            winner = w
    return winner == "--stopped" and any(f in words for f in LIVE_SCOPE_FILTERS)


def carrier_contribution(event):
    """The contribution this event supplies, or None if it is not a carrier."""
    if not event.get("target"):
        return None
    action = event.get("action")
    if action in AGENT_OWNED:
        return action
    if action == "alert":
        summary = str(event.get("summary") or "")
        for prefix, contribution in ALERT_SUMMARY:
            if summary.startswith(prefix):
                return contribution
    return None


def template_of(case):
    """The producer template this case was cloned from, from its own case.txt."""
    p = os.path.join(SRC, case, "case.txt")
    if not os.path.exists(p):
        return None
    m = re.search(r"\btemplate=(\S+)", open(p, encoding="utf-8", errors="replace").read())
    return m.group(1) if m else None


def alert_contributions(template, session):
    """target -> its CURRENT contribution, from the producer template's ledger bytes.

    Newest decisive event wins. A carrier is cleared by the agent ACTING AGAIN
    (actor == target) or by a target-named clearance action — a past carrier is not a
    present fact. Nothing is inferred from the session's ranked attention, which names
    a class without naming an owner."""
    if not template or "/" not in template:
        return {}
    arm, variant = template.split("/", 1)
    p = os.path.join(SRC, "templates", arm, "fixture-bytes", variant,
                     "sessions", session, "events.jsonl")
    if not os.path.exists(p):
        return {}
    events = []
    for line in open(p, encoding="utf-8", errors="replace"):
        line = line.strip()
        if not line:
            continue
        try:
            events.append(json.loads(line))
        except ValueError:
            pass
    current = {}
    for i, e in enumerate(events):
        contribution = carrier_contribution(e)
        if contribution:
            current[e["target"]] = (contribution, i)
        target, actor, action = e.get("target"), e.get("actor"), e.get("action")
        if target and action in CLEAR_ACTIONS and target in current \
                and i > current[target][1]:
            del current[target]
        if actor in current and i > current[actor][1]:
            del current[actor]
    return {k: v[0] for k, v in current.items()}


def loss_sessions(case):
    """Sessions whose data suffered ACTUAL read/parse loss, from the manifest.

    SC-509b's trigger is actual loss, not sparsity. The evidence is the manifest's
    OWN marker: a session directory whose `meta` is absent from the manifest, or
    present with the hash column reading UNREADABLE. Deliberately NOT the mode
    enumeration `missing_selector_sessions` uses — an enumerated mode list is a
    checker carrying a list of names, and the first mode nobody thought of reads as
    readable. Measured on this corpus: the two predicates agree exactly (every meta
    is 444, 644, or mode 0 with hash UNREADABLE, and all seven mode-0 metas are
    UNREADABLE), so this is the same population by better evidence, not a different
    one. Selector-missing and read-loss are different facts that coincide here, so
    they get different predicates rather than one alias."""
    mb = os.path.join(SRC, case, "manifest.before.tsv")
    if not os.path.exists(mb):
        return []
    rows = [l.rstrip("\n").split("\t") for l in open(mb, encoding="utf-8", errors="replace")]
    sess = {r[-1].split("/")[2] for r in rows
            if r[0] == "dir" and re.match(r"\./sessions/[^./][^/]*$", r[-1])}
    metas = {r[-1].split("/")[2]: r[2] for r in rows
             if r[0] == "file" and re.match(r"\./sessions/[^./][^/]*/meta$", r[-1])}
    return sorted((sess - set(metas)) | {n for n, h in metas.items() if h == "UNREADABLE"})


def main():
    rows, seen, unproved = [], set(), []
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
                         m.group(1) if m else "ABSENT", "2", "equals", "SOURCE", "OBSERVED",
                         "successor digest is schema version 2"))
            # SC-017o, RE-DERIVED under the 2026-08-24 entitlement ruling.
            #
            # The FIELD is mandated unconditionally on every successor digest and its
            # absence from every version-1 capture is decidable. That is the whole of
            # what this corpus earns.
            #
            # The VALUE is not earned anywhere, measured rather than argued: `false`
            # needs an INDEPENDENTLY ENTITLED enumeration with a FINAL FAILURE, and
            # across all 148 P1 cases every captured connect failure names ONLY the
            # case's own live.sock — ZERO name a session's RECORDED server. Ambient
            # entitlement turns on a selected ambient server, and SC-1410c leaves
            # AE_TMUX_SERVER selection unclassified, so the ambient probe cannot earn it
            # either. A missing or unreadable meta is SC-405i/SC-509b record loss and
            # never SC-017o by itself. So the VALUE is UNSCORABLE — and it is a ROW in
            # this table, beside the presence locus, because C3/C5/C16 keep an
            # unscorable obligation IN THE DENOMINATOR. It was briefly kept in a side
            # file instead; that file is deleted, since a gate that reads an unpinned
            # file has made it authority whatever its label says.
            rows.append((case, consumer, "SC-017o", "digest", "inventory_complete",
                         "present" if '"inventory_complete"' in text else "ABSENT",
                         "present", "present", "OBSERVED", "OBSERVED",
                         "the field is mandated unconditionally; its VALUE is unscorable "
                         "on this corpus and is recorded as such, not asserted"))
            # THE VALUE IS AN OBLIGATION, NOT A FOOTNOTE. C3/C5/C16 define UNSCORABLE
            # as an obligation that REMAINS IN THE DENOMINATOR and require every
            # unscorable locus preserved in the result, so moving it to a side file
            # broke three criteria at once. It is a row, addressable beside the
            # presence locus, carrying the predicate `undecidable` — a schema extension
            # made because the accounting needs the locus, not a locus dropped because
            # the schema was inconvenient.
            rows.append((case, consumer, "SC-017o", "digest", "inventory_complete (value)",
                         "ABSENT", "the enumeration's actual completeness", "undecidable",
                         "OBSERVED", "UNSCORABLE",
                         "no captured connect failure names a session's RECORDED server, so "
                         "no independently entitled enumeration is shown to have finally "
                         "failed; ambient entitlement turns on the AE_TMUX_SERVER selection "
                         "SC-1410c leaves unclassified"))

            # ---- SC-509b + SC-509c, both JSON loci on a row that ALREADY carries
            # SC-509d, so neither can create a carrying row. The locus determines the
            # row; there is no assignment choice to make.
            loss = loss_sessions(case)
            declared = declared_contributions(case)
            scope_empty = empty_live_scope(r["normalised_argv"])
            try:
                doc = json.loads(text)
            except ValueError:
                doc = None
            if scope_empty:
                # SC-521c: the whole session set must be empty, so the document's
                # MEMBERSHIP is the obligation and nothing inside it can be. Deleting
                # the impossible descendants alone would leave the retained comparison
                # still expecting the frozen nonempty document.
                # NOTE the ordering: this reads `doc`, so it must run AFTER the parse.
                # It first ran before it and silently reported the PREVIOUS row's
                # session count — a stale-variable read that the from-values exposed
                # only because I printed them.
                n_sessions = len((doc or {}).get("sessions", []) or [])
                rows.append((case, consumer, "SC-521c", "digest", "sessions[] (set)",
                             str(n_sessions), "empty", "equals", "OBSERVED", "OBSERVED",
                             "--stopped is the winning selector under SC-521b and a "
                             "stopped session satisfies no live-scope predicate under "
                             "SC-521c, so this set is empty in both orderings"))
            template = template_of(case)
            for sess in ([] if scope_empty else (doc or {}).get("sessions", []) or []):
                name = sess.get("name")
                alerts = alert_contributions(template, name or "")
                if name in loss:
                    rows.append((case, consumer, "SC-509b", "digest",
                                 "sessions[].degraded",
                                 "present" if "degraded" in sess else "ABSENT",
                                 "true", "equals", "OBSERVED", "OBSERVED",
                                 "session %s: the manifest proves its meta absent or "
                                 "unreadable, so this entry suffered ACTUAL read/parse "
                                 "loss rather than sparsity" % name))
                for ag in sess.get("agents", []) or []:
                    ref = ag.get("ref")
                    if not ref or ag.get("reason") is not None:
                        continue
                    # PRIMARY evidence: the frozen digest populates `agents[].state`
                    # for the very agent whose `reason` it leaves null. That field
                    # names the OWNER (this entry's ref) and the EXACT contribution
                    # (its own declared class) in the captured bytes — it is the
                    # information SC-509c says the surface already has and fails to
                    # put where the contract requires.
                    own = ag.get("state")
                    if own in AGENT_OWNED:
                        proved, evidence = [own], "its own declared state"
                    elif ref in alerts:
                        proved, evidence = [alerts[ref]], "a watchdog alert naming it as target"
                    else:
                        proved = sorted(declared.get(ref, set()))
                        evidence = "a state event naming it as actor"
                    att = sess.get("attention")
                    if len(proved) == 1:
                        # The locus names the SESSION AND THE AGENT, because neither
                        # alone is an address. `agents[].reason` collapsed 128
                        # obligations to 88 keys — a cardinality that would have
                        # matched the census's 88 while being a different set. And
                        # (case, consumer, ref) is not unique either: 64 such keys
                        # name an agent that appears under MORE THAN ONE session in
                        # the same digest. A key that is not a key makes every set
                        # comparison built on it meaningless.
                        rows.append((case, consumer, "SC-509c", "digest",
                                     "sessions[%s].agents[%s].reason" % (name, ref),
                                     "null", proved[0], "equals",
                                     "OBSERVED", "OBSERVED",
                                     "%s: %s names the owner and the exact contribution "
                                     "%s in fixed producer bytes"
                                     % (ref, evidence, proved[0])))
                    elif len(proved) > 1:
                        unproved.append((case, consumer, name or "", ref,
                                         "sessions[%s].agents[%s].reason" % (name, ref),
                                         str(att), "AMBIGUOUS-CONTRIBUTION",
                                         "the owner is named but declared %s; which one "
                                         "the snapshot carries needs the latest-relevant "
                                         "rule of the UNRATIFIED SC-907, so it is not an "
                                         "EXACT contribution" % "/".join(proved)))
                    elif att in AGENT_OWNED:
                        unproved.append((case, consumer, name or "", ref,
                                         "sessions[%s].agents[%s].reason" % (name, ref),
                                         str(att), "OWNER-NOT-ESTABLISHED",
                                         "session attention is an agent-owned class, but "
                                         "no event names which roster agent owns it; "
                                         "dead/stale/throttled are derived, never declared"))

        # ---- SC-017p/q/r + SC-509e: per-agent liveness (contract 01353d8c) ----
        # SC-017q's matrix is an IMPLICATION, not an orthogonality: session `unknown`
        # implies agent `unknown`. So wherever the session diverges to unknown, every
        # roster agent's health diverges with it — and the two surfaces move in
        # OPPOSITE directions from the same frozen defect, which is why these are two
        # separate obligations rather than one.
        sel_missing = selector_missing(case)
        sup = "OBSERVED" if sel_missing else "UNSCORABLE"
        if incomplete:
            if digest and '"alive"' in text:
                n_false = len(re.findall(r'"alive"\s*:\s*false', text))
                n_true = len(re.findall(r'"alive"\s*:\s*true', text))
                if n_false:
                    rows.append((case, consumer, "SC-509e", "digest", "agents[].alive",
                                 "false", "null", "all-of", "OBSERVED", sup,
                                 f"{n_false} agent(s) recorded false from an unavailable "
                                 "pane query; unprovable is null"))
                if n_true:
                    rows.append((case, consumer, "SC-509e", "digest", "agents[].alive",
                                 "true", "null", "all-of", "OBSERVED", sup,
                                 f"{n_true} agent(s) recorded true, but session unknown "
                                 "implies agent unknown"))
            if listish and not digest and re.search(r"^\s{2}\S+:\S+\s", text, re.M):
                rows.append((case, consumer, "SC-017r", "stdout", "agent health marker",
                             "blank", "unambiguous unknown", "all-of", "OBSERVED", sup,
                             "frozen renders alive and absent identically as a blank "
                             "marker; unknown must be non-silent"))

        # SELECTOR-MISSING IS AN INDEPENDENT SUFFICIENT CAUSE OF `unknown`, and gating
        # the liveness obligations on `incomplete` hid that. The chain never touches
        # the case query: SC-405l makes the durable selector missing, SC-017j keeps the
        # candidate and forbids name-only reconciliation, SC-017l makes its liveness
        # unknown, SC-017m renders unknown in the default and --all views. A live
        # sighting may ADD a running candidate if ownership is proven, but it cannot
        # REMOVE the durable unknown one.
        #
        # So this locus — an unknown row is PRESENT for that candidate — is scorable
        # from the manifest and the frozen bytes even where the full row set is not.
        # Deliberately NOT relabelled SC-017l (no status transition is claimed) and
        # deliberately NOT a whole-row-set prediction.
        if listish and sel_missing and not incomplete:
            names = missing_selector_sessions(case)
            shown = [n for n in names if re.search(r"\b%s\b" % re.escape(n), text)]
            rows.append((case, consumer, "SC-017m",
                         "digest" if digest else "stdout", "unknown row present",
                         "present" if shown else "absent", "unknown", "present",
                         "OBSERVED", "OBSERVED",
                         "selector missing by construction; %s"
                         % (", ".join(names) if names else "candidate")))

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
                             "stopped", "unknown", "all-of", "OBSERVED", sup,
                             f"{n} captured occurrence(s) must all move"))
            else:
                rows.append((case, consumer, "SC-017m", stream, "(row set)",
                             "empty", "unknown rows present", "present", "OBSERVED", sup,
                             "default view shows running then unknown; absent becomes present"))
            # The SC-017o HUMAN DIAGNOSTIC is deliberately NOT emitted. It is earned
            # only by an independently entitled enumeration with a final failure, and the
            # previous derivation earned it from `unreachable(case)` — the ambient probe
            # against the case's own live.sock, which is exactly the fact the ruling says
            # cannot earn it. 172 obligations rested on that basis; none survives.

    unproved.sort()
    with open(UNPROVED, "w", encoding="utf-8") as fh:
        fh.write("# SC-509c loci EXCLUDED for want of a carrier. Reported, never guessed.\n")
        fh.write("# AT THE RULED GRAIN: (case, consumer, session, agent_ref, locus), the same\n")
        fh.write("# address the accepted table uses. It was previously keyed without the\n")
        fh.write("# session, so 34 rows mapped ambiguously to two same-attention sessions and\n")
        fh.write("# their no-carrier claim could not be evaluated per address. An exclusion\n")
        fh.write("# file below the ruled grain cannot substantiate its own claims.\n")
        fh.write("# NOT a claim of impossibility: no carrier was FOUND by the search this\n")
        fh.write("# generator performs — the agent's own state, a state event naming it as\n")
        fh.write("# actor, and a producer-template alert naming it as target.\n")
        fh.write("\t".join(["case", "consumer", "session", "agent_ref", "locus",
                            "session_attention", "kind", "why"]) + "\n")
        for x in unproved:
            fh.write("\t".join(str(v) for v in x) + "\n")

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

    sup = collections.Counter(x[9] for x in rows)
    for k in sorted(sup): print("  support %-11s %4d" % (k, sup[k]))
    per = collections.Counter(x[2] for x in rows)
    carriers = len({(x[0], x[1]) for x in rows})
    print(f"P1 rows {len(seen)}   obligations {len(rows)}   rows carrying >=1: {carriers}")
    for k in sorted(per):
        print(f"  {k:<10} {per[k]:4d}")
    print(f"derived EXPECTED-DIVERGENCE {carriers}   EXPECTED-MATCH {len(seen) - carriers}")
    kinds = collections.Counter(x[6] for x in unproved)
    print(f"SC-509c loci EXCLUDED for want of exact evidence: {len(unproved)}")
    for k in sorted(kinds):
        print(f"  {k:<24} {kinds[k]:4d}")

if __name__ == "__main__":
    main()
