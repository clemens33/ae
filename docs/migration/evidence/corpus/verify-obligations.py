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
import argparse, csv, io, json, os, re, subprocess, sys, collections

HERE = os.path.dirname(os.path.abspath(__file__))
SRC = os.path.normpath(os.path.join(HERE, "..", "batch-c-artifacts"))
CONTRACT = "docs/migration/semantic-contract.md"
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
FRESH_REQUIRED_FIELDS = frozenset({
    "contract_path", "contract_blob", "p1_rows", "obligation_rows",
    "obligations_sha256", "added_roster_gap_sha256", "sc509c_unproved_sha256",
})
# THE IDENTITY/PAYLOAD SPLIT, IN BYTES RATHER THAN IN A HEAD. For OBLIGATIONS.tsv the
# ADDRESS is (case, consumer, obligation_id, locus); everything else is row DATA.
# Keying an address on payload lets a contradictory duplicate buy its own address by
# changing the very field that made it contradictory.
ADDRESS = ("case", "consumer", "obligation_id", "locus")
# THE POPULATION EACH ID MAY APPEAR ON, declared rather than left to whichever parse
# condition a loop happened to use. Measured against the accepted table before being
# written down: SC-017l/m appear on digest and human listings, SC-017h/r on human
# listings, and SC-509b/c/d/e and SC-017o on digests only. OWED-ZERO IS AN OBLIGATION
# TO CHECK, NOT A ROW TO SKIP — a loop that skips a class is quiet exactly outside the
# domain its author imagined, which is where a fabricated row goes to hide.
ID_POPULATION = {
    "SC-017l": ("digest", "human-listing"),
    "SC-017m": ("digest", "human-listing"),
    "SC-017o": ("digest",),
    "SC-017h": ("human-listing",),
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


# ---- THE MODULE INVARIANT, stated because it took three instances to see:
#
#   THE OWED SIDE IS WHERE DERIVATION AND SCOPING LIVE.
#   THE HELD SIDE IS COMPARED IN ITS ENTIRETY, NEVER FILTERED.
#
# Every filter applied to the HELD side re-scopes the gate to the population its
# author imagined, so anything outside that imagination is discarded before the
# comparison can see it. Today's three instances, each a reasonable-looking
# optimisation: want_attn compared only agent-owned contributions; ID_POPULATION
# constrained a row CLASS and was mistaken for an owed population; held_shapes
# filtered to stopped loci, so an addition on a RUNNING session was invisible.
#
# The mechanism: one GLOBAL comparison in which every table row is CONSUMED EXACTLY
# ONCE by some family's owed set, and an unconsumed row is a finding. An id with no
# owed family cannot consume anything, so it is DECLARED here rather than silently
# exempt — which is what once let the entire SC-017r family be deleted wholesale
# and stay green.
GAP = os.path.join(HERE, "UNOBSERVABLE-ADDED-ROSTER.tsv")
UNPROVED = os.path.join(HERE, "SC-509C-UNPROVED.tsv")

OWED_FAMILY_IDS = frozenset({
    "SC-509b", "SC-405g",          # loss facts
    "SC-509d",                     # schema_version, one per digest
    "SC-017o",                     # inventory_complete, the two fixed rows
    "SC-509e",                     # agent liveness on unreachable digests
    "SC-509", "SC-017g",           # stopped facts
    "SC-509c",                     # reason carriers
    "SC-518", "SC-518a",           # request closure
    "SC-521c",                     # live-scope set obligations
    "SC-017l", "SC-017m",          # unknown liveness, graduated 2026-08-25
    "SC-017h",                     # human declared state at fixed roster grain
    "SC-017r",                     # human agent health at fixed roster grain
})
# Ids with no owed derivation YET. STAGING ONLY — none of these may survive into a
# final artifact, per colead's ruling, and the doctrine line is theirs verbatim:
#
#     UNSCORABLE IS SUPPORT, NEVER PERMISSION FOR UNKNOWN POPULATION.
#
# A gap listed here is visible rather than accidental, which is the only virtue it
# has; it is not a licence. Each entry names the source ruled for its family, all
# derived IN THIS VERIFIER from INVOCATIONS + committed stdout + tmux.before and the
# manifest — never from the generator, because a family imported from the thing it
# checks cannot police it. Removing an entry requires building that family.
# EMPTY, and that is the point: colead's rule is that no staging gap may survive
# into a final artifact. The dict stays because the partition line prints its size,
# and a future gap must be DECLARED here rather than silently exempt.
NO_OWED_FAMILY = {
}


def gate_manifest_candidate_modes(case):
    """Independent manifest traversal: candidate -> recorded meta mode."""
    path = os.path.join(SRC, case, "manifest.before.tsv")
    if not os.path.exists(path):
        return {}
    out = {}
    for row in csv.reader(open(path, encoding="utf-8", errors="replace"),
                          delimiter="\t"):
        if len(row) < 2 or row[0] != "file":
            continue
        match = re.fullmatch(r"\./sessions/([^./][^/]*)/meta", row[-1])
        if match:
            out[match.group(1)] = row[1]
    return out


def gate_recorded_selector(case, candidate):
    """Independent `(state, server)` normalization for one fixed candidate."""
    case_path = os.path.join(SRC, case, "case.txt")
    case_lines = (open(case_path, encoding="utf-8", errors="replace").readlines()
                  if os.path.exists(case_path) else [])
    direct = set()
    for line in case_lines:
        fields = dict(part.split("=", 1) for part in line.split()
                      if "=" in part and len(part.split("=", 1)) == 2)
        if fields.get("session") == candidate and fields.get("socket"):
            direct.add(fields["socket"])
    if len(direct) == 1:
        return "positive", next(iter(direct))
    if len(direct) > 1:
        return "ambiguous", None

    modes = gate_manifest_candidate_modes(case)
    if candidate not in modes:
        return "meta-absent", None
    if modes[candidate] in {"000", "0", "100", "200"}:
        return "mode-unusable", None
    template = template_of(case)
    if not template or "/" not in template:
        return "unresolved", None
    arm, variant = template.split("/", 1)
    path = os.path.join(SRC, "templates", arm, "fixture-bytes", variant,
                        "sessions", candidate, "meta")
    if not os.path.exists(path):
        return "unresolved", None
    values = collections.defaultdict(list)
    for line in open(path, encoding="utf-8", errors="replace"):
        key, sep, value = line.rstrip("\n").partition("=")
        if sep:
            values[key].append(value)
    servers = values.get("tmux_server", [])
    kinds = values.get("tmux_server_kind", [])
    if len(servers) == 1 and servers[0] and len(kinds) <= 1 \
            and (not kinds or kinds == ["socket"]):
        return "positive", servers[0]
    if not servers or servers == [""]:
        return "missing", None
    return "ambiguous", None


def gate_attempted_servers(case, consumer):
    """Server spellings on actual session/pane enumeration trace commands."""
    path = os.path.join(SRC, case, "out", consumer + ".tmuxtrace")
    if not os.path.exists(path):
        return frozenset()
    out = set()
    for line in open(path, encoding="utf-8", errors="replace"):
        cells = {}
        for cell in line.rstrip("\n").split("\t"):
            key, sep, value = cell.partition("=")
            if sep:
                cells[key] = value
        argv = cells.get("argv", "")
        if any(command in argv.split() for command in
               ("list-sessions", "list-panes", "has-session")):
            if cells.get("AE_TMUX_SERVER"):
                out.add(cells["AE_TMUX_SERVER"])
    return frozenset(out)


def gate_topology_text(case, consumer):
    stage = consumer.partition("/")[0] if "/" in consumer else ""
    paths = ([os.path.join(SRC, case, "tmux.%s.txt" % stage)] if stage else [])
    paths += [os.path.join(SRC, case, "tmux.before.txt")]
    for path in paths:
        if os.path.isfile(path):
            return open(path, encoding="utf-8", errors="replace").read()
    return ""


def gate_query_outcome(case, consumer, server):
    attempts = gate_attempted_servers(case, consumer)
    if attempts != frozenset({server}):
        return "unobserved"
    text = gate_topology_text(case, consumer)
    if not text:
        return "unobserved"
    if any(marker in text for marker in ("error connecting", "no server running")):
        return "failed"
    sections = {line for line in text.splitlines() if line.startswith("## ")}
    return "success" if {"## panes", "## sessions"} <= sections else "unobserved"


def gate_candidate_causes(case, consumer):
    """Candidate-grained liveness causes, independently traversed from the gate."""
    attempts = gate_attempted_servers(case, consumer)
    out = {}
    for candidate in durable_candidates(case):
        state, server = gate_recorded_selector(case, candidate)
        if state in ("meta-absent", "mode-unusable"):
            out[candidate] = ("selector-%s" % state,)
        elif state != "positive":
            out[candidate] = ("selector-%s" % state,)
        elif server not in attempts:
            out[candidate] = ("selector-server-unattempted",)
        else:
            outcome = gate_query_outcome(case, consumer, server)
            if outcome == "failed":
                out[candidate] = ("selector-server-failed",)
            elif outcome == "unobserved":
                out[candidate] = ("selector-server-outcome-unobserved",)
    return out


def gate_candidate_support(causes):
    return ("UNSCORABLE" if any(c in ("selector-server-unattempted",
                                       "selector-server-outcome-unobserved")
                                for c in causes)
            else "OBSERVED")


def view_shows_unknown(argv):
    """SC-017m's selection rule, read off the invocation.

    `--stopped` shows ONLY stopped, so an unknown candidate is filtered out of it;
    the default/`--running` view and `--all` both show unknown. Written as a positive
    test for the one view that excludes it, so a new flag defaults to INCLUDING
    unknown rather than silently hiding it -- hiding it is the #105 defect itself.
    """
    return "--stopped" not in argv.split()

def owed_unknown_v(case, consumer, argv, text, surface):
    """SC-017l + SC-017m owed shapes, derived by the CONTRACT'S OWN PHRASING.

    A second METHOD, not a second copy. The generator classifies each candidate and
    branches on the class; this builds the two sets the contract names -- "derive the
    contract-required exact identity/status set from fixed candidates plus view
    semantics" and compare it against what the frozen document actually renders --
    then reads the obligations off the difference. Same rows, opposite direction of
    travel, so a wrong branch on one side shows up as a shape the other does not owe.
    """
    if surface not in LISTING:
        return set()
    cause_by_candidate = gate_candidate_causes(case, consumer)
    if not cause_by_candidate:
        return set()
    digest = '"schema_version"' in text
    stream = "digest" if digest else "stdout"
    lpat = "sessions[%s].status" if digest else "candidate[%s].status"
    mpat = "sessions[%s]" if digest else "view.members[%s]"
    rep = candidate_representation(case, consumer, text)
    # THE REQUIRED SET: every durable candidate whose true status is `unknown` is
    # selected by a view that shows unknown, and by no view that does not.
    shows_unknown = view_shows_unknown(argv)
    required = set(cause_by_candidate) if shows_unknown else set()
    # THE FROZEN SET: what the captured document actually renders for those names.
    frozen = {c for c, v in rep.items()
              if c in cause_by_candidate and v.startswith("aligned:")}

    owed = set()
    for c in sorted(cause_by_candidate):
        causes = cause_by_candidate[c]
        support = gate_candidate_support(causes)
        present = c in frozen
        if c in required and not present:
            owed.add(("SC-017l", stream, lpat % c, "ABSENT", "unknown",
                      "equals", "OBSERVED", support))
            owed.add(("SC-017m", stream, mpat % c, "ABSENT", "present",
                      "equals", "OBSERVED", support))
        elif c in required and present:
            owed.add(("SC-017l", stream, lpat % c, rep[c].split(":", 1)[1], "unknown",
                      "equals", "OBSERVED", support))
        elif present and c not in required:
            owed.add(("SC-017m", stream, mpat % c, "present", "ABSENT",
                      "equals", "OBSERVED", support))
    return owed


def gate_agent_rows(text):
    """Independent human table traversal with the status-specific row grammar."""
    lines = text.splitlines()
    head = [n for n, l in enumerate(lines) if re.match(r"^SESSION\s+STATUS\b", l)]
    if not head:
        return []
    offset = lines[head[0]].index("STATUS")
    out, session, status = [], None, None
    for line in lines[head[0] + 1:]:
        if not line.strip():
            break
        toks = [(m.start(), m.group()) for m in re.finditer(r"\S+", line)]
        if not line[0].isspace():
            caps = [v for _, v in toks if v.isalpha() and v.isupper()]
            if len(caps) >= 3 or len(toks) < 2 or toks[1][0] < offset:
                break
            session, status = toks[0][1], toks[1][1]
            continue
        agent = gate_human_agent_row(line, status)
        if agent and session is not None:
            raw_sid = agent[1]
            sid = "-" if raw_sid in (None, "", "pending", "-") else raw_sid[:8]
            out.append((session, status, agent[0], sid, agent[2], agent[3]))
    return out


def gate_fixed_roster_slots(case, consumer, session):
    """Independently recover fixed slot/ref/SID records, never from pane display."""
    stage = consumer.partition("/")[0] if "/" in consumer else ""
    sources = ([os.path.join(SRC, case, "roster.%s.txt" % stage)] if stage else [])
    template = template_of(case)
    if template and "/" in template:
        arm, variant = template.split("/", 1)
        sources.append(os.path.join(SRC, "templates", arm, "fixture-bytes", variant,
                                    "sessions", session, "meta"))
    path = next((p for p in sources if os.path.isfile(p)), None)
    if path is None:
        return []
    out = []
    for raw in open(path, encoding="utf-8", errors="replace"):
        match = re.fullmatch(r"agent\.([^=]+)=(.+):([^:]*)", raw.rstrip("\n"))
        if not match:
            continue
        slot, ref, raw_sid = match.groups()
        sid = "-" if raw_sid in ("", "pending", "-") else raw_sid[:8]
        out.append((slot, ref, sid))
    return out


def gate_topology_observation(case, consumer, server):
    """Gate-side topology reader retaining slot identity and display separately."""
    outcome = gate_query_outcome(case, consumer, server)
    if outcome != "success":
        return outcome, set(), {}
    sessions, panes, section = set(), collections.defaultdict(list), None
    unscoped = []
    for raw in gate_topology_text(case, consumer).splitlines():
        if raw.startswith("## "):
            section = raw[3:]
            continue
        fields = raw.split("|") if raw else []
        if section == "sessions" and fields:
            sessions.add(fields[0])
        elif section == "panes" and len(fields) in (5, 6, 7):
            # Discriminate the frozen capture schemas by pane-id provenance.
            if re.fullmatch(r"%\d+", fields[0]) and len(fields) == 7:
                panes[fields[-1]].append(
                    (fields[2], fields[1], fields[3], fields[5]))
            elif re.fullmatch(r"%\d+", fields[0]) and len(fields) == 6:
                unscoped.append((fields[2], fields[1], fields[3], None))
            elif len(fields) == 5 and re.fullmatch(
                    r"%\d+", fields[1] if len(fields) > 1 else ""):
                panes[fields[0]].append(
                    (fields[3], fields[2], fields[4], None))
    if len(sessions) == 1 and unscoped:
        panes[next(iter(sessions))].extend(unscoped)
    return outcome, sessions, dict(panes)


def gate_health_target(case, consumer, session, slot, candidate_causes):
    causes = candidate_causes.get(session)
    if causes:
        return "unambiguous unknown", causes, gate_candidate_support(causes)
    state, server = gate_recorded_selector(case, session)
    if state != "positive":
        return None
    if server not in gate_attempted_servers(case, consumer):
        causes = ("selector-server-unattempted",)
        return "unambiguous unknown", causes, gate_candidate_support(causes)
    outcome, sessions, panes = gate_topology_observation(case, consumer, server)
    if outcome != "success":
        causes = (("selector-server-failed",) if outcome == "failed"
                  else ("selector-server-outcome-unobserved",))
        return "unambiguous unknown", causes, gate_candidate_support(causes)
    if session not in sessions:
        return "dead", ("exact-session-absent",), "OBSERVED"
    if slot is None:
        return "unambiguous unknown", ("pane-slot-unbound",), "OBSERVED"
    entries = panes.get(session, [])
    by_slot = collections.Counter(entry[0] for entry in entries)
    matches = [entry for entry in entries if entry[0] == slot]
    if not matches:
        if all(re.fullmatch(r"\S+", pane_slot or "") and by_slot[pane_slot] == 1
               for pane_slot, _ref, _cmd, _dead in entries):
            return "dead", ("exact-pane-absent",), "OBSERVED"
        return "unambiguous unknown", ("pane-association-unusable",), "OBSERVED"
    if len(matches) != 1:
        return "unambiguous unknown", ("pane-association-ambiguous",), "OBSERVED"
    _slot, _display_ref, command, pane_dead = matches[0]
    if pane_dead == "1":
        return "dead", ("pane-dead",), "OBSERVED"
    shells = {"", "bash", "zsh", "fish", "sh", "dash"}
    if pane_dead == "0" and command not in shells:
        return "alive", ("pane-alive",), "OBSERVED"
    return "unambiguous unknown", ("pane-live-predicate-unproved",), "OBSERVED"


def gate_state_target(case, session, agent):
    if "events-skipped" in gate_loss_kinds(case, session):
        return "unknown"
    value = stopped_declared(case, session).get(agent, "-")
    return value if value not in (None, "") else "unknown"


def owed_agent_state_v(case, consumer, text, surface):
    """SC-017h fixed-projection Counters, independently derived."""
    if surface not in LISTING or '"schema_version"' in text:
        return set()
    grouped = collections.defaultdict(list)
    for session, _status, name, sid, state, _health in gate_agent_rows(text):
        grouped[(session, name, sid)].append(
            ("ABSENT" if state is None else state,
             gate_state_target(case, session, name)))
    owed = set()
    for (session, name, sid), pairs in grouped.items():
        sources, targets = zip(*pairs)
        if collections.Counter(sources) == collections.Counter(targets):
            continue
        suffix = "(class)" if len(pairs) > 1 else ""
        locus = "agents[%s:%s:%s]%s.state" % (session, name, sid, suffix)
        frm = (" ".join("%s x%d" % (v, n) for v, n in
                        sorted(collections.Counter(sources).items()))
               if len(pairs) > 1 else sources[0])
        to = (" ".join("%s x%d" % (v, n) for v, n in
                       sorted(collections.Counter(targets).items()))
              if len(pairs) > 1 else targets[0])
        owed.add(("SC-017h", "stdout", locus, frm, to, "equals",
                  "OBSERVED", "OBSERVED"))
    return owed


def owed_agent_health_v(case, consumer, text, surface):
    """SC-017r fixed-projection Counters, independently derived."""
    if surface not in LISTING or '"schema_version"' in text:
        return set()
    candidate_causes = gate_candidate_causes(case, consumer)
    per_session = collections.defaultdict(list)
    for session, _status, name, sid, _state, marker in gate_agent_rows(text):
        per_session[session].append(
            (name, sid, "ABSENT" if marker is None else (marker or "blank")))

    owed = set()
    for session, entries in per_session.items():
        identities = collections.Counter((name, sid) for name, sid, _ in entries)
        for (name, sid), count in identities.items():
            values = collections.Counter(
                value for entry_name, entry_sid, value in entries
                if (entry_name, entry_sid) == (name, sid)
            )
            roster_slots = [slot for slot, roster_name, roster_sid
                            in gate_fixed_roster_slots(case, consumer, session)
                            if (roster_name, roster_sid) == (name, sid)]
            slots = roster_slots if len(roster_slots) == count else [None] * count
            targets = [gate_health_target(case, consumer, session, slot,
                                          candidate_causes)
                       for slot in slots]
            if any(target is None for target in targets):
                continue
            target_values = [target[0] for target in targets]
            support = ("UNSCORABLE" if any(target[2] == "UNSCORABLE"
                                           for target in targets) else "OBSERVED")
            # Presentation is the scored value. Frozen blank is semantically alive
            # but is not the successor's literal `alive` cell, so semantic
            # normalization here would erase the exact divergence SC-017r records.
            if values == collections.Counter(target_values):
                continue
            if count == 1:
                owed.add(("SC-017r", "stdout",
                          "agents[%s:%s:%s].health" % (session, name, sid),
                          next(iter(values)), target_values[0],
                          "equals", "OBSERVED", support))
            else:
                owed.add(("SC-017r", "stdout",
                          "agents[%s:%s:%s](class).health" % (session, name, sid),
                          " ".join("%s x%d" % (v, n) for v, n in sorted(values.items())),
                          " ".join("%s x%d" % (v, n) for v, n
                                   in sorted(collections.Counter(target_values).items())),
                          "equals", "OBSERVED", support))
    return owed


def gate_human_agent_row(line, status):
    """Independent status-aware reading of a frozen human agent subrow.

    Unlike the generator's printer-format regex, this derives the schema from token
    arity after the enclosing session row has supplied status.  Stopped has exactly
    ref+session_id.  Running has ref+session_id+state and an optional fourth health
    token.  This keeps an empty trailing health cell empty instead of consuming the
    state token, and keeps stopped membership without inventing value cells.
    """
    if not line.startswith("  "):
        return None
    fields = line.split()
    if not fields or not re.fullmatch(r"\S+:\S+", fields[0]):
        return None
    if status == "stopped" and len(fields) == 2:
        return fields[0], fields[1], None, None
    if status == "running" and len(fields) in (3, 4):
        return fields[0], fields[1], fields[2], fields[3] if len(fields) == 4 else ""
    return None


def unobservable_added_roster(case, consumer, argv, text, surface):
    """The UNOBSERVABLE-ADDED-ROSTER population, ENUMERATED BY OCCURRENCE.

    SC-017h's and SC-017r's duties over agents on rows SC-017m ADDS are UNCHANGED by
    the amendment -- "what is absent is evidence, not obligation". This evidence
    base cannot name those agents: each session's meta is carried as a HASH and the
    captured agents output is scoped to its own capturing session. Measured
    recoverable rosters for added rows: ZERO.

    A NAMED POPULATION rather than a paragraph, because a prose bound is not checkable
    and cannot be subtracted from a completeness claim. Every added-session occurrence
    is an explicit member, so the gap has a size, an enumeration and a membership test,
    and cannot quietly absorb a session nobody noticed. It records what CANNOT be
    observed; it never licenses minting an agent fact for it.
    """
    if surface not in LISTING:
        return set()
    rep = candidate_representation(case, consumer, text)
    if not view_shows_unknown(argv):
        return set()
    causes = gate_candidate_causes(case, consumer)
    return {(case, consumer, c) for c, v in rep.items()
            if c in causes and not v.startswith("aligned:")}

def added_roster_gap_population():
    """Enumerated over the DECLARED invocation universe, never accumulated as a side
    effect of whichever block happened to run.

    First cut hooked this into the consumption block and reported 767, because that
    block does not run for every invocation -- a population measured wherever the code
    passes is a population shaped by control flow. HUMAN surfaces only (SC-017h/r do
    not own JSON agent state/health; SC-509/default parity does) and only views where
    unknown qualifies, because a `--stopped` view adds no rows for m and therefore
    no agent rosters for h or r.
    """
    members = set()
    with open(INV, encoding="utf-8") as fh:
        for r in csv.DictReader(fh, delimiter="\t"):
            if r["phase"] != "P1" or r["surface"] not in LISTING:
                continue
            case = os.path.dirname(r["case"])
            text = body(case, r["consumer"])
            if '"schema_version"' in text:
                continue
            if not view_shows_unknown(r["normalised_argv"]):
                continue
            members |= unobservable_added_roster(case, r["consumer"],
                                                 r["normalised_argv"], text,
                                                 r["surface"])
    return members


def gate_added_roster_gap(out, members, gap_text, gap_label):
    """The gap must be DECLARED, and the declaration must MATCH.

    First cut failed whenever SC-017r had an owed family at all, which would have
    made the gate permanently red the moment r graduated -- and that is not what the
    amendment says. The row is explicit that this is an EMPIRICAL COVERAGE GAP and
    not a normative exclusion: "the duty stands UNCHANGED; what is absent is
    evidence, not obligation". So an owed family over the OBSERVABLE population is
    correct, and what must never pass is an UNDECLARED gap -- coverage that looks
    total because nothing says otherwise.

    The declaration is a committed enumeration, compared in BOTH directions against
    the derived population. A member the file omits is a session quietly absorbed; a
    member the file invents is a gap claimed where none exists. Prose could do
    neither check.
    """
    declared = set()
    with io.StringIO(gap_text) as fh:
        # The reasoning header is part of the artifact, so the reader skips comment
        # lines rather than the artifact dropping its reasoning to suit the reader.
        rows = (l for l in fh if not l.startswith("#"))
        for row in csv.DictReader(rows, delimiter="\t"):
            declared.add((row["case"], row["consumer"], row["added_session"]))
    for m in sorted(members - declared):
        fail(out, "ADDED-ROSTER-GAP", "%s/%s adds %s whose roster is unnameable and "
             "which the declaration omits" % m)
    for m in sorted(declared - members):
        fail(out, "ADDED-ROSTER-GAP", "%s/%s declares %s as an unnameable roster, but "
             "it is not in the derived population" % m)
    return len(members)


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
                     "loss facts", "unknown liveness l/m", "agent declared state h",
                     "agent health r",
                     "schema version", "inventory completeness", "agent liveness e"}
