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
import json, os, re, subprocess, sys, collections, tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
REPO_ROOT = os.path.normpath(os.path.join(HERE, "..", "..", "..", ".."))
SRC = os.path.normpath(os.path.join(HERE, "..", "batch-c-artifacts"))
INV = os.path.join(HERE, "INVOCATIONS.tsv")
OUT = os.path.join(HERE, "OBLIGATIONS.tsv")
UNPROVED = os.path.join(HERE, "SC-509C-UNPROVED.tsv")
FRESH = os.path.join(HERE, "FRESHNESS.tsv")
GAP = os.path.join(HERE, "UNOBSERVABLE-ADDED-ROSTER.tsv")
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


def atomic_write_text(path, text):
    """Publish one complete text artifact by same-directory atomic replacement."""
    directory = os.path.dirname(os.path.abspath(path))
    mode = (os.stat(path).st_mode & 0o777) if os.path.exists(path) else 0o644
    fd, tmp = tempfile.mkstemp(prefix=os.path.basename(path) + ".tmp.", dir=directory)
    try:
        os.fchmod(fd, mode)
        with os.fdopen(fd, "w", encoding="utf-8", newline="") as fh:
            fh.write(text)
        os.replace(tmp, path)
    except BaseException:
        try:
            os.close(fd)
        except OSError:
            pass
        try:
            os.unlink(tmp)
        except FileNotFoundError:
            pass
        raise

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
    """(session, owner) -> {contribution}, from the case's FIXED event bytes.

    KEYED BY SESSION AND ACTOR, never by actor alone. SC-509c is session+agent
    grained, and a case-level ledger keyed on the actor would let one carrier
    authorize EVERY same-ref agent in a composite digest — `fake:lead` appears under
    six sessions in A2/c01-filters, so an actor-only key fabricates five addresses
    the bytes never named. That is the key-collapse defect this locus was widened to
    escape. The carrier's session comes from FIXED provenance: case.txt's own
    `session=` line, which all 47 carrying cases declare (measured).

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
    cp = os.path.join(SRC, case, "case.txt")
    sm = re.search(r"\bsession=(\S+)",
                   open(cp, encoding="utf-8", errors="replace").read()) \
        if os.path.exists(cp) else None
    if not sm:
        return out                    # unbindable carrier authorizes nothing
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


# The documented stable event keys, from events.md's own table. SC-510e's rule is
# stated over "any documented stable key", so this list IS the rule's domain and a
# key missing from it silently narrows the rule.
EVENT_STABLE_KEYS = frozenset({
    "ts", "actor", "action", "target", "ref", "summary", "body_file",
    "actor_slot", "actor_session", "target_slot", "target_session",
})


def events_complete(template, session):
    """Is this session's event ledger COMPLETE, i.e. free of malformed records?

    SC-520 makes a skipped malformed COMPLETE line observable; SC-975b exempts a
    buffered UNTERMINATED TAIL, which is a different fact and must not be conflated
    with it. Measured across every template: exactly one session has a malformed
    complete line (G3/tg1) and exactly one has an unterminated tail (G8/tg1) — they
    are different sessions, so a predicate that fused them would be wrong on both.

    A state parsed out of a damaged ledger is NOT ESTABLISHED. The parsers here skip
    unparseable lines to stay robust, and that robustness is exactly what let six
    rows claim the successor would render `working`/`done` from a source SC-509b
    rules unreadable.
    """
    if not template or "/" not in template:
        return True
    arm, variant = template.split("/", 1)
    p = os.path.join(SRC, "templates", arm, "fixture-bytes", variant,
                     "sessions", session, "events.jsonl")
    if not os.path.exists(p):
        return True
    lines = open(p, encoding="utf-8", errors="replace").read().split("\n")
    unterminated = bool(lines) and lines[-1] != ""
    for n, line in enumerate(lines, 1):
        if not line.strip():
            continue
        if n == len(lines) and unterminated:
            continue                       # SC-975b: a buffered tail is not malformed
        seen = []
        try:
            json.loads(line, object_pairs_hook=lambda ps: (
                seen.extend(k for k, _ in ps), dict(ps))[1])
        except ValueError:
            return False                   # SC-520: a malformed COMPLETE record
        # SC-510e: a record carrying two members of any DOCUMENTED STABLE key is
        # skipped and counted, degrading the session by SC-520's path — RFC 8259
        # makes duplicate-name resolution non-interoperable and first/last-winner
        # selection is forbidden fabrication. SC-510f: duplicate UNKNOWN keys stay
        # INERT and never degrade, which is why the c09/c10 fixtures are tolerated
        # BY RULE rather than by anything the parser happens to do.
        counts = collections.Counter(seen)
        if any(c > 1 and k in EVENT_STABLE_KEYS for k, c in counts.items()):
            return False
    return True


# The members each loss KIND makes unreadable, taken from the successor's own gates
# rather than from a list someone maintained: src/digest.rs pushes `state` and
# `reason` only `if events_complete`, and `goal_set_epoch` under
# `self.knowledge.events.is_complete()`. `degraded` is AGGREGATE visibility and
# identifies nothing per member, so it is owed by either kind and neither kind's
# member list may be read off it.
EVENT_DERIVED_SESSION_MEMBERS = ("goal_set_epoch", "last_active_epoch",
                                 "attention", "attention_rank")
EVENT_DERIVED_AGENT_MEMBERS = ("state", "reason")


# ---- THE DECLARED SOURCE-TO-MEMBER MAPPING, AND THE ONLY COPY OF IT.
#
# THE AUTHORITY IS THE CONTRACT, NOT THE SERIALIZER. A table whose owed-member list
# is read off the product cannot police the product: if the serializer wrongly gates
# a member tomorrow, a product-derived mapping inherits the mistake and the table
# agrees with the defect BY CONSTRUCTION. That is the schema-serves-tool inversion
# pointing the other way, and the first draft of this block made exactly that error.
#
# Each member's source is stated by its own contract row:
#   SC-405b   mode, origin, work_dir, goal are META keys
#   SC-405f   goal_set_epoch is "not a meta key; the digest derives it from the
#             event stream"
#   SC-017e   last_active_epoch is an ae EVENT within the window
#   SC-017g   the attention triad's value is the MAX across agent contributions,
#             which are event-derived
#   SC-017h   declared state is event-derived ("an inexact or unreadable
#             event-derived state renders `unknown`")
#   SC-509c   a reason is an agent-owned contribution, event-derived
# and SC-509b supplies the rule that an unreadable optional fact OMITS while
# `degraded` stays AGGREGATE visibility identifying nothing per member.
#
# The successor's gates are CORROBORATION, checked separately by
# serializer_agrees(): a disagreement between this contract-derived mapping and what
# src/digest.rs actually gates is a FINDING against one of them, never a resolution.
# That check is what catches the next serializer regression; deriving from the gates
# would have made such a regression unobservable here.
#
# THE CHECKER READS THIS DECLARATION; IT DOES NOT CARRY A COPY. The previous control
# transcribed a four-member list and so was self-confirming — it caught a dropped ROW
# and could never catch a dropped MEMBER CLASS, which is precisely how a narrowed
# population greened. A transcribed expectation can only ever under-report.
# THE CLOSED LIST OF LOSS KINDS THIS DERIVATION RECOGNIZES. Every census correction
# in this slice came from a kind the predicate had not enumerated — meta-only, then
# malformed JSON only, then duplicate-known event keys, then duplicate meta keys. A
# predicate hides an unrecognized kind; a LIST shows it as a gap. Adding a kind here
# is the visible act; a kind absent from this tuple is not derived at all.
LOSS_KINDS = (
    "meta-absent",      # manifest: the meta is absent or UNREADABLE
    "meta-duplicate",   # SC-405a + SC-509b: a DUPLICATED documented meta key
    "events-skipped",   # SC-520 malformed COMPLETE record, or SC-510e duplicate
                        # documented event key (SC-510f keeps unknown dups inert)
)

# The member set each kind makes unreadable. THE GRAIN IS (KIND, MEMBER), not source
# alone: losing the whole meta takes out all four meta members, while a duplicate of
# ONE documented key takes out THAT KEY ONLY — the same duplicate-vs-unattributed
# asymmetry the serializer draws, where a duplicate of a specific key is always
# incomplete FOR THAT KEY while an unattributed fault is incomplete only where the
# value is also absent. `meta-duplicate` therefore carries no fixed list: its members
# are the keys actually duplicated.
LOSS_MEMBERS = {
    # `degraded` is the ONLY truly common member — it is the aggregate visibility
    # flag and every established loss raises it. `needs_attention` is NOT common:
    # it is owed only where the loss actually reaches the attention inputs. A
    # duplicated `goal` leaves the roster and the ledger intact, so false/null/0
    # stays EXACT quiet there and a partial-evidence row would be false. Relevance
    # is derived per kind, never assumed from `degraded` — which is exactly what
    # `degraded` identifies nothing per member means, applied to itself.
    "common": {"session": ("degraded",), "agent": ()},
    "meta-absent": {"session": ("needs_attention", "mode", "origin",
                                "work_dir", "goal"), "agent": ()},
    "meta-duplicate": {"session": (), "agent": ()},        # data-dependent, see below
    "events-skipped": {"session": ("needs_attention", "goal_set_epoch",
                                   "last_active_epoch", "attention",
                                   "attention_rank"),
                       "agent": ("state", "reason")},
}

# Scalars the PRODUCT treats as meta keys, named here so the comparison has a
# subject. This is not authority — it is the other side of the corroboration.
PRODUCT_META_SCALARS = frozenset({"ae_version", "goal", "branch", "mode", "origin",
                                  "work_dir", "status", "created", "uuid"})
DOCUMENTED_META_KEYS = frozenset({"mode", "origin", "work_dir", "goal", "branch",
                                  "status", "created", "uuid"})

# THE ANOMALY ROW UNIVERSE IS DERIVED FROM THE CONTRACT, NOT TRANSCRIBED. A new
# anomaly row widens this census automatically the moment it lands, which a
# hand-kept class list cannot do — and a kind no predicate constructs shows up as a
# row with no disposition rather than as an invisible hole. The previous version
# asserted `kinds <= LOSS_KINDS`, which was tautological: the same function built
# the set it then checked.
ANOMALY_HEADLINE = re.compile(
    r"\bmalformed|\bduplicate|\bdegrad|\bunreadable|\bskipped|\bunterminated|\binert\b",
    re.I)

# Every anomaly row's DISPOSITION, one of: a recognizer kind, `benign` with the
# reason the row itself gives, or `out-of-scope` with the surface it governs. A row
# derived from the contract with NO entry here is the gap this control exists to
# show. Zero-population classes stay listed with their zero measured — owed-zero
# discipline applied to kinds rather than to rows.
ANOMALY_DISPOSITION = {
    "SC-405i": "meta-absent",
    "SC-405e": "meta-duplicate",
    "SC-520": "events-skipped",
    "SC-510e": "events-skipped",
    "SC-405d": "benign: unknown meta keys are tolerated and never degrade",
    "SC-510f": "benign: duplicate UNKNOWN event keys stay inert",
    "SC-519": "benign: absent and zero-byte event logs are quiet, not degraded",
    "SC-975b": "benign: a buffered unterminated tail is not malformed",
    "SC-307": "out-of-scope: bash-era malformed-line behavior, not a digest member",
    "SC-210": "out-of-scope: delivery degradation, not a session member",
    "SC-211a": "out-of-scope: helper refusal mode",
    "SC-211b": "out-of-scope: helper refusal mode",
    "SC-211c": "out-of-scope: helper refusal mode",
    "SC-211d": "out-of-scope: helper refusal mode",
    "SC-805": "out-of-scope: archive inertness",
    "SC-830": "out-of-scope: compact digest-only degradation",
    "SC-1409b": "out-of-scope: telegram config",
    "SC-1409c": "out-of-scope: telegram config",
}


def contract_anomaly_rows():
    """Anomaly/loss rows, READ OUT OF THE PINNED CONTRACT BLOB."""
    txt = subprocess.run(["git", "cat-file", "blob", contract_blob()],
                         capture_output=True, text=True,
                         cwd=REPO_ROOT).stdout
    rows = re.findall(r"^\*\*(SC-[0-9a-z]+) — (.+?)\*\*", txt, re.M | re.S)
    return {rid for rid, head in rows if ANOMALY_HEADLINE.search(head)}


KIND_REASON = {
    "meta-absent": "the manifest proves its meta absent or unreadable",
    "meta-duplicate": "the meta carries a DUPLICATED documented key, which SC-405a "
                      "with SC-509b makes actual parse loss for that key",
    "events-skipped": "the event ledger carries a record the contract rules SKIPPED "
                      "— a malformed COMPLETE record under SC-520, or a duplicate "
                      "DOCUMENTED key under SC-510e",
}


def owed_loss_members(kinds, duplicated=()):
    """(session members, agent members) owed for a session, from the declaration.

    Restored after a text slice of mine swallowed it — the sixth time an unbounded
    span between two anchors removed more than it was aimed at. The rule I keep
    relearning: assert what the span CONTAINS, not merely that it starts and ends
    where expected.
    """
    session, agent = [], []
    for kind in ("common",) + tuple(sorted(kinds)):
        if kind == "meta-duplicate":
            session.extend(m for m in duplicated if m not in session)
            continue
        spec = LOSS_MEMBERS.get(kind)
        if not spec:
            continue
        session.extend(m for m in spec["session"] if m not in session)
        agent.extend(m for m in spec["agent"] if m not in agent)
    return tuple(session), tuple(agent)


def member_owner(member, kinds, duplicated=()):
    """Which ESTABLISHED kind makes THIS member unreadable.

    One union-level reason cannot truthfully explain each member when two kinds
    coincide: a session with both a lost meta and a skipped record would have had
    every member blamed on whichever branch the code happened to take. The authority
    text is bound to the member's OWN source, and `degraded` — owed by any loss — is
    bound to the whole established kind set instead of to one of them.
    """
    if member == "degraded":
        return None
    if "meta-duplicate" in kinds and member in duplicated:
        return "meta-duplicate"
    for kind in sorted(kinds):
        spec = LOSS_MEMBERS.get(kind, {})
        if member in spec.get("session", ()) or member in spec.get("agent", ()):
            return kind
    return None


# ---- THE AUTHORITY SIGNATURE, ruled 2026-08-25.
#
# A from==to row is indistinguishable from an unreasoned coincidence unless the
# bytes that carry its entitlement are FIXED. But binding full prose would make the
# gate transcribe paragraphs, so the ruling splits the field: a canonical
# machine-readable PREFIX that joins the owed-vs-held comparison, and a free prose
# tail that stays narrative. The gate constructs the SIGNATURE independently — an
# order of magnitude lighter than the prose — and a disagreement is a FINDING
# against one of them, never a resolution.
ENTITLEMENT_CLASSES = {
    "partial-evidence-from-readable-facts",   # a false->false indicator row
    "unreadable-member-omits",                # any member whose source failed
    "actual-loss-visible",                    # the degraded qualifier itself
    "temporary-presence-projection/value-unscored",   # SC-405g branch presence
}
# Terms the accepted contract RETIRED. Forbidden across the WHOLE authority field,
# prefix and prose alike — a retired term in narrative still teaches the wrong rule
# to the next reader, and this table's rows are read as the ruling.
RETIRED_AUTHORITY_TERMS = ("lower bound", "lower-bound", "monotone lower")


def authority_signature(owner, kinds, member, entitlement):
    """The canonical prefix. Field order is fixed so the string is comparable."""
    if entitlement not in ENTITLEMENT_CLASSES:
        raise SystemExit("FATAL: %r is not a declared entitlement class" % entitlement)
    return "SIG owner=%s kind=%s member=%s class=%s ::" % (
        owner, ",".join(sorted(kinds)) if kinds else "-", member, entitlement)


def parse_signature(authority):
    """(owner, kinds, member, class) or None when the prefix is absent/malformed."""
    # `member` is a LOCUS and a locus may contain spaces — `sessions[tg1].branch
    # (presence)` did, so a \S+ capture silently failed to parse 29 real rows and
    # reported them as unsigned. Anchor on the next field name instead of on
    # whitespace: the delimiter is `class=`, not a space.
    m = re.match(r"^SIG owner=(\S+) kind=(\S+) member=(.+?) class=(\S+) ::", authority or "")
    if not m:
        return None
    return (m.group(1), tuple(sorted(m.group(2).split(","))) if m.group(2) != "-" else (),
            m.group(3), m.group(4))


def serializer_agrees():
    """Does the SUCCESSOR's gating match the CONTRACT-derived mapping?

    CORROBORATION, NEVER AUTHORITY. A disagreement is a FINDING against one of them
    — possibly a serializer regression, possibly a contract row that has not caught
    up — and it is reported rather than resolved in either direction. Deriving the
    mapping FROM these gates would make a regression unobservable, because the table
    would agree with the defect by construction.

    This function was lost to one of my own text slices and rebuilt; the first
    version also compared only the EVENT-derived members, so it would have stayed
    silent on a meta-key disagreement. That silence was itself the finding, and the
    meta comparison below exists because of it.
    """
    src = os.path.join(REPO_ROOT, "src", "digest.rs")
    if not os.path.exists(src):
        return []
    text = open(src, encoding="utf-8", errors="replace").read()
    findings = []
    for member in LOSS_MEMBERS["events-skipped"]["session"] + \
            LOSS_MEMBERS["events-skipped"]["agent"]:
        if '"%s"' % member not in text:
            findings.append("serializer does not mention %r, which the contract makes "
                            "event-derived" % member)
    if "events_complete" not in text and "events.is_complete()" not in text:
        findings.append("serializer has no events-completeness gate at all")
    # THE META KEY UNIVERSE, both directions. A scalar the product recognizes but
    # this derivation does not is a duplicate the census cannot see; one this
    # derivation names and the product does not is a rule with no implementation.
    product_meta = set(re.findall(r"pub (\w+): Option<", text))
    unknown = sorted(product_meta & PRODUCT_META_SCALARS - DOCUMENTED_META_KEYS)
    for key in unknown:
        findings.append(
            "META-KEY DISAGREEMENT %r: the product recognizes it as a meta scalar "
            "(a duplicate emits DuplicateKey, degrades, and SC-405g then omits "
            "branch) while DOCUMENTED_META_KEYS excludes it and the pinned contract "
            "carries no source/provenance row for it. NOT RESOLVED HERE — reported "
            "as the disagreement it is; the ruling is authority-amendment versus "
            "presentation-only read path" % key)
    return findings


# ---- SC-017l / SC-017m, the settled DELTA three-way. Both keys turned 2026-08-25.
#
#   (1) frozen-omitted where unknown qualifies   m ADD + paired l   at one identity
#   (2) present-stopped in an all-like view      l ALONE            (membership stable)
#   (3) present-stopped in a --stopped view      m REMOVAL only     (no aligned candidate)
#
# THE PREMISE, NAMED SO IT IS CONTESTABLE ONCE RATHER THAN INHERITED FOREVER: case (2)
# owes no m row because DEFAULT PARITY already polices stable membership — in an
# all-like view the membership entitlement is unconditional in both worlds, so bytes
# AND semantics agree and a row recording nothing would be duplicate authority.
#
# THAT PREMISE IS A DEPENDENCY, NOT AN ASSUMPTION, and it is only half-satisfied
# today. Measured: parity is real on the HUMAN surface (human_project.compare fails
# on an unowned extra and on a missing retained session) and ABSENT on the DIGEST
# surface (no default-parity scorer exists in the tree). Of the 134 case-(2)
# occurrences, 48 ARE DIGEST and 86 are human — so 48 are unpoliced until the digest
# parity component lands. The delta rule is sound WHERE PARITY EXISTS, and parity is
# a component that must RUN on that surface, never an assumption about one.
#
# Arm (3) has population ZERO in this corpus. It is implemented with its zero stated
# rather than dropped — owed-zero applied to a ruled arm — and carries a synthetic
# control, because a ruled arm with no population is not proven by having been ruled.
VIEW_SELECTORS = ("--running", "--stopped", "--all")


def view_kind(argv):
    """default | all-like | stopped-only, from the invocation's own normalised argv."""
    winner = None
    for w in argv.split():
        if w in VIEW_SELECTORS:
            winner = w
    if winner == "--stopped":
        return "stopped-only"
    if winner in ("--all", "--running"):
        return "all-like"
    return "default"


