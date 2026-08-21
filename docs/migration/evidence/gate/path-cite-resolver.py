#!/usr/bin/env python3
"""MANIFEST path-citation resolver.

Every backticked token in MANIFEST.md that looks like a path is resolved against an
ordered list of BASES, plus a group/member -> fixture-bytes MAPPING. A token containing a
wildcard must expand NONEMPTY and every expansion must exist. Emits a machine PATH-CITE
table (one row per citation: token, class, base that resolved it, expansions) and exits
non-zero on any dangling citation.

Classes, each independently red-proofable:
  tree      — resolves under the artifact tree root
  arms      — resolves under arms/
  templates — resolves under templates/
  twd       — resolves under twd-precursor/
  repo      — resolves under the repository root (docs/... and other repo-root paths)
  mapping   — a `<GROUP>/<member>` citation resolved through the group/member ->
              templates/<GROUP>/fixture-bytes/<member> mapping
  wildcard  — contains * or ? or <...>; expanded and every expansion checked
"""
import glob, os, re, sys

TREE = sys.argv[1] if len(sys.argv) > 1 else os.environ.get(
    "BATCH_C_ARTIFACTS",
    "/Users/ckriech/projects/clemens33/ae-rust/docs/migration/evidence/batch-c-artifacts")
REPO = sys.argv[2] if len(sys.argv) > 2 else "/Users/ckriech/projects/clemens33/ae-rust"
OUT = os.path.join(TREE, "PATH-CITES.tsv")

BASES = [("tree", TREE), ("arms", os.path.join(TREE, "arms")),
         ("templates", os.path.join(TREE, "templates")),
         ("twd", os.path.join(TREE, "twd-precursor")),
         ("repo", REPO)]

# CONTEXT bases: a citation may legitimately be written relative to the directory it is
# describing — a case dir, a template group dir, a member's fixture-bytes dir, a T-WD arm
# dir. Each is a glob over the real tree, so the resolver checks that the relative
# citation exists in at least one REAL context of that kind rather than assuming it.
CONTEXT_BASES = [
    ("case-rel",     os.path.join(TREE, "arms", "*", "*")),
    ("armroot-rel",  os.path.join(TREE, "arms", "*")),
    ("group-rel",    os.path.join(TREE, "templates", "*")),
    ("member-rel",   os.path.join(TREE, "templates", "*", "fixture-bytes", "*")),
    ("session-rel",  os.path.join(TREE, "templates", "*", "fixture-bytes", "*", "sessions", "*")),
    ("twdarm-rel",   os.path.join(TREE, "twd-precursor", "*")),
    ("twdsub-rel",   os.path.join(TREE, "twd-precursor", "*", "*")),
    ("groupmeta-rel", os.path.join(TREE, "templates", "*", "_meta")),
    # Batch H layout: arm dirs sit directly under the tree, cases under those, and the
    # batch documents sit one level up beside the tree.
    ("harm-rel",     os.path.join(TREE, "A-*")),
    ("hcase-rel",    os.path.join(TREE, "A-*", "*")),
    ("hparent-rel",  os.path.dirname(os.path.abspath(TREE))),
]

def resolve_context(tok_pattern):
    """Return (context-name, first hit, total hits) for a token resolved relative to any
    real context directory, or None."""
    for name, ctx in CONTEXT_BASES:
        hits = sorted(glob.glob(os.path.join(ctx, tok_pattern)))
        hits = [h for h in hits if os.path.exists(h)]
        if hits:
            return name, hits[0], len(hits)
    return None

man = os.path.join(TREE, "MANIFEST.md")
text = open(man, encoding="utf-8").read()

TOKEN = re.compile(r"`([^`\n]+)`")
# A token is treated as a path citation when it contains a '/' and looks path-shaped.
PATHISH = re.compile(r"^[A-Za-z0-9_.*?<>/\[\]-]+$")

# Slash-less tokens with a file-ish extension are citations too — FINGERPRINTS.tsv,
# SHA256SUMS.txt, MANIFEST.md, ledger.tsv, case.txt and friends are exactly the class a
# slash-only rule silently skips.
FILEISH = (".txt", ".tsv", ".md", ".jsonl", ".json", ".patch", ".sh", ".py", ".log", ".c")

def is_pathish(t):
    if not PATHISH.match(t):
        return False
    if t.startswith("-"):
        return False
    if "/" in t:
        return True
    return t.endswith(FILEISH)