_families_seen = set()
_rows_compared = collections.Counter()


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
    # COVERAGE IS ASSERTED, NEVER INFERRED. A comparison inherits the scope of the
    # block it is written in, so where a check LIVES is part of what it asserts --
    # this one sat inside a digest-only branch and compared 160 of 434 rows while the
    # gate reported green. Every held row a comparison actually sees is counted here
    # and reconciled against the table at the end, so a check hoisted into a narrower
    # scope fails loudly with its own arithmetic instead of passing quietly.
    for shape, n in held.items():
        _rows_compared[shape[0]] += n
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
            # ONE WRITER PER LOCUS: a session whose loss makes `reason` unreadable
            # owes ABSENCE under SC-509b, emitted by owed_loss, not a contribution.
            if "reason" in owed_loss_members(gate_loss_kinds(case, nm), gate_duplicated_meta_keys(case, nm))[1]:
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


EVENT_STABLE_KEYS = frozenset({
    "ts", "actor", "action", "target", "ref", "summary", "body_file",
    "actor_slot", "actor_session", "target_slot", "target_session",
})
# Re-declared from the SAME CONTRACT ROWS the generator cites (SC-405b, SC-405f,
# SC-017e/g/h, SC-509c, with SC-509b's omit rule) — not imported, so the two can
# disagree. The serializer is corroboration elsewhere and never the source here.
# The closed kind list, re-declared here. `degraded` is the ONLY common member;
# `needs_attention` is owed only where the lost input feeds attention, so a
# duplicated `goal` leaves the quiet triad exact.
LOSS_KINDS = ("meta-absent", "meta-duplicate", "events-skipped")
DOCUMENTED_META_KEYS = frozenset({"mode", "origin", "work_dir", "goal", "branch",
                                  "status", "created", "uuid"})
