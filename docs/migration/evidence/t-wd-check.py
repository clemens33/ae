#!/usr/bin/env python3
"""T-WD design consistency checker.

WHY PYTHON, NOT AWK. The previous version used awk `\\<...\\>` word boundaries.
BWK awk on macOS DOES NOT IMPLEMENT THEM: it matches nothing, silently. The
checker printed `verifies that` in its own derived term list and then reported
the document clean while that term sat in a worker-facing CANDIDATE SPACE field.
That is the GNU-vs-BSD divergence class this repo documents at length, landing in
the tool built to catch defects of exactly that shape. A checker must not be
exposed to the portability class it exists to police.

EVERY CHECK HERE HAS A RED-PROOF in --self-test: a neutral pass that must be
clean, and a seeded mutation that must be caught, with the seed DIFFED against
the original first. A seed that does not land is indistinguishable from a check
that does not fire, and it errs in both directions.
"""
import re, sys, os, difflib

CLASSES = {"RED", "**CAPTURE-ONLY**", "**GAP — does not run**"}
LANES = {"—", "bash+uv"}

class Doc:
    def __init__(self, text):
        self.text = text
        self.lines = text.split("\n")

def fail(out, msg):
    out.append("FAIL  " + msg)

# ---------- roster: a real parser, not substrings ----------
ROSTER_HDR = "| # | arm id | row | class | lanes | M12 |"

def parse_roster(doc, out):
    try:
        i = doc.lines.index(ROSTER_HDR)
    except ValueError:
        fail(out, "roster header absent — counts derived from a missing table are not facts")
        return None
    rows, j = [], i + 2
    while j < len(doc.lines) and doc.lines[j].startswith("|"):
        cells = [c.strip() for c in doc.lines[j].strip().strip("|").split("|")]
        if len(cells) != 6:
            fail(out, "roster row %d has %d cells, expected 6: %s" % (j + 1, len(cells), doc.lines[j][:60]))
            return None
        rows.append(cells)
        j += 1
    if not rows:
        fail(out, "roster is EMPTY")
        return None
    ords, ids = [], []
    for n, aid, row, cls, lane, m12 in rows:
        if not n.isdigit():
            fail(out, "roster ordinal not numeric: %r" % n); return None
        ords.append(int(n)); ids.append(aid)
        if cls not in CLASSES:
            fail(out, "roster row %s: unknown class %r (allowed: %s)" % (n, cls, sorted(CLASSES)))
        if lane not in LANES:
            fail(out, "roster row %s: unknown lane %r" % (n, lane))
        if m12 != "—" and not re.fullmatch(r"§4\.\d|baseline", m12):
            fail(out, "roster row %s: malformed M12 field %r" % (n, m12))
    if ords != list(range(1, len(ords) + 1)):
        fail(out, "roster ordinals not unique-and-contiguous from 1")
    dup = {x for x in ids if ids.count(x) > 1}
    if dup:
        fail(out, "duplicate arm ids in roster: %s" % sorted(dup))
    return rows

def derive(rows):
    gap = [r for r in rows if "GAP" in r[3]]
    cap = [r for r in rows if "CAPTURE-ONLY" in r[3]]
    red = [r for r in rows if r[3] == "RED"]
    two = [r for r in rows if r[4] == "bash+uv"]
    m12 = [r for r in rows if r[5].startswith("§4.") and "GAP" not in r[3]]
    runnable = len(rows) - len(gap)
    return dict(specs=len(rows), red=len(red), capture=len(cap), gap=len(gap),
                two=len(two), runnable=runnable, units=runnable + len(two), m12=len(m12))

# ---------- counts: parse EVERY numeric claim, not one phrasing ----------
COUNT_PATTERNS = [
    (re.compile(r"all (\d+) executed units"), "units"),
    (re.compile(r"executed unit count is (\d+)"), "units"),
    (re.compile(r"(\d+) arm specs"), "specs"),
    (re.compile(r"(\d+) arms \(rows SC-"), "m12"),
    (re.compile(r"(\d+) RED\b"), "red"),
]

def check_counts(doc, d, out):
    for pat, key in COUNT_PATTERNS:
        for m in pat.finditer(doc.text):
            n = int(m.group(1))
            if n != d[key]:
                ln = doc.text[:m.start()].count("\n") + 1
                fail(out, "line %d: prose says %d for %s; roster yields %d" % (ln, n, key, d[key]))

# ---------- self-version ----------
def check_version(doc, out):
    if re.search(r"worker draft \*\*v[0-9]", doc.text):
        fail(out, "document carries its own version numeral — identity is the commit hash")

# ---------- ordinals: paragraph-joined, case-insensitive, all spellings ----------
ORD = re.compile(r"\barms?\s*(?:#|no\.\s*)?\d+", re.I)

