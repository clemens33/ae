#!/usr/bin/env python3
"""GATE — checks the obligation table against the captured bytes, the P1 population,
and the contract's CURRENT hash. No write path: no open-for-write, no temp, no rename.

Four classes, and the freshness one is the reason this file exists. A derived artifact
goes stale the moment its source grows and nothing re-runs to say so; the previous
column was found stale by a human noticing. This makes staleness a gate result.
"""
import calendar
import hashlib
import time
import argparse, csv, json, os, re, subprocess, sys, collections

HERE = os.path.dirname(os.path.abspath(__file__))
SRC = os.path.normpath(os.path.join(HERE, "..", "batch-c-artifacts"))
OBL = os.path.join(HERE, "OBLIGATIONS.tsv")
FRESH = os.path.join(HERE, "FRESHNESS.tsv")
INV = os.path.join(HERE, "INVOCATIONS.tsv")
STREAMS = {"digest", "stdout", "stderr"}
# `relational` is the SC-522 run-time form, ruled 2026-08-24: the obligation is a
# RELATION the successor's own output must satisfy, joined explicitly to PINNED
# fixture input, never a static value derived from a frozen clock. It is not
# `undecidable` — the rule is statable and this corpus simply cannot witness it —
# and it is not `equals`, because there is no single owed value to equal.
PREDICATES = {"equals", "at-least", "all-of", "present", "undecidable", "relational"}
SUPPORT = {"OBSERVED", "UNSCORABLE"}
LISTING = ("ae list", "ae ls")
# THE IDENTITY/PAYLOAD SPLIT, IN BYTES RATHER THAN IN A HEAD. For OBLIGATIONS.tsv the
# ADDRESS is (case, consumer, obligation_id, locus); everything else is row DATA.
# Keying an address on payload lets a contradictory duplicate buy its own address by
# changing the very field that made it contradictory.
ADDRESS = ("case", "consumer", "obligation_id", "locus")
# THE POPULATION EACH ID MAY APPEAR ON, declared rather than left to whichever parse
# condition a loop happened to use. Measured against the accepted table before being
# written down: SC-017l/m appear on digest and human listings, SC-017r on human
# listings, and SC-509b/c/d/e and SC-017o on digests only. OWED-ZERO IS AN OBLIGATION
# TO CHECK, NOT A ROW TO SKIP — a loop that skips a class is quiet exactly outside the
# domain its author imagined, which is where a fabricated row goes to hide.
ID_POPULATION = {
    "SC-017l": ("digest", "human-listing"),
    "SC-017m": ("digest", "human-listing"),
    "SC-017o": ("digest",),
    "SC-017r": ("human-listing",),
    "SC-509b": ("digest",),
    "SC-509c": ("digest",),
    "SC-509d": ("digest",),
    "SC-509e": ("digest",),
    "SC-521c": ("digest",),
    # The stopped-session families. SC-509 carries the agents[] state field and
    # SC-017g the session attention value; both are JSON-only.
    "SC-509": ("digest",),
    "SC-017g": ("digest",),
    # SC-518/SC-518a live on the requests capture, which is neither a digest nor
    # a listing. It gets its OWN class rather than being called `opaque`: a
    # structured status table is not an opaque blob, and a taxonomy that lies
    # about a row's shape cannot police what may appear on it.
    "SC-518": ("requests",),
    "SC-405g": ("digest",),
    "SC-518a": ("requests",),
}
# The FIXED SEMANTIC COLUMNS, in header order. `authority` is EXCLUDED BY DECLARATION,
# not by omission: it is narrative, and binding prose would make a reworded explanation
# a gate failure. Everything else is asserted WHOLE — a shape binding SOME fields leaves
# the rest free to drift while the row still looks like itself.
FIXED = ("obligation_id", "stream", "locus", "from", "to", "predicate",
         "baseline_provenance", "support")
PRESENCE_ROW = ("SC-017o", "digest", "inventory_complete", "ABSENT", "present",
                "present", "OBSERVED", "OBSERVED")
# The SECOND legal undecidable, and it is admitted as an EXACT SHAPE WITH ONE
# VARIABLE, never as a loosened rule. Every field is fixed except the session name
# inside the locus, so a mandatory scorable locus still cannot launder itself into
# `undecidable` by claiming the predicate — which is the exact laundering the
# single-shape check was built to stop.
RELATIONAL_LOCUS = re.compile(
    r"^sessions\[[^\]]+\]\.(needs_attention|attention|attention_rank)$")
ATTN_VALUE_LOCUS = re.compile(r"^sessions\[[^\]]+\]\.attention \(value\)$")
ATTN_VALUE_ROW = ("SC-017g", "digest", None, "null",
                  "the most-actionable reason at capture time", "undecidable",
                  "OBSERVED", "UNSCORABLE")
VALUE_ROW = ("SC-017o", "digest", "inventory_complete (value)", "ABSENT",
             "the enumeration's actual completeness", "undecidable",
             "OBSERVED", "UNSCORABLE")
AGENT_OWNED = ("dead", "stale", "waiting-user", "blocked", "throttled")
ALERT_SUMMARY = (("agent process dead", "dead"), ("max nudges reached", "stale"),
                 ("throttled for", "throttled"))


LIVE_SCOPE_FILTERS = ("--needs-attn", "--active")
SELECTORS = ("--running", "--stopped", "--all")


def empty_live_scope(argv):
    """SELECTOR-FIRST, re-derived here rather than trusted from the generator: the
    winning selector is the LAST of --running/--stopped/--all (SC-521b), and a stopped
    session satisfies no live-scope predicate (SC-521c), so --stopped plus a live
    filter is empty in both orderings. No descendant obligation is owed inside a
    session set the contract makes empty."""
    words = argv.split()
    winner = None
    for w in words:
        if w in SELECTORS:
            winner = w
    return winner == "--stopped" and any(f in words for f in LIVE_SCOPE_FILTERS)