LOSS_MEMBERS = {
    "common": {"session": ("degraded",), "agent": ()},
    "meta-absent": {"session": ("needs_attention", "mode", "origin",
                                "work_dir", "goal"), "agent": ()},
    "meta-duplicate": {"session": (), "agent": ()},       # data-dependent
    "events-skipped": {"session": ("needs_attention", "goal_set_epoch",
                                   "last_active_epoch", "attention",
                                   "attention_rank"),
                       "agent": ("state", "reason")},
}


def gate_duplicated_meta_keys(case, session):
    """Documented meta keys appearing more than once — SC-405a + SC-509b make a
    duplicate ACTUAL parse loss for THAT KEY, since no row defines duplicate-member
    precedence and first/last-winner selection would be fabrication."""
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


def owed_loss_members(kinds, duplicated=()):
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


def gate_loss_kinds(case, session):
    kinds = set()
    if session in gate_loss_sessions(case):
        kinds.add("meta-absent")
    if gate_duplicated_meta_keys(case, session):
        kinds.add("meta-duplicate")
    if not gate_events_complete(case, session):
        kinds.add("events-skipped")
    assert kinds <= set(LOSS_KINDS), "a kind outside the declared closed list"
    return kinds


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


def gate_events_complete(case, session):
    """Re-derived independently: is the session's ledger free of malformed COMPLETE
    records? SC-975b's buffered unterminated TAIL is exempt and is a DIFFERENT fact —
    measured across every template, exactly one session has each and they are not the
    same session, so a predicate fusing them would be wrong about both."""
    p = os.path.join(SRC, case, "case.txt")
    if not os.path.exists(p):
        return True
    m = re.search(r"\btemplate=(\S+)", open(p, encoding="utf-8", errors="replace").read())
    if not m or "/" not in m.group(1):
        return True
    arm, variant = m.group(1).split("/", 1)
    f = os.path.join(SRC, "templates", arm, "fixture-bytes", variant,
                     "sessions", session, "events.jsonl")
    if not os.path.exists(f):
        return True
    lines = open(f, encoding="utf-8", errors="replace").read().split("\n")
    unterminated = bool(lines) and lines[-1] != ""
    for n, line in enumerate(lines, 1):
        if not line.strip():
            continue
        if n == len(lines) and unterminated:
            continue
        seen = []
        try:
            json.loads(line, object_pairs_hook=lambda ps: (
                seen.extend(k for k, _ in ps), dict(ps))[1])
        except ValueError:
            return False                   # SC-520: a malformed COMPLETE record
        counts = collections.Counter(seen)
        if any(c > 1 and k in EVENT_STABLE_KEYS for k, c in counts.items()):
            return False                   # SC-510e; SC-510f keeps unknown dups inert
    return True


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


