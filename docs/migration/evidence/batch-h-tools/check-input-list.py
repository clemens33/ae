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
GEN = os.path.join(HERE, "derive-input-list.py")
census, listing = sys.argv[1], sys.argv[2]
redproof = "--redproof" in sys.argv

def generate(census_path, out_path):
    subprocess.run([sys.executable, GEN, census_path, out_path], check=True,
                   stdout=subprocess.DEVNULL)

def pairs(md):
    out, cur = set(), None
    for ln in open(md, encoding="utf-8"):
        ln = ln.rstrip("\n")
        if ln.startswith("## Surface:"): cur = ln[len("## Surface:"):].strip()
        elif ln.startswith("- ") and cur: out.add((cur, ln[2:].strip()))
    return out

def census_pairs(md):
    out, cur = set(), None
    for ln in open(md, encoding="utf-8"):
        ln = ln.rstrip("\n")
        if ln.startswith("## ") or ln.startswith("### "):
            t = re.sub(r"\s+—.*$", "", ln); t = re.sub(r"^#+\s*", "", t)
            t = re.sub(r"\s*\(ae:[^)]*\)", "", t).strip()
            cur = None if any(k in t.lower() for k in ("argument census", "row ->")) else t
        elif ln.startswith("|") and cur:
            c = [x.strip() for x in ln.strip("|").split("|")]
            if c and c[0] and not c[0].lower().startswith("input") and not set(c[0]) <= set("-: "):
                for part in [p.strip() for p in c[0].split(" / ")]:
                    if part: out.add((cur, part))
    return out

fails = []
tmp = tempfile.mkdtemp()

gen_path = os.path.join(tmp, "gen.md")
generate(census, gen_path)
if open(gen_path, encoding="utf-8").read() != open(listing, encoding="utf-8").read():
    fails.append("DIFF-CLEAN: the committed list is not what the generator produces")

lp, cp = pairs(listing), census_pairs(census)
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
    ok = True
    ok &= run_variant(lambda t: t.replace("- `--help`\n", "", 1), "dropped class")
    ok &= run_variant(lambda t: t + "- `--invented-flag`\n", "extra class")
    ok &= run_variant(lambda t: t.replace("- `-h`", "- `-h` | ACCEPTED", 1), "outcome column smuggled in")
    ok &= run_variant(lambda t: t.replace("- `help`", "- an unresolvable target", 1), "outcome-labelled adjective")
    ok &= run_variant(lambda t: t.replace("- `version`", "- `version` (ae:16845)", 1), "citation leakage")
    print(f"  ALL CAUGHT: {'yes' if ok else 'NO — a check is blind'}")

print(f"census_pairs={len(cp)} list_pairs={len(lp)} failures={len(fails)}")
for f in fails: print(f"  FAIL {f}")
sys.exit(1 if fails else 0)