def group_member_map(tok):
    """`G5/m1-control` style: a group/member citation, resolved through the template
    fixture-bytes mapping."""
    m = re.fullmatch(r"([A-Z][A-Za-z0-9]*)/([A-Za-z0-9_.-]+)", tok)
    if not m:
        return None
    g, mem = m.group(1), m.group(2)
    cand = os.path.join(TREE, "templates", g, "fixture-bytes", mem)
    meta = os.path.join(TREE, "templates", g, "_meta", mem + ".txt")
    if os.path.exists(cand) or os.path.exists(meta):
        return cand if os.path.exists(cand) else meta
    return None

rows, bad = [], []
seen = set()
for tok in TOKEN.findall(text):
    tok = tok.strip()
    if tok in seen or not is_pathish(tok):
        continue
    seen.add(tok)
    wild = any(c in tok for c in "*?<")
    if wild:
        # <placeholder> segments are treated as a single-level wildcard
        pat = re.sub(r"<[^>/]+>", "*", tok)
        hits = []
        for name, base in BASES:
            h = sorted(glob.glob(os.path.join(base, pat)))
            if h:
                hits = h
                rows.append((tok, "wildcard", name, str(len(h)), h[0][len(base)+1:]))
                break
        if not hits:
            ctx = resolve_context(pat)
            if ctx:
                rows.append((tok, "wildcard", ctx[0], str(ctx[2]), ctx[1][len(TREE)+1:]))
                continue
            bad.append((tok, "wildcard", "expanded to nothing under every base or context"))
        else:
            missing = [h for h in hits if not os.path.exists(h)]
            if missing:
                bad.append((tok, "wildcard", f"{len(missing)} expansion(s) do not exist"))
        continue
    hit = None
    for name, base in BASES:
        cand = os.path.join(base, tok)
        if os.path.exists(cand):
            hit = (name, cand)
            break
    if hit is None:
        mapped = group_member_map(tok)
        if mapped:
            rows.append((tok, "mapping", "templates", "1", mapped[len(TREE)+1:]))
            continue
        ctx = resolve_context(tok)
        if ctx:
            rows.append((tok, "context", ctx[0], str(ctx[2]), ctx[1][len(TREE)+1:]))
            continue
        bad.append((tok, "path", "does not resolve under any base, context, or the group/member mapping"))
        continue
    rows.append((tok, "path", hit[0], "1", hit[1][len(TREE)+1:] if hit[1].startswith(TREE) else hit[1]))

# The recorded resolution is normalised to TREE-RELATIVE. It used to be whatever
# string the glob produced, which depends on whether the caller passed the tree as an
# absolute or a relative path — so the same tree, gated two ways, produced two
# different committed files and a "dirty tree" that was an invocation artifact.
def _rel(x):
    ax, at = os.path.abspath(x), os.path.abspath(TREE)
    return os.path.relpath(ax, at) + ("/" if x.endswith("/") else "") if ax.startswith(at) else x
rows = [tuple(list(r[:4]) + [_rel(r[4])]) if len(r) > 4 else r for r in rows]
buf = ["citation\tclass\tresolved_base\texpansions\tfirst_resolution\n"]
for r in sorted(rows):
    buf.append("\t".join(r) + "\n")
for b in sorted(bad):
    buf.append("\t".join((b[0], b[1], "UNRESOLVED", "0", b[2])) + "\n")
new = "".join(buf)
# Visible, not fatal: a committed-but-stale index used to be overwritten in silence
# and the gate reported over the freshly generated file. A mid-capture tree
# legitimately has a stale index, so mismatch must not change exit status.
if os.path.exists(OUT):
    old = open(OUT).read()
    if old != new:
        # Drift SIGNAL only: this is a positional mismatch count, not an edit
        # distance. Insert one line at the top and nearly every later line
        # compares unequal, so the number is magnitude-of-disagreement, not
        # how many edits it would take to repair.
        ol, nl = old.splitlines(), new.splitlines()
        n = sum(1 for a, b in zip(ol, nl) if a != b) + abs(len(ol) - len(nl))
        print(f"DRIFT file={OUT} positional_mismatches={n}")
with open(OUT, "w") as fh:
    fh.write(new)

print(f"tree={TREE}")
print(f"citations_checked={len(rows)+len(bad)} resolved={len(rows)} unresolved={len(bad)}")
by = {}
for r in rows:
    by[(r[1], r[2])] = by.get((r[1], r[2]), 0) + 1
for k in sorted(by):
    print(f"  class={k[0]:9s} base={k[1]:10s} count={by[k]}")
for b in bad:
    print(f"  UNRESOLVED [{b[1]}] {b[0]} — {b[2]}")
print(f"table={OUT}")
sys.exit(1 if bad else 0)
