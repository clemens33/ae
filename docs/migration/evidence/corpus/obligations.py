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
import calendar
import hashlib
import time
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


# ---- THE PER-INVOCATION CLOCK BINDING, RESOLVED FROM FIXED BYTES.
#
# Ruled 2026-08-24 (gpt56sol:colead, relayed by fable5:lead): the consumer-name
# prefix is NOT authority. Binding a clock by label shape is consumer-shape
# inference, the same class the C-locale binding was corrected for. The binding
# is a RECORDED fact, and the record is the pinned capture harness
# arms/A2/harness/arm-a2.sh — CORPUS-MANIFEST.tsv line 1730, class PROVENANCE,
# sha256 f9e5a07f..., 10468 bytes — whose lines 122-134 read:
#
#     local win now
#     for win in inside outside; do
#         [[ "$win" == inside ]] && now=$inside || now=$outside
#         ARM_FAKE_NOW=$now run_consumer "win_${win}_list_active"      ... list --active
#         ...                                                          (nine labels)
#     done
#
# ARM_FAKE_NOW=$now is set in the SAME STATEMENT that names the consumer, so the
# window an invocation ran at is recorded at the point of capture rather than
# inferred afterwards from what it was called. The nine suffixes below are
# TRANSCRIBED from lines 125-133; the two numeric clocks are PARSED from each
# case's own activity-window.txt (harness lines 112-113 derive them as the tg1
# events mtime + 60s and + 100000s against a documented 300s window).
#
# There is deliberately no startswith("win_inside_") inference anywhere: the map
# is a literal cross-product of transcribed constants, and it is RECONCILED with
# fixed INVOCATIONS.tsv in BOTH directions. A missing, extra, renamed, ambiguous
# or unmapped window consumer is FATAL. The harness sha256 is re-checked at
# generation, so a transcription whose source has moved fails instead of
# quietly citing bytes that no longer say what it claims.
CLOCK_HARNESS = "arms/A2/harness/arm-a2.sh"
CLOCK_HARNESS_SHA256 = \
    "f9e5a07f17865a488e65fb5e1e1c2e4a088bcd80bbdd35e9953b4201fd1c5932"
CLOCK_WINDOWS = ("inside", "outside")
CLOCK_SUFFIXES = ("list_active", "list_active_json", "list_busy", "active_all",
                  "all_active", "active_stopped", "stopped_active",
                  "needsattn_active", "active_needsattn")


def clock_windows(case):
    """The two recorded numeric clocks for a case, or None if it records none."""
    p = os.path.join(SRC, case, "activity-window.txt")
    if not os.path.exists(p):
        return None
    txt = open(p, encoding="utf-8").read()
    got = {}
    for win in CLOCK_WINDOWS:
        m = re.search(r"^%s_window_now=(\d+)" % win, txt, re.M)
        if not m:
            raise SystemExit("FATAL: %s records no %s_window_now" % (p, win))
        got[win] = int(m.group(1))
    if got["inside"] == got["outside"]:
        raise SystemExit("FATAL: %s records one clock twice, so the two windows "
                         "are indistinguishable and neither is bound" % p)
    return got


def clock_binding(pairs):
    """(case, consumer) -> (window, now_epoch) for every window invocation.

    `pairs` is every (case, consumer) row of fixed INVOCATIONS.tsv AS A LIST, not
    a set: a set would collapse a duplicated row and hide the ambiguity the
    ruling requires to fail.
    """
    h = os.path.join(SRC, CLOCK_HARNESS)
    sha = hashlib.sha256(open(h, "rb").read()).hexdigest()
    if sha != CLOCK_HARNESS_SHA256:
        raise SystemExit("FATAL: %s is sha256 %s, not the manifest-pinned %s; the "
                         "binding transcribed above no longer cites these bytes"
                         % (CLOCK_HARNESS, sha, CLOCK_HARNESS_SHA256))
    binding = {}
    for case in sorted({c for c, _ in pairs}):
        clocks = clock_windows(case)
        if clocks is None:
            continue
        for win in CLOCK_WINDOWS:
            for suf in CLOCK_SUFFIXES:
                binding[(case, "win_%s_%s" % (win, suf))] = (win, clocks[win])
    counts = collections.Counter(p for p in pairs if p[1].startswith("win_"))
    ambiguous = sorted(k for k, n in counts.items() if n > 1)
    if ambiguous:
        raise SystemExit("FATAL: %d window consumer(s) appear more than once in fixed "
                         "INVOCATIONS.tsv, so no invocation they name is uniquely "
                         "bound: %s" % (len(ambiguous), ambiguous[:4]))
    absent = sorted(set(binding) - set(counts))
    extra = sorted(set(counts) - set(binding))
    if absent:
        raise SystemExit("FATAL: %d harness-produced window consumer(s) missing from "
                         "fixed INVOCATIONS.tsv: %s" % (len(absent), absent[:4]))
    if extra:
        raise SystemExit("FATAL: %d window consumer(s) in fixed INVOCATIONS.tsv that "
                         "the pinned harness does not produce: %s"
                         % (len(extra), extra[:4]))
    return binding


