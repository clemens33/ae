#!/usr/bin/env python3
"""T-WD design consistency checker.

WHY THIS IS NOT AWK. The first version used awk `\\<...\\>` word boundaries, which
BWK awk on macOS does not implement: it matched nothing, silently, and reported the
document clean while printing the very term it failed to find. A checker must not be
exposed to the portability class it exists to police.

WHY THESE CHECKS ARE CONTRACTS, NOT SPELLING FILTERS. The second version matched the
literal "worker draft **vN" and five count phrasings — so "worker draft v9" and
"Execution units = 999" both passed. Worse, its self-test SEEDED EXACTLY THE SPELLINGS
THE PREDICATES ACCEPTED, so the red-proof was circular: it proved the predicate matches
what the predicate matches. Every check below is either a structural contract (an exact
expected form, or a class of construct forbidden outright) or a join between two
independently-parsed structures. The self-test seeds OPPOSED spellings on purpose.

WHY EVERY FAILURE CARRIES A STABLE ID. The previous self-test credited ANY non-zero
exit, so an unrelated failure could keep a dead check green. Each mutation now declares
the id it must provoke and a bound on how many lines it may change; a mutation whose
delta exceeds its bound is not a local test of one check.
"""
import re, sys, os, difflib

TITLE = "# T-WD design — watchdog cluster — worker draft (NOTHING APPROVED, NOTHING RUN)"
CLASSES = {"RED", "**CAPTURE-ONLY**", "**GAP — does not run**"}
LANES = {"—", "bash+uv"}
ROSTER_HDR = "| # | arm id | row | class | lanes | M12 |"
BAR_HDR = "| id | site | frozen anchor |"
SURFACE_HDR = "| row | neutral surface line | family |"
HEAD = re.compile(r"^#### (\d+)\. `([^`]+)` — (.*)$")
# ONLY the countables a table derives. A broader list flagged legitimate prose
# ("two gates", "three-part requirement") and would have trained me to ignore it.
DERIVED_KEYS = ["specs", "red", "capture", "gap", "two", "runnable", "units", "m12"]
# Every key the block derives, plus the prose nouns. The belt previously covered the
# nouns and NONE of the keys, so "Runnable population = 999" passed while the design
# claimed unknown representations were impossible.
COUNTABLE = r"(?:arms?|arm specs?|specs?|executed units?|units?|barriers?|%s|populations?)" % "|".join(DERIVED_KEYS)

# ---- quantities, as a GENERATIVE grammar rather than a list ----
# English cardinals are a CLOSED GENERATIVE SYSTEM, so the acceptor is built from the
# grammar instead of enumerating members. The previous contract listed number words up
# to "forty" and took NUMERALS as its subject; "forty-four executed units" walked
# through both — the hyphen defeated the list AND the `(?<![-\w])` lookbehind that had
# been added to stop `GATE-2 ARM` false-firing. A contract over ONE REPRESENTATION of a
# thing is still a filter; this one is over the CATEGORY.
_UNITS = "zero one two three four five six seven eight nine".split()
_TEENS = ("ten eleven twelve thirteen fourteen fifteen sixteen seventeen eighteen "
          "nineteen").split()
_TENS  = "twenty thirty forty fifty sixty seventy eighty ninety".split()
_SCALE = "hundred thousand million".split()
_NUMWORD = set(_UNITS) | set(_TEENS) | set(_TENS) | set(_SCALE) | {"a", "no", "every", "all", "several", "many", "few", "both"}
_QTY_TOKEN = r"[A-Za-z]+(?:-[A-Za-z]+)*"

def _is_quantity(tok):
    """A token is a quantity if it is an integer, or if every hyphen/space-joined part
    is a member of the cardinal grammar. Composition is by rule: 'forty-four',
    'twenty-three', 'one hundred' are accepted without being listed."""
    t = tok.strip().lower().strip(".,;:")
    if not t: return False
    if t.isdigit(): return True
    parts = re.split(r"[-\s]+", t)
    return bool(parts) and all(p in _NUMWORD for p in parts)

COUNTABLE_RE = re.compile(COUNTABLE, re.I)

def find_count_claims(line):
    """Any quantity ADJACENT to a derived countable, in either order. Adjacency is
    token-wise so no spelling needs listing — but it must not cross a SENTENCE
    boundary and must not read a citation's line number or a heading's index as a
    quantity. All three produced false positives on the first run ("## 2. Arm
    classes", "(aewatch:1618). The arm", "without one. Barrier reachability"), and a
    checker that cries wolf trains its author to skim it."""
    if line.lstrip().startswith("#"): return []
    line = re.sub(r"\b[A-Za-z_][A-Za-z0-9_.-]*:\d+(?:[-–]\d+)?", " ", line)   # citations
    hits = []
    for sent in re.split(r"(?<=[.;])\s+", line):
        hits += _scan_sentence(sent)
    return hits