def unknown_qualifies(kind):
    """SC-017m's own selection rule: --stopped shows only stopped, so an unknown
    candidate does not qualify there; default and --all include it."""
    return kind != "stopped-only"


SESSION_STATUS = frozenset({"running", "stopped"})
SESSION_HEADER = re.compile(r"^SESSION\s+STATUS\b")


def candidate_shown(text):
    """name -> status, read from the SESSION TABLE ITSELF.

    PROVENANCE, NOT INTERSECTION. This was a whole-body line scrape filtered by the
    caller's candidate universe, and colead is right that the filter only converts a
    silent misclassification into a loud stop: the scrape yields 36 keys that are not
    sessions at all, and three of them -- STATUS, pending, replied -- are LEGAL
    SESSION NAMES, so a durable candidate literally named `pending` would have let an
    unrelated requests-table row through the universe filter to shadow its real
    listing row or abort an otherwise valid corpus. A filter cannot fix that, because
    the two rows are indistinguishable ONCE BOTH ARE KEYS. Only the section they came
    from tells them apart.

    So the region is bound by the table's own grammar: the `SESSION STATUS ...`
    header, then its column-0 rows, ending at the first blank line or EOF (measured
    across 302 tables here: none contains a blank line, and nothing at column 0
    follows one). Indented lines are that row's continuations -- the goal line and
    the agent cells -- never session rows.
    """
    shown = {}
    if '"schema_version"' in text:
        try:
            for s in json.loads(text).get("sessions") or []:
                shown[s.get("name")] = s.get("status")
        except ValueError:
            pass
        return shown
    lines = text.splitlines()
    head = [i for i, l in enumerate(lines) if SESSION_HEADER.match(l)]
    if not head:
        return shown
    # THE BLANK LINE IS NOT THE BOUNDARY. Measured: with a blank separator the region
    # stopped correctly, but with the requests table butted straight onto the session
    # table it ran on and `pending` came back as `ask` -- the exact shadowing colead
    # named, reproduced. The region ends where the LAYOUT ends: a session row's
    # status token starts at or after the header's STATUS column (a name that
    # overflows its column pushes it right, never left), and another table's row has
    # its second token far to the left of it.
    offset = lines[head[0]].index("STATUS")
    for line in lines[head[0] + 1:]:
        if not line.strip():
            break
        if line[0].isspace():
            continue
        toks = [(m.start(), m.group()) for m in re.finditer(r"\S+", line)]
        if len(toks) < 2 or toks[1][0] < offset:
            break
        shown[toks[0][1]] = toks[1][1]
    return shown