def owed_schema_version(case, consumer, text):
    """SC-509d: every digest owes EXACTLY ONE schema_version row.

    The `from` is READ from the capture, never assumed — the same discipline the
    relational rows are being repaired to. A capture that is not a digest owes
    EMPTY, and that empty is compared like any other member.
    """
    if '"schema_version"' not in text:
        return set()
    m = re.search(r'"schema_version"\s*:\s*(\d+)', text)
    return {("SC-509d", "digest", "schema_version", m.group(1) if m else "ABSENT",
             "2", "equals", "SOURCE", "OBSERVED")}


def owed_inventory_complete(text):
    """SC-017o: every digest owes EXACTLY the two fixed rows, presence and value.

    Folded INTO the union rather than left beside it. Enforced-beside meant the
    rows were checked for shape by their own guard while the global consumption
    comparison could not see them, so an SC-017o row at an invented locus was
    unconsumed by nobody.
    """
    if '"schema_version"' not in text:
        return set()
    return {PRESENCE_ROW, VALUE_ROW}


def owed_agent_liveness(case, consumer, text):
    """SC-509e: the UNREACHABLE digest population owes one agents[].alive move.

    The population is the unreachable case set — the same evidence SC-017l/m join
    against — and the row is owed once per digest that carries agent entries. A
    digest with NO agent rows owes EMPTY, and that empty is compared like any other
    member: owed-zero is an obligation to check, not a row to skip.
    """
    if '"schema_version"' not in text or not unreachable(case):
        return set()
    try:
        doc = json.loads(text)
    except ValueError:
        return set()
    has_agents = any((s.get("agents") or [])
                     for s in (doc.get("sessions") or []))
    if not has_agents:
        return set()                     # owes EMPTY, and the empty is compared
    return {("SC-509e", "digest", "agents[].alive", "false", "null", "all-of",
             "OBSERVED", "UNSCORABLE")}


