#!/usr/bin/env python3
"""GENERATOR — partitions the corpus's consumer rows into P1 (read) and P2 (write).

THE AXIS IS READ VERSUS WRITE, NOT BINARY VERSUS HELPER (seat ruling). A generated
helper can be a P1 surface — VISION:93 names `requests` explicitly — and the binary
can carry a write. Shape is used ONLY for normalisation, never for scope.

NORMALISATION strips what is host- or run-specific (absolute scratch prefixes, the
capture host's home, per-case directories) to stable placeholders, and PRESERVES
everything semantic: subcommand, flags, flag ORDER, session names, env prefixes,
and the distinction between shapes.

Writes INVOCATIONS.tsv. Verification lives in verify-invocations.py, which has no
write path — a gate reads, a generator writes.
"""
import csv, os, re, sys, glob, collections

HERE = os.path.dirname(os.path.abspath(__file__))
SRC = os.path.normpath(os.path.join(HERE, "..", "batch-c-artifacts"))
OUT = os.path.join(HERE, "INVOCATIONS.tsv")

# Read/write is decided from the INVOCATION, against the frozen script's own
# documentation — never inferred from which shape the argv wears.
#   `ae next` usage text: "Read-only by default: prints ..."; --attach "jumps to
#   that session: switch-client inside tmux, attach-session outside".
READ_BINARY = {"list", "ls", "status", "next", "doctor"}
READ_HELPER = {"requests", "agents", "events-tail"}
WRITE_MARKERS = [("next", "--attach"), ("doctor", "--refresh")]

HOST_PREFIXES = [
    (re.compile(r"^\S*/bash\s+\S*/ae(\s|$)"), "ae "),                 # frozen ae via bash
    (re.compile(r"^\S*/ae(\s|$)"), "ae "),                            # bare path to ae
]
HOME_RE = re.compile(r"\S*/\.ae/sessions/([^/\s]+)/([A-Za-z_][A-Za-z0-9_-]*)")
PATHNOTE_RE = re.compile(r"PATH=\S*/(a5-[a-z0-9-]+)/bin")

def normalise(argv):
    """Strip host/run-specific prefixes to placeholders; preserve all semantics."""
    s = argv.strip()
    env = ""
    m = re.match(r"^((?:[A-Z_][A-Z0-9_]*=\S+\s+)+)(.*)$", s)
    if m:
        env, s = m.group(1), m.group(2)
    m = HOME_RE.search(s)
    if m:
        s = HOME_RE.sub(r"<AE_HOME>/sessions/\1/\2", s)
    else:
        for pat, repl in HOST_PREFIXES:
            if pat.match(s):
                s = pat.sub(repl, s, count=1); break
    s = PATHNOTE_RE.sub(r"PATH=<CASE>/\1/bin", s)
    return (env + s).strip()

INSTRUMENT_RE = re.compile(r"(?:^|\s)(\S*/(hooked|frozen)/ae)(?:\s|$)")

def instrument(argv):
    """WHICH BINARY produced the capture. This is PROVENANCE, not part of the
    invocation: `frozen/ae list --json` and `hooked/ae list --json` are the SAME
    invocation run through different instruments, so they must normalise together —
    but the distinction is real and is kept HERE rather than discarded."""
    m = INSTRUMENT_RE.search(argv)
    if m: return m.group(2)
    if "/.ae/sessions/" in argv: return "generated-helper"
    return "unrecorded"

def classify(norm):
    """-> (phase, surface, reason). UNRESOLVED is a real answer, not a fallback."""
    m = HOME_RE.sub(r"\2", norm) if "<AE_HOME>" in norm else None
    if "<AE_HOME>/sessions/" in norm:
        helper = norm.split("/sessions/", 1)[1].split("/", 1)[1].split()[0]
        if helper in READ_HELPER:
            return "P1", "helper:" + helper, "read surface; VISION:93 names requests/events queries"
        return "UNRESOLVED", "helper:" + helper, "helper not on the read list and not classifiable from argv alone"
    toks = norm.split()
    if not toks or toks[0] != "ae":
        # an env-prefixed binary invocation still starts `ae` after the env
        toks = [t for t in toks if "=" not in t.split("/")[0]] or toks
        if not toks or toks[0] != "ae":
            return "UNRESOLVED", norm, "argv does not begin with a recognised invocation"
    rest = toks[1:]
    rest = [r for r in rest if r != "--local"] or rest
    if not rest:
        return "UNRESOLVED", "ae", "no subcommand recorded"
    sub = rest[0]
    for s_, flag in WRITE_MARKERS:
        if sub == s_ and flag in rest:
            return "P2", "ae " + sub + " " + flag, "performs an action beyond reporting"
    if sub in READ_BINARY:
        return "P1", "ae " + sub, "read surface"
    if sub.startswith("-"):
        return "P1", "ae " + sub, "flag-only form of the read surface"
    return "P2", "ae <session>", "session-name token is a launch candidate, a lifecycle surface"

def main():
    rows, seen_files = [], 0
    for f in sorted(glob.glob(os.path.join(SRC, "arms", "*", "*", "consumers.tsv"))):
        seen_files += 1
        rel = os.path.relpath(f, SRC)
        with open(f, encoding="utf-8") as fh:
            r = csv.reader(fh, delimiter="\t")
            hdr = next(r, None)
            for c in r:
                if len(c) < 10: continue
                argv = c[9]
                if argv == "-": 
                    rows.append((rel, c[0], c[1], "UNRESOLVED", "-", "-", "unrecorded", "no argv recorded")); continue
                n = normalise(argv)
                phase, surface, why = classify(n)
                rows.append((rel, c[0], c[1], phase, surface, n, instrument(argv), why))
    with open(OUT, "w", encoding="utf-8") as fh:
        fh.write("case\tconsumer\trc\tphase\tsurface\tnormalised_argv\tinstrument\treason\n")
        for x in rows: fh.write("\t".join(x) + "\n")
    cnt = collections.Counter(x[3] for x in rows)
    print("read %d consumers.tsv files, %d rows" % (seen_files, len(rows)))
    for k in sorted(cnt): print("  %-11s %d" % (k, cnt[k]))
    print("wrote %s" % OUT)

if __name__ == "__main__":
    main()