def candidate_class(case, text):
    """candidate -> (class, frozen status). SC-017j keeps a live-only running row
    and a durable candidate of the same NAME as two identities, so a namesake never
    consumes the candidate — there is no flag here, deliberately: a seam that can
    select the known-wrong behaviour is an alternate path in shipped code."""
    universe = frozenset(loss_candidates(case))
    shown = candidate_shown(text)
    out = {}
    for c in sorted(universe):
        if c not in shown:
            out[c] = ("omitted", None)
        elif shown[c] == "running":
            out[c] = ("live-only-namesake", shown[c])
        else:
            # NO CATCH-ALL. The third branch used to be a bare `else`, so any status
            # that was not `running` became `aligned` -- a class whose population is
            # unbound. Measured, every one of the 230 is literally `stopped`, so the
            # branch has never been exercised by a second value and an unanalysed one
            # (exited, dead, unknown) would be silently called aligned and emit a row
            # claiming alignment for a state nobody analysed. The corpus is FROZEN and
            # digest-pinned, so an off-universe status cannot arise from data drift --
            # only from a moved pin or a changed parser, both of which must stop the
            # run rather than produce a row. Same repair shape as the selector legs:
            # bind the population to the declared universe instead of accepting a
            # remainder.
            assert shown[c] in SESSION_STATUS, (
                "unanalysed session status %r for candidate %r in %s -- the aligned "
                "class is declared over %s; extend the analysis, do not widen the "
                "branch" % (shown[c], c, case, sorted(SESSION_STATUS)))
            out[c] = ("aligned", shown[c])
    return out


def loss_candidates(case):
    """Durable candidates from the MANIFEST — the only source that can see one an
    output omitted, since absence cannot be read from a document lacking it."""
    mb = os.path.join(SRC, case, "manifest.before.tsv")
    if not os.path.exists(mb):
        return []
    rows = [l.rstrip("\n").split("\t") for l in open(mb, encoding="utf-8", errors="replace")]
    return sorted({r[-1].split("/")[2] for r in rows
                   if r[0] == "dir" and re.match(r"\./sessions/[^./][^/]*$", r[-1])})


def _manifest_meta_modes(case):
    """candidate -> recorded mode for meta files the manifest can see."""
    mb = os.path.join(SRC, case, "manifest.before.tsv")
    if not os.path.exists(mb):
        return {}
    rows = [line.rstrip("\n").split("\t")
            for line in open(mb, encoding="utf-8", errors="replace")]
    return {row[-1].split("/")[2]: row[1] for row in rows
            if row[0] == "file"
            and re.match(r"\./sessions/[^./][^/]*/meta$", row[-1])}


def candidate_recorded_selector(case, candidate):
    """Normalize one durable candidate's fixed recorded-server selector.

    Template-backed cases bind clone bytes through case.txt's template identity and
    clone fingerprint. Direct live cases record session+socket together in case.txt.
    Never substitute the ambient AE_TMUX_SERVER: SC-017j/k make that a different
    provenance even when its basename happens to match.
    """
    # A direct-live capture binds session and socket together in case.txt and has
    # stage-specific manifests rather than manifest.before.tsv.  Read that binding
    # before consulting the clone-manifest leg; otherwise c20's positive selector is
    # incorrectly called absent and its exact pane observation can never reach p.
    case_path = os.path.join(SRC, case, "case.txt")
    case_text = open(case_path, encoding="utf-8", errors="replace").read() \
        if os.path.exists(case_path) else ""
    direct = []
    for line in case_text.splitlines():
        sm = re.search(r"\bsession=(\S+)", line)
        sock = re.search(r"\bsocket=(\S+)", line)
        if sm and sock and sm.group(1) == candidate:
            direct.append(sock.group(1))
    if len(set(direct)) == 1:
        return "positive", direct[0]
    if len(set(direct)) > 1:
        return "ambiguous", None

    modes = _manifest_meta_modes(case)
    if candidate not in modes:
        return "meta-absent", None
    if modes[candidate] in ("000", "0", "100", "200"):
        return "mode-unusable", None
    meta = None
    template = template_of(case)
    if template and "/" in template:
        arm, variant = template.split("/", 1)
        path = os.path.join(SRC, "templates", arm, "fixture-bytes", variant,
                            "sessions", candidate, "meta")
        if os.path.exists(path):
            meta = open(path, encoding="utf-8", errors="replace").read()
    if meta is None:
        return "unresolved", None
    servers = re.findall(r"^tmux_server=(.*)$", meta, re.M)
    kinds = re.findall(r"^tmux_server_kind=(.*)$", meta, re.M)
    if len(servers) == 1 and servers[0] and len(kinds) <= 1 \
            and (not kinds or kinds[0] == "socket"):
        return "positive", servers[0]
    if not servers or (len(servers) == 1 and not servers[0]):
        return "missing", None
    return "ambiguous", None