def _scan_sentence(line):
    toks = re.findall(r"[A-Za-z0-9][A-Za-z0-9-]*|=|:", line)
    hits = []
    for i, t in enumerate(toks):
        if not COUNTABLE_RE.fullmatch(t): continue
        for j in (i - 1, i - 2):                       # "44 arm specs", "forty-four executed units"
            if j >= 0 and _is_quantity(toks[j]) and toks[j].lower() not in ("a", "no", "every", "all", "both", "several", "many", "few"):
                hits.append("%s %s" % (toks[j], t)); break
        for j in (i + 1, i + 2):                       # "units = 999", "units: 44"
            if j < len(toks) and toks[j] in ("=", ":") and j + 1 < len(toks) and _is_quantity(toks[j + 1]):
                hits.append("%s %s %s" % (t, toks[j], toks[j + 1])); break
    return hits

ORD = re.compile(r"\barms?\s*(?:#|no\.\s*)?\d+", re.I)

class Out(list):
    def add(self, cid, msg): self.append((cid, msg))

# ---------------- structure parsing ----------------
def table(lines, hdr, ncells, out=None, name=""):
    """ncells was ACCEPTED AND IGNORED, so an extra cell in the barrier table passed."""
    try: i = lines.index(hdr)
    except ValueError: return None, None, None
    rows, j = [], i + 2
    while j < len(lines) and lines[j].startswith("|"):
        c = [x.strip() for x in lines[j].strip().strip("|").split("|")]
        if out is not None and len(c) != ncells:
            out.add("TABLE-ARITY", "%s row line %d has %d cells, expected %d" % (name, j + 1, len(c), ncells))
        rows.append((j, c)); j += 1
    return i, j, rows

def parse_roster(L, out):
    i, j, rows = table(L, ROSTER_HDR, 6)
    if rows is None: out.add("ROSTER-ABSENT", "roster header absent"); return None, None, None
    if not rows: out.add("ROSTER-EMPTY", "roster is empty"); return None, None, None
    parsed = []
    for ln, c in rows:
        if len(c) != 6:
            out.add("ROSTER-ARITY", "row line %d has %d cells, expected 6" % (ln + 1, len(c))); return None, None, None
        n, aid, row, cls, lane, m12 = c
        aid = aid.strip("`")
        if not n.isdigit(): out.add("ROSTER-ORD", "non-numeric ordinal %r" % n); return None, None, None
        if cls not in CLASSES: out.add("ROSTER-CLASS", "row %s: unknown class %r" % (n, cls))
        if lane not in LANES: out.add("ROSTER-LANE", "row %s: unknown lane %r" % (n, lane))
        if m12 != "—" and not re.fullmatch(r"§4\.\d|baseline", m12):
            out.add("ROSTER-M12", "row %s: malformed M12 field %r" % (n, m12))
        parsed.append((int(n), aid, row, cls, lane, m12))
    ords = [p[0] for p in parsed]
    if ords != list(range(1, len(ords) + 1)):
        out.add("ROSTER-SEQ", "ordinals are not unique-and-contiguous from 1")
    ids = [p[1] for p in parsed]
    dup = sorted({x for x in ids if ids.count(x) > 1})
    if dup: out.add("ROSTER-DUPID", "duplicate arm ids: %s" % dup)
    return parsed, i, j

SECTION = re.compile(r"^#{1,3} ")

def body_end(L, start):
    """An arm body ends at the next arm heading OR the next section heading.
    Running it to EOF made the LAST arm swallow every later section — 151 false
    BAR-ARM reports on the first run, all from one bound."""
    for n in range(start + 1, len(L)):
        if HEAD.match(L[n]) or SECTION.match(L[n]): return n
    return len(L)

def field(block, name):
    """A field runs from its bullet to the next bullet — Barriers fields WRAP,
    and reading only the first line under-collects the declared set."""
    acc, on = [], False
    for l in block:
        if l.startswith("- **%s**" % name): on = True; acc.append(l); continue
        if on:
            if l.startswith("- **"): break
            acc.append(l)
    return " ".join(acc)

