#!/usr/bin/env python3
"""Gate the brief-facing input list against the seat-facing census.

Four checks, and one of them is the only STRUCTURAL guarantee here:

1. DIFF-CLEAN — regenerating the list from the census reproduces the committed file byte
   for byte. A hand-edit is a failure, not a merge.
2. SET EQUALITY — the (surface, input) pairs in the list are exactly the census's expanded
   pairs. A dropped class and an added class are both failures; a brief that silently
   loses a row would let an arm miss the contract and still look complete.
3. COLUMNAR DROP, PROVEN BY INJECTION — a synthetic census row carrying a NOVEL outcome
   label (a word this file never enumerates) is fed to the generator, and the label must
   not appear in its output. This is what a vocabulary filter cannot give you: the check
   passes for words nobody has thought of, because the drop is by column.
4. RESIDUAL LEXICAL BELT — the committed list is grepped for the outcome vocabulary we
   happen to know plus source citations. This is a BELT, not the guarantee: it can only
   catch words on a list, and a list is beaten by the first word that is not on it. It
   exists to catch a census whose INPUT column has itself acquired outcome language.

`--redproof` runs the failure injections and reports whether each one is caught.

usage: check-input-list.py <census.md> <list.md> [--redproof]
"""
import os, re, subprocess, sys, tempfile

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
from importlib.machinery import SourceFileLoader
_gen_mod = SourceFileLoader("_gen", os.path.join(HERE, "derive-input-list.py"))
# The splitter must be THE generator's, not a copy: two copies of a rule drift, and this
# one decides whether a spelling group is one entry or three.
split_spellings = None
GEN = os.path.join(HERE, "derive-input-list.py")
if len(sys.argv) < 3:
    sys.stderr.write(__doc__.strip().splitlines()[-1] + "\n")
    sys.exit(2)
census, listing = sys.argv[1], sys.argv[2]
redproof = "--redproof" in sys.argv

def _load_splitter():
    import types, re as _re
    src = open(os.path.join(HERE, "derive-input-list.py")).read()
    ns = {"re": _re}
    body = src[src.index("def split_spellings"):src.index("body, keep, seen, dupes")]
    exec(compile(body, "derive-input-list.py", "exec"), ns)
    return ns["split_spellings"]

split_spellings = _load_splitter()

def generate(census_path, out_path):
    # rc 2 means the generator itself found duplicates; that is reported by the caller.
    subprocess.run([sys.executable, GEN, census_path, out_path],
                   stdout=subprocess.DEVNULL)

def pairs(md):
    """Returns (set, duplicates). A duplicate record is a defect: it hides an ambiguity in
    the census and it makes set equality pass while the two files differ in content."""
    out, cur, dup = set(), None, []
    for ln in open(md, encoding="utf-8"):
        ln = ln.rstrip("\n")
        if ln.startswith("## Surface:"): cur = ln[len("## Surface:"):].strip()
        elif ln.startswith("- ") and cur:
            k = (cur, ln[2:].strip())
            if k in out: dup.append(k)
            out.add(k)
    return out, dup