def alert_owners(case, session):
    """Re-derived INDEPENDENTLY of the generator, from the producer template bytes:
    the watchdog alert's `target` names the owner, its `summary` the contribution,
    and a later event by that agent supersedes it (alert-once-then-quiet)."""
    cp = os.path.join(SRC, case, "case.txt")
    if not os.path.exists(cp):
        return {}
    m = re.search(r"\btemplate=(\S+)", open(cp, encoding="utf-8", errors="replace").read())
    if not m or "/" not in m.group(1):
        return {}
    arm, variant = m.group(1).split("/", 1)
    ep = os.path.join(SRC, "templates", arm, "fixture-bytes", variant,
                      "sessions", session, "events.jsonl")
    if not os.path.exists(ep):
        return {}
    events = []
    for line in open(ep, encoding="utf-8", errors="replace"):
        line = line.strip()
        if line:
            try:
                events.append(json.loads(line))
            except ValueError:
                pass
    cur = {}
    for i, e in enumerate(events):
        t, sm = e.get("target"), str(e.get("summary") or "")
        # Same derived carrier grammar, re-implemented independently: the ACTION is
        # the contribution when it names one, otherwise an `alert` names it in its
        # summary. A summary-only reading dropped every action=throttled carrier.
        if t:
            got = None
            if e.get("action") in AGENT_OWNED:
                got = e.get("action")
            elif e.get("action") == "alert":
                for prefix, contribution in ALERT_SUMMARY:
                    if sm.startswith(prefix):
                        got = contribution
            if got:
                cur[t] = (got, i)
        if t and str(e.get("action") or "").endswith("-cleared") and t in cur \
                and i > cur[t][1]:
            del cur[t]
        if e.get("actor") in cur and i > cur[e.get("actor")][1]:
            del cur[e.get("actor")]
    return {k: v[0] for k, v in cur.items()}


# The clock binding, RE-DERIVED HERE rather than imported. A gate that imports
# the generator's constants cannot disagree with it, and gate/generator drift is
# exactly the defect this file exists to catch — it has already shipped once,
# when the gate kept an old carrier grammar and demanded moves the generator had
# correctly stopped emitting. These literals are transcribed from the same fixed
# bytes (arms/A2/harness/arm-a2.sh lines 122-134, manifest-pinned PROVENANCE).
CLOCK_HARNESS = "arms/A2/harness/arm-a2.sh"
CLOCK_HARNESS_SHA256 = \
    "f9e5a07f17865a488e65fb5e1e1c2e4a088bcd80bbdd35e9953b4201fd1c5932"
CLOCK_WINDOWS = ("inside", "outside")
CLOCK_SUFFIXES = ("list_active", "list_active_json", "list_busy", "active_all",
                  "all_active", "active_stopped", "stopped_active",
                  "needsattn_active", "active_needsattn")


def clock_binding(out, pairs):
    """(case, consumer) -> now_epoch, or {} with a finding if the source moved."""
    h = os.path.join(SRC, CLOCK_HARNESS)
    try:
        sha = hashlib.sha256(open(h, "rb").read()).hexdigest()
    except IOError:
        fail(out, "CLOCK-SOURCE", "%s is unreadable, so no window invocation can be "
             "bound to the clock it was captured at" % CLOCK_HARNESS)
        return {}
    if sha != CLOCK_HARNESS_SHA256:
        fail(out, "CLOCK-SOURCE", "%s is sha256 %s, not the pinned %s; the transcribed "
             "binding no longer cites these bytes" % (CLOCK_HARNESS, sha[:12],
                                                      CLOCK_HARNESS_SHA256[:12]))
        return {}
    binding = {}
    for case in sorted({c for c, _ in pairs}):
        p = os.path.join(SRC, case, "activity-window.txt")
        if not os.path.exists(p):
            continue
        txt = open(p, encoding="utf-8").read()
        clocks = {}
        for win in CLOCK_WINDOWS:
            m = re.search(r"^%s_window_now=(\d+)" % win, txt, re.M)
            if m:
                clocks[win] = int(m.group(1))
        if len(clocks) != len(CLOCK_WINDOWS) or len(set(clocks.values())) != len(clocks):
            fail(out, "CLOCK-SOURCE", "%s does not record two distinct window clocks"
                 % case)
            continue
        for win in CLOCK_WINDOWS:
            for suf in CLOCK_SUFFIXES:
                binding[(case, "win_%s_%s" % (win, suf))] = clocks[win]
    counts = collections.Counter(p for p in pairs if p[1].startswith("win_"))
    for cid, extra_set, msg in (
            ("CLOCK-MISSING", set(binding) - set(counts),
             "harness-produced window consumer(s) absent from fixed INVOCATIONS.tsv"),
            ("CLOCK-UNMAPPED", set(counts) - set(binding),
             "window consumer(s) the pinned harness does not produce"),
            ("CLOCK-AMBIGUOUS", {k for k, n in counts.items() if n > 1},
             "window consumer(s) appearing more than once, so uniquely bound to nothing")):
        if extra_set:
            fail(out, cid, "%d %s: %s" % (len(extra_set), msg, sorted(extra_set)[:3]))
    return binding


ACTIVE_WINDOW_SECS = 300


def _epoch(ts):
    try:
        return calendar.timegm(time.strptime(ts, "%Y-%m-%dT%H:%M:%SZ"))
    except (ValueError, TypeError):
        return None


def template_of(case):
    p = os.path.join(SRC, case, "case.txt")
    if not os.path.exists(p):
        return None
    m = re.search(r"\btemplate=(\S+)", open(p, encoding="utf-8", errors="replace").read())
    return m.group(1) if m else None


def last_event_epoch(template, session):
    """The newest ae EVENT timestamp, RE-DERIVED here from the fixture bytes.

    EVENTS, NEVER MTIME, and never the frozen document's last_active_epoch. This
    check exists because the generator once derived the successor `--active` set
    by SEEDING IT FROM THE FROZEN DOCUMENT — the mtime-sourced artifact under
    test — so it could add SC-524 futures but never remove an mtime false
    positive. The gate was structurally blind to it: it verified the clock and
    the address and never the VALUE. Re-derived, not imported, so the two can
    disagree.
    """
    if not template or "/" not in template:
        return None
    arm, variant = template.split("/", 1)
    p = os.path.join(SRC, "templates", arm, "fixture-bytes", variant,
                     "sessions", session, "events.jsonl")
    if not os.path.exists(p):
        return None
    newest = None
    for line in open(p, encoding="utf-8", errors="replace"):
        line = line.strip()
        if not line:
            continue
        try:
            e = json.loads(line)
        except ValueError:
            continue
        t = _epoch(e.get("ts"))
        if t is not None and (newest is None or t > newest):
            newest = t
    return newest