def parse_bodies(L, out):
    bodies = []
    for n, l in enumerate(L):
        m = HEAD.match(l)
        if m:
            num, aid, tail = int(m.group(1)), m.group(2), m.group(3)
            # STRICT: an unrecognised heading tail previously DEFAULTED to RED, so a
            # body class nobody defined passed as the commonest one. Default-to-valid is
            # the shape that makes a type check decorative.
            # SUBSTRING recognizers, not exact-form checks: "DECLARED GAPISH" and
            # "CAPTURE-ONLYISH" are accepted as their base classes. This catches an
            # UNKNOWN tail, which is what defaulted to RED before; it does not catch a
            # MALFORMED one. Stated because the tool should say what it does.
            if "DECLARED GAP" in tail: cls = "**GAP — does not run**"
            elif "CAPTURE-ONLY" in tail: cls = "**CAPTURE-ONLY**"
            elif re.match(r"RED\b", tail): cls = "RED"
            else:
                cls = None
                out.add("BODY-CLASS", "arm `%s`: heading tail names no known class: %r" % (aid, tail[:40]))
            bodies.append((num, aid, cls, n))
    ids = [b[1] for b in bodies]
    dup = sorted({x for x in ids if ids.count(x) > 1})
    if dup: out.add("BODY-DUPID", "duplicate arm ids among bodies: %s" % dup)
    return bodies

# ---------------- B1: the join ----------------
def check_join(roster, bodies, out):
    r = {(p[0], p[1], p[3]) for p in roster}
    b = {(x[0], x[1], x[2]) for x in bodies}
    for miss in sorted(r - b):
        out.add("JOIN-ROSTER-ONLY", "roster row %s `%s` (%s) has no matching body heading" % miss)
    for extra in sorted(b - r):
        out.add("JOIN-BODY-ONLY", "body heading %s `%s` (%s) has no matching roster row" % extra)
    rid = {p[1]: p for p in roster}; bid = {x[1]: x for x in bodies}
    for aid in sorted(set(rid) & set(bid)):
        if rid[aid][0] != bid[aid][0]:
            out.add("JOIN-ORDINAL", "`%s`: roster ordinal %d, body ordinal %d" % (aid, rid[aid][0], bid[aid][0]))
        if rid[aid][3] != bid[aid][2]:
            out.add("JOIN-CLASS", "`%s`: roster class %r, body class %r" % (aid, rid[aid][3], bid[aid][2]))

def derive(roster):
    gap = [p for p in roster if "GAP" in p[3]]; cap = [p for p in roster if "CAPTURE-ONLY" in p[3]]
    red = [p for p in roster if p[3] == "RED"]; two = [p for p in roster if p[4] == "bash+uv"]
    m12 = [p for p in roster if p[5].startswith("§4.") and "GAP" not in p[3]]
    runnable = len(roster) - len(gap)
    return dict(specs=len(roster), red=len(red), capture=len(cap), gap=len(gap),
                two=len(two), runnable=runnable, units=runnable + len(two), m12=len(m12))

# ---------------- zones ----------------
def zones(L, ri, rj):
    """protected = everything normative; exempt = ONLY the exact roster and the exact
    execution-order region. Arbitrary tables are NOT exempt — the previous version
    exempted every table row, which un-protected the neutral surface table."""
    exempt = set(range(ri, rj)) if ri is not None else set()
    for k, l in enumerate(L):
        if l.startswith("**Execution order"):
            e = k
            while e < len(L) and L[e].strip(): e += 1
            exempt |= set(range(k, e))
        if l.startswith("> "): exempt.add(k)
    hist = next((k for k, l in enumerate(L) if l.startswith("## 6.")), len(L))
    exempt |= set(range(hist, len(L)))
    return exempt

# ---------------- B3: structural contracts ----------------
def check_title(L, out):
    if not L or L[0] != TITLE:
        out.add("TITLE", "title line is not the exact expected form (any version numeral or edit fails)")

COUNT_BLOCK_OPEN = "```counts (generated by t-wd-check.py --emit-counts; do not hand-edit)"

def count_block(L):
    """Locate the generated counts block, which is regenerated and compared on every
    run — so the BLOCK's own figures cannot drift.

    WHAT THIS DOES NOT DO. Excluding counts from prose is a RECOGNIZER BELT, not
    provenance enforcement: it reads decimal integers and English cardinals only.
    Roman numerals and hexadecimal pass. An earlier version of this docstring claimed
    the opposite and is corrected here rather than left as the tool's own account of
    itself."""
    try: i = L.index(COUNT_BLOCK_OPEN)
    except ValueError: return None, None
    j = i + 1
    while j < len(L) and not L[j].startswith("```"): j += 1
    return i, j

def render_counts(d):
    return ["%s=%s" % (k, d[k]) for k in sorted(d)]

def check_count_block(L, d, out):
    i, j = count_block(L)
    if i is None:
        out.add("COUNT-BLOCK-ABSENT", "the generated counts block is missing — counts have no home"); return set()
    have = [x for x in L[i + 1:j] if x.strip()]
    want = render_counts(d)
    if have != want:
        out.add("COUNT-BLOCK", "counts block does not match the roster: have %s, roster yields %s" % (have, want))
    return set(range(i, j + 1))