CLOCK_POPULATION = "list_all_json"


ACTIVE_WINDOW_SECS = 300


def _epoch(ts):
    """An ISO-Z timestamp as epoch seconds, or None if it is not one."""
    try:
        return calendar.timegm(time.strptime(ts, "%Y-%m-%dT%H:%M:%SZ"))
    except (ValueError, TypeError):
        return None


def last_event_epoch(template, session):
    """The newest ae EVENT timestamp for a session, from the fixture bytes.

    EVENTS. NEVER THE FILE MTIME, AND NEVER THE FROZEN DOCUMENT'S OWN
    last_active_epoch. SC-017e names "an ae event within ~5min"; the frozen bash
    sourced the events.jsonl FILE MTIME instead, and in the A2 composite fixture
    the two differ by 930s — tg1's newest event is 16:12:57Z (1787242377) while
    the file's mtime is 16:28:27Z (1787243307), which is what put tg1 60s inside
    a 300s window it is really 990s outside of.
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


def clock_population(case):
    """Every session the case's own --all capture reports, with its status.

    --all does not filter on activity, so this names the inventory without
    consulting the predicate under test.
    """
    text = body(case, CLOCK_POPULATION)
    try:
        return json.loads(text).get("sessions", []) or []
    except ValueError:
        raise SystemExit("FATAL: %s has no parseable %s capture, so its session "
                         "inventory is unknown" % (case, CLOCK_POPULATION))


def render_set(names):
    """A stable rendering of a session set — sorted, so two derivations of the
    same set are byte-comparable, and never a bare count."""
    return "+".join(sorted(names)) if names else "empty"


def clock_active_set(case, now):
    """The successor's `--active` set at `now`, derived from EVENT bytes alone.

    NEVER SEEDED FROM THE FROZEN DOCUMENT. The first cut of this function started
    from the captured `--active` set and only ADDED sessions passing an event
    test, so it could never remove a frozen false positive: its prose said
    successor set, its algorithm was retained frozen set plus SC-524 additions.
    The frozen set is the mtime-sourced artifact under test, so seeding from it
    reproduced the defect with measurement's authority behind it.

    SC-521c: only status running or unknown can satisfy a live-scope predicate.
    SC-017e: an ae event within the window. SC-524: a FUTURE event counts as
    active, so the comparison is one-sided — an event newer than `now` is inside
    the window however far ahead it sits, which is the loud-false-positive
    direction the ruling chose. NO MTIME INPUT ANYWHERE.

    An unreachable case is FATAL: SC-521c widens `--active` to status `unknown`,
    and which sessions the successor would call unknown is not decidable from a
    v1 capture. No bound case is unreachable today; the guard exists so that the
    day one is, this stops rather than inventing a set.
    """
    if unreachable(case):
        raise SystemExit("FATAL: %s is unreachable, so SC-521c's `unknown` widening "
                         "makes its --active set underivable from a v1 capture" % case)
    template = template_of(case)
    live = [s for s in clock_population(case) if s.get("status") == "running"]
    resolved, active = 0, set()
    for s in live:
        t = last_event_epoch(template, s.get("name"))
        if t is None:
            continue
        resolved += 1
        if t > now or now - t <= ACTIVE_WINDOW_SECS:
            active.add(s.get("name"))
    if live and not resolved:
        raise SystemExit("FATAL: %s has %d running session(s) and event bytes for none "
                         "of them under template %r; an empty --active set here would "
                         "be a path bug wearing the right answer"
                         % (case, len(live), template))
    return active


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


# ---- SC-518 (identity) and SC-518a (ordering), over the FULL requests population.
#
# The predicate is run over every P1 requests invocation, not over a named list of
# shapes, so the conflict cases FALL OUT and the converse is proven rather than
# asserted. Contract blob pinned in FRESHNESS.tsv.
OPENINGS = ("ask", "review")
REQ_SURFACE = "helper:requests"


def _ident(e, side):
    """Routed / Display / Unassociated for one participant of one event.

    SC-518: routed compares when BOTH sides carry slot+session, display when
    NEITHER does, anything between matches nothing. PRESENT-BUT-EMPTY IS BETWEEN,
    not absent — ae@72c7293:4551 rejects an empty member with `-n` exactly as it
    rejects a missing one, which is why A7 c17/c18 are mixed and not display.
    """
    slot, sess = e.get(side + "_slot"), e.get(side + "_session")
    if slot and sess:
        return ("routed", slot, sess)
    if slot is None and sess is None:
        return ("display", e.get(side), None)
    return ("unassociated", None, None)


def _same(a, b):
    """Routed matches routed and display matches display. NOTHING else matches,
    INCLUDING unassociated to unassociated."""
    if a[0] != b[0] or a[0] == "unassociated":
        return False
    return a[1] is not None and a[1] == b[1] and a[2] == b[2]


def req_ledger(case):
    """Every request-lifecycle event of the case's producer session, in APPEND
    order, each tagged with its 1-based LEDGER LINE.

    SC-518a orders by ledger position and never by `ts`: a timestamp is a
    writer's clock and skew must not carry a terminal across an opening.
    """
    p = os.path.join(SRC, case, "case.txt")
    if not os.path.exists(p):
        return None
    txt = open(p, encoding="utf-8", errors="replace").read()
    tm = re.search(r"\btemplate=(\S+)", txt)
    sm = re.search(r"\bsession=(\S+)", txt)
    if not (tm and sm) or "/" not in tm.group(1):
        return None
    arm, variant = tm.group(1).split("/", 1)
    f = os.path.join(SRC, "templates", arm, "fixture-bytes", variant,
                     "sessions", sm.group(1), "events.jsonl")
    if not os.path.exists(f):
        return []
    out = []
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
            out.append(e)
    return out


def ruled_requests(case, events):
    """ref -> (status, summary, mover) under SC-518 + SC-518a.

    The SHOWN opening is the newest ask/review for the ref (events.md:112). A
    reply closes it only if it sits LATER IN THE LEDGER (SC-518a) and mirrors both
    ends (SC-518). `mover` names WHICH ROW rejected the reply frozen would have
    used — derived from the reason, never assigned per case: a reply that
    precedes its opening was rejected on ORDER, anything else on IDENTITY.
    """
    if any(e.get("action") == "cancel" for e in events):
        raise SystemExit(
            "FATAL: a cancel event reached the ruled derivation in %s. Cancel "
            "authorization is defined by NO contract row, SC-518a authorizes none, "
            "and the corpus measured ZERO cancels — so this is a new fixture that "
            "needs a ruling. A default here would be a ruling nobody made." % case)
    opening = {}
    for e in events:
        if e.get("action") in OPENINGS:
            ref = e.get("ref")
            if ref not in opening or e["_line"] > opening[ref]["_line"]:
                opening[ref] = e
    out = {}
    for ref, op in opening.items():
        replies = [e for e in events if e.get("action") == "reply" and e.get("ref") == ref]
        status, summary, mover = "pending", op.get("summary"), None
        for t in replies:
            if t["_line"] > op["_line"] and \
               _same(_ident(t, "actor"), _ident(op, "target")) and \
               _same(_ident(t, "target"), _ident(op, "actor")):
                status, summary = "replied", t.get("summary")
        if status == "pending" and replies:
            frozen_pick = max(replies, key=lambda e: e["_line"])
            mover = "SC-518a" if frozen_pick["_line"] < op["_line"] else "SC-518"
        out[ref] = (status, summary, mover)
    return out


def capture_requests(case, consumer):
    """(ref -> (status, summary)) as the capture actually rendered it."""
    text = body(case, consumer)
    rows = {}
    for line in text.splitlines()[1:]:
        f = line.split(None, 5)
        if len(f) >= 6:
            rows[f[2]] = (f[0], f[5])
    return rows


def dynamic_subject(case, consumers):
    """True when the case's OWN captures disagree about a ref's status.

    THE CLAIM IS DELIBERATELY THE SMALL ONE: disagreement proves the fixed
    template is NOT AN ENTITLED EXPECTED-VALUE SOURCE for both captures. It does
    NOT prove the producer moved during the run — consumer nondeterminism or
    another run-time input would look identical from here, and output
    disagreement is not a provenance oracle. The stronger attribution belongs to
    the D arm's own writer-barrier artifact, cited there and not derived here.
    """
    seen = collections.defaultdict(set)
    for c in consumers:
        for ref, (st, _) in capture_requests(case, c).items():
            seen[ref].add(st)
    return any(len(v) > 1 for v in seen.values())


# ---- STOPPED-SESSION FACTS: selection changes what is SHOWN, never what is TRUE.
#
# Measured: all 96 stopped-session entries in the P1 digests carry
# needs_attention/attention/attention_rank nulled, and every one that carries
# agents has EVERY agent state null. SC-509 mandates the agents[] and session
# attention fields, SC-017g defines the attention VALUE, SC-509c the reason —
# and SC-521c changes SELECTION, never the facts of a row already selected.
#
# THE RANK SCALE IS MEASURED, NOT DERIVED FROM THE PRIORITY SENTENCE. SC-017g
# reads "dead > stale > waiting-user > blocked > throttled > unanswered", and the
# captured scale runs the OTHER WAY: 1=unanswered rising to 6=dead, with 0 for no
# attention. Reading the sentence as the numbering gives blocked=4; the bytes say
# 3, across every P1 digest that carries an attention.
ATTN_RANK = {"unanswered": 1, "throttled": 2, "blocked": 3,
             "waiting-user": 4, "stale": 5, "dead": 6}


def session_events(template, session):
    """A session's own producer ledger, or None when it cannot be resolved."""
    if not template or "/" not in template:
        return None
    arm, variant = template.split("/", 1)
    p = os.path.join(SRC, "templates", arm, "fixture-bytes", variant,
                     "sessions", session, "events.jsonl")
    if not os.path.exists(p):
        return None
    out = []
    for line in open(p, encoding="utf-8", errors="replace"):
        line = line.strip()
        if not line:
            continue
        try:
            out.append(json.loads(line))
        except ValueError:
            continue
    return out