def active_set_at(case, now):
    """The successor `--active` set at `now`, from event bytes and --all alone."""
    try:
        pop = json.loads(body(case, "list_all_json")).get("sessions", []) or []
    except ValueError:
        return None
    template = template_of(case)
    active = set()
    for s in pop:
        if s.get("status") != "running":
            continue
        t = last_event_epoch(template, s.get("name"))
        if t is not None and (t > now or now - t <= ACTIVE_WINDOW_SECS):
            active.add(s.get("name"))
    return active


def render_set(names):
    return "+".join(sorted(names)) if names else "empty"


KEYSET = os.path.join(HERE, "SC-509C-KEYSET.tsv")
KEYSET_DECL = re.compile(r"^# CUT-FROM:\s+(\S+)\s+blob\s+([0-9a-f]{40})\s*$", re.M)
REASON_LOCUS = re.compile(r"^sessions\[([^\]]+)\]\.agents\[([^\]]+)\]\.reason$")


def _509c_keys(rows):
    """(case, consumer, session, agent_ref) for every SC-509c row of a table."""
    keys = set()
    for r in rows:
        if r.get("obligation_id") != "SC-509c":
            continue
        m = REASON_LOCUS.match(r.get("locus", ""))
        if m:
            keys.add((r["case"], r["consumer"], m.group(1), m.group(2)))
    return keys


def check_keyset(out, obls, quiet):
    """SC-509C-KEYSET.tsv is FROZEN, and this proves the freeze is honest.

    The relation is historical — the file must equal the SC-509c key set of the
    blob it declares it was cut from — so it never demands re-derivation, unlike
    the `^SOURCE:` freshness grammar. What CAN rot is the claim itself: a hand
    edit, a partial regeneration, or a declaration pointing at the wrong blob.
    The live delta is printed rather than asserted, because a frozen census
    diverging from a moving table is the expected state, not a finding.
    """
    if not os.path.exists(KEYSET):
        return
    text = open(KEYSET, encoding="utf-8").read()
    decls = KEYSET_DECL.findall(text)
    if len(decls) != 1:
        fail(out, "KEYSET-FROZEN", "SC-509C-KEYSET.tsv declares %d CUT-FROM relations; "
             "exactly one is required, or the freeze is a claim nothing checks" % len(decls))
        return
    _, blob = decls[0]
    try:
        src = subprocess.run(["git", "cat-file", "blob", blob], cwd=HERE,
                             capture_output=True, text=True, check=True).stdout
    except (subprocess.CalledProcessError, OSError):
        fail(out, "KEYSET-FROZEN", "SC-509C-KEYSET.tsv is cut from blob %s, which git "
             "cannot resolve" % blob[:12])
        return
    cut = _509c_keys(csv.DictReader(src.splitlines(), delimiter="\t"))
    body_lines = [l for l in text.split("\n") if l and not l.startswith("#")]
    head = body_lines[0].split("\t")
    col = {k: head.index(k) for k in ("case", "consumer", "session", "agent_ref")
           if k in head}
    if len(col) != 4:
        fail(out, "KEYSET-FROZEN", "SC-509C-KEYSET.tsv is missing key column(s) %s"
             % sorted({"case", "consumer", "session", "agent_ref"} - set(col)))
        return
    frozen = set()
    for l in body_lines[1:]:
        f = l.split("\t")
        frozen.add(tuple(f[col[k]] for k in ("case", "consumer", "session", "agent_ref")))
    if frozen != cut:
        fail(out, "KEYSET-FROZEN", "SC-509C-KEYSET.tsv holds %d keys but blob %s holds "
             "%d SC-509c keys (+%d/-%d); the declared freeze does not describe this file"
             % (len(frozen), blob[:12], len(cut), len(frozen - cut), len(cut - frozen)))
        return
    live = _509c_keys(obls)
    if not quiet:
        print("  SC-509C-KEYSET.tsv frozen at blob %s: %d keys, honest; live table has "
              "%d (+%d/-%d since the cut)"
              % (blob[:12], len(frozen), len(live), len(live - frozen), len(frozen - live)))


REQ_SURFACE = "helper:requests"
OPENINGS = ("ask", "review")


def _gident(e, side):
    slot, sess = e.get(side + "_slot"), e.get(side + "_session")
    if slot and sess:
        return ("routed", slot, sess)
    if slot is None and sess is None:
        return ("display", e.get(side), None)
    return ("unassociated", None, None)


def _gsame(a, b):
    if a[0] != b[0] or a[0] == "unassociated":
        return False
    return a[1] is not None and a[1] == b[1] and a[2] == b[2]


def gate_ruled_requests(case):
    return gate_ruled_requests_for(case, None)


def gate_ruled_requests_for(case, session):
    """ref -> (status, summary), RE-DERIVED from the producer ledger.

    Independent of the generator on purpose: without this the gate could police
    the ADDRESS of an SC-518/SC-518a row and never its existence, and a deleted
    ordering move passed green — measured by red-proof.
    Returns None when the case declares no template, or a cancel appears (whose
    authorization no row defines, so no status may be derived).
    """
    p = os.path.join(SRC, case, "case.txt")
    if not os.path.exists(p):
        return None
    txt = open(p, encoding="utf-8", errors="replace").read()
    tm = re.search(r"\btemplate=(\S+)", txt)
    sm = re.search(r"\bsession=(\S+)", txt)
    # A MULTI-SESSION CASE DECLARES NO SINGLE `session=`, so an explicit session
    # argument is the authority when one is given. Requiring case.txt's field
    # unconditionally made every composite case derive None — which read as "no
    # requests here" and silently disagreed with the generator.
    if not tm or "/" not in tm.group(1) or (sm is None and session is None):
        return None
    arm, variant = tm.group(1).split("/", 1)
    f = os.path.join(SRC, "templates", arm, "fixture-bytes", variant,
                     "sessions", session or sm.group(1), "events.jsonl")
    if not os.path.exists(f):
        return {}
    ev = []
    for n, line in enumerate(open(f, encoding="utf-8", errors="replace"), 1):
        line = line.strip()
        if not line:
            continue
        try:
            e = json.loads(line)
        except ValueError:
            continue
        if e.get("action") in OPENINGS + ("reply", "cancel"):
            e["_line"] = n
            ev.append(e)
    if any(e.get("action") == "cancel" for e in ev):
        return None
    opening = {}
    for e in ev:
        if e.get("action") in OPENINGS:
            r = e.get("ref")
            if r not in opening or e["_line"] > opening[r]["_line"]:
                opening[r] = e
    out = {}
    for ref, op in opening.items():
        status, summary = "pending", op.get("summary")
        replies = [t for t in ev if t.get("action") == "reply" and t.get("ref") == ref]
        for t in replies:
            if t["_line"] > op["_line"] and \
               _gsame(_gident(t, "actor"), _gident(op, "target")) and \
               _gsame(_gident(t, "target"), _gident(op, "actor")):
                status, summary = "replied", t.get("summary")
        # THE MOVER IS DERIVED HERE TOO, so a row can be checked for OWNERSHIP and
        # not merely for existence. Measured bypass: relabelling one A6 m2 row
        # SC-518a -> SC-518 moved the counts 20/4 -> 21/3 and the gate stayed green.
        mover = None
        if status == "pending" and replies:
            pick = max(replies, key=lambda e: e["_line"])
            mover = "SC-518a" if pick["_line"] < op["_line"] else "SC-518"
        out[ref] = (status, summary, op.get("ts"), mover)
    return out