def check_counts(L, exempt, d, out):
    for n, l in enumerate(L):
        if n in exempt or l.startswith(">"): continue
        s = re.sub(r"`[^`]*`", "", l)
        for h in find_count_claims(s):
            out.add("COUNT-CLAIM", "line %d: quantity adjacent to a derived countable (%r) — counts live ONLY in the generated block" % (n + 1, h))

def check_ordinals(L, exempt, out):
    para = []
    def scan(p):
        joined = " ".join(x[1] for x in p)
        for m in ORD.finditer(joined):
            off, ln = 0, p[0][0]
            for n, l in p:
                if off + len(l) + 1 > m.start(): ln = n; break
                off += len(l) + 1
            if ln not in exempt:
                out.add("ORDINAL", "line %d: ordinal arm reference %r — use a stable arm id" % (ln + 1, m.group(0)))
    for n, l in enumerate(L):
        if not l.strip():
            if para: scan(para)
            para = []
        elif n in exempt: 
            if para: scan(para)
            para = []
        else: para.append((n, l))
    if para: scan(para)

# ---------------- vocabulary over the FULL protected scope ----------------
def derive_terms(text, out):
    m = re.search(r"A committed linter rejects the design.*?`([^`]*)`\.", text, re.S)
    if not m:
        out.add("TERMS", "cannot derive lint terms from M1 — refusing to run a shorter list"); return []
    return [t.strip() for t in m.group(1).replace("\n", " ").split("|") if t.strip()]

def blank_code(L):
    """Backtick spans WRAP ACROSS LINES — M1's own term declaration is one. Blanking
    per line leaves the tail of a wrapped span exposed, which reported M1's
    declaration of the banned terms as a use of them."""
    text = "\n".join(L)
    out, i, n = [], 0, len(text)
    while i < n:
        j = text.find("`", i)
        if j < 0: out.append(text[i:]); break
        k = text.find("`", j + 1)
        if k < 0: out.append(text[i:]); break
        out.append(text[i:j]); out.append(re.sub(r"[^\n]", " ", text[j:k + 1])); i = k + 1
    return "".join(out).split("\n")

def check_vocab(L, exempt, terms, out):
    if not terms: return
    B = blank_code(L)
    pat = re.compile(r"\b(" + "|".join(re.escape(t) for t in terms) + r")\b", re.I)
    for n, l in enumerate(B):
        if n in exempt or L[n].startswith(">"): continue
        m = pat.search(l)
        if m: out.add("VOCAB", "line %d: banned term %r in normative text" % (n + 1, m.group(1)))

# ---------------- barriers: table, both directions, per-arm association ----------------
def check_barriers(L, bodies, out):
    i, j, rows = table(L, BAR_HDR, 3, out, "barrier table")
    if rows is None: out.add("BAR-TABLE", "typed barrier table absent"); return
    ids = []
    for ln, c in rows:
        m = re.match(r"`([^`]+)`", c[0])
        if m: ids.append(m.group(1))
    dup = sorted({x for x in ids if ids.count(x) > 1})
    if dup: out.add("BAR-DUP", "duplicate ids in the barrier table: %s" % dup)
    tbl = set(ids); used = set()
    # Only CUT-/BAR- shaped ids are collected, so a token like `NOT-A-BARRIER` in a
    # Barriers field is INVISIBLE here rather than reported as unknown. The check is
    # "declared ids exist and are used", not "the field contains only barrier ids".
    for n, l in enumerate(L):
        if l.startswith(">") or (i <= n < j): continue
        used |= set(re.findall(r"`((?:CUT|BAR)-[A-Z0-9]+-[A-Z0-9-]+)`", l))
    for x in sorted(used - tbl): out.add("BAR-UNKNOWN", "barrier id used but not in the typed table: %s" % x)
    for x in sorted(tbl - used): out.add("BAR-ORPHAN", "barrier id in the typed table but never used: %s" % x)
    # per-arm association: any barrier named inside an arm must appear in its Barriers field
    BID = r"`((?:CUT|BAR)-[A-Z0-9]+-[A-Z0-9-]+)`"
    for (num, aid, cls, start) in bodies:
        block = L[start:body_end(L, start)]
        # An arm is its FIELD BULLETS. Section preambles and tables that happen to
        # follow the last arm of a section are NOT part of it — reading raw block
        # text attributed a whole section's barrier table to the arm above it.
        fields, cur = {}, None
        for l in block:
            m = re.match(r"- \*\*([^*]+)\*\*", l)
            if m: cur = m.group(1); fields.setdefault(cur, []).append(l)
            elif cur and l.startswith("  "): fields[cur].append(l)
            elif not l.strip(): cur = None
            else: cur = None
        declared = set(re.findall(BID, " ".join(fields.get("Barriers", []))))
        mentioned = set()
        for k, v in fields.items():
            if k == "Barriers": continue
            mentioned |= set(re.findall(BID, " ".join(v)))
        for x in sorted(mentioned - declared):
            out.add("BAR-ARM", "arm `%s` names %s in its fields but does not declare it under Barriers" % (aid, x))