def attempted_servers(case, consumer):
    """Recorded server spellings actually attempted by this frozen invocation."""
    path = os.path.join(SRC, case, "out", consumer + ".tmuxtrace")
    if not os.path.exists(path):
        return frozenset()
    attempted = set()
    for line in open(path, encoding="utf-8", errors="replace"):
        if not re.search(r"\b(list-sessions|list-panes|has-session)\b", line):
            continue
        m = re.search(r"\bAE_TMUX_SERVER=(\S+)", line)
        if m:
            attempted.add(m.group(1))
    return frozenset(attempted)


def topology_text(case, consumer):
    """The fixed topology snapshot paired with this invocation stage."""
    stage = consumer.split("/", 1)[0] if "/" in consumer else None
    candidates = ([os.path.join(SRC, case, "tmux.%s.txt" % stage)] if stage else [])
    candidates.append(os.path.join(SRC, case, "tmux.before.txt"))
    return next((open(p, encoding="utf-8", errors="replace").read()
                 for p in candidates if os.path.exists(p)), "")


def topology_query_state(case, consumer, server):
    """success | failed | unobserved for one attempted recorded server.

    A topology snapshot carries no socket label.  It can answer for a server only
    when the trace contains exactly one attempted server and it is that server.  A
    multi-server trace without per-server outcome artifacts remains unobserved rather
    than letting one success paint all candidates.
    """
    if attempted_servers(case, consumer) != frozenset({server}):
        return "unobserved"
    text = topology_text(case, consumer)
    if not text:
        return "unobserved"
    if "error connecting" in text or "no server running" in text:
        return "failed"
    return "success" if "## sessions" in text and "## panes" in text else "unobserved"


def candidate_causes(case, consumer):
    """candidate -> its OWN SC-017l cause tuple.

    ATTEMPT PRECEDES OUTCOME. A trace that never queried a candidate's positively
    recorded server is `selector-server-unattempted`, never a failed query. This is
    why only c20 varies in the current corpus: its direct-live case records the same
    socket its consumer trace queries. Template clones preserve their `/tmp/aecx/tpl`
    selectors while their consumers query per-case ambient sockets; a successful
    ambient snapshot cannot answer for those durable candidates.

    Per-candidate grain is essential in composite cases: one bad selector cannot
    paint another candidate whose own recorded server was attempted successfully.
    """
    attempts = attempted_servers(case, consumer)
    out = {}
    for candidate in loss_candidates(case):
        state, server = candidate_recorded_selector(case, candidate)
        if state == "meta-absent":
            out[candidate] = ("selector-meta-absent",)
        elif state == "mode-unusable":
            out[candidate] = ("selector-mode-unusable",)
        elif state != "positive":
            out[candidate] = ("selector-%s" % state,)
        elif server not in attempts:
            out[candidate] = ("selector-server-unattempted",)
        else:
            outcome = topology_query_state(case, consumer, server)
            if outcome == "failed":
                out[candidate] = ("selector-server-failed",)
            elif outcome == "unobserved":
                out[candidate] = ("selector-server-outcome-unobserved",)
    return out


# The surface column of INVOCATIONS.tsv is a CLOSED VOCABULARY, and the non-selecting
# half is independently corroborated: the Run1 runner classifies both helper surfaces
# as unimplemented in the successor CLI, so they render no selection view.
SURFACE_UNIVERSE = frozenset({"ae list", "ae ls", "helper:requests",
                              "helper:events-tail"})

def emit_unknown_family(case, consumer, argv, text, surface):
    """SC-017l and SC-017m rows for one invocation, under the settled delta split.

    Every row carries its CAUSE — required, never optional, because an optional
    field goes missing quietly and a required one cannot. The cause values are the
    ANALYSED ones (unreachable / selector-meta-absent / selector-mode-unusable), not
    a single collapsed label: those three cite different SC-017l clauses even though
    all three land on `unknown`.
    """
    # THE POPULATION IS THE LISTING SURFACES, and the function owns that rather than
    # trusting its caller to filter. Unguarded it emitted 90 pairs on helper:requests
    # and helper:events-tail -- documents with NO session-selection view at all, where
    # every durable candidate is trivially "not shown" and the omission branch fires
    # on nothing. Same class as every unbound population today: the code could not
    # see the surface, so it answered for all of them.
    # DECLARED SET, AND AN ASSERT OUTSIDE IT — reason2's pointer, and it is the same
    # repair as the aligned branch and the selector legs. `not helper-prefixed` is the
    # bare-else shape: it answers for surfaces nobody has classified. The surface
    # column is a closed vocabulary, so a fifth surface must STOP the run and be
    # classified deliberately, rather than be silently dropped by a filter -- silence
    # is how a population goes missing without anything disagreeing.
    assert surface in SURFACE_UNIVERSE, (
        "unclassified surface %r -- SC-017l/m owe rows on the listing surfaces %s and "
        "nothing on the non-selecting ones %s; classify it, do not let a filter answer"
        % (surface, sorted(LISTING), sorted(SURFACE_UNIVERSE - set(LISTING))))
    if surface not in LISTING:
        return []
    cause_by_candidate = candidate_causes(case, consumer)
    if not cause_by_candidate:
        return []
    # SURFACE IS PART OF THE IDENTITY, ruled. The first cut hardcoded stream=stdout
    # and the human loci for BOTH surfaces, so 104 l and 56 m rows sat on JSON
    # documents wearing a human address -- two surfaces collapsed into one identity,
    # in the family whose cause field exists to stop exactly that collapse. The
    # vocabulary is the table's established convention, not a new invention: SC-509b
    # already addresses JSON members as digest / sessions[<name>].<member>, and the
    # JSON membership row is object PRESENCE, the shape the degraded ABSENT->true
    # rows already use.
    if '"schema_version"' in text:
        stream = "digest"
        loc_value, loc_member = "sessions[%s].status", "sessions[%s]"
    else:
        stream = "stdout"
        loc_value, loc_member = "candidate[%s].status", "view.members[%s]"
    kind = view_kind(argv)
    qualifies = unknown_qualifies(kind)
    rows = []
    for cand, (cls, frozen) in sorted(candidate_class(case, text).items()):
        causes = cause_by_candidate.get(cand, ())
        if not causes:
            continue
        support = ("OBSERVED" if any(c in ("selector-meta-absent",
                                           "selector-mode-unusable") for c in causes)
                   else "UNSCORABLE")
        cause = ",".join(causes)
        loc_l = loc_value % cand
        loc_m = loc_member % cand
        if cls in ("omitted", "live-only-namesake") and qualifies:
            # (1) the candidate does not appear and unknown qualifies here: the
            # membership ADD and its paired value row, AT ONE IDENTITY.
            rows.append((case, consumer, "SC-017l", stream, loc_l, "ABSENT",
                         "unknown", "equals", "OBSERVED", support,
                         authority_signature("SC-017l", causes, loc_l,
                                             "unreadable-member-omits")
                         + " cause=%s class=%s: the durable candidate is not "
                         "represented%s, and SC-017l yields unknown — never stopped, "
                         "never absence"
                         % (cause, cls,
                            "; a live-only running row carries its NAME but SC-017j "
                            "keeps them two identities and the namesake does not "
                            "consume it" if cls == "live-only-namesake" else "")))
            rows.append((case, consumer, "SC-017m", stream, loc_m, "ABSENT",
                         "present", "equals", "OBSERVED", support,
                         authority_signature("SC-017m", causes, loc_m,
                                             "unreadable-member-omits")
                         + " cause=%s view=%s: unknown qualifies for this view, so "
                         "the candidate joins its exact selected set — the membership "
                         "half of the pair" % (cause, kind)))
        elif cls == "aligned" and qualifies:
            # (2) present and still selected: membership is STABLE, so no m row —
            # default parity polices stable membership and a row recording nothing
            # would be duplicate authority. NOTE the dependency: parity is a
            # component that must RUN on this surface, and it is measured present on
            # human and ABSENT on digest, where 48 of these sit.
            rows.append((case, consumer, "SC-017l", stream, loc_l,
                         str(frozen).lower(), "unknown", "equals", "OBSERVED", support,
                         authority_signature("SC-017l", causes, loc_l,
                                             "partial-evidence-from-readable-facts")
                         + " cause=%s class=aligned view=%s: the candidate is present "
                         "and still selected, so membership is stable and only its "
                         "VALUE moves — l's own grain" % (cause, kind)))
        elif cls == "aligned" and not qualifies:
            # (3) present in a --stopped view where unknown does NOT qualify: the
            # successor excludes it, so an m REMOVAL and no l row, since no aligned
            # candidate remains in that view. POPULATION ZERO in this corpus —
            # implemented with its zero stated, and controlled synthetically,
            # because a ruled arm with no population is not proven by the ruling.
            rows.append((case, consumer, "SC-017m", stream, loc_m, "present",
                         "ABSENT", "equals", "OBSERVED", support,
                         authority_signature("SC-017m", causes, loc_m,
                                             "unreadable-member-omits")
                         + " cause=%s view=stopped-only: unknown does not qualify "
                         "where only stopped shows, so the successor excludes the "
                         "candidate — a membership REMOVAL with no l row, because no "
                         "aligned candidate remains in this view" % cause))
    return rows