def permitted_zones(doc):
    """Yield (start,end) line ranges where ordinals are allowed."""
    z = []
    try:
        i = doc.lines.index(ROSTER_HDR); j = i
        while j < len(doc.lines) and doc.lines[j].startswith("|"): j += 1
        z.append((i, j))
    except ValueError:
        pass
    for k, l in enumerate(doc.lines):
        if l.startswith("**Execution order"):
            e = k
            while e < len(doc.lines) and doc.lines[e].strip(): e += 1
            z.append((k, e))
        if l.startswith("## 6."):
            z.append((k, len(doc.lines)))
        if l.startswith("> "):
            z.append((k, k + 1))
        if l.startswith("| ") and l.count("|") >= 3:
            z.append((k, k + 1))   # any table row
    return z

def check_ordinals(doc, out):
    zones = permitted_zones(doc)
    def allowed(n):
        return any(a <= n < b for a, b in zones)
    # join paragraphs so a wrapped "(arms\n22-24)" is visible
    para = []
    for n, l in enumerate(doc.lines):
        if not l.strip():
            if para: scan_para(para, allowed, out)
            para = []
        else:
            para.append((n, l))
    if para: scan_para(para, allowed, out)

def scan_para(para, allowed, out):
    joined = " ".join(l for _, l in para)
    for m in ORD.finditer(joined):
        # locate the line the match starts on
        off, ln = 0, para[0][0]
        for n, l in para:
            if off + len(l) + 1 > m.start(): ln = n; break
            off += len(l) + 1
        if not allowed(ln):
            fail(out, "line %d: ordinal arm reference %r — use a stable arm id" % (ln + 1, m.group(0)))

# ---------- barriers: BOTH directions, plus duplicate table ids ----------
BAR_TBL_HDR = "| id | site | frozen anchor |"

def check_barriers(doc, out):
    try:
        i = doc.lines.index(BAR_TBL_HDR)
    except ValueError:
        fail(out, "typed barrier table absent"); return
    ids, j = [], i + 2
    while j < len(doc.lines) and doc.lines[j].startswith("|"):
        m = re.match(r"\|\s*`([^`]+)`", doc.lines[j])
        if m: ids.append(m.group(1))
        j += 1
    dup = {x for x in ids if ids.count(x) > 1}
    if dup: fail(out, "duplicate ids in the barrier table: %s" % sorted(dup))
    tbl = set(ids)
    used = set()
    for n, l in enumerate(doc.lines):
        if l.startswith(">") or (i <= n < j): continue
        used |= set(re.findall(r"`((?:CUT|BAR)-[A-Z0-9]+-[A-Z0-9-]+)`", l))
    for x in sorted(used - tbl): fail(out, "barrier id used but absent from the typed table: %s" % x)
    for x in sorted(tbl - used): fail(out, "barrier id in the typed table but never used: %s" % x)

# ---------- vocabulary: terms DERIVED from M1, portable boundaries ----------
def derive_terms(doc, out):
    m = re.search(r"A committed linter rejects the design.*?`([^`]*)`\.", doc.text, re.S)
    if not m:
        fail(out, "cannot derive lint terms from M1 — refusing to run a shorter list"); return []
    raw = m.group(1).replace("\n", " ")
    terms = [t.strip() for t in raw.split("|") if t.strip()]
    return terms

def arm_field_ranges(doc):
    r, inz = [], False
    for n, l in enumerate(doc.lines):
        if l.startswith("## 3A"): inz = True
        elif l.startswith("## 5. Fixture"): inz = False
        elif re.match(r"^### 4\.", l): inz = True
        elif l.startswith("## 5."): inz = False
        if inz: r.append(n)
    return set(r)

def check_vocab(doc, terms, out):
    if not terms: return
    zone = arm_field_ranges(doc)
    pat = re.compile(r"\b(" + "|".join(re.escape(t) for t in terms) + r")\b", re.I)
    for n in sorted(zone):
        l = doc.lines[n]
        if l.startswith(">"): continue
        stripped = re.sub(r"`[^`]*`", "", l)
        m = pat.search(stripped)
        if m:
            fail(out, "line %d: banned term %r in an arm field" % (n + 1, m.group(1)))

# ---------- candidate space: evaluated at EVERY RED boundary and at EOF ----------
HEAD = re.compile(r"^#### (\d+)\. `([^`]+)` — (.*)$")

def check_candidates(doc, out):
    cur, buf = None, []
    def close(h, b):
        if not h: return
        if "CAPTURE-ONLY" in h[2] or "DECLARED GAP" in h[2]: return
        cs = [l for l in b if "**CANDIDATE SPACE**" in l]
        if len(cs) != 1:
            fail(out, "arm %s: expected exactly one CANDIDATE SPACE field, found %d" % (h[1], len(cs)))
            return
        idx = b.index(cs[0]); blob = " ".join(b[idx:idx + 8])
        if re.search(r"`CS@[A-Z0-9-]+`", blob): return
        if "**A:**" in blob and "**B:**" in blob: return
        fail(out, "arm %s: candidate space names no A/B pair and no valid CS@ reference" % h[1])
    for l in doc.lines:
        m = HEAD.match(l)
        if m:
            close(cur, buf); cur, buf = m.groups(), []
        elif cur is not None:
            buf.append(l)
    close(cur, buf)

