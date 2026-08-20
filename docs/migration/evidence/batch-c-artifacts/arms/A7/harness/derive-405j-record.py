#!/usr/bin/env python3
"""SC-405j identity record, derived from captured bytes.

Includes the responsiveness controls the row needs. The FIRST attempt built these cases on
a LONE ASK: all seven rendered identically, which was a property of the fixture — with no
reply to pair against, the routing members are inert and nothing can depend on them. The
cases were rebuilt on an ask->REPLY pair, where the keys can matter, and the record below
shows the readings are no longer uniform. Nothing is interpreted.
"""
import hashlib, os, re, sys
A = sys.argv[1]
CASES = [
    ("a7-c12-405j-pair-full-fresh-rw",   "1  full + fresh routing keys (CONTROL, unmutated)"),
    ("a7-c13-405j-pair-stale-keys-rw",   "2  all four present, naming a slot and session this is not"),
    ("a7-c14-405j-pair-slot-only-rw",    "3a partial: slot members only, session members deleted"),
    ("a7-c15-405j-pair-session-only-rw", "3b partial: session members only, slot members deleted"),
    ("a7-c16-405j-pair-keyless-rw",      "4  keyless legacy: no routing members at all"),
    ("a7-c17-405j-pair-one-empty-rw",    "5a one member present as the EMPTY STRING"),
    ("a7-c18-405j-pair-all-empty-rw",    "5b all four present as the EMPTY STRING"),
]
L = ["## SC-405j identity record — derived from captured bytes",
     "derivation=pure post-processing; each source file's sha256 is recorded", "",
     "All cases share ONE display name and differ only in the completeness of the REPLY's",
     "routing members. The base is a real ask and its real identity-valid reply.", ""]
seen = {}
for case, label in CASES:
    p = os.path.join(A, case, "out", "requests-all.stdout")
    if not os.path.exists(p):
        L.append(f"  {label:58s} <no capture>"); continue
    raw = open(p, "rb").read()
    rows = [l for l in raw.decode("utf-8", "replace").splitlines() if l and not l.startswith("STATUS")]
    status = rows[0].split()[0] if rows else "<no row>"
    summary = " ".join(rows[0].split()[5:]) if rows else ""
    seen.setdefault(status, []).append(label.split()[0])
    L.append(f"  {label:58s} status={status:8s} summary={summary!r}")
    L.append(f"      src_sha256={hashlib.sha256(raw).hexdigest()}")
L.append("")
L.append("## responsiveness")
L.append(f"  distinct statuses observed across the cases: {sorted(seen)}")
L.append(f"  grouped: " + "; ".join(f"{k} <- cases {','.join(v)}" for k, v in sorted(seen.items())))
L.append("  A single distinct status across all seven would mean the fixture cannot")
L.append("  discriminate key completeness and the row would be INCONCLUSIVE here.")
L.append(f"  discriminating={'yes' if len(seen) > 1 else 'no'}")
L.append("")
L.append("## why these cases sit on a PAIR and not a lone ask")
L.append("  The first attempt used a lone ask. All seven readings were identical because a")
L.append("  routing member with nothing to pair against cannot affect anything — a property")
L.append("  of the fixture, not of the product. The superseded cases were removed rather")
L.append("  than published; the A6 SC-518 captures had already shown the consumer responds")
L.append("  sharply to the pairing inputs, which is what made the flat result suspect.")
open(os.path.join(A, "identity-405j-record.txt"), "w").write("\n".join(L) + "\n")
print("\n".join(L[-8:]))
