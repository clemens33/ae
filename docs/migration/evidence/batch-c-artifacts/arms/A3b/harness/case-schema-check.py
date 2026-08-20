#!/usr/bin/env python3
"""Per-case artifact SCHEMA check — the third gate half.

files==listed cannot see a file deleted TOGETHER with its SHA256SUMS line, and the
citation resolver deduplicates relative tokens globally and resolves them in ANY matching
context, so neither half checks per-case COMPLETENESS. This does.

Each case directory declares its KIND through its own admissibility ledger — the ledger is
written by the checks as they run, so the kind cannot be forged by deleting a file:
  barrier : the ledger contains barrier-ARMED
  twin    : the ledger contains twin-note
  live    : the ledger contains live-topology-built / events-tail-STARTED / stage-snapshot
  document: none of the above
  hooked  : an additional facet — the ledger contains hook-inactive-equivalence-START
A case must contain every file its kind requires (schema table, `case-schema.tsv`), and
ledger.tsv membership must be bidirectional: every ledger row has at least one case
directory, and every case directory appears in ledger.tsv.
"""
import os, re, sys

TREE = sys.argv[1] if len(sys.argv) > 1 else os.environ.get(
    "BATCH_C_ARTIFACTS",
    "/Users/ckriech/projects/clemens33/ae-rust/docs/migration/evidence/batch-c-artifacts")
SCHEMA = sys.argv[2] if len(sys.argv) > 2 else os.path.join(
    os.path.dirname(os.path.abspath(__file__)), "case-schema.tsv")

req = {}
for line in open(SCHEMA):
    line = line.rstrip("\n")
    if not line or line.startswith("kind\t"):
        continue
    if line.startswith("#"):
        continue
    k, f = line.split("\t", 1)
    req.setdefault(k, []).append(f)

bad, checked = [], 0
arms = os.path.join(TREE, "arms")
for arm in sorted(os.listdir(arms)) if os.path.isdir(arms) else []:
    adir = os.path.join(arms, arm)
    if not os.path.isdir(adir):
        continue
    ledger_tsv = os.path.join(adir, "ledger.tsv")
    declared = set()
    if os.path.exists(ledger_tsv):
        for i, line in enumerate(open(ledger_tsv)):
            if i == 0 or not line.strip():
                continue
            declared.add(line.split("\t")[0].strip())
    cases = [c for c in sorted(os.listdir(adir))
             if os.path.isdir(os.path.join(adir, c)) and c != "harness"]
    seen = set()
    for c in cases:
        cdir = os.path.join(adir, c)
        led = os.path.join(cdir, "admissibility-ledger.txt")
        if not os.path.exists(led):
            bad.append((f"{arm}/{c}", "admissibility-ledger.txt", "MISSING (cannot even determine the case kind)"))
            checked += 1
            continue
        text = open(led, encoding="utf-8", errors="replace").read()
        kinds = ["any"]
        if "event=barrier-ARMED" in text: kinds.append("barrier")
        elif "event=twin-note" in text:   kinds.append("twin")
        staged = "event=stage-snapshot" in text
        kinds.append("staged" if staged else "unstaged")
        if "event=events-tail-STARTED" in text:
            kinds.append("pane-follow")
        # consumer-run is decided by whether the case ACTUALLY ran a consumer, not by
        # exclusion: a controller-only twin legitimately runs none and has no out/.
        if any(e in text for e in ("event=consumer-START", "event=plain-consumer-START",
                                   "event=barrier-consumer-COMPLETE", "event=attach-consumer-COMPLETE")):
            kinds.append("consumer-run")
        if "event=live-topology-built" in text: kinds.append("live")
        if not any(k in kinds for k in ("barrier", "twin", "live")) and not staged \
                and "event=events-tail-STARTED" not in text:
            kinds.append("document")
        if "event=hook-inactive-equivalence-START" in text: kinds.append("hooked")
        # A case is tmux-bound when its OWN recorded environment gave it a server. That is
        # the case's declaration, not an inference from which files happen to exist — the
        # A5 doctor arms run under a controlled PATH with no tmux at all and must not be
        # asked for snapshots of a server they never had.
        envp = os.path.join(cdir, "env.txt")
        if os.path.exists(envp) and "AE_TMUX_SERVER=" in open(envp, encoding="utf-8", errors="replace").read():
            kinds.append("tmux-bound-staged" if staged else "tmux-bound-unstaged")
        for k in kinds:
            for f in req.get(k, []):
                if f.startswith("glob:"):
                    _, n, pat = f.split(":", 2)
                    import glob as _g
                    hits = _g.glob(os.path.join(cdir, pat))
                    ok = len(hits) >= int(n)
                    if not ok:
                        bad.append((f"{arm}/{c}", f, f"kind '{k}' requires at least {n} matching {pat}, found {len(hits)}"))
                    continue
                p = os.path.join(cdir, f.rstrip("/"))
                ok = os.path.isdir(p) and bool(os.listdir(p)) if f.endswith("/") else os.path.exists(p)
                if not ok:
                    bad.append((f"{arm}/{c}", f, f"required by kind '{k}' but absent or empty"))
        checked += 1
        base = re.sub(r"-(controlled-path|ro|rw|live|twin|barrier|attach|follow)$", "", c)
        seen.add(base)
    # CASE INDEX membership, directory-exact and content-bound.
    idx = os.path.join(adir, "CASES.tsv")
    if not os.path.exists(idx):
        bad.append((f"{arm}/CASES.tsv", "-", "case index missing — per-case completeness cannot be checked"))
    else:
        indexed = {}
        for i, line in enumerate(open(idx)):
            if i == 0 or not line.strip():
                continue
            parts = line.rstrip("\n").split("\t")
            indexed[parts[0]] = parts[1] if len(parts) > 1 else ""
        for cd, lh in sorted(indexed.items()):
            led_p = os.path.join(adir, cd, "admissibility-ledger.txt")
            if not os.path.exists(led_p):
                bad.append((f"{arm}/{cd}", "admissibility-ledger.txt",
                            "indexed in CASES.tsv but the case directory or its ledger is gone"))
                continue
            import hashlib
            got = hashlib.sha256(open(led_p, "rb").read()).hexdigest()
            if lh and got != lh:
                bad.append((f"{arm}/{cd}", "admissibility-ledger.txt",
                            f"ledger content does not match the case index (indexed {lh[:12]}, found {got[:12]})"))
        for cd in sorted(set(cases) - set(indexed)):
            bad.append((f"{arm}/{cd}", "-", "case directory exists but is not in CASES.tsv"))
    for d in sorted(declared - seen):
        bad.append((f"{arm}/ledger.tsv", d, "declared in ledger.tsv but no case directory exists"))
    for s in sorted(seen - declared):
        bad.append((f"{arm}/{s}", "-", "case directory exists but is not declared in ledger.tsv"))

print(f"tree={TREE}")
print(f"cases_checked={checked} schema_violations={len(bad)}")
for b in bad:
    print(f"  SCHEMA-FAIL {b[0]} :: {b[1]} — {b[2]}")
sys.exit(1 if bad else 0)