def gate_capture_requests(case, consumer):
    rows = {}
    for line in body(case, consumer).splitlines()[1:]:
        f = line.split(None, 5)
        if len(f) >= 6:
            rows[f[2]] = (f[0], f[5])
    return rows


# THE BELOW-THRESHOLD LETTER, PINNED TO THE ACCEPTED CONTRACT. blob 327d1733
# (commit d4534483, sha256 09025346...), colead-accepted. SC-017g rules it in bytes:
# an entry needing no attention "renders them `false`, `null` and `0`. Absence of a
# member is NOT" a quiet signal. These relational loci sit on tg5/tg2un/tg10, none of
# which is in the loss population, so the COMPLETE-entry shape applies — the degraded
# omission rule and SC-405g's branch exception reach none of them.
#
# Below the threshold the ruled letters EQUAL what the capture already shows, so the
# relation is a move only above it. The row is owed either way, because which side a
# successor lands on is not knowable from fixed bytes.
ATTN_RANK = {"unanswered": 1, "throttled": 2, "blocked": 3,
             "waiting-user": 4, "stale": 5, "dead": 6}


def gate_pending_ts(case, session):
    """The OLDEST still-pending opening's ts, re-derived through SC-518+SC-518a."""
    ruled = gate_ruled_requests_for(case, session)
    if not ruled:
        return None
    oldest = None
    for ref, (status, _, ts, _m) in ruled.items():
        if status == "pending" and ts and (oldest is None or ts < oldest):
            oldest = ts
    return oldest


def held_shapes(carriers, case, consumer, ids, where=None):
    """The FIXED tuple of every held row in `ids` — every column that carries
    meaning, narrative authority excluded BY DECLARATION."""
    return collections.Counter(
        (o["obligation_id"], o["stream"], o["locus"], o["from"], o["to"],
         o["predicate"], o["baseline_provenance"], o["support"])
        for o in carriers.get((case, consumer), [])
        if o["obligation_id"] in ids and (where is None or where(o)))


# THE DIRECT GUARD ON THE FAILURE MODE THAT DESTROYED THIS FILE ONCE. An unbounded
# text slice removed a comparison block and the gate then reported VERIFIED — a
# green verdict for checks that no longer existed, which is the loudest a lost check
# ever gets. Green rc is not evidence that the checks ran; the NAME SET is. If a
# structural edit drops a call site, or renames one, this fails instead of passing.
EXPECTED_FAMILIES = {"SC-518/518a", "SC-521c", "SC-509c reasons", "stopped facts",
                     "loss facts"}
_families_seen = set()


def compare_owed(out, case, consumer, family, owed, held):
    """THE ONE COMPARISON EVERY DERIVATION FEEDS. Counter equality over complete
    FIXED tuples, both directions, owed-empty included.

    Built after five measured bypasses that separate existence/value fragments all
    let through: a relocated attention row, an invented state, an invented reason,
    a relabelled mover, a collapsed from==to target, an empty-scope divergence
    turned into a match, and a predicate swapped under unchanged values. Every one
    of them is the same defect — an exact SHAPE checked without an exact
    POPULATION — so the repair is one mechanism, not seven checkers.
    """
    _families_seen.add(family)
    owed = collections.Counter(owed)
    for shape, n in (owed - held).items():
        fail(out, "OWED-MISSING", "%s/%s [%s] owes %s x%d and carries no such row"
             % (case, consumer, family, shape[:3], n))
    for shape, n in (held - owed).items():
        fail(out, "OWED-EXTRA", "%s/%s [%s] carries %s x%d, which the ruled derivation "
             "does not owe" % (case, consumer, family, shape[:3], n))


def _is_stopped_locus(case, doc, locus):
    """Does this locus name a session the digest reports STOPPED?"""
    m = re.match(r"^sessions\[([^\]]+)\]", locus or "")
    if not m:
        return False
    for x in doc.get("sessions", []) or []:
        if x.get("name") == m.group(1):
            return x.get("status") == "stopped"
    return False


def owed_requests(case, consumer, dynamic):
    """The COMPLETE owed FULL-SHAPE multiset for one requests invocation.

    Shapes are the eight FIXED columns minus the narrative authority, so a row is
    bound by its mover id, stream, locus, captured `from`, ruled `to`, predicate
    and provenance/support — not by its locus name. Measured bypasses this closes,
    both green before it existed: an SC-518a row relabelled SC-518, and a status
    row whose target was collapsed to from==to while its locus stayed put.
    A dynamic-excluded or template-less invocation owes EXACTLY THE EMPTY SET, and
    that is compared like any other member.
    """
    if dynamic:
        return set()
    ruled = gate_ruled_requests_for(case, None)
    if ruled is None:
        return set()
    cap = gate_capture_requests(case, consumer)
    owed = set()
    for ref, (got_st, got_sum) in cap.items():
        if ref not in ruled:
            continue
        want_st, want_sum, _ts, mover = ruled[ref]
        for field, got, want in (("status", got_st, want_st),
                                 ("summary", got_sum, want_sum)):
            if got == want:
                continue
            owed.add((mover or "SC-518", "stdout", "requests[%s].%s" % (ref, field),
                      got, want, "equals", "OBSERVED", "OBSERVED"))
    return owed