def added_roster_gap_population(p1):
    """Added-session occurrences whose h/r roster members fixed bytes cannot name.

    Generator-side traversal uses `candidate_class`; the gate derives the same set
    through its independent `candidate_representation` path.
    """
    members = set()
    for row in p1:
        if row["surface"] not in LISTING:
            continue
        case, consumer = os.path.dirname(row["case"]), row["consumer"]
        if not unknown_qualifies(view_kind(row["normalised_argv"])):
            continue
        text = body(case, consumer)
        if '"schema_version"' in text:
            continue
        causes = candidate_causes(case, consumer)
        for candidate, (cls, _status) in candidate_class(case, text).items():
            if candidate in causes and cls in ("omitted", "live-only-namesake"):
                members.add((case, consumer, candidate))
    return members


def parse_human_agent_row(line, status):
    """Parse one frozen human agent subrow at the printer's actual schema.

    Running and stopped rows are DIFFERENT records in frozen ae@72c7293:

      running: two-space indent, ref, session_id, declared state, health
      stopped: two-space indent, ref, session_id

    The running health cell is trailing and may be empty.  The stopped row has no
    state or health cell at all; its session_id is still roster evidence and must
    never be relabelled as either value.  Return (ref, session_id, state, health),
    using None for fields the stopped printer does not emit and "" for an emitted
    but empty running health cell.
    """
    if status == "stopped":
        m = re.fullmatch(r"  (\S+:\S+) {2,}(\S+) *", line)
        return (m.group(1), m.group(2), None, None) if m else None
    if status == "running":
        m = re.fullmatch(r"  (\S+:\S+) {2,}(\S+) {2,}(\S+) {2,}(\S*) *", line)
        return (m.group(1), m.group(2), m.group(3), m.group(4)) if m else None
    return None


def rendered_short_session_id(value):
    """Frozen `_parse_agent_entry`'s stable rendered session-id identity field."""
    if value in (None, "", "pending", "-"):
        return "-"
    return value[:8]


def rendered_roster(text):
    """(session, agent, short sid, health) rows, associated by human nesting.

    An agent row is INDENTED under the session row it belongs to, so the association
    is positional and must be read that way -- collecting agent lines without tracking
    which session row precedes them would produce a roster with no owner, which is the
    nameless-locus shape the legacy r rows died of.  The rendered short session_id is
    a retained stable identity field on BOTH printer schemas and is fixed before the
    independently mutable state and health cells are read.
    """
    return [(session, agent, sid, health)
            for session, _status, agent, sid, _state, health
            in rendered_agent_rows(text)]


def rendered_agent_rows(text):
    """Human agent rows with enclosing session/status and all printer fields.

    Return `(session, status, ref, short_sid, state, health)`.  The status-aware
    printer parser is the only column reader; h and r consume named tuple fields
    from this literal grammar instead of independently re-parsing columns.
    """
    out = []
    if '"schema_version"' in text:
        return out                  # h/r are human-only; digest families own JSON
    lines = text.splitlines()
    head = [i for i, l in enumerate(lines) if SESSION_HEADER.match(l)]
    if not head:
        return out
    offset = lines[head[0]].index("STATUS")
    session = status = None
    for line in lines[head[0] + 1:]:
        if not line.strip():
            break
        toks = [(m.start(), m.group()) for m in re.finditer(r"\S+", line)]
        if not line[0].isspace():
            if len(toks) < 2 or toks[1][0] < offset:
                break
            session, status = toks[0][1], toks[1][1]
            continue
        agent = parse_human_agent_row(line, status)
        if agent and session:
            out.append((session, status, agent[0],
                        rendered_short_session_id(agent[1]), agent[2], agent[3]))
    return out


def health_multiset(markers):
    """An ORDER-FREE canonical rendering of a class's health values.

    A string, deliberately, because the obligation table's columns are text and a
    Counter's repr is not stable across readers. Sorted by value so the SAME multiset
    always renders identically no matter what order the rows arrived in -- which is
    the whole point: exchanging two entries inside the class must produce byte-equal
    output, or the emission would key an obligation on a binding the human bytes do
    not carry.
    """
    return " ".join("%s x%d" % (v, n)
                    for v, n in sorted(collections.Counter(markers).items()))


def _candidate_support(causes):
    """Whether fixed artifacts contain the deciding candidate fact."""
    return ("UNSCORABLE" if any(c in ("selector-server-unattempted",
                                       "selector-server-outcome-unobserved")
                                for c in causes)
            else "OBSERVED")


def fixed_roster_slots(case, consumer, session):
    """Fixed ``(slot, rendered ref, short sid)`` roster records for one session.

    SC-602 makes ``@ae_slot`` identity and ``@ae_agent`` display-only.  The live
    c20 arm froze its stage roster separately; template-backed cases froze the same
    ``agent.<slot>=<ref>:<session_id>`` records in meta.  Pane display text is never
    used to infer a slot.
    """
    stage = consumer.split("/", 1)[0] if "/" in consumer else None
    paths = ([os.path.join(SRC, case, "roster.%s.txt" % stage)] if stage else [])
    template = template_of(case)
    if template and "/" in template:
        arm, variant = template.split("/", 1)
        paths.append(os.path.join(SRC, "templates", arm, "fixture-bytes", variant,
                                  "sessions", session, "meta"))
    path = next((p for p in paths if os.path.exists(p)), None)
    if path is None:
        return []
    roster = []
    for line in open(path, encoding="utf-8", errors="replace"):
        key, sep, value = line.rstrip("\n").partition("=")
        if not sep or not key.startswith("agent.") or key.startswith("agent_bin."):
            continue
        slot = key[len("agent."):]
        ref, sid_sep, sid = value.rpartition(":")
        if slot and sid_sep and ref:
            roster.append((slot, ref, rendered_short_session_id(sid)))
    return roster


def topology_observation(case, consumer, server):
    """One successful server snapshot as `(sessions, panes)` or its query state.

    `panes[session]` contains `(slot, rendered_ref, command, pane_dead)`.  Slot is
    retained before display is inspected: SC-602 makes ``@ae_slot`` identity while
    ``@ae_agent`` is display-only.  The two fixed capture grammars are explicit:
    template captures put session first; live captures put pane id first and session
    last.  No basename, ambient, or display-ref join occurs.
    """
    state = topology_query_state(case, consumer, server)
    if state != "success":
        return state, set(), {}
    sessions, panes, section, sole_session_panes = set(), {}, None, []
    for line in topology_text(case, consumer).splitlines():
        if line == "## panes":
            section = "panes"
            continue
        if line == "## sessions":
            section = "sessions"
            continue
        if line.startswith("## "):
            section = None
            continue
        if not line:
            continue
        fields = line.split("|")
        if section == "sessions" and fields:
            sessions.add(fields[0])
        elif section == "panes":
            if fields[0].startswith("%") and len(fields) == 7:
                session, ref, slot, command, pane_dead = (
                    fields[-1], fields[1], fields[2], fields[3], fields[5])
            elif fields[0].startswith("%") and len(fields) == 6:
                # A4's fixed live snapshot omits session and pane_dead from each
                # pane row. Its sessions section is singleton, so association is
                # still exact; health stays unknown because SC-017s requires the
                # absent pane_dead field for the positive alive route.
                sole_session_panes.append((fields[2], fields[1], fields[3], None))
                continue
            elif len(fields) == 5:
                session, ref, slot, command, pane_dead = (
                    fields[0], fields[2], fields[3], fields[4], None)
            else:
                continue
            panes.setdefault(session, []).append((slot, ref, command, pane_dead))
    if len(sessions) == 1 and sole_session_panes:
        panes.setdefault(next(iter(sessions)), []).extend(sole_session_panes)
    return state, sessions, panes


def agent_health_target(case, consumer, session, slot, cause_by_candidate):
    """`(target, causes, support)` for one exact roster slot, or None.

    Candidate liveness unknown routes to q before pane inspection.  The p route is
    reachable only through a positive recorded selector, an actual attempt against
    that exact spelling, and its successful session/pane snapshot.
    """
    causes = cause_by_candidate.get(session)
    if causes:
        return "unambiguous unknown", causes, _candidate_support(causes)

    selector_state, server = candidate_recorded_selector(case, session)
    if selector_state != "positive":
        return None                    # neither durable nor direct-live identity
    if server not in attempted_servers(case, consumer):
        causes = ("selector-server-unattempted",)
        return "unambiguous unknown", causes, _candidate_support(causes)
    state, sessions, panes = topology_observation(case, consumer, server)
    if state != "success":
        causes = (("selector-server-failed",) if state == "failed"
                  else ("selector-server-outcome-unobserved",))
        return "unambiguous unknown", causes, _candidate_support(causes)
    if session not in sessions:
        return "dead", ("exact-session-absent",), "OBSERVED"

    # A human row names a display ref, not its pane identity.  Without fixed roster
    # bytes joining that ref+sid class to a slot, a successful session snapshot does
    # not authorize a pane target.  In particular, a pane displaying the same ref
    # under another slot is not evidence about this roster member.
    if slot is None:
        return "unambiguous unknown", ("pane-slot-unbound",), "OBSERVED"
    observed = panes.get(session, [])
    matches = [p for p in observed if p[0] == slot]
    usable = all(bool(re.fullmatch(r"\S+", pane_slot or ""))
                 for pane_slot, _ref, _command, _dead in observed)
    unambiguous = all(n == 1 for n in collections.Counter(
        pane_slot for pane_slot, _ref, _command, _dead in observed).values())
    if not matches:
        if usable and unambiguous:
            return "dead", ("exact-pane-absent",), "OBSERVED"
        return "unambiguous unknown", ("pane-association-unusable",), "OBSERVED"
    if len(matches) != 1:
        return "unambiguous unknown", ("pane-association-ambiguous",), "OBSERVED"
    _slot, _display_ref, command, pane_dead = matches[0]
    if pane_dead == "1":
        return "dead", ("pane-dead",), "OBSERVED"
    if pane_dead == "0" and command not in ("", "bash", "zsh", "fish", "sh", "dash"):
        return "alive", ("pane-alive",), "OBSERVED"
    return "unambiguous unknown", ("pane-live-predicate-unproved",), "OBSERVED"