# ---------------- candidate space, keyed from the JOINED row ----------------
def check_candidates(L, roster, bodies, out):
    rid = {p[1]: p for p in roster}
    for (num, aid, cls, start) in bodies:
        r = rid.get(aid)
        if r is None or r[3] != "RED": continue
        block = L[start:body_end(L, start)]
        # A FIELD BULLET, not any line: the field was previously satisfied by the
        # phrase appearing anywhere in the arm, including controller prose. This is a
        # PREFIX test — "- **CANDIDATE SPACE**X" still matches — so it catches a
        # misplaced field, not a malformed one.
        cs = [x for x in block if x.startswith("- **CANDIDATE SPACE**")]
        stray = [x for x in block if "**CANDIDATE SPACE**" in x and not x.startswith("- **CANDIDATE SPACE**")]
        if stray: out.add("CAND-STRAY", "arm `%s`: CANDIDATE SPACE appears outside its field bullet" % aid)
        if len(cs) != 1:
            out.add("CAND-COUNT", "arm `%s`: expected exactly one CANDIDATE SPACE field, found %d" % (aid, len(cs))); continue
        idx = block.index(cs[0]); blob = " ".join(block[idx:idx + 8])
        if re.search(r"`CS@[A-Z0-9-]+`", blob): continue
        if "**A:**" in blob and "**B:**" in blob: continue
        out.add("CAND-PAIR", "arm `%s`: no A/B pair and no valid CS@ reference" % aid)

# ---------------- internal references ----------------
def check_shadow_lists(L, bodies, out):
    """A barrier id may appear ONLY in the typed table, in an arm's field bullets, or
    in a blockquote. Anywhere else is a SHADOW LIST — a second copy someone must keep
    in agreement with the table. The design shipped one whose own sentence read
    "this paragraph names no ids of its own" directly above the enumeration."""
    i, j, _ = table(L, BAR_HDR, 3)
    allowed = set(range(i, j)) if i is not None else set()
    for (num, aid, cls, start) in bodies:
        allowed |= set(range(start, body_end(L, start)))
    hist = next((k for k, l in enumerate(L) if l.startswith("## 6.")), len(L))
    allowed |= set(range(hist, len(L)))   # the change log is history, not a live list
    BID = r"`((?:CUT|BAR)-[A-Z0-9]+-[A-Z0-9-]+)`"
    # A single id in context is a reference; THREE OR MORE in one paragraph is a LIST,
    # and a list outside the typed table is a second copy someone must keep in
    # agreement. Flagging every mention instead flagged the per-row matrices, which
    # legitimately name the barrier an arm uses.
    para, first = [], 0
    def scan(para, first):
        if not para: return
        if all(n in allowed for n in range(first, first + len(para))): return
        ids = set()
        for l in para: ids |= set(re.findall(BID, l))
        if len(ids) >= 3:
            out.add("SHADOW-LIST", "line %d: %d barrier ids enumerated outside the typed table — shadow list" % (first + 1, len(ids)))
    for n, l in enumerate(L):
        if not l.strip():
            scan(para, first); para, first = [], n + 1
        else:
            if not para: first = n
            para.append(l)
    scan(para, first)

def check_surface(L, roster, out):
    """SURFACE_HDR was defined and consumed NOWHERE — deleting a row passed. The
    surface table must carry exactly the row set the roster covers."""
    i, j, rows = table(L, SURFACE_HDR, 3, out, "surface table")
    if rows is None: out.add("SURFACE-ABSENT", "neutral surface table absent"); return
    # SET membership, so a DUPLICATED surface row passes. Membership is what P1 needs
    # (every row with arms has a line); uniqueness is not checked and is not claimed.
    have = {c[0] for _, c in rows}
    want = {p[2] for p in roster}
    for x in sorted(want - have): out.add("SURFACE-MISSING", "row %s has arms but no neutral surface line" % x)
    for x in sorted(have - want): out.add("SURFACE-EXTRA", "surface line for %s, which no arm covers" % x)