def gate_declared_contributions(case):
    """(session, actor) -> {agent-owned classes}, from the case's own event bytes.

    Keyed by SESSION and actor for the same reason the generator is: an actor-only
    key lets one case-level carrier authorize every same-ref agent across a
    composite digest, which is cross-session fabrication wearing a carrier's name.
    The session comes from case.txt's fixed `session=` line.

    Re-derived independently of the generator. This is the THIRD branch of the
    ruled reason grammar and the one that produces most owed-EMPTY addresses: it
    is empty across this corpus, so an agent with no captured state and no alert
    naming it proves nothing, and OWNER-NOT-ESTABLISHED is the owed answer for
    that address rather than a reason to stop checking it.
    """
    out = {}
    p = os.path.join(SRC, case, "events.bytes.jsonl")
    if not os.path.exists(p):
        return out
    cp = os.path.join(SRC, case, "case.txt")
    sm = re.search(r"\bsession=(\S+)",
                   open(cp, encoding="utf-8", errors="replace").read()) \
        if os.path.exists(cp) else None
    if not sm:
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
            out.setdefault((sm.group(1), actor), set()).add(cls)
    return out


def owed_reason(case, doc):
    """The COMPLETE owed SC-509c multiset over EVERY digest agent, not only stopped.

    The ruled state/alert/action grammar, in priority order: the agent's own
    declared state (the CAPTURED one for a live session, the PRODUCER-declared one
    for a stopped session whose capture is nulled), else a watchdog alert naming it
    as target, else a state event naming it as actor. Exactly one proved
    contribution owes a row; anything else — none, or ambiguous — owes EMPTY, and
    owed-empty is compared like any other member.

    Built because the row-level any/none converse could not see a single deleted
    locus or a single fabricated one whenever the row carried legitimate reasons
    elsewhere, which is every interesting digest.
    """
    owed = set()
    decl_case = gate_declared_contributions(case)
    for x in doc.get("sessions", []) or []:
        nm = x.get("name") or ""
        stopped = x.get("status") == "stopped"
        sdecl = stopped_declared(case, nm) if stopped else {}
        alerts = alert_owners(case, nm)
        for a in x.get("agents") or []:
            ref = a.get("ref")
            if a.get("reason") not in (None, ""):
                continue
            own = sdecl.get(ref) if stopped else a.get("state")
            if own in AGENT_OWNED:
                proved = [own]
            elif ref in alerts:
                proved = [alerts[ref]]
            else:
                proved = sorted(decl_case.get((nm, ref), set()))
            if len(proved) != 1:
                continue                      # OWNER-NOT-ESTABLISHED: owes EMPTY
            owed.add(("SC-509c", "digest",
                      "sessions[%s].agents[%s].reason" % (nm, ref),
                      "null", proved[0], "equals", "OBSERVED", "OBSERVED"))
    return owed


def gate_loss_sessions(case):
    """Sessions with ACTUAL read/parse loss, RE-DERIVED from the manifest's own
    marker: a session directory whose `meta` is absent, or present with the hash
    column reading UNREADABLE. Never the capture's `degraded` key — that is a
    successor-only member no frozen entry carries, so asking for it returns an empty
    population that reads like a proof of absence.
    """
    mb = os.path.join(SRC, case, "manifest.before.tsv")
    if not os.path.exists(mb):
        return []
    rows = [l.rstrip("\n").split("\t") for l in open(mb, encoding="utf-8", errors="replace")]
    sess = {r[-1].split("/")[2] for r in rows
            if r[0] == "dir" and re.match(r"\./sessions/[^./][^/]*$", r[-1])}
    metas = {r[-1].split("/")[2]: r[2] for r in rows
             if r[0] == "file" and re.match(r"\./sessions/[^./][^/]*/meta$", r[-1])}
    return sorted((sess - set(metas)) | {n for n, h in metas.items() if h == "UNREADABLE"})


def owed_loss(case, doc):
    """The COMPLETE owed multiset for the LOSS families: SC-509b and SC-405g.

    Independent of the generator. The loss population is proved from FIXED sources —
    the manifest showing a meta absent or unreadable — never from a `degraded` key in
    the capture, because `degraded` is a SUCCESSOR-ONLY member that zero frozen
    entries carry. Asking the captures for it measures ZERO and reads as "no
    population"; that mistake is why this derivation reads the manifest instead.

    Per loss occurrence the qualifier and the qualified travel together:
      sessions[<n>].degraded        ABSENT -> true
      sessions[<n>].needs_attention false  -> false   (the degraded-context lower
                                                       bound, owed even though the
                                                       bytes do not move)
    SC-405g adds `branch` null -> ABSENT where NO branch observation exists; an
    OBSERVED branch renders regardless of degraded, and that exception reaches no
    other member by its own terms.
    """
    owed = set()
    loss = gate_loss_sessions(case)
    for x in doc.get("sessions", []) or []:
        nm = x.get("name") or ""
        if nm not in loss:
            continue
        owed.add(("SC-509b", "digest", "sessions[%s].degraded" % nm,
                  "present" if "degraded" in x else "ABSENT", "true",
                  "equals", "OBSERVED", "OBSERVED"))
        owed.add(("SC-509b", "digest", "sessions[%s].needs_attention" % nm,
                  "false" if x.get("needs_attention") is False
                  else str(x.get("needs_attention")).lower(),
                  "false", "equals", "OBSERVED", "OBSERVED"))
        # THE OPTIONAL MEMBERS. Dropped once from this very function, which is why
        # the reconciliation control below is now CODE: an expectation that lives
        # only in messages gets dropped exactly like that. They are exact-or-omitted
        # and a loss session's roster is unenumerable, so missing facts could add,
        # clear or supersede the maximum and neither member is established.
        for member in ("attention", "attention_rank"):
            val = x.get(member)
            owed.add(("SC-509b", "digest", "sessions[%s].%s" % (nm, member),
                      "null" if val is None else str(val).lower(),
                      "ABSENT", "equals", "OBSERVED", "OBSERVED"))
        if "branch" in x and x.get("branch") is None:
            owed.add(("SC-405g", "digest", "sessions[%s].branch" % nm,
                      "null", "ABSENT", "equals", "OBSERVED", "OBSERVED"))
    # RECONCILIATION CONTROL, ENCODED. Every loss occurrence owes EXACTLY FOUR
    # SC-509b loci — degraded, needs_attention, attention, attention_rank — plus at
    # most one SC-405g branch move. This is not a second authority over the table: it
    # checks THIS DERIVATION's own completeness, which is the check that was missing
    # when 28 loci silently vanished from it and the count lived only in a message.
    occurrences = sum(1 for x in doc.get("sessions", []) or []
                      if (x.get("name") or "") in loss)
    b_rows = sum(1 for t in owed if t[0] == "SC-405g")
    if len(owed) != occurrences * 4 + b_rows:
        raise RuntimeError(
            "LOSS-POPULATION-ARITY %s: %d loss occurrence(s) must owe %d SC-509b loci "
            "plus %d branch move(s), got %d owed"
            % (case, occurrences, occurrences * 4, b_rows, len(owed)))
    return owed