def stopped_facts(template, session):
    """(states, contributions, has_opening) from a session's fixed bytes.

    `states` is the NEWEST declared state per actor — the field SC-509 mandates,
    which is not the same question as attention: A1 tg1 declares working/done and
    owes state with NO attention at all, which is why the two classes move
    independently and neither can be derived from the other.
    """
    events = session_events(template, session)
    if events is None:
        return None
    states = {}
    for e in events:
        if e.get("action") == "state" and e.get("actor"):
            states[e["actor"]] = e.get("ref")
    # AN OPENING IS NOT AN UNANSWERED REQUEST. Only a request left PENDING under
    # the ruled closure derivation can ever become `unanswered`, so the question
    # is asked with SC-518 + SC-518a rather than by looking for an ask. A1 tg1
    # holds an ask that its own reply closes: it owes STATE and no attention, and
    # a coarse has-an-opening test would have called it undecidable.
    ledger = [dict(e, _line=n) for n, e in enumerate(events, 1)
              if e.get("action") in OPENINGS + ("reply", "cancel")]
    pending = any(v[0] == "pending"
                  for v in ruled_requests(session, ledger).values()) if ledger else False
    contrib = dict(alert_contributions(template, session))
    for actor, st in states.items():
        if st in AGENT_OWNED:
            contrib[actor] = st
    return states, contrib, pending