def declared_state_for(case, session, agent):
    """Contract-required state from the candidate's own fixed producer ledger."""
    template = template_of(case)
    if not events_complete(template, session):
        return "unknown", ("events-skipped",)
    facts = stopped_facts(template, session)
    states = facts[0] if facts is not None else {}
    value = states.get(agent, "-")
    if value in (None, ""):
        return "unknown", ("event-state-inexact",)
    return value, ("producer-ledger-exact",)


def emit_agent_state(case, consumer, text, surface):
    """SC-017h at fixed `(session, rendered ref, short sid)` class grain."""
    assert surface in SURFACE_UNIVERSE, "unclassified surface %r" % surface
    if surface not in LISTING or '"schema_version"' in text:
        return []
    classes = {}
    for session, _status, agent, sid, _state, _health in rendered_agent_rows(text):
        classes.setdefault((session, agent, sid), [])
    for session, _status, agent, sid, state, _health in rendered_agent_rows(text):
        source = "ABSENT" if state is None else state
        target, causes = declared_state_for(case, session, agent)
        classes[(session, agent, sid)].append((source, target, causes))

    rows = []
    for (session, agent, sid), values in sorted(classes.items()):
        sources = [v[0] for v in values]
        targets = [v[1] for v in values]
        if collections.Counter(sources) == collections.Counter(targets):
            continue
        if len(values) == 1:
            locus = "agents[%s:%s:%s].state" % (session, agent, sid)
            frm, to = sources[0], targets[0]
        else:
            locus = "agents[%s:%s:%s](class).state" % (session, agent, sid)
            frm, to = health_multiset(sources), health_multiset(targets)
        causes = tuple(sorted({c for _s, _t, cs in values for c in cs}))
        rows.append((case, consumer, "SC-017h", "stdout", locus, frm, to,
                     "equals", "OBSERVED", "OBSERVED",
                     authority_signature("SC-017h", causes, locus,
                                         "partial-evidence-from-readable-facts")
                     + " source state follows the status-specific frozen printer; "
                     "target state follows the candidate's own fixed producer "
                     "ledger, at exact class multiplicity"))
    return rows


def emit_agent_health(case, consumer, text, surface):
    """SC-017r at the grain ruled in contract blob 2c832b31, read from those
    bytes rather than from any summary of them.

    IDENTITY IS A FIXED PRE-SUCCESSOR HUMAN PROJECTION and value bytes are no part of
    it: session identity + rendered agent ref + rendered short session_id, the stable
    identity fields the human projection RETAINS, EXCLUDING health and every
    independently mutable state, reason and attention cell. Keying on those would let
    a value edit silently re-partition the population -- which is why the rejected
    v2's "retained non-health tuple" was a mutable key and this is not.

    THE CLASS IS FIXED BEFORE ANY HEALTH VALUE IS READ. The partition below is built
    from the projection alone and only then are the values collected, so a health
    difference can never move an agent between classes.

    Cardinality ONE is identity-addressed. Cardinality >1 owes an ORDER-FREE COUNT of
    source AND per-slot target health values at EXACT multiplicity: target slots can
    legitimately disagree even though their human ref+SID class collides. DROP fails,
    WRONG MULTIPLICITY fails, and EXCHANGE is not observed and therefore neutral -- a
    consequence of the evidence, not a tolerance granted. Neither a LIST (which
    invents an order the bytes do not carry) nor a SET (which drops a real agent) is
    permitted; both make their totals agree with something.

    HUMAN-ONLY. Digest agent multiplicity and health stay with the JSON rows and
    default parity; the frozen digest corroborates that no cross-surface escape
    recovers the lost identity, and corroborating evidence is not a scored surface.
    """
    assert surface in SURFACE_UNIVERSE, "unclassified surface %r" % surface
    if surface not in LISTING or '"schema_version"' in text:
        return []
    cause_by_candidate = candidate_causes(case, consumer)

    # STEP 1 -- PARTITION BY THE PROJECTION, health not yet consulted.
    classes = {}
    for session, agent, sid, _marker in rendered_roster(text):
        classes.setdefault((session, agent, sid), [])
    # STEP 2 -- only now collect the values into their already-settled classes.
    for session, agent, sid, marker in rendered_roster(text):
        # The frozen printers distinguish two source facts: stopped rows do not
        # emit a health cell at all, while running rows emit one whose trailing
        # bytes may be empty.  Preserve that semantic split in the table.
        classes[(session, agent, sid)].append(
            "ABSENT" if marker is None else (marker or "blank")
        )

    rows = []
    for (session, agent, sid), markers in sorted(classes.items()):
        # Join the fixed human class back to its fixed roster slots BEFORE reading
        # pane values.  A class may contain several slots with different targets;
        # calling the target function once per display ref and repeating that answer
        # would erase the heterogeneous multiset SC-017p/r explicitly preserve.
        roster_slots = [slot for slot, roster_ref, roster_sid
                        in fixed_roster_slots(case, consumer, session)
                        if (roster_ref, roster_sid) == (agent, sid)]
        slots = roster_slots if len(roster_slots) == len(markers) \
            else [None] * len(markers)
        targets = [agent_health_target(case, consumer, session, slot,
                                       cause_by_candidate)
                   for slot in slots]
        if any(target is None for target in targets):
            continue
        target_values = [target[0] for target in targets]
        causes = tuple(dict.fromkeys(cause for target in targets
                                     for cause in target[1]))
        support = ("UNSCORABLE" if any(target[2] == "UNSCORABLE"
                                       for target in targets) else "OBSERVED")
        # This table binds PRESENTATION divergence, not only semantic liveness. The
        # frozen carrier `blank` means alive but renders an empty cell; successor
        # literal `alive` is different observable output and remains an obligation.
        # Compare carrier bytes to target presentation directly — never normalize
        # blank->alive or !->dead before deciding whether a row exists.
        if collections.Counter(markers) == collections.Counter(target_values):
            continue
        if len(markers) == 1:
            locus = "agents[%s:%s:%s].health" % (session, agent, sid)
            frm, to = markers[0], target_values[0]
            note = ("the projection establishes the roster association at this "
                    "identity, so health is owed at it")
        else:
            locus = "agents[%s:%s:%s](class).health" % (session, agent, sid)
            frm = health_multiset(markers)
            to = health_multiset(target_values)
            note = ("%d agents render under one display name and short session_id "
                    "and the human bytes carry no occurrence identity for them, so "
                    "the owed fact is an "
                    "ORDER-FREE COUNT at exact multiplicity: dropping one fails, the "
                    "wrong multiplicity fails, exchanging two is NOT OBSERVED and "
                    "therefore neutral -- the collision stays a frozen defect and "
                    "this grain is not its licence" % len(markers))
        rows.append((case, consumer, "SC-017r", "stdout", locus, frm, to, "equals",
                     "OBSERVED", support,
                     authority_signature("SC-017r", causes, locus,
                                         "partial-evidence-from-readable-facts")
                     + " cause=%s: SC-017p/q derives this target from the "
                     "candidate's recorded server, exact session and exact pane "
                     "observation; the human cell stays non-silent and three-way "
                     "distinguishable; %s" % (",".join(causes), note)))
    return rows


def loss_class_census():
    """Every contract anomaly row, and what this derivation does with it.

    The check the tautological assertion pretended to be. The row set is DERIVED
    from the pinned contract, so a new anomaly row appears here without anyone
    remembering to add it, and any row without a disposition is reported.
    """
    gaps = []
    derived = contract_anomaly_rows()
    for rid in sorted(derived - set(ANOMALY_DISPOSITION)):
        gaps.append("UNDISPOSED-ANOMALY-ROW %s: the contract defines an anomaly row "
                    "that no recognizer derives and nothing declares benign or "
                    "out-of-scope" % rid)
    for rid in sorted(set(ANOMALY_DISPOSITION) - derived):
        if ANOMALY_DISPOSITION[rid].startswith("benign") and rid == "SC-975b":
            continue          # cited by the predicate, headline carries no keyword
        gaps.append("STALE-DISPOSITION %s: disposed here but no longer an anomaly row "
                    "in the pinned contract" % rid)
    kinds = {v for v in ANOMALY_DISPOSITION.values()
             if not v.startswith(("benign", "out-of-scope"))}
    for k in sorted(kinds - set(LOSS_KINDS)):
        gaps.append("UNIMPLEMENTED-KIND %s: a row is disposed to it and no predicate "
                    "constructs it" % k)
    return gaps