def selector_legs(case):
    """(meta-absent identities, present-but-unusable identities), classified.

    A PREDICATE THAT SCANS ROWS CANNOT SEE A MISSING ROW. The repair is not a
    better marker — it is to derive the population from the DECLARED UNIVERSE and
    SUBTRACT what is present, so absence is COMPUTED rather than detected. Same
    shape as owed-zero, and as asking a capture for `degraded: true` (a
    successor-only key) and reading the empty result as proof of absence. Third
    instance of that shape in this slice.

    So: enumerate the durable candidates first, then classify each. The first
    version scanned rows for a bad mode and could therefore only ever find the
    present-but-unusable leg; it reported 10 invocations where there are 20 and lost
    a9-c05-meta-absent entirely.

    THE TWO LEGS ARE DIFFERENT CAUSES WEARING ONE LABEL, and they are returned
    separately for that reason: `meta-absent` is a missing recorded server,
    `mode-unusable` is an exact live name whose ownership evidence is unreadable —
    SC-017l's first and third clauses. Both land on `unknown`, so the VALUE is the
    same and the CITATION is not. A single `selector-missing` cause value in an
    emitted row would re-collapse the exact distinction whose collapse produced this
    bug, and the next reader would inherit the collapsed form as the analysed one.

    Two legs, returned separately so each can be deleted and re-proved on its own:
    a total hides which leg went missing.
    """
    mb = os.path.join(SRC, case, "manifest.before.tsv")
    if not os.path.exists(mb):
        return (), ()
    rows = [l.rstrip("\n").split("\t") for l in open(mb, encoding="utf-8", errors="replace")]
    candidates = {r[-1].split("/")[2] for r in rows
                  if r[0] == "dir" and re.match(r"\./sessions/[^./][^/]*$", r[-1])}
    metas = {r[-1].split("/")[2]: r[1] for r in rows
             if r[0] == "file" and re.match(r"\./sessions/[^./][^/]*/meta$", r[-1])}
    absent = tuple(sorted(c for c in candidates if c not in metas))
    unusable = tuple(sorted(c for c in candidates
                            if metas.get(c) in ("000", "0", "100", "200")))
    return absent, unusable


def gate_missing_selector(case):
    """Identities whose selector is missing, either leg."""
    absent, unusable = selector_legs(case)
    return sorted(set(absent) | set(unusable))


def durable_candidates(case):
    """Durable session candidates, enumerated from the MANIFEST.

    An independent source: the manifest lists what exists on disk, so a candidate
    the frozen view OMITTED is still enumerable here. That is the whole point —
    absence cannot be read from an output that does not contain it.

    Contract-blob independent, so this is derivable while the pin is held.
    """
    mb = os.path.join(SRC, case, "manifest.before.tsv")
    if not os.path.exists(mb):
        return []
    rows = [l.rstrip("\n").split("\t") for l in open(mb, encoding="utf-8", errors="replace")]
    return sorted({r[-1].split("/")[2] for r in rows
                   if r[0] == "dir" and re.match(r"\./sessions/[^./][^/]*$", r[-1])})


def owed_candidate_rows(case, candidates, support):
    """ONE ROW PER CANDIDATE, which is why the loop exists rather than a scalar.

    Measured on this corpus: all 268 unreachable listing cases carry EXACTLY ONE
    durable candidate, so 268 passes prove nothing about the plural path. The
    synthetic control in the red-proof feeds a fabricated two-candidate view and
    requires two rows — the gap is BOUNDED rather than admitted, exactly as the
    reason grammar's unexercised third branch is.
    """
    return {("SC-017l", "stdout", "candidate[%s].status" % c, "ABSENT", "unknown",
             "equals", "OBSERVED", support) for c in candidates}


SESSION_STATUS_DECLARED = frozenset({"running", "stopped"})