def check_internal_refs(L, out):
    here = os.path.dirname(os.path.abspath(__file__))
    for n, l in enumerate(L):
        for f in re.findall(r"`(t-wd-[A-Za-z0-9._-]+)`", l):
            if not os.path.exists(os.path.join(here, f)):
                out.add("REF-FILE", "line %d: references `%s`, which does not exist" % (n + 1, f))

def run(path, quiet=False):
    out = Out()
    try: text = open(path, encoding="utf-8").read()
    except OSError as e:
        print("FAIL  READ  cannot read %s: %s" % (path, e)); return 1, out
    L = text.split("\n")
    roster, ri, rj = parse_roster(L, out)
    bodies = parse_bodies(L, out)
    if roster:
        d = derive(roster)
        if not quiet: print("derived: " + " ".join("%s=%s" % kv for kv in sorted(d.items())))
        check_join(roster, bodies, out)
        ex = zones(L, ri, rj)
        ex |= check_count_block(L, d, out)
        check_counts(L, ex, d, out)
        check_ordinals(L, ex, out)
        terms = derive_terms(text, out)
        if not quiet: print("derived lint terms: %s" % "|".join(terms))
        check_vocab(L, ex, terms, out)
        check_candidates(L, roster, bodies, out)
    check_title(L, out)
    check_barriers(L, bodies, out)
    if roster: check_surface(L, roster, out)
    check_shadow_lists(L, bodies, out)
    check_internal_refs(L, out)
    if not quiet:
        for cid, msg in out: print("FAIL  %-18s %s" % (cid, msg))
        print("OK — all checks clean" if not out else "CHECKER REPORTED %d FAILURE(S)" % len(out))
    return (1 if out else 0), out