def census_pairs(md):
    """(in-scope pairs, duplicates, scope errors) — mirrors the generator, including that
    duplicate and conflicting-ownership detection happens BEFORE the scope filter. A
    filtered-first reader cannot see two identical OOB rows or an IN/OOB conflict."""
    out, cur, dup, errs = set(), None, [], []
    seen_scope, ncols, idx = {}, None, None
    for ln in open(md, encoding="utf-8"):
        ln = ln.rstrip("\n")
        if ln.startswith("## ") or ln.startswith("### "):
            t = re.sub(r"\s+—.*$", "", ln); t = re.sub(r"^#+\s*", "", t)
            t = re.sub(r"\s*\(ae:[^)]*\)", "", t).strip()
            tl = t.lower()
            cur = None if ("(seat-only)" in tl or any(k in tl for k in ("argument census", "row ->"))) else t
            idx = None
            continue
        if not (ln.startswith("|") and cur):
            continue
        c = [x.strip() for x in ln.strip("|").split("|")]
        if not c or not c[0] or set(c[0]) <= set("-: "):
            continue
        if c[0].lower().startswith("input"):
            if c[-1] != "Scope":
                errs.append(f"{cur}: header has no trailing Scope column"); idx = None
            else:
                idx, ncols = len(c) - 1, len(c)
            continue
        if idx is None:
            errs.append(f"{cur}: data row before a valid header"); continue
        if len(c) != ncols:
            errs.append(f"{cur}: row has {len(c)} columns, header has {ncols}"); continue
        scope = c[idx]
        if not re.fullmatch(r"IN|OOB:[A-Za-z0-9_.\-]+", scope):
            errs.append(f"{cur}: bad scope value {scope!r}"); continue
        for part in split_spellings(c[0]):
            k = (cur, part)
            if k in seen_scope and seen_scope[k] != scope:
                errs.append(f"{cur}: conflicting scope for {part!r}")
            elif k in seen_scope:
                dup.append(k)
            seen_scope[k] = scope
            if scope == "IN":
                out.add(k)
    return out, dup, errs

fails = []
tmp = tempfile.mkdtemp()

gen_path = os.path.join(tmp, "gen.md")
generate(census, gen_path)
if open(gen_path, encoding="utf-8").read() != open(listing, encoding="utf-8").read():
    fails.append("DIFF-CLEAN: the committed list is not what the generator produces")

lp, lp_dupes = pairs(listing)
cp, cp_dupes, cp_errs = census_pairs(census)
for d in lp_dupes: fails.append(f"DUPLICATE_LIST: {d}")
for d in cp_dupes: fails.append(f"DUPLICATE_CENSUS: {d}")
for e in cp_errs: fails.append(f"SCOPE_ERROR: {e}")
for extra in sorted(lp - cp): fails.append(f"EXTRA in list, absent from census: {extra}")
for miss in sorted(cp - lp): fails.append(f"MISSING from list, present in census: {miss}")

NOVEL = "zqx-outcome-token-not-enumerated-anywhere"
inj = os.path.join(tmp, "inj.md")
src = open(census, encoding="utf-8").read()
anchor = "| no args | the print-current branch, ae:12836-12845 | ACCEPTED |"
if anchor in src:
    src2 = src.replace(anchor, anchor + f"\n| a synthetic probe input | somewhere, ae:1 | {NOVEL} |")
    open(inj, "w", encoding="utf-8").write(src2)
    out2 = os.path.join(tmp, "inj-out.md"); generate(inj, out2)
    body = open(out2, encoding="utf-8").read()
    if NOVEL in body:
        fails.append("COLUMNAR DROP: a novel outcome label reached the generated list")
    if "a synthetic probe input" not in body:
        fails.append("COLUMNAR DROP: the injected row's INPUT did not reach the list — "
                     "the check would pass vacuously")
else:
    fails.append("COLUMNAR DROP: injection anchor not found; the check could not run")

BELT = re.compile(r"\b(ACCEPTED|REJECTED|IGNORED|HANGS|OUT-OF-BATCH|resolvable|unresolvable"
                  r"|valid|invalid|succeeds|fails)\b|ae:[0-9]")
# Scoped to derived CONTENT lines (entries and surface headings). The header is the
# generator's own prose: including it would make the belt fire on the sentence that
# describes the belt, and a check that fires on its own documentation is one people learn
# to wave through.
for i, ln in enumerate(open(listing, encoding="utf-8"), 1):
    if not (ln.startswith("- ") or ln.startswith("## Surface:")):
        continue
    if BELT.search(ln):
        fails.append(f"LEXICAL BELT: line {i}: {ln.strip()[:70]}")