def candidate_representation(case, consumer, text):
    """candidate -> 'omitted' | 'aligned:<status>' | 'live-only-namesake'.

    A live-only namesake is a frozen row carrying the candidate's NAME whose status
    is running while the durable candidate itself is unrepresented — SC-017j keeps
    them two identities, so the namesake does not consume the candidate and the
    candidate is still owed its pair.
    """
    # THE TWO DERIVATIONS SHARED A DEFECT HERE, which is worth more than the defect.
    # This function and the generator's candidate_class were written by one author
    # from one reading, so they carried the SAME unbounded scrape and the SAME
    # catch-all third branch -- and on any future off-universe status they would have
    # AGREED, silently. Agreement between two copies of one mistake is not
    # confirmation; independence only exists where the two actually differ in method.
    # So the repair is deliberately NOT the generator's: it restricts by universe
    # first and reports an unanalysed status through the gate's own failure channel,
    # where the generator asserts and stops. Different mechanism, same bound.
    want = frozenset(durable_candidates(case))
    shown = {}
    if '"schema_version"' in text:
        try:
            for sess in json.loads(text).get("sessions") or []:
                shown[sess.get("name")] = sess.get("status")
        except ValueError:
            pass
    else:
        # SECOND METHOD, NOT A SECOND COPY. The generator bounds the region and reads
        # field 1 of each column-0 row; this binds to the header's own COLUMN OFFSET
        # and takes the first whole token at or after it. Two different structural
        # claims about the same table, so a wrong one diverges instead of agreeing.
        #
        # Probed before adoption, and the probe EARNED ITS KEEP: the naive form
        # (text[offset:]) disagreed on 8 bodies because a 27-character session name
        # overflows the 26-column SESSION field and pushed the status right -- the
        # slice landed mid-name and read its last character, `d`, as the status. Two
        # names in this corpus are at or over the width. Corrected to whole tokens,
        # the two methods agree on all 664 text bodies.
        lines = text.splitlines()
        head = [n for n, l in enumerate(lines) if re.match(r"^SESSION\s+STATUS\b", l)]
        if head:
            offset = lines[head[0]].index("STATUS")
            for line in lines[head[0] + 1:]:
                if not line.strip():
                    break
                if line[0].isspace():
                    continue
                toks = [(m.start(), m.group())
                        for m in re.finditer(r"\S+", line)]
                # A SECOND END CONDITION, not a second copy of the generator's. It
                # bounds the region by NEW-TABLE DETECTION -- three or more all-caps
                # word tokens is a header row, not a session -- where the generator
                # bounds it by column alignment. Both hold on the butted-together
                # fixture and on the overflowing name; a wrong one diverges.
                caps = [v for _, v in toks if v.isalpha() and v.isupper()]
                if len(caps) >= 3:
                    break
                tail = [v for p, v in toks if p >= offset]
                if toks and tail:
                    shown[toks[0][1]] = tail[0]
    out = {}
    for c in sorted(want):
        if c not in shown:
            out[c] = "omitted"
        # SC-017j: a live-only running row carrying the candidate's NAME is a
        # DIFFERENT IDENTITY and does not consume it. There is deliberately no flag
        # here — a seam that can select the known-wrong behaviour is an alternate
        # gate behaviour in shipped code, and it can be invoked by accident or drift
        # into use. The red-proof mutates a SCRATCH COPY of this file instead, so
        # production has one identity-preserving path and no switch.
        elif shown[c] == "running":
            out[c] = "live-only-namesake"
        elif shown[c] in SESSION_STATUS_DECLARED:
            out[c] = "aligned:%s" % shown[c]
        else:
            # Not classified. An unanalysed status becomes a NAMED value the gate
            # fails on, never a silent member of the aligned class.
            out[c] = "unanalysed-status:%s" % shown[c]
    return out


def owed_loss(case, doc):
    """The COMPLETE owed multiset for the LOSS families, MAPPING-DRIVEN.

    The owed member set is DERIVED from the declared source-to-member mapping — the
    checker reads the terms it enforces rather than carrying a copy. The previous
    control transcribed a four-member list and was therefore self-confirming: it
    caught a dropped ROW and could never catch a dropped MEMBER CLASS, which is
    exactly how a narrowed population greened.

    The population itself is proved from FIXED sources — the manifest for meta loss,
    the ledger and the contract's SC-520/SC-510e rules for event loss — never from a
    `degraded` key, which is successor-only and identifies nothing per member.
    """
    owed = set()
    for x in doc.get("sessions", []) or []:
        nm = x.get("name") or ""
        kinds = gate_loss_kinds(case, nm)
        if not kinds:
            continue
        smem, amem = owed_loss_members(kinds, gate_duplicated_meta_keys(case, nm))
        for member in smem:
            # Same asymmetry as the generator: `degraded` is SUCCESSOR-ONLY and never
            # in a frozen capture — its obligation IS its absence — so the
            # present-in-capture guard must not reach it. Fixing this in the
            # generator alone left the two derivations disagreeing on all 29.
            if member != "degraded" and member not in x:
                continue
            val = x.get(member)
            rendered = "null" if val is None else str(val).lower()
            if member == "degraded":
                owed.add(("SC-509b", "digest", "sessions[%s].degraded" % nm,
                          "present" if "degraded" in x else "ABSENT", "true",
                          "equals", "OBSERVED", "OBSERVED"))
            elif member == "needs_attention":
                owed.add(("SC-509b", "digest", "sessions[%s].needs_attention" % nm,
                          rendered, "false", "equals", "OBSERVED", "OBSERVED"))
            else:
                owed.add(("SC-509b", "digest", "sessions[%s].%s" % (nm, member),
                          rendered, "ABSENT", "equals", "OBSERVED", "OBSERVED"))
        for a in x.get("agents") or []:
            for member in amem:
                if member not in a:
                    continue
                av = a.get(member)
                owed.add(("SC-509b", "digest",
                          "sessions[%s].agents[%s].%s" % (nm, a.get("ref"), member),
                          "null" if av is None else str(av).lower(), "ABSENT",
                          "equals", "OBSERVED", "OBSERVED"))
        if "branch" in x:
            owed.add(("SC-405g", "digest", "sessions[%s].branch (presence)" % nm,
                      "present", "ABSENT", "equals", "OBSERVED", "OBSERVED"))
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
            # ONE WRITER PER LOCUS: when the LOSS derivation owns this member for this
            # session, owed_loss emits it and the stopped family must not.
            if decl.get(ref) and a.get("state") in (None, "") \
                    and "state" not in owed_loss_members(gate_loss_kinds(case, nm), gate_duplicated_meta_keys(case, nm))[1]:
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
        # B4: THE BASELINE IS READ HERE TOO, from the same capture the generator
        # reads — independently, so the two can disagree. Both sides held literals
        # before this, so the 36 `from` cells were shared constants under a module
        # whose FROM check is named "re-read, never trusted".
        for locus, below, above in (("needs_attention", "false", "true"),
                                    ("attention", "null", "unanswered"),
                                    ("attention_rank", "0", "1")):
            frm = "null" if x.get(locus) is None else str(x.get(locus)).lower()
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