def duplicated_meta_keys(case, session):
    """Documented meta keys appearing MORE THAN ONCE, from the fixed bytes.

    SC-405a's metadata-anomaly semantics with SC-509b make a duplicated documented
    key ACTUAL parse loss for THAT KEY: no row defines duplicate-member precedence,
    so first/last-winner selection would be fabrication. An UNKNOWN duplicated key is
    a tolerated control and never appears here.
    """
    p = os.path.join(SRC, case, "case.txt")
    if not os.path.exists(p):
        return ()
    m = re.search(r"\btemplate=(\S+)", open(p, encoding="utf-8", errors="replace").read())
    if not m or "/" not in m.group(1):
        return ()
    arm, variant = m.group(1).split("/", 1)
    f = os.path.join(SRC, "templates", arm, "fixture-bytes", variant,
                     "sessions", session, "meta")
    if not os.path.exists(f):
        return ()
    keys = [l.split("=", 1)[0].strip()
            for l in open(f, encoding="utf-8", errors="replace") if "=" in l]
    counts = collections.Counter(keys)
    return tuple(sorted(k for k, c in counts.items()
                        if c > 1 and k in DOCUMENTED_META_KEYS))


def loss_kinds(case, session):
    """The kinds of ACTUAL loss established for a session, from fixed sources.

    Two independent triggers, and conflating them was the defect: loss_sessions()
    read the MANIFEST only, so it saw a meta absent or unreadable and was blind to
    every SC-520 malformed COMPLETE event record. `degraded` is aggregate and
    identifies nothing per member, so which members omit is decided by WHICH SOURCE
    failed, never by the flag.
    """
    kinds = set()
    if session in loss_sessions(case):
        kinds.add("meta-absent")
    if duplicated_meta_keys(case, session):
        kinds.add("meta-duplicate")
    if not events_complete(template_of(case), session):
        kinds.add("events-skipped")
    # NOT `assert kinds <= LOSS_KINDS` — that was TAUTOLOGICAL. This function
    # constructs its set from exactly those three predicates, so the assertion could
    # never fire and could never reveal a fourth kind, while its comment claimed to
    # close the missing-kind failure. A check whose subject is generated by the code
    # it checks is the self-confirming class, one level up from the arity control
    # that carried a copy of its own member list. The real check is
    # loss_class_census() below, which compares the RECOGNIZED kinds against the
    # contract's own anomaly classes and reports what has no predicate.
    return kinds


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
    pending_ts = None
    if ledger:
        ruled = ruled_requests(session, ledger)
        openings = {e.get("ref"): e for e in ledger if e.get("action") in OPENINGS}
        for ref, v in ruled.items():
            if v[0] != "pending":
                continue
            ts = (openings.get(ref) or {}).get("ts")
            if ts and (pending_ts is None or ts < pending_ts):
                pending_ts = ts          # OLDEST pending opening: it crosses first
    contrib = dict(alert_contributions(template, session))
    for actor, st in states.items():
        if st in AGENT_OWNED:
            contrib[actor] = st
    return states, contrib, pending_ts