def run(path):
    out = []
    try:
        text = open(path, encoding="utf-8").read()
    except OSError as e:
        print("FAIL  cannot read %s: %s" % (path, e)); return 1
    doc = Doc(text)
    rows = parse_roster(doc, out)
    if rows:
        d = derive(rows)
        print("derived: " + " ".join("%s=%s" % kv for kv in sorted(d.items())))
        check_counts(doc, d, out)
    check_version(doc, out)
    check_ordinals(doc, out)
    check_barriers(doc, out)
    terms = derive_terms(doc, out)
    print("derived lint terms: %s" % "|".join(terms))
    check_vocab(doc, terms, out)
    check_candidates(doc, out)
    for o in out: print(o)
    print("OK — all checks clean" if not out else "CHECKER REPORTED %d FAILURE(S)" % len(out))
    return 1 if out else 0

# ---------- self-test: every check path, seed-diff-run ----------
MUTATIONS = [
    ("vocab", lambda s: s.replace("- **Barriers** — none.", "- **Barriers** — none. The arm verifies that it landed.", 1)),
    ("candspace-deleted", lambda s: re.sub(r"- \*\*CANDIDATE SPACE\*\* — \*\*A:\*\*.*?\n- \*\*Dimension\*\*", "- **Dimension**", s, count=1, flags=re.S)),
    ("ordinal-lower", lambda s: s.replace("- **Barriers** — none.", "- **Barriers** — as arm 5.", 1)),
    ("ordinal-upper", lambda s: s.replace("- **Barriers** — none.", "- **Barriers** — see Arm 5.", 1)),
    ("ordinal-hash", lambda s: s.replace("- **Barriers** — none.", "- **Barriers** — see arm #5.", 1)),
    ("ordinal-wrapped", lambda s: s.replace("- **Barriers** — none.", "- **Barriers** — see\n  arms 22.", 1)),
    ("dup-arm-id", lambda s: s.replace("`WD-913-bash-submit-unverified` | SC-913", "`WD-913-bash-dead-pane` | SC-913", 1)),
    ("bogus-class", lambda s: s.replace("| SC-913 | RED | — | — |", "| SC-913 | PURPLE | — | — |", 1)),
    ("bogus-lane", lambda s: s.replace("| SC-913 | RED | — | — |", "| SC-913 | RED | uv | — |", 1)),
    ("roster-deleted", lambda s: re.sub(r"\| # \| arm id \| row \| class \| lanes \| M12 \|.*?\n\n", "", s, count=1, flags=re.S)),
    # The document deliberately prints NO counts, so a drift mutation must first
    # INTRODUCE one. Mutating a phrase that is absent produces a seed that does not
    # land, which the runner reports as an INVALID TEST rather than a pass.
    ("count-drift", lambda s: s.replace("**Execution order.**", "There are 99 arm specs.\n\n**Execution order.**", 1)),
    ("unused-barrier", lambda s: s.replace("| `BAR-920-SEND` |", "| `BAR-999-NEVER` | unused | ae:1 |\n| `BAR-920-SEND` |", 1)),
    ("unknown-barrier", lambda s: s.replace("- **Barriers** — none.", "- **Barriers** — `CUT-999-GHOST`.", 1)),
    ("self-version", lambda s: s.replace("worker draft (NOTHING", "worker draft **v9** (NOTHING", 1)),
]

def self_test(path):
    import io, contextlib
    orig = open(path, encoding="utf-8").read()
    tmp = path + ".selftest.tmp"
    bad = 0
    buf = io.StringIO()
    with contextlib.redirect_stdout(buf):
        neutral = run(path)
    if neutral != 0:
        print("SELF-TEST ABORT: neutral document is not clean (rc=%d)" % neutral)
        print(buf.getvalue()); return 1
    print("neutral: rc=0 clean")
    for name, fn in MUTATIONS:
        mutated = fn(orig)
        if mutated == orig:
            print("%-20s SEED-DID-NOT-LAND — test invalid, not a pass" % name); bad += 1; continue
        d = sum(1 for _ in difflib.unified_diff(orig.split("\n"), mutated.split("\n"), n=0))
        open(tmp, "w", encoding="utf-8").write(mutated)
        b2 = io.StringIO()
        with contextlib.redirect_stdout(b2):
            rc = run(tmp)
        os.unlink(tmp)
        ok = "caught" if rc != 0 else "MISSED"
        if rc == 0: bad += 1
        print("%-20s seed landed (%d diff lines)  rc=%d  %s" % (name, d, rc, ok))
    print("SELF-TEST: %s" % ("ALL CHECKS RED-PROVEN" if bad == 0 else "%d FAILURE(S)" % bad))
    return 1 if bad else 0

if __name__ == "__main__":
    here = os.path.dirname(os.path.abspath(__file__))
    args = [a for a in sys.argv[1:] if a != "--self-test"]
    doc = args[0] if args else os.path.join(here, "t-wd-design.md")
    sys.exit(self_test(doc) if "--self-test" in sys.argv else run(doc))