def owed_stopped(case, doc):
    """The COMPLETE owed fixed-shape multiset for the stopped-session families.

    Returns {(obligation_id, locus, from, to, predicate, support)}. EMPTY IS A
    MEMBER: a quiet stopped session owes nothing and that is an answer, not a gap.
    Built because the previous checks proved required loci EXIST and never that
    unrequired loci are ABSENT — measured: relabelling one attention row from a
    pending session to a quiet one removed an owed row AND invented one, and the
    gate returned rc=0. Exact shape without exact population is the same class as
    the unbound predicate and the unbound id map.
    """
    owed = set()
    for x in doc.get("sessions", []) or []:
        if x.get("status") != "stopped":
            continue
        nm = x.get("name") or ""
        decl = stopped_declared(case, nm)
        owners = alert_owners(case, nm)
        contrib = dict(owners)
        for actor, st in decl.items():
            if st in AGENT_OWNED:
                contrib[actor] = st
        for a in x.get("agents") or []:
            ref = a.get("ref")
            if decl.get(ref) and a.get("state") in (None, ""):
                owed.add(("SC-509", "digest",
                          "sessions[%s].agents[%s].state" % (nm, ref),
                          "null", decl[ref], "equals", "OBSERVED", "OBSERVED"))
            # The reason family moved to owed_reason(), which covers EVERY digest
            # agent rather than only stopped ones — a stopped-only set left the
            # non-stopped carriers on the boolean converse.
        if contrib:
            best = max(contrib.values(), key=lambda c: ATTN_RANK.get(c, 0))
            for locus, got, want in (
                    ("needs_attention", x.get("needs_attention"), True),
                    ("attention", x.get("attention"), best),
                    ("attention_rank", x.get("attention_rank"), ATTN_RANK[best])):
                if got == want:
                    continue
                owed.add(("SC-017g", "digest", "sessions[%s].%s" % (nm, locus),
                          "null" if got is None else str(got).lower(),
                          "null" if want is None else str(want).lower(),
                          "equals", "OBSERVED", "OBSERVED"))
            continue
        pts = gate_pending_ts(case, nm)
        if not pts:
            continue                      # quiet: owes EMPTY, and that is checked
        for locus, below, above, frm in (
                ("needs_attention", "false", "true", "false"),
                ("attention", "null", "unanswered", "null"),
                ("attention_rank", "0", "1", "0")):
            owed.add(("SC-017g", "digest", "sessions[%s].%s" % (nm, locus), frm,
                      "%s when generated_at - %s <= threshold, %s when strictly greater"
                      % (below, pts, above), "relational", "OBSERVED", "OBSERVED"))
    return owed


def stopped_declared(case, session):
    """ref -> newest declared state, from the session's OWN producer bytes.

    Re-derived here, not imported: the generator and the gate must be able to
    disagree about what a stopped session declares.
    """
    p = os.path.join(SRC, case, "case.txt")
    if not os.path.exists(p):
        return {}
    m = re.search(r"\btemplate=(\S+)", open(p, encoding="utf-8", errors="replace").read())
    if not m or "/" not in m.group(1):
        return {}
    arm, variant = m.group(1).split("/", 1)
    f = os.path.join(SRC, "templates", arm, "fixture-bytes", variant,
                     "sessions", session, "events.jsonl")
    if not os.path.exists(f):
        return {}
    out = {}
    for line in open(f, encoding="utf-8", errors="replace"):
        line = line.strip()
        if not line:
            continue
        try:
            e = json.loads(line)
        except ValueError:
            continue
        if e.get("action") == "state" and e.get("actor"):
            out[e["actor"]] = e.get("ref")
    return out


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