def stopped_attention(contrib, pending_ts):
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
    if pending_ts:
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
                states, contrib, pending_ts = facts
                # ONE WRITER PER LOCUS. When the loss derivation owns a member for this
                # session, the stopped path must not emit it too — disabling the old
                # guard alone put the SC-509 state rows straight back beside the new
                # SC-509b ones, which the gate would have caught as a duplicate address.
                _loss_amem = owed_loss_members(
                    loss_kinds(case, name or ""),
                    duplicated_meta_keys(case, name or ""))[1]
                for ag in sess.get("agents") or []:
                    ref = ag.get("ref")
                    want = states.get(ref)
                    if want and ag.get("state") in (None, "") and "state" not in _loss_amem:
                        if True:
                            rows.append((case, consumer, "SC-509", "digest",
                                     "sessions[%s].agents[%s].state" % (name, ref),
                                     "null", want, "equals", "OBSERVED", "OBSERVED",
                                     "%s declares state %s in fixed producer bytes; the "
                                     "session being stopped changes what is SELECTED, "
                                     "never what the record says" % (ref, want)))
                    if contrib.get(ref) and ag.get("reason") in (None, "") \
                            and "reason" not in _loss_amem:
                        rows.append((case, consumer, "SC-509c", "digest",
                                     "sessions[%s].agents[%s].reason" % (name, ref),
                                     "null", contrib[ref], "equals", "OBSERVED", "OBSERVED",
                                     "%s: fixed producer bytes name the owner and the "
                                     "agent-owned contribution %s" % (ref, contrib[ref])))
                attn = stopped_attention(contrib, pending_ts)
                if attn is None:
                    # SC-522, RULED RELATIONAL 2026-08-24. Not a static target
                    # derived from the frozen generated_at — ae:4141 and ae:3648 are
                    # SEPARATE ordered `date` calls, so the frozen generated_at is
                    # provably not the clock the frozen answer used, and pinning it
                    # would resurrect the dead clock seam as a table row. Not a
                    # permanent `undecidable` either: the rule is statable, and this
                    # corpus's inability to witness it is a property of the corpus.
                    #
                    # THE JOIN IS EXPLICIT AND IS NOT INTRA-DOCUMENT. The request ts
                    # and its pendingness are NOT in the digest: pendingness is the
                    # PINNED fixture ledger reduced through SC-518 + SC-518a, and the
                    # scorer must join it to the successor's own generated_at. Saying
                    # "both operands in one document" was wrong and is corrected here.
                    #
                    # ALL THREE SC-017g FIELDS ARE ADDRESSED. Checking `attention`
                    # alone left needs_attention and attention_rank unaccounted, so
                    # they move JOINTLY: false/null/0 below the threshold and
                    # true/unanswered/1 strictly above it.
                    stopped_undecidable.add((case, name))
                    for locus, below, above in (
                            ("needs_attention", "false", "true"),
                            ("attention", "null", "unanswered"),
                            ("attention_rank", "0", "1")):
                        rows.append((case, consumer, "SC-017g", "digest",
                                     "sessions[%s].%s" % (name, locus),
                                     ("null" if sess.get(locus) is None
                                      else str(sess.get(locus)).lower()),
                                     "%s when generated_at - %s <= threshold, %s when "
                                     "strictly greater" % (below, pending_ts, above),
                                     "relational", "OBSERVED", "OBSERVED",
                                     "SC-522 strictly-past, evaluated in the SUCCESSOR "
                                     "digest's own frame: its generated_at joined to the "
                                     "PINNED fixture opening %s left pending under "
                                     "SC-518+SC-518a, against AE_ATTN_REQUEST_SECS "
                                     "(default 1800). UNSCORABLE until the phase-4 scorer "
                                     "implements and red-proves this predicate"
                                     % pending_ts))
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
                kinds = loss_kinds(case, name or "")
                if kinds:
                    # MAPPING-DRIVEN. The owed member set comes from the declared
                    # SOURCE-TO-MEMBER mapping, which is read from the CONTRACT rows
                    # — never from the serializer, and never from a copy carried by
                    # the checker. Adding a member class here is one line in the
                    # declaration and the emission follows.
                    smem, _amem = owed_loss_members(
                        kinds, duplicated_meta_keys(case, name or ""))
                    dupk = duplicated_meta_keys(case, name or "")

                    def _why(member):
                        """The reason for THIS member, from its own owning kind."""
                        k = member_owner(member, kinds, dupk)
                        if k is None:      # `degraded` is owed by the whole set
                            return " and ".join(KIND_REASON[x] for x in sorted(kinds))
                        return KIND_REASON[k]
                    for member in smem:
                        # `degraded` is SUCCESSOR-ONLY and is never in a frozen
                        # capture — its whole obligation is ABSENT -> true — so a
                        # blanket present-in-capture guard silently deleted every
                        # qualifier row this slice exists for. Measured: 14 became 0.
                        if member != "degraded" and member not in sess:
                            continue
                        val = sess.get(member)
                        rendered = "null" if val is None else str(val).lower()
                        if member == "degraded":
                            rows.append((case, consumer, "SC-509b", "digest",
                                         "sessions[%s].degraded" % name,
                                         "present" if "degraded" in sess else "ABSENT",
                                         "true", "equals", "OBSERVED", "OBSERVED",
                                         authority_signature(
                                             "SC-509b", kinds,
                                             "sessions[%s].degraded" % name,
                                             "actual-loss-visible")
                                         + " session %s: %s, so this entry suffered "
                                         "ACTUAL loss rather than sparsity"
                                         % (name, _why(member))))
                        elif member == "needs_attention":
                            rows.append((case, consumer, "SC-509b", "digest",
                                         "sessions[%s].needs_attention" % name,
                                         rendered, "false", "equals", "OBSERVED",
                                         "OBSERVED",
                                         "session %s: needs_attention renders as an "
                                         "ALWAYS-PRESENT PARTIAL-EVIDENCE INDICATOR, and "
                                         "it is owed here because %s — a loss that "
                                         "REACHES THE ATTENTION INPUTS. Neither value "
                                         "proves the exact final attention only when the "
                                         "loss could affect those inputs; where exactness "
                                         "is established despite UNRELATED loss the triad "
                                         "stays exact, which is why a duplicated `goal` "
                                         "owes no row here" % (name, _why("needs_attention"))))
                            rows[-1] = rows[-1][:10] + (
                                authority_signature(
                                    "SC-509b", kinds,
                                    "sessions[%s].needs_attention" % name,
                                    "partial-evidence-from-readable-facts")
                                + " " + rows[-1][10],)
                        else:
                            rows.append((case, consumer, "SC-509b", "digest",
                                         "sessions[%s].%s" % (name, member),
                                         rendered, "ABSENT", "equals", "OBSERVED",
                                         "OBSERVED",
                                         "session %s: %s is unreadable here — %s — and "
                                         "an unreadable optional fact OMITS. `degraded` "
                                         "is aggregate visibility and identifies nothing "
                                         "per member; this member's own source is what "
                                         "decides" % (name, member, _why(member))))
                            rows[-1] = rows[-1][:10] + (
                                authority_signature(
                                    "SC-509b", {member_owner(member, kinds, dupk)}
                                    if member_owner(member, kinds, dupk) else kinds,
                                    "sessions[%s].%s" % (name, member),
                                    "unreadable-member-omits")
                                + " " + rows[-1][10],)
                    # AGENT MEMBERS, emitted HERE rather than on the stopped path.
                    # They rode the stopped-session branch, so a RUNNING event-loss
                    # session got none of them — four occurrences owing ten rows
                    # produced six, and only predicting the total first exposed it.
                    # Membership belongs to the LOSS derivation; being stopped is a
                    # different fact that happened to coincide.
                    for ag in sess.get("agents") or []:
                        aref = ag.get("ref")
                        for member in _amem:
                            if member not in ag:
                                continue
                            aval = ag.get(member)
                            rows.append((case, consumer, "SC-509b", "digest",
                                         "sessions[%s].agents[%s].%s" % (name, aref, member),
                                         "null" if aval is None else str(aval).lower(),
                                         "ABSENT", "equals", "OBSERVED", "OBSERVED",
                                         "%s: %s — so this agent's %s is unreadable and "
                                         "the member omits. A value parsed out of a "
                                         "skipped-record ledger is not established, "
                                         "however cleanly the surviving lines parse"
                                         % (aref, _why(member), member)))
                            rows[-1] = rows[-1][:10] + (
                                authority_signature(
                                    "SC-509b", kinds,
                                    "sessions[%s].agents[%s].%s" % (name, aref, member),
                                    "unreadable-member-omits")
                                + " " + rows[-1][10],)
                    # SC-405g / OC-P4-BRANCH-VALUE: PRESENCE ONLY. The value is
                    # exempted by the register, so an exact-value row here would
                    # partially score the very value the OC exempts — and two rows on
                    # one locus is the second-authority class. One row, presence
                    # predicate, across the WHOLE union rather than a hardcoded count.
                    if "branch" in sess:
                        bval = sess.get("branch")
                        rows.append((case, consumer, "SC-405g", "digest",
                                     "sessions[%s].branch (presence)" % name,
                                     "present", "ABSENT", "equals", "OBSERVED",
                                     "OBSERVED",
                                     "session %s: a degraded entry with no branch "
                                     "observation omits the member; the VALUE is "
                                     "exempted by OC-P4-BRANCH-VALUE, so only its "
                                     "PRESENCE is scored here (frozen renders %s)"
                                     % (name, "null" if bval is None else repr(bval))))
                        rows[-1] = rows[-1][:10] + (
                            authority_signature(
                                "SC-405g", kinds,
                                "sessions[%s].branch (presence)" % name,
                                "temporary-presence-projection/value-unscored")
                            + " " + rows[-1][10],)
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
                        proved = sorted(declared.get((name, ref), set()))
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
        # ---- SC-017h / SC-017l / SC-017m / SC-017r, independently derived.
        # The shipped rows were nameless and invocation-grained: an l locus of
        # `status cell` with no candidate in it, an m locus of `(row set)`, an r
        # marker naming no agent. None can satisfy the pinned CANDIDATE-GRAINED
        # invariant -- "for every omitted durable candidate, its SC-017m
        # view-membership contribution must PAIR with an SC-017l absent-to-unknown
        # obligation AT THE SAME CANDIDATE IDENTITY" -- because a row carrying no
        # identity has nothing to pair at. They are replaced, not deleted: the old
        # occurrence keys are re-fed as EVIDENCE and every one is reconciled, while
        # the cardinalities are deliberately NOT inherited.
        rows.extend(emit_unknown_family(case, consumer, r["normalised_argv"], text,
                                        r["surface"]))
        # SC-017h shares r's fixed pre-value human identity. Stopped rows contribute
        # source state ABSENT because their printer grammar has no state cell.
        rows.extend(emit_agent_state(case, consumer, text, r["surface"]))
        # SC-017r is emitted at fixed session + rendered ref + rendered short-sid
        # identity, with exact semantic-value multiplicity for collisions.  The
        # printer-schema parser preserves stopped membership while distinguishing
        # its absent health cell from the stopped session_id field.
        rows.extend(emit_agent_health(case, consumer, text, r["surface"]))

        # The SC-017o HUMAN DIAGNOSTIC is deliberately NOT emitted. It is earned
        # only by an independently entitled enumeration with a final failure, and the
        # previous derivation earned it from `unreachable(case)` — the ambient probe
        # against the case's own live.sock, which is exactly the fact the ruling says
        # cannot earn it. 172 obligations rested on that basis; none survives.

    unproved.sort()
    unproved_lines = [
        "# SC-509c loci EXCLUDED for want of a carrier. Reported, never guessed.\n",
        "# AT THE RULED GRAIN: (case, consumer, session, agent_ref, locus), the same\n",
        "# address the accepted table uses. It was previously keyed without the\n",
        "# session, so 34 rows mapped ambiguously to two same-attention sessions and\n",
        "# their no-carrier claim could not be evaluated per address. An exclusion\n",
        "# file below the ruled grain cannot substantiate its own claims.\n",
        "# NOT a claim of impossibility: no carrier was FOUND by the search this\n",
        "# generator performs — the agent's own state, a state event naming it as\n",
        "# actor, and a producer-template alert naming it as target.\n",
        "\t".join(["case", "consumer", "session", "agent_ref", "locus",
                    "session_attention", "kind", "why"]) + "\n",
    ]
    unproved_lines.extend("\t".join(str(v) for v in x) + "\n" for x in unproved)
    unproved_text = "".join(unproved_lines)
    atomic_write_text(UNPROVED, unproved_text)

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
    gap = added_roster_gap_population(p1)
    contract_id = contract_blob()
    out_text = "\t".join(HDR) + "\n" + "".join(
        "\t".join(str(v) for v in x) + "\n" for x in rows)
    gap_text = (
        "# SC-017h's and SC-017r's duties over these agents are UNCHANGED (contract\n"
        "# %s): what is absent is EVIDENCE, not obligation. Each session's meta is\n"
        "# carried as a HASH and the captured agents output is scoped to its own capturing\n"
        "# session, so no added row's roster is nameable from this corpus. Enumerated per\n"
        "# occurrence so the gap has a size and a membership test and cannot absorb a\n"
        "# session unnoticed.\n"
        "case\tconsumer\tadded_session\n" % contract_id[:12]
        + "".join("\t".join(member) + "\n" for member in sorted(gap))
    )
    out_sha256 = hashlib.sha256(out_text.encode("utf-8")).hexdigest()
    gap_sha256 = hashlib.sha256(gap_text.encode("utf-8")).hexdigest()
    unproved_sha256 = hashlib.sha256(unproved_text.encode("utf-8")).hexdigest()
    fresh_text = (
        "# Freshness relation — the SOURCE this derivation was made against.\n"
        "# A lineage stamp says where an artifact came from; only a hash\n"
        "# comparison says whether the source has MOVED since. FRESHNESS is also the\n"
        "# tuple manifest and is published LAST, after atomic replacement of all three\n"
        "# data members: OBLIGATIONS, UNOBSERVABLE-ADDED-ROSTER and SC-509C-UNPROVED.\n"
        "field\tvalue\n"
        f"contract_path\t{CONTRACT}\n"
        f"contract_blob\t{contract_id}\n"
        f"p1_rows\t{len(seen)}\n"
        f"obligation_rows\t{len(rows)}\n"
        f"obligations_sha256\t{out_sha256}\n"
        f"added_roster_gap_sha256\t{gap_sha256}\n"
        f"sc509c_unproved_sha256\t{unproved_sha256}\n"
    )

    # FRESHNESS is the commit marker for this four-file snapshot. UNPROVED was
    # published above; OUT and GAP follow. Before FRESHNESS moves, a verifier seeing
    # any new member rejects the old hashes; after it moves, all three bound content
    # identities are already live. No member is ever truncated in place because
    # every publication is same-directory temp + rename.
    atomic_write_text(OUT, out_text)
    atomic_write_text(GAP, gap_text)
    atomic_write_text(FRESH, fresh_text)

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