if redproof:
    print("## red-proof — each injection must be CAUGHT")
    def run_variant(mutate, label):
        v = os.path.join(tmp, "v.md"); g = os.path.join(tmp, "vg.md")
        open(v, "w", encoding="utf-8").write(mutate(open(listing, encoding="utf-8").read()))
        generate(census, g)
        same = open(v, encoding="utf-8").read() == open(g, encoding="utf-8").read()
        eq = pairs(v) == census_pairs(census)
        belt = any(BELT.search(l) for l in open(v, encoding="utf-8"))
        caught = (not same) or (not eq) or belt
        print(f"  {label:34s} caught={'YES' if caught else 'NO'}")
        return caught
    arms = [
        (lambda t: t.replace("- `--help`\n", "", 1), "dropped class"),
        (lambda t: t + "- `--invented-flag`\n", "extra class"),
        (lambda t: t.replace("- `-h`", "- `-h` | ACCEPTED", 1), "outcome column smuggled in"),
        (lambda t: t.replace("- `help`", "- an unresolvable target", 1), "outcome-labelled adjective"),
        (lambda t: t.replace("- `version`", "- `version` (ae:16845)", 1), "citation leakage"),
        (lambda t: t.replace("- `mine`\n", "- `mine`\n- `mine`\n", 1), "duplicate record"),
    ]
    def run_census_variant(mutate, label):
        """Mutate the CENSUS, regenerate, and require the pipeline to notice. The previous
        suite mutated only the generated list, so every ownership and duplicate claim about
        the SOURCE was untested."""
        v = os.path.join(tmp, "c.md"); g = os.path.join(tmp, "cg.md")
        open(v, "w", encoding="utf-8").write(mutate(open(census, encoding="utf-8").read()))
        r = subprocess.run([sys.executable, GEN, v, g], capture_output=True, text=True)
        _, d, e = census_pairs(v)
        gen_pairs, _ = pairs(g) if os.path.exists(g) else (set(), [])
        caught = bool(d or e) or r.returncode != 0 or gen_pairs != cp
        print(f"  {label:34s} caught={'YES' if caught else 'NO'}")
        return caught

    IN_ROW  = "| `-h` | outer case arm, ae:16841-16843 | ACCEPTED | SC-012b | IN |"
    OOB_ROW = "| `--init` | ae:16732-16738 | ACCEPTED | OUT-OF-BATCH — SC-932 | OOB:SC-932 |"
    census_arms = [
        (lambda t: t.replace(IN_ROW, IN_ROW.replace("| IN |", "| OOB:SC-999 |"), 1), "census IN -> OOB"),
        (lambda t: t.replace(OOB_ROW, OOB_ROW.replace("| OOB:SC-932 |", "| IN |"), 1), "census OOB -> IN"),
        (lambda t: t.replace(IN_ROW, IN_ROW + "\n" + IN_ROW, 1), "census duplicate in-scope row"),
        (lambda t: t.replace(OOB_ROW, OOB_ROW + "\n" + OOB_ROW, 1), "census duplicate OOB row"),
        (lambda t: t.replace(IN_ROW, IN_ROW + "\n" + IN_ROW.replace("| IN |", "| OOB:SC-932 |"), 1), "census conflicting ownership"),
        (lambda t: t.replace(IN_ROW, IN_ROW.replace("| IN |", "| MAYBE |"), 1), "census invalid scope value"),
        (lambda t: t.replace("| Input | Reaches | Class | Row | Scope |", "| Input | Reaches | Class | Row |", 1), "census header missing Scope"),
    ]
    for mutate, label in census_arms:
        if not run_census_variant(mutate, label):
            fails.append(f"RED-PROOF BLIND: the '{label}' injection was not caught")

    for mutate, label in arms:
        if not run_variant(mutate, label):
            # A red arm that reports caught=NO and leaves rc 0 is a red-proof that cannot
            # fail — the exact shape this whole batch exists to eliminate. It is a FAILURE.
            fails.append(f"RED-PROOF BLIND: the '{label}' injection was not caught")
    print(f"  ALL CAUGHT: {'yes' if not any(f.startswith('RED-PROOF') for f in fails) else 'NO — a check is blind'}")

print(f"census_pairs={len(cp)} list_pairs={len(lp)} failures={len(fails)}")
for f in fails: print(f"  FAIL {f}")
sys.exit(1 if fails else 0)