def main(quiet=False, obl=None, fresh=None, inv=None):
    out = []
    obl, fresh, inv = obl or OBL, fresh or FRESH, inv or INV
    for p in (obl, fresh, inv):
        if not os.path.exists(p):
            print("FAIL  MISSING  %s" % os.path.basename(p)); return 1
    rec = {}
    for line in open(fresh, encoding="utf-8"):
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

    obls = list(csv.DictReader(open(obl, encoding="utf-8"), delimiter="\t"))
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
    p1 = [r for r in csv.DictReader(open(inv, encoding="utf-8"), delimiter="\t") if r["phase"] == "P1"]
    p1_rows = p1
    req_by_case = collections.defaultdict(list)
    for r in p1:
        if r["surface"] == REQ_SURFACE:
            req_by_case[os.path.dirname(r["case"])].append(r["consumer"])
    binding = clock_binding(out, [(os.path.dirname(r["case"]), r["consumer"])
                                  for r in p1])
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
            scope_empty = empty_live_scope(r["normalised_argv"])
            want_b = (not scope_empty) and any(
                x.get("name") in loss for x in doc.get("sessions", []) or [])
            # `want_c` and its MISSING-509c / SURFACE pair are DELETED, not kept as a
            # cheap second opinion. They knew only two of the reason grammar's three
            # branches, so a running agent owed via the THIRD — a state event naming
            # it as actor, with no captured state and no alert — would have been
            # emitted by the generator, owed by owed_reason, and then rejected SURFACE
            # by this. A second authority that is a subset of the first is not
            # redundancy; it is a false-failure waiting for the fixture that reaches
            # its blind spot. Exact Counter equality subsumes both directions.
            if want_b and "SC-509b" not in ids:
                fail(out, "MISSING-509b", "%s/%s has a session with unreadable meta and owes "
                     "no degraded move" % (case, consumer))
            if not want_b and "SC-509b" in ids:
                fail(out, "SURFACE", "%s/%s owes a degraded move with no read-loss session"
                     % (case, consumer))
            # SC-521C-ARITY is deleted too. It outlived the comment below that said
            # all seven fragments were gone — the comment was false while it stood,
            # which is worse than the fragment. Exact Counter equality already proves
            # arity in both directions.
            # ---- SC-521c: COMPLETE OWED FULL-SHAPE MULTISET, both populations.
            # Arity/locus fragments let two mutations through, both measured green:
            # an empty-scope row collapsed to from==to (a mandated divergence turned
            # into a match), and a clock-bound row whose predicate was changed from
            # `equals` to `present` (same values, different meaning to the scorer).
            # Shapes bind every column, so neither survives.
            bound = (case, consumer) in binding
            owed_521 = set()
            if scope_empty:
                n_s = len((doc or {}).get("sessions", []) or [])
                owed_521.add(("SC-521c", "digest", "sessions[] (set)", str(n_s),
                              "empty", "equals", "OBSERVED", "OBSERVED"))
            elif bound:
                now_ = binding[(case, consumer)]
                der = active_set_at(case, now_)
                if der is not None:
                    cap_set = render_set([x.get("name") for x in
                                          ((doc or {}).get("sessions") or [])])
                    owed_521.add(("SC-521c", "digest",
                                  "sessions[] (set) @ now=%d" % now_, cap_set,
                                  render_set(der), "equals", "OBSERVED", "OBSERVED"))
            compare_owed(out, case, consumer, "SC-521c", owed_521,
                         held_shapes(carriers, case, consumer, ("SC-521c",)))

            # ---- THE STOPPED FAMILIES, through the same comparison. This block was
            # destroyed once by an unbounded text slice of mine and re-added here;
            # the SC-521c fragment checks it replaced (ARITY, BOTH, SURFACE,
            # CLOCK-ARITY, CLOCK, FROM, VALUE) are all IMPLIED by exact multiset
            # equality and are deliberately not restored — seven named fragments
            # were what let the bypasses through.
            live_doc = doc if not scope_empty else {}
            compare_owed(out, case, consumer, "loss facts",
                         owed_loss(case, live_doc),
                         held_shapes(carriers, case, consumer, ("SC-509b", "SC-405g")))
            compare_owed(out, case, consumer, "SC-509c reasons",
                         owed_reason(case, live_doc),
                         held_shapes(carriers, case, consumer, ("SC-509c",)))
            compare_owed(out, case, consumer, "stopped facts",
                         owed_stopped(case, live_doc),
                         held_shapes(carriers, case, consumer,
                                     ("SC-509", "SC-017g"),
                                     lambda o: o["locus"].startswith("sessions[")
                                     and _is_stopped_locus(case, live_doc, o["locus"])))
        elif "SC-509b" in ids or "SC-509c" in ids:
            fail(out, "SURFACE", "%s/%s is not a digest yet owes a JSON-only obligation"
                 % (case, consumer))

        if not listish and ("SC-017l" in ids or "SC-017m" in ids):
            fail(out, "SURFACE", "%s/%s is not a listing yet owes a listing obligation" % (case, consumer))

    # Every digest owes BOTH SC-017o loci: the mandated boolean PRESENCE and the
    # semantic VALUE. Checked against the P1 population rather than against the table,
    # so a wholesale removal of either has nowhere to hide — the denominator does not
    # come from the thing being checked.
    for r in p1_rows:
        case2, consumer2 = os.path.dirname(r["case"]), r["consumer"]
        text2 = body(case2, consumer2)
        is_digest = '"schema_version"' in text2
        # ---- SC-518 / SC-518a: COMPLETE OWED FULL-SHAPE MULTISET, both ways.
        # The earlier version derived got/want correctly and then reduced the
        # table to LOCUS NAMES, so a relabelled mover and a collapsed from==to
        # target both survived. Shapes bind the mover id and every value.
        if r["surface"] == REQ_SURFACE:
            dyn = collections.defaultdict(set)
            for c3 in req_by_case.get(case2, []):
                for ref3, (st3, _s3) in gate_capture_requests(case2, c3).items():
                    dyn[ref3].add(st3)
            dynamic = any(len(v) > 1 for v in dyn.values())
            owed_r = owed_requests(case2, consumer2, dynamic)
            compare_owed(out, case2, consumer2, "SC-518/518a", owed_r,
                         held_shapes(carriers, case2, consumer2, ("SC-518", "SC-518a")))
        row_class = "digest" if is_digest else (
            "human-listing" if r["surface"] in LISTING else
            "requests" if r["surface"] == "helper:requests" else "opaque")
        # Every obligation must sit on a row its id is allowed to appear on. This is
        # the same exact-set rule as the multiset above, applied to the population
        # boundary instead of to one digest's row set.
        for o in carriers.get((case2, consumer2), []):
            # DECLARING WHO MAY USE EACH KNOWN MEMBER DOES NOT CLOSE THE SET UNLESS AN
            # UNDECLARED MEMBER FAILS. The map was consulted with .get() and skipped
            # when it missed, so an id absent from the declaration received no check at
            # all and could inflate the denominator anywhere. That is the same
            # closed-set defect as the unbound `undecidable` predicate, one level up —
            # third instance in this file, each time because a new declaration made the
            # DECLARED members safe and left the undeclared ones free. Adding a future
            # obligation id must add its population declaration in the same change.
            if o["obligation_id"] not in ID_POPULATION:
                fail(out, "UNKNOWN-ID", "%s/%s carries %s, which declares no population; "
                     "an id with no declaration is not checkable and must not exist"
                     % (case2, consumer2, o["obligation_id"]))
            elif row_class not in ID_POPULATION[o["obligation_id"]]:
                fail(out, "POPULATION-ID", "%s/%s carries %s on a %s row; that id belongs "
                     "to %s" % (case2, consumer2, o["obligation_id"], row_class,
                                "/".join(ID_POPULATION[o["obligation_id"]])))
        loci = {o["locus"] for o in carriers.get((case2, consumer2), [])
                if o["obligation_id"] == "SC-017o"}
        # THE COMPLETE MULTISET, over EVERY P1 row rather than only digests. Requiring that each exact
        # shape appears once proves the required rows EXIST and never that nothing
        # ELSE does — presence-verified, removals-unverified, wearing an arity
        # costume. An invented third SC-017o locus with its own logical address
        # passed every check while inflating the obligation denominator with a
        # fabricated comparison. The owed set is exactly two rows, so the check is
        # equality against exactly two rows.
        owed = collections.Counter({PRESENCE_ROW: 1, VALUE_ROW: 1}) if is_digest \
            else collections.Counter()
        got = collections.Counter(tuple(o[c] for c in FIXED)
                                  for o in carriers.get((case2, consumer2), [])
                                  if o["obligation_id"] == "SC-017o")
        if got != owed:
            for shape in sorted(set(got) - set(owed)):
                fail(out, "EXTRA-017o", "%s/%s carries an SC-017o row that is neither "
                     "owed shape: %s" % (case2, consumer2, shape))
            for shape, n in sorted(owed.items()):
                if got.get(shape, 0) != n:
                    fail(out, "DUPLICATE-017o" if got.get(shape, 0) > n
                         else "MISSING-017o-SHAPE",
                         "%s/%s carries %d row(s) equal to an owed SC-017o shape where "
                         "%d is owed: %s" % (case2, consumer2, got.get(shape, 0), n, shape))
        # The two loci-NAME checks that used to sit here are SUBSUMED by the multiset
        # equality above: a name present with the wrong shape, a name absent, and a
        # name that should not be there at all are now one comparison with three
        # named outcomes. Two checks answering half the question each is how the
        # arity bypass survived.

    # ---- 4b. `undecidable` implies UNSCORABLE, and the side file is not authority ----
    # A semantic target nobody can decide cannot be OBSERVED: the predicate and the
    # support must agree, or the row claims a scoring it does not have.
    # A NEW CLOSED-SET MEMBER IS OPEN UNTIL SOMETHING BINDS WHO MAY USE IT. Adding
    # `undecidable` to the predicate domain made it adoptable by ANY row, so a
    # mandatory scorable locus could launder itself into PARTIAL by claiming it —
    # measured: one SC-509d row flipped to undecidable/UNSCORABLE and the gate stayed
    # green. The predicate is now valid IFF the WHOLE row is the one shape it exists
    # for, and the completeness-value locus is valid only with that same whole shape,
    # so neither the predicate nor the locus can drift alone.
    for o in obls:
        shape = tuple(o[c] for c in FIXED)
        if o["predicate"] == "relational":
            if not (shape[0] == "SC-017g" and shape[1] == "digest"
                    and RELATIONAL_LOCUS.match(shape[2] or "")
                    and shape[6] == "OBSERVED" and shape[7] == "OBSERVED"):
                fail(out, "RELATIONAL-SHAPE", "%s/%s: `relational` is the SC-522 stopped "
                     "attention form only, and stays UNSCORABLE until the phase-4 scorer "
                     "implements and red-proves it; this row is %s"
                     % (o["case"], o["consumer"], shape))
        attn_ok = (ATTN_VALUE_LOCUS.match(shape[2] or "") is not None
                   and shape[:2] + shape[3:] == ATTN_VALUE_ROW[:2] + ATTN_VALUE_ROW[3:])
        if o["predicate"] == "undecidable" and shape != VALUE_ROW and not attn_ok:
            fail(out, "UNDECIDABLE", "%s/%s: `undecidable` is carried by exactly two row "
                 "shapes — the SC-017o completeness value and the SC-017g stopped-session "
                 "attention value; this row is %s" % (o["case"], o["consumer"], shape))
        elif o["obligation_id"] == "SC-017o" and shape[2] == PRESENCE_ROW[2] \
                and shape != PRESENCE_ROW:
            fail(out, "PRESENCE-SHAPE", "%s/%s: the completeness-presence locus must carry "
                 "its exact fixed row; this one is %s" % (o["case"], o["consumer"], shape))
        elif shape[2] == VALUE_ROW[2] and shape != VALUE_ROW:
            fail(out, "VALUE-SHAPE", "%s/%s: the completeness-value locus must carry its "
                 "exact fixed row; this one is %s" % (o["case"], o["consumer"], shape))

    # EXACTLY ONE of each SC-017o locus per digest, and no duplicated logical address
    # anywhere. Set membership is not exact shape: a duplicated value row and a garbage
    # `to` both passed before this. Distinct-count must equal the population — the same
    # key-that-is-not-a-key rule this table already learned one file over.
    addr = collections.Counter(tuple(o[c] for c in ADDRESS) for o in obls)
    for k, n in sorted(addr.items()):
        if n > 1:
            fail(out, "DUPLICATE-ADDRESS", "%s/%s %s %s appears %d times" % (k + (n,)))
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
    missing_fam = EXPECTED_FAMILIES - _families_seen
    extra_fam = _families_seen - EXPECTED_FAMILIES
    if missing_fam:
        fail(out, "FAMILY-SET", "the owed-multiset comparison never ran for %s — a check "
             "that does not run cannot fail, and green rc says nothing about it"
             % sorted(missing_fam))
    if extra_fam:
        fail(out, "FAMILY-SET", "an undeclared comparison family ran: %s; the declared set "
             "is the contract" % sorted(extra_fam))
    check_keyset(out, obls, quiet)
    if not quiet:
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
    # Path overrides exist so the red-proof can seed ISOLATED COPIES. A red-proof
    # that mutates the tracked evidence file to test its own checker exposes seeded
    # bytes to every concurrent reader, and this session has already shipped one.
    ap = argparse.ArgumentParser()
    ap.add_argument("--obl")
    ap.add_argument("--fresh")
    ap.add_argument("--inv")
    a = ap.parse_args()
    sys.exit(main(obl=a.obl, fresh=a.fresh, inv=a.inv)[0])