# ---------------- self-test: named id, local delta, opposed spellings ----------------
M = [
 ("VOCAB", 8, "vocab-armfield",    lambda s: s.replace("- **Barriers** — none.", "- **Barriers** — none. It verifies that it landed.", 1)),
 ("VOCAB", 8, "vocab-surface",     lambda s: s.replace("| D25 | the watchdog daemon process itself", "| D25 | the watchdog daemon should process itself", 1)),
 ("ORDINAL", 8, "ord-lower",       lambda s: s.replace("- **Barriers** — none.", "- **Barriers** — as arm 5.", 1)),
 ("ORDINAL", 8, "ord-upper",       lambda s: s.replace("- **Barriers** — none.", "- **Barriers** — see Arm 5.", 1)),
 ("ORDINAL", 8, "ord-hash",        lambda s: s.replace("- **Barriers** — none.", "- **Barriers** — see arm #5.", 1)),
 ("ORDINAL", 8, "ord-noword",      lambda s: s.replace("- **Barriers** — none.", "- **Barriers** — see arm no. 5.", 1)),
 ("ORDINAL", 8, "ord-wrapped",     lambda s: s.replace("- **Barriers** — none.", "- **Barriers** — see\n  arms 22.", 1)),
 ("ORDINAL", 8, "ord-surface",     lambda s: s.replace("| SC-834a | the `_recover-pending`", "| SC-834a | see arm 3, the `_recover-pending`", 1)),
 ("TITLE", 4, "version-starred",   lambda s: s.replace("worker draft (NOTHING", "worker draft **v9** (NOTHING", 1)),
 ("TITLE", 4, "version-natural",   lambda s: s.replace("worker draft (NOTHING", "worker draft v9 (NOTHING", 1)),
 ("COUNT-BLOCK", 4, "count-block-drift", lambda s: s.replace("units=44", "units=45", 1)),
 ("COUNT-BLOCK-ABSENT", 24, "count-block-gone", lambda s: re.sub(r"```counts \(generated.*?```\n", "", s, count=1, flags=re.S)),
 ("COUNT-CLAIM", 8, "count-phrase",lambda s: s.replace("**Execution order.**", "There are 99 arm specs.\n\n**Execution order.**", 1)),
 ("COUNT-CLAIM", 8, "count-equals",lambda s: s.replace("**Execution order.**", "Execution units = 999 checks.\n\n**Execution order.**", 1)),
 ("COUNT-CLAIM", 8, "count-word-hyphen", lambda s: s.replace("**Execution order.**", "There are forty-four executed units in this design.\n\n**Execution order.**", 1)),
 ("COUNT-CLAIM", 8, "count-word-scale", lambda s: s.replace("**Execution order.**", "There are one hundred arms.\n\n**Execution order.**", 1)),
 ("COUNT-CLAIM", 8, "count-colon", lambda s: s.replace("**Execution order.**", "Barriers: eleven.\n\n**Execution order.**", 1)),
 ("COUNT-CLAIM", 8, "count-word",  lambda s: s.replace("**Execution order.**", "There are four barriers.\n\n**Execution order.**", 1)),
 ("ROSTER-DUPID", 6, "dup-id",     lambda s: s.replace("`WD-913-bash-submit-unverified` | SC-913", "`WD-913-bash-dead-pane` | SC-913", 1)),
 ("ROSTER-CLASS", 6, "bad-class",  lambda s: s.replace("| SC-913 | RED | — | — |", "| SC-913 | PURPLE | — | — |", 1)),
 ("ROSTER-LANE", 6, "bad-lane",    lambda s: s.replace("| SC-913 | RED | — | — |", "| SC-913 | RED | uv | — |", 1)),
 ("ROSTER-ABSENT", 400, "no-roster", lambda s: re.sub(r"\| # \| arm id \| row \| class \| lanes \| M12 \|.*?\n\n", "", s, count=1, flags=re.S)),
 ("JOIN-BODY-ONLY", 4, "ghost-body", lambda s: s.replace("#### 1. `WD-D25-serve-at-start`", "#### 1. `WD-D25-GHOST`", 1)),
 ("JOIN-ROSTER-ONLY", 6, "ghost-row", lambda s: s.replace("| 1 | `WD-D25-serve-at-start`", "| 1 | `WD-D25-PHANTOM`", 1)),
 ("BAR-ORPHAN", 6, "orphan-bar",   lambda s: s.replace("| `BAR-920-SEND` |", "| `BAR-999-NEVER` | unused | ae:1 |\n| `BAR-920-SEND` |", 1)),
 ("BAR-UNKNOWN", 8, "ghost-bar",   lambda s: s.replace("- **Barriers** — none.", "- **Barriers** — `CUT-999-GHOST`.", 1)),
 ("BAR-ARM", 8, "undeclared-bar",  lambda s: s.replace("- **Named manipulation** — the fake agent process is exited once.", "- **Named manipulation** — at `CUT-928A-OPEN`, the fake agent process is exited once.", 1)),
 ("SHADOW-LIST", 8, "shadow-list", lambda s: s.replace("**Execution order.**", "See `CUT-926-STOP-INTENT`, `CUT-928-LOCK` and `BAR-920-SEND`.\n\n**Execution order.**", 1)),
 # --- one local target per stable id (the coverage ruling). Ids without a cheap
 # local mutation are NOT claimed; the suite prints owned/total and names the gap.
 ("BODY-CLASS", 4, "body-class-bogus", lambda s: s.replace("#### 1. `WD-D25-serve-at-start` — RED", "#### 1. `WD-D25-serve-at-start` — PURPLE", 1)),
 ("CAND-STRAY", 6, "cand-stray", lambda s: s.replace("- **Dimension** — target lock.", "The **CANDIDATE SPACE** is discussed here.\n- **Dimension** — target lock.", 1)),
 ("TABLE-ARITY", 4, "bar-extra-cell", lambda s: s.replace("| `BAR-920-SEND` | the daemon's nudge-se", "| `BAR-920-SEND` | x | the daemon's nudge-se", 1)),
 ("SURFACE-MISSING", 4, "surface-row-gone", lambda s: re.sub(r"\n\| SC-980 \| the incumbent alert[^\n]*\|", "", s, count=1)),
 ("SURFACE-EXTRA", 4, "surface-row-extra", lambda s: s.replace("| SC-980 | the incumbent alert", "| SC-999 | a row no arm covers | F0 |\n| SC-980 | the incumbent alert", 1)),
 ("BAR-DUP", 4, "bar-dup-id", lambda s: s.replace("| `BAR-920-SEND` |", "| `BAR-929-PUB` | dup | ae:1 |\n| `BAR-920-SEND` |", 1)),
 ("BAR-TABLE", 30, "bar-table-gone", lambda s: re.sub(r"\| id \| site \| frozen anchor \|.*?\n\n", "", s, count=1, flags=re.S)),
 ("BODY-DUPID", 4, "body-dup-id", lambda s: s.replace("#### 2. `WD-D25-serve-after-flip`", "#### 2. `WD-D25-serve-at-start`", 1)),
 ("JOIN-ORDINAL", 4, "join-ordinal", lambda s: s.replace("#### 2. `WD-D25-serve-after-flip`", "#### 3. `WD-D25-serve-after-flip`", 1)),
 ("JOIN-CLASS", 4, "join-class", lambda s: s.replace("| 1 | `WD-D25-serve-at-start` | D25 | RED |", "| 1 | `WD-D25-serve-at-start` | D25 | **CAPTURE-ONLY** |", 1)),
 ("ROSTER-ARITY", 4, "roster-arity", lambda s: s.replace("| 1 | `WD-D25-serve-at-start` | D25 | RED | — | — |", "| 1 | `WD-D25-serve-at-start` | D25 | RED | — |", 1)),
 ("ROSTER-ORD", 4, "roster-ord", lambda s: s.replace("| 1 | `WD-D25-serve-at-start`", "| x | `WD-D25-serve-at-start`", 1)),
 ("ROSTER-SEQ", 4, "roster-seq", lambda s: s.replace("| 2 | `WD-D25-serve-after-flip`", "| 9 | `WD-D25-serve-after-flip`", 1)),
 ("ROSTER-M12", 4, "roster-m12", lambda s: s.replace("| SC-920 | RED | — | §4.1 |", "| SC-920 | RED | — | 4.1 |", 1)),
 ("TERMS", 40, "terms-gone", lambda s: s.replace("A committed linter rejects the design", "A committed linter examines the design", 1)),
 ("ROSTER-EMPTY", 4, "roster-empty", lambda s: s.replace("| # | arm id | row | class | lanes | M12 |\n|---|---|---|---|---|---|\n", "| # | arm id | row | class | lanes | M12 |\n|---|---|---|---|---|---|\n\n", 1)),
 ("SURFACE-ABSENT", 4, "surface-gone", lambda s: s.replace("| row | neutral surface line | family |", "| row | neutral surface text | family |", 1)),
 ("REF-FILE", 8, "dead-file-ref",  lambda s: s.replace("**Execution order.**", "See `t-wd-nonexistent.sh`.\n\n**Execution order.**", 1)),
 # LOCAL: drop one arm's field MARKER, not a 244-line span. A mutation that rewrites
 # a quarter of the document is not a test of one check — it is a test of whether
 # anything at all still works.
 ("CAND-COUNT", 4, "cand-removed", lambda s: s.replace("- **CANDIDATE SPACE** — **A:** the selector decides", "- **CANDIDATE-GONE** — **A:** the selector decides", 1)),
 ("CAND-PAIR", 4, "cand-no-pair", lambda s: s.replace("- **CANDIDATE SPACE** — **A:** the selector decides which implementation serves", "- **CANDIDATE SPACE** — it decides which implementation serves", 1)),
]