def stopped_attention(contrib, has_pending):
    """(needs_attention, attention, rank) or None when it is not decidable.

    An agent-owned contribution decides it outright. With none, the session is
    quiet ONLY if nothing could make it unanswered — and `unanswered` is rank 1,
    the LOWEST, so it can never outrank a contribution that exists. That makes
    the split exact rather than approximate: it decides the answer only where no
    contribution exists AND an opening does, and there SC-522's strictly-past
    threshold needs a clock these captures do not record. Undecidable there, and
    said so, rather than assumed quiet.
    """
    if contrib:
        best = max(contrib.values(), key=lambda c: ATTN_RANK.get(c, 0))
        return True, best, ATTN_RANK[best]
    if has_pending:
        return None
    return False, None, 0


def main():
    p1 = [r for r in csv.DictReader(open(INV, encoding="utf-8"), delimiter="\t")
          if r["phase"] == "P1"]
    # Built FIRST, so a broken clock binding stops the generator before it writes
    # a table whose window rows would silently carry the wrong clock.
    binding = clock_binding([(os.path.dirname(r["case"]), r["consumer"]) for r in p1])
    rows, seen, unproved = [], set(), []
    req_consumers = collections.defaultdict(list)
    for r in p1:
        if r["surface"] == REQ_SURFACE:
            req_consumers[os.path.dirname(r["case"])].append(r["consumer"])
    req_excluded, req_zero, req_live = set(), 0, 0
    stopped_unresolved, stopped_undecidable = set(), set()
    for r in p1:
        case, consumer = os.path.dirname(r["case"]), r["consumer"]
        seen.add((case, consumer))
        text = body(case, consumer)
        digest = '"schema_version"' in text
        listish = r["surface"] in LISTING
        incomplete = unreachable(case)

        if r["surface"] == REQ_SURFACE:
            # SC-518 / SC-518a over the FULL requests population. Every row is
            # accounted: scanned, proven-zero-owed, or excluded WITH ITS REASON.
            events = req_ledger(case)
            cap = capture_requests(case, consumer)
            if events is None:
                # No template declaration (a LIVE case, not cloned). It owes
                # nothing only if it renders no request rows — checked, because
                # "no source" and "nothing owed" are different answers.
                if cap:
                    raise SystemExit("FATAL: %s/%s renders %d request row(s) with no "
                                     "template to derive from" % (case, consumer, len(cap)))
                req_live += 1
            elif dynamic_subject(case, req_consumers[case]):
                # DYNAMIC-SUBJECT / FIXED-TEMPLATE-INAPPLICABLE. The case's own
                # captures disagree about a ref's status, which disqualifies the
                # fixed template as an entitled expected-value source for BOTH.
                # It does NOT establish why they disagree: run-time production,
                # consumer nondeterminism and another run-time input all look
                # identical from here. Disagreement is an entitlement
                # disqualifier, never a provenance oracle.
                req_excluded.add((case, consumer))
            elif not events:
                if cap:
                    raise SystemExit("FATAL: %s/%s renders %d request row(s) with no "
                                     "lifecycle events" % (case, consumer, len(cap)))
                req_zero += 1
            else:
                ruled = ruled_requests(case, events)
                for ref in sorted(cap):
                    if ref not in ruled:
                        continue
                    got_st, got_sum = cap[ref]
                    want_st, want_sum, mover = ruled[ref]
                    # STATUS AND SUMMARY ARE SEPARATE OBLIGATIONS, derived and
                    # addressed separately. Collapsing them would make a capture
                    # count readable as a locus count, which the contract row
                    # explicitly forbids.
                    for field, got, want in (("status", got_st, want_st),
                                             ("summary", got_sum, want_sum)):
                        if got == want:
                            continue
                        rows.append((case, consumer, mover or "SC-518", "stdout",
                                     "requests[%s].%s" % (ref, field), got, want,
                                     "equals", "OBSERVED", "OBSERVED",
                                     "%s: the reply frozen closed this with was rejected "
                                     "on %s; the shown opening is at ledger line of the "
                                     "newest ask/review for this ref and order is ledger "
                                     "position, never ts"
                                     % (ref, "ORDER (SC-518a)" if mover == "SC-518a"
                                        else "IDENTITY (SC-518)")))
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
            elif (case, consumer) in binding and doc is not None:
                # MEMBER 3 — the window invocations, at the clock the pinned capture
                # harness RECORDED for each one (see CLOCK_HARNESS above). Nothing
                # here reads the consumer's name for meaning; `binding` supplies the
                # clock and the label is only its key.
                win, now = binding[(case, consumer)]
                # `froz` is the CAPTURED set and is this obligation's `from`. The
                # successor set is derived INDEPENDENTLY from event bytes — never
                # seeded from `froz`, which is the mtime-sourced document under test.
                froz = [x.get("name") for x in (doc.get("sessions") or [])]
                succ = clock_active_set(case, now)
                rows.append((case, consumer, "SC-521c", "digest",
                             "sessions[] (set) @ now=%d" % now,
                             render_set(froz), render_set(succ), "equals",
                             "OBSERVED", "OBSERVED",
                             "ARM_FAKE_NOW=%d is the %s-window clock this invocation "
                             "was captured at, bound in the same statement that named "
                             "it (arm-a2.sh:125-133, sha256 %s); the successor set is "
                             "derived from the case's --all population and SC-017e EVENT "
                             "timestamps, never from this document and never from a file "
                             "mtime, with SC-524 futures counted active"
                             % (now, win, CLOCK_HARNESS_SHA256[:12])))
            template = template_of(case)
            # ---- STOPPED-SESSION FACTS. SC-521c changes SELECTION; a row already
            # selected keeps its facts. Each output is bound to the row that
            # governs IT — SC-509 for the agents[] state field, SC-509c for the
            # reason, SC-017g for the attention value — never batch-stamped to
            # whichever row happened to expose them.
            for sess in ([] if scope_empty else (doc or {}).get("sessions", []) or []):
                if sess.get("status") != "stopped":
                    continue
                name = sess.get("name")
                facts = stopped_facts(template, name or "")
                if facts is None:
                    stopped_unresolved.add((case, name))
                    continue
                states, contrib, pending = facts
                for ag in sess.get("agents") or []:
                    ref = ag.get("ref")
                    want = states.get(ref)
                    if want and ag.get("state") in (None, ""):
                        rows.append((case, consumer, "SC-509", "digest",
                                     "sessions[%s].agents[%s].state" % (name, ref),
                                     "null", want, "equals", "OBSERVED", "OBSERVED",
                                     "%s declares state %s in fixed producer bytes; the "
                                     "session being stopped changes what is SELECTED, "
                                     "never what the record says" % (ref, want)))
                    if contrib.get(ref) and ag.get("reason") in (None, ""):
                        rows.append((case, consumer, "SC-509c", "digest",
                                     "sessions[%s].agents[%s].reason" % (name, ref),
                                     "null", contrib[ref], "equals", "OBSERVED", "OBSERVED",
                                     "%s: fixed producer bytes name the owner and the "
                                     "agent-owned contribution %s" % (ref, contrib[ref])))
                attn = stopped_attention(contrib, pending)
                if attn is None:
                    stopped_undecidable.add((case, name))
                    rows.append((case, consumer, "SC-017g", "digest",
                                 "sessions[%s].attention (value)" % name,
                                 "null", "the most-actionable reason at capture time",
                                 "undecidable", "OBSERVED", "UNSCORABLE",
                                 "no agent-owned contribution exists and a request is "
                                 "PENDING under SC-518/SC-518a, so `unanswered` may apply "
                                 "— but SC-522's threshold is strictly-past and needs a "
                                 "clock these captures do not record. Unanswered is rank "
                                 "1, so this is the only shape it can decide"))
                else:
                    need, value, rank = attn
                    for locus, got, wanted in (
                            ("needs_attention", sess.get("needs_attention"), need),
                            ("attention", sess.get("attention"), value),
                            ("attention_rank", sess.get("attention_rank"), rank)):
                        if got == wanted:
                            continue
                        rows.append((case, consumer, "SC-017g", "digest",
                                     "sessions[%s].%s" % (name, locus),
                                     "null" if got is None else str(got).lower(),
                                     "null" if wanted is None else str(wanted).lower(),
                                     "equals", "OBSERVED", "OBSERVED",
                                     "SC-017g takes the MAX across agent contributions; "
                                     "the rank scale is MEASURED (1=unanswered rising to "
                                     "6=dead), not read off the priority sentence, which "
                                     "runs the other way"))
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
                    # ONE WRITER PER LOCUS, scoped to THIS branch only. A stopped
                    # session's reason rows are emitted by the stopped-facts path,
                    # which derives from producer bytes because the captured state is
                    # nulled; letting this branch also reach them duplicated the
                    # address on every stopped session an alert named. Skipping the
                    # whole SESSION instead was the over-broad first fix and silently
                    # dropped its SC-509b degraded rows — both caught by the gate,
                    # neither by reading the diff.
                    if sess.get("status") == "stopped":
                        continue
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
    print("  stopped-session facts: %d session(s) UNDECIDABLE attention (pending request, "
          "no clock), %d UNRESOLVED (no template/session bytes)"
          % (len(stopped_undecidable), len(stopped_unresolved)))
    for k in sorted(stopped_unresolved):
        print("    UNRESOLVED  %s / %s" % k)
    print("  requests population: %d excluded DYNAMIC-SUBJECT/FIXED-TEMPLATE-INAPPLICABLE, "
          "%d proven zero-owed (no lifecycle events), %d live/no-template proven zero-owed"
          % (len(req_excluded), req_zero, req_live))
    for k in sorted(req_excluded):
        print("    EXCLUDED  %s / %s" % k)
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
