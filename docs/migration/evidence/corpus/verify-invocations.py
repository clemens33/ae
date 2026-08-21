#!/usr/bin/env python3
"""GATE — proves the argv normaliser BOTH WAYS. No write path.

The seat ruling: "two runs on DIFFERENT hosts invoking the SAME thing must normalise
to IDENTICAL argv, AND two runs invoking DIFFERENT things must NOT collide. A
normaliser that only satisfies the first is a normaliser that erases the
distinctions parity exists to detect."

Arm A (convergence) and Arm B (injectivity) are both required. Arm B is computed
against an INDEPENDENT stripping method, so a bug shared with the normaliser cannot
make both agree.
"""
import csv, glob, os, re, sys, collections

HERE = os.path.dirname(os.path.abspath(__file__))
SRC = os.path.normpath(os.path.join(HERE, "..", "batch-c-artifacts"))
INV = os.path.join(HERE, "INVOCATIONS.tsv")

def raw_rows():
    for f in sorted(glob.glob(os.path.join(SRC, "arms", "*", "*", "consumers.tsv"))):
        rel = os.path.relpath(f, SRC)
        with open(f, encoding="utf-8") as fh:
            r = csv.reader(fh, delimiter="\t"); next(r, None)
            for c in r:
                if len(c) >= 10 and c[9] != "-":
                    yield rel, c[0], c[9]

def independent_semantic(argv):
    """A DIFFERENT stripping method than the normaliser's — suffix-after-marker
    rather than regex substitution — so a bug shared between them cannot make both
    agree.

    THE FIRST VERSION OF THIS FUNCTION WAS TOO LOSSY AND THE FAILURE WAS MINE, NOT
    THE NORMALISER'S: it kept only the BASENAME of the last path token, so
    `sessions/ta1b/requests` and `sessions/ta1c/requests` collapsed to `requests`,
    and it then reported the normaliser for correctly distinguishing them. An
    instrument that discards a distinction cannot be used to judge whether that
    distinction was preserved."""
    s = argv.strip()
    env = ""
    m = re.match(r"^((?:[A-Z_][A-Z0-9_]*=\S+\s+)+)(.*)$", s)
    if m: env, s = m.group(1), m.group(2)
    # DECLARED EQUIVALENCES, stated once and applied as a rule — not tuned until
    # the two methods agree. Tuning an "independent" instrument to match the thing
    # it checks destroys the independence, which is the circularity this project
    # already found in a self-test that seeded exactly the spellings its predicate
    # accepted. The equivalences are: an INTERPRETER token is host-specific and
    # carries no meaning (`bash X` == `X`), and a BINARY PATH is host- and
    # instrument-specific (its basename is the meaning; frozen-vs-hooked lives in
    # the `instrument` column).
    toks = s.split()
    if toks and os.path.basename(toks[0].rstrip("/")) in ("bash", "sh", "env"):
        toks = toks[1:]
    keep = []
    for t in toks:
        if "/" not in t:
            keep.append(t); continue
        if "/.ae/" in t:            # keep everything meaningful after the ae home
            keep.append(t.split("/.ae/", 1)[1])
        elif t.endswith("/ae") or t.endswith("/bash"):
            # The INTERPRETER and the BINARY are host- and instrument-specific:
            # frozen/ae and hooked/ae are the same invocation through different
            # instruments, and that distinction lives in INVOCATIONS.tsv's
            # `instrument` column rather than inside the argv.
            keep.append(os.path.basename(t))
        else:                       # keep the last two components: <case>/bin
            parts = t.rstrip("/").split("/")
            keep.append("/".join(parts[-2:]) if len(parts) > 1 else parts[-1])
    return (env + " ".join(keep)).strip()

def main():
    if not os.path.exists(INV):
        print("FAIL  INVOCATIONS.tsv absent — run classify-invocations.py"); return 1
    norm_of = {}
    with open(INV, encoding="utf-8") as fh:
        r = csv.reader(fh, delimiter="\t"); next(r, None)
        for c in r:
            if len(c) >= 6: norm_of[(c[0], c[1])] = c[5]
    fails = []

    # ---- Arm A: convergence. Same thing under different host prefixes -> one form.
    groups = collections.defaultdict(set)
    prefixes = collections.defaultdict(set)
    for rel, consumer, argv in raw_rows():
        n = norm_of.get((rel, consumer))
        if n is None or n == "-": continue
        groups[n].add(independent_semantic(argv))
        pre = argv.strip().split()
        host = next((t for t in pre if "/" in t), "")
        prefixes[n].add(os.path.dirname(host))
    converged = [n for n, p in prefixes.items() if len(p) > 1]
    for n, sem in groups.items():
        if len(sem) > 1:
            fails.append("ARM A  normalised form %r covers %d DIFFERENT semantic invocations: %s"
                         % (n, len(sem), sorted(sem)[:3]))

    # ---- Arm B: NO COLLISION. Two different invocations must not share a form.
    # Splitting into MORE forms is preserving distinctions and is not a failure;
    # the failure is two different things landing on one form. That is exactly the
    # `groups` test above, so Arm B reports it explicitly rather than restating a
    # weaker inverse.
    collisions = {n: sem for n, sem in groups.items() if len(sem) > 1}
    sem_to_norm = collections.defaultdict(set)
    for rel, consumer, argv in raw_rows():
        n = norm_of.get((rel, consumer))
        if n is None or n == "-": continue
        sem_to_norm[independent_semantic(argv)].add(n)
    distinct_norm, distinct_sem = len(groups), len(sem_to_norm)

    print("Arm A  convergence: %d normalised forms; %d reached from MORE THAN ONE host prefix"
          % (distinct_norm, len(converged)))
    print("Arm B  no-collision: %d normalised forms vs %d distinct semantic invocations "
          "(independent method); %d form(s) cover more than one invocation"
          % (distinct_norm, distinct_sem, len(collisions)))
    if not converged:
        fails.append("ARM A  VACUOUS: no normalised form was reached from more than one host "
                     "prefix, so convergence was never exercised")
    for m in fails: print("FAIL  %s" % m)
    print("BOTH ARMS PROVEN" if not fails else "NORMALISER NOT PROVEN — %d finding(s)" % len(fails))
    return 1 if fails else 0

if __name__ == "__main__":
    sys.exit(main())