def self_test(path):
    orig = open(path, encoding="utf-8").read()
    rc, _ = run(path, quiet=True)
    if rc != 0:
        print("SELF-TEST ABORT: neutral document is not clean"); run(path); return 1
    print("neutral: rc=0 clean")
    tmp = path + ".selftest.tmp"; bad = 0
    for want, maxdelta, name, fn in M:
        mut = fn(orig)
        if mut == orig:
            print("%-18s SEED-DID-NOT-LAND — invalid test, NOT a pass" % name); bad += 1; continue
        delta = sum(1 for l in difflib.unified_diff(orig.split("\n"), mut.split("\n"), n=0) if l[:1] in "+-" and l[:3] not in ("+++", "---"))
        open(tmp, "w", encoding="utf-8").write(mut)
        rc, out = run(tmp, quiet=True); os.unlink(tmp)
        ids = {c for c, _ in out}
        ok = rc != 0 and want in ids
        local = delta <= maxdelta
        if not ok or not local: bad += 1
        print("%-18s delta=%-4d %-9s rc=%d ids=%s %s" % (
            name, delta, "local" if local else "TOO-BROAD", rc,
            ",".join(sorted(ids)) or "-", "" if ok and local else "<-- FAIL"))
    # COVERAGE, stated as the tool proves it. "Complete path proof" was a claim the
    # suite did not support: incidental firing is not a red proof of a predicate.
    ids = set(re.findall(r'out\.add\("([A-Z0-9-]+)"', open(__file__, encoding="utf-8").read()))
    owned = {w for w, _, _, _ in M}
    unowned = sorted(ids - owned)
    print("COVERAGE: %d/%d stable ids have a local target" % (len(ids & owned), len(ids)))
    if unowned: print("UNOWNED (not claimed): %s" % ", ".join(unowned))
    print("SELF-TEST: %s" % ("ALL TARGETED PATHS RED-PROVEN BY NAMED ID" if bad == 0 else "%d FAILURE(S)" % bad))
    return 1 if bad else 0

if __name__ == "__main__":
    here = os.path.dirname(os.path.abspath(__file__))
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    doc = args[0] if args else os.path.join(here, "t-wd-design.md")
    if "--emit-counts" in sys.argv:
        L = open(doc, encoding="utf-8").read().split("\n")
        roster, _, _ = parse_roster(L, Out())
        print(COUNT_BLOCK_OPEN); [print(x) for x in render_counts(derive(roster))]; print("```")
        sys.exit(0)
    if "--self-test" in sys.argv: sys.exit(self_test(doc))
    sys.exit(run(doc)[0])