def main(quiet=False, obl=None, fresh=None, inv=None, gap=None, unproved=None):
    out = []
    # FRESHNESS plus its three data members — OBLIGATIONS, the added-roster
    # declaration and SC-509C-UNPROVED — form one generated four-file tuple. A
    # partial override once compared a temporary table against the live declaration
    # and manufactured a semantic mismatch. Refuse that crossing: callers either
    # select the entire tuple or the entire live set.
    artifact_overrides = (obl, fresh, gap, unproved)
    if any(p is not None for p in artifact_overrides) and not all(
            p is not None for p in artifact_overrides):
        fail(out, "ARTIFACT-TUPLE",
             "--obl, --fresh, --gap and --unproved must be overridden together; "
             "partial selection crosses generated artifact sets")
        if not quiet:
            for cid, msg in out:
                print("FAIL  %-14s %s" % (cid, msg))
        return 1, {c for c, _ in out}
    obl, fresh, gap, unproved, inv = (obl or OBL, fresh or FRESH, gap or GAP,
                                      unproved or UNPROVED, inv or INV)
    for p in (obl, fresh, gap, unproved, inv):
        if not os.path.exists(p):
            print("FAIL  MISSING  %s" % os.path.basename(p)); return 1

    # Snapshot every generated DATA member exactly once, then read the manifest
    # LAST. Semantic checks below parse these exact in-memory bytes. Reopening a path
    # after hashing would admit a rename between the hash and parse (TOCTOU), making
    # a mixed generation look coherent even though no coherent snapshot was read.
    data_bytes = {}
    for path in (obl, gap, unproved):
        with open(path, "rb") as fh:
            data_bytes[path] = fh.read()
    with open(fresh, "rb") as fh:
        fresh_bytes = fh.read()
    obl_text = data_bytes[obl].decode("utf-8")
    gap_text = data_bytes[gap].decode("utf-8")
    unproved_text = data_bytes[unproved].decode("utf-8")
    fresh_text = fresh_bytes.decode("utf-8")
    # Exact manifest schema. A dict comprehension would silently choose a winner for
    # duplicate claims, so a file stating both X and !X could verify when the matching
    # value happened to come last. Comments are permitted; the sole data header and
    # every key/value record are closed and unique.
    manifest_lines = [line.rstrip("\n") for line in io.StringIO(fresh_text)
                      if line.strip() and not line.startswith("#")]
    rec, rec_counts = {}, collections.Counter()
    if not manifest_lines or manifest_lines[0] != "field\tvalue":
        fail(out, "FRESHNESS-SCHEMA",
             "%s must start its data region with exact header field<TAB>value" % fresh)
    for lineno, line in enumerate(manifest_lines[1:], 2):
        fields = line.split("\t")
        if len(fields) != 2:
            fail(out, "FRESHNESS-SCHEMA", "%s data row %d has %d fields, expected 2"
                 % (fresh, lineno, len(fields)))
            continue
        key, value = fields
        rec_counts[key] += 1
        rec[key] = value
    missing = sorted(FRESH_REQUIRED_FIELDS - set(rec_counts))
    unknown = sorted(set(rec_counts) - FRESH_REQUIRED_FIELDS)
    duplicated = sorted(key for key, count in rec_counts.items() if count != 1)
    if missing:
        fail(out, "FRESHNESS-SCHEMA", "%s omits required field(s) %s"
             % (fresh, missing))
    if unknown:
        fail(out, "FRESHNESS-SCHEMA", "%s carries unknown field(s) %s"
             % (fresh, unknown))
    if duplicated:
        fail(out, "FRESHNESS-SCHEMA", "%s repeats field(s) %s; manifests have no "
             "winner rule" % (fresh, duplicated))
    if rec.get("contract_path") != CONTRACT:
        fail(out, "FRESHNESS-SCHEMA", "%s records contract_path=%r, expected %r"
             % (fresh, rec.get("contract_path"), CONTRACT))

    # FRESHNESS is the content manifest for the other three generator outputs and
    # publishes last. Counts alone cannot distinguish equal-cardinality generations;
    # exact hashes make any mixed live/temp or old/new tuple a named failure.
    for field, path, content in (
            ("obligations_sha256", obl, data_bytes[obl]),
            ("added_roster_gap_sha256", gap, data_bytes[gap]),
            ("sc509c_unproved_sha256", unproved, data_bytes[unproved])):
        got = rec.get(field)
        actual = hashlib.sha256(content).hexdigest()
        if got is None:
            fail(out, "ARTIFACT-TUPLE", "%s records no %s content identity"
                 % (fresh, field))
        elif got != actual:
            fail(out, "ARTIFACT-TUPLE", "%s records %s=%s but %s hashes to %s"
                 % (fresh, field, got, path, actual))

    # ---- 1. FRESHNESS: has the source moved since this was derived? ----
    # THE CONTRACT SIDE IS HEAD-RELATIVE, DELIBERATELY; the tuple side is the loaded
    # WORKTREE snapshot. In CI those coincide with committed bytes. In a shared
    # developer checkout they may not, so the success line names both read sides.
    # ---- B5: A FRESHNESS FILE MAY NOT PUBLISH UNCHECKED NUMBERS. Both counts were
    # decoration: seeding p1_rows=999 and obligation_rows=7 passed rc=0. They are
    # now bound to INDEPENDENTLY LOADED inputs — the P1 rows this gate reads from
    # INVOCATIONS and the data rows it reads from the table — so a recorded number
    # that stopped matching what it counts is a named failure rather than prose.
    def _count_check():
        want_p1 = sum(1 for r in csv.DictReader(open(inv, encoding="utf-8"),
                                                delimiter="\t") if r["phase"] == "P1")
        want_obl = len(obl_text.splitlines()) - 1
        for field, want in (("p1_rows", want_p1), ("obligation_rows", want_obl)):
            got = rec.get(field)
            if got is None:
                fail(out, "FRESHNESS-COUNT", "%s records no %s; a freshness file may "
                     "not omit a number it is meant to publish" % (fresh, field))
            elif got.strip() != str(want):
                fail(out, "FRESHNESS-COUNT", "%s records %s=%s but the loaded inputs "
                     "carry %d" % (fresh, field, got, want))
    _count_check()
    now = head_blob()
    if rec.get("contract_blob") != now:
        fail(out, "STALE", "derived against contract blob %s; HEAD is %s — re-derive"
             % (rec.get("contract_blob", "?")[:12], now[:12]))

    obls = list(csv.DictReader(io.StringIO(obl_text), delimiter="\t"))
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
        # ---- SC-017l/m, COMPARED FOR EVERY INVOCATION, not only the digest ones.
        # This comparison first sat inside `if digest_text(text) is not None:` and
        # the gate went green while 274 of the 434 l/m rows -- every stdout row --
        # were compared by NOTHING. Two red-proof seeds went MISSED and that is the
        # only reason it surfaced: the mutations they applied happened to land on
        # human rows. Third instance today of the same shape, and the first one in a
        # check rather than in a population: a comparison inherits the scope of the
        # block it is written in, so where a check LIVES is part of what it asserts.
        owed_lm = owed_unknown_v(case, consumer, r["normalised_argv"], text,
                                 r["surface"])
        compare_owed(out, case, consumer, "unknown liveness l/m", owed_lm,
                     held_shapes(carriers, case, consumer,
                                 ("SC-017l", "SC-017m")))
        compare_owed(out, case, consumer, "agent declared state h",
                     owed_agent_state_v(case, consumer, text, r["surface"]),
                     held_shapes(carriers, case, consumer, ("SC-017h",)))
        compare_owed(out, case, consumer, "agent health r",
                     owed_agent_health_v(case, consumer, text, r["surface"]),
                     held_shapes(carriers, case, consumer, ("SC-017r",)))
        compare_owed(out, case, consumer, "schema version",
                     owed_schema_version(case, consumer, text),
                     held_shapes(carriers, case, consumer, ("SC-509d",)))
        compare_owed(out, case, consumer, "inventory completeness",
                     owed_inventory_complete(text),
                     held_shapes(carriers, case, consumer, ("SC-017o",)))
        compare_owed(out, case, consumer, "agent liveness e",
                     owed_agent_liveness(case, consumer, text),
                     held_shapes(carriers, case, consumer, ("SC-509e",)))
        # THE EXCLUSIVE-SPLIT CHECK IS GONE, and its removal is the ruling rather
        # than a convenience. It required a listing to owe EITHER SC-017l OR
        # SC-017m and failed WRONG-KIND on both -- 138 findings against the
        # re-derived table. The pinned contract retracts exactly that: "Absence is
        # owned HERE and ALSO by SC-017m, at DIFFERENT GRAINS (ruling, colead
        # 2026-08-25, retracting an earlier exclusive split): an omitted durable
        # candidate owes BOTH." What replaces it is stronger, because it is the
        # invariant the contract states: every owed shape is derived per candidate
        # and compared in one place, so a missing pair member fails as OWED-MISSING
        # at its own identity instead of as a kind dispute at the invocation.
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
            # `want_b` and its MISSING-509b / SURFACE pair are DELETED for the same
            # reason `want_c` was: they know only the READ-LOSS trigger for SC-509b,
            # so a row owed because a session's event LEDGER is damaged reads to them
            # as a degraded move with no read-loss session. The exact Counter owns
            # this population in both directions.
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
            # ---- W3: THE GLOBAL CONSUMPTION COMPARISON. The union of every
            # family's owed set is compared against ALL held rows whose id has a
            # family — no `where` filter, because a filter on the HELD side
            # re-scopes the gate to the population its author imagined and
            # discards additions outside it before they can be seen. An id with no
            # family cannot consume anything and is listed in NO_OWED_FAMILY as
            # STAGING ONLY.
            union = (owed_loss(case, live_doc)
                     | owed_reason(case, live_doc)
                     | owed_stopped(case, live_doc)
                     | owed_schema_version(case, consumer, text))
            union |= owed_521
            union |= owed_inventory_complete(text)
            union |= owed_agent_liveness(case, consumer, text)
            union |= owed_lm
            held_all = held_shapes(carriers, case, consumer, OWED_FAMILY_IDS)
            for shape, n in (held_all - collections.Counter(union)).items():
                fail(out, "UNCONSUMED-ROW", "%s/%s carries %s x%d, which no owed family "
                     "claims — the held side is compared in its entirety, so an "
                     "addition outside every derivation is visible here"
                     % (case, consumer, shape[:3], n))
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
    # THE PARTITION PRINTS ITS OWN COUNT. I wrote "four gaps" in a report and then
    # listed five ids — prose carrying a count that the code already knows is the
    # same defect as a control carrying a copy of its list, one register down.
    # THE GAP CHECK RUNS WHETHER OR NOT ANYONE IS LOOKING. It is a gate check, not
    # a report line, so it sits OUTSIDE the quiet guard -- a check that only fires
    # when output is enabled is a check with a flag that disables it.
    in_table = collections.Counter(o["obligation_id"] for o in obls
                                   if o["obligation_id"] in OWED_FAMILY_IDS)
    for oid in sorted(set(in_table) | set(_rows_compared)):
        if in_table[oid] != _rows_compared[oid]:
            fail(out, "COVERAGE", "%s: %d row(s) in the table, %d reached a "
                 "comparison — a check compares what its enclosing scope lets it "
                 "reach, and the difference is invisible unless it is counted"
                 % (oid, in_table[oid], _rows_compared[oid]))
    n_gap = gate_added_roster_gap(out, added_roster_gap_population(),
                                  gap_text, gap)
    if not quiet:
        print("  UNOBSERVABLE-ADDED-ROSTER: %d occurrence(s) enumerated; SC-017h/SC-017r "
              "completeness is refused while nonempty; declaration matched "
              "in both directions" % n_gap)
        print("  owed-family partition: %d id(s) with a family, %d STAGING-ONLY gap(s) "
              "(%s)" % (len(OWED_FAMILY_IDS), len(NO_OWED_FAMILY),
                        ", ".join(sorted(NO_OWED_FAMILY))))
    # ---- THE AUTHORITY SIGNATURE, checked. The prefix is the bytes that
    # distinguish an entitled row from an unreasoned coincidence, so it joins the
    # comparison; the prose tail stays narrative. The gate CONSTRUCTS the expected
    # signature independently — a disagreement is a finding against one side, never
    # a resolution. And retired terms are forbidden across the WHOLE field, prefix
    # and prose alike: a retired term in narrative still teaches the wrong rule.
    for o in obls:
        auth = o.get("authority") or ""
        low = auth.lower()
        for term in RETIRED_AUTHORITY_TERMS:
            if term in low:
                fail(out, "AUTHORITY-RETIRED", "%s/%s %s carries the retired term %r in "
                     "its authority" % (o["case"], o["consumer"], o["locus"], term))
                break
        if o["obligation_id"] not in ("SC-509b", "SC-405g"):
            continue
        sig = parse_signature(auth)
        if sig is None:
            fail(out, "AUTHORITY-SIGNATURE", "%s/%s %s carries no parseable signature "
                 "prefix; a from==to row without one is indistinguishable from an "
                 "unreasoned coincidence" % (o["case"], o["consumer"], o["locus"]))
            continue
        owner, _kinds, member, klass = sig
        if owner != o["obligation_id"]:
            fail(out, "AUTHORITY-SIGNATURE", "%s/%s %s signs owner=%s on an %s row"
                 % (o["case"], o["consumer"], o["locus"], owner, o["obligation_id"]))
        if member != o["locus"]:
            fail(out, "AUTHORITY-SIGNATURE", "%s/%s signs member=%s on locus %s"
                 % (o["case"], o["consumer"], member, o["locus"]))
        if klass not in ENTITLEMENT_CLASSES:
            fail(out, "AUTHORITY-SIGNATURE", "%s/%s %s signs entitlement class %r, "
                 "which is not in the closed vocabulary"
                 % (o["case"], o["consumer"], o["locus"], klass))
        elif o["to"] == "ABSENT" and klass not in (
                "unreadable-member-omits", "temporary-presence-projection/value-unscored"):
            fail(out, "AUTHORITY-SIGNATURE", "%s/%s %s omits a member but signs %r"
                 % (o["case"], o["consumer"], o["locus"], klass))
        elif o["from"] == o["to"] and klass != "partial-evidence-from-readable-facts":
            fail(out, "AUTHORITY-SIGNATURE", "%s/%s %s is a from==to row signed %r"
                 % (o["case"], o["consumer"], o["locus"], klass))
    check_keyset(out, obls, quiet)
    if not quiet:
        print("verdict (DERIVED, no stored column): %d EXPECTED-DIVERGENCE + %d EXPECTED-MATCH "
              "= %d P1 rows" % (divergence, len(universe) - divergence, len(universe)))
        for k in sorted(per): print("  %-10s %4d" % (k, per[k]))
        for cid, msg in out[:20]: print("FAIL  %-14s %s" % (cid, msg))
        if not out:
            print("OBLIGATIONS VERIFIED — WORKTREE table tuple is fresh against "
                  "COMMITTED contract %s at HEAD" % now[:12])
            print("  (contract is read from HEAD; OBLIGATIONS/FRESHNESS/GAP/UNPROVED "
                  "are read from the worktree; a clean checkout may differ)")
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
    ap.add_argument("--gap")
    ap.add_argument("--unproved")
    ap.add_argument("--inv")
    a = ap.parse_args()
    sys.exit(main(obl=a.obl, fresh=a.fresh, gap=a.gap,
                  unproved=a.unproved, inv=a.inv)[0])
