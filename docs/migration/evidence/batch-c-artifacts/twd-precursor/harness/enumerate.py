#!/usr/bin/env python3
"""Per-specimen enumeration of a T-WD arm's harvested event bytes.

Value-blind: emits byte facts only (offset, length, hashes, raw line, the
captured action/actor/ref/summary byte values, and the cycle label at which the
line first appeared in the per-cycle snapshots). No verdicts, no expectations.
"""
import hashlib, json, os, re, sys

arm = sys.argv[1]
cap = sys.argv[2]
out_dir = sys.argv[3]

final = os.path.join(cap, "events.final.jsonl")
if not os.path.exists(final):
    # fall back to the newest per-cycle snapshot
    cands = sorted(f for f in os.listdir(cap) if f.startswith("events.") and f.endswith(".jsonl"))
    if not cands:
        print(f"{arm}: no events snapshots", file=sys.stderr); sys.exit(1)
    final = os.path.join(cap, cands[-1])

raw = open(final, "rb").read()

# order the per-cycle snapshots by their stamp epoch so "first seen" is real
stamps = {}
for f in os.listdir(cap):
    if f.startswith("stamp.") and f.endswith(".txt"):
        lbl = f[len("stamp."):-len(".txt")]
        ep = 0
        for line in open(os.path.join(cap, f)):
            if line.startswith("epoch="):
                ep = int(line.strip().split("=", 1)[1])
        stamps[lbl] = ep
ordered = [l for l, _ in sorted(stamps.items(), key=lambda kv: kv[1])]
snap = {}
for lbl in ordered:
    p = os.path.join(cap, f"events.{lbl}.jsonl")
    snap[lbl] = open(p, "rb").read() if os.path.exists(p) else b""

def field(obj, k):
    v = obj.get(k, None)
    return "\x00ABSENT" if v is None else v

specs = []
off = 0
lineno = 0
for line in raw.splitlines(keepends=True):
    lineno += 1
    body = line[:-1] if line.endswith(b"\n") else line
    try:
        obj = json.loads(body.decode("utf-8"))
    except Exception:
        obj = {}
    first = "-"
    for lbl in ordered:
        if line in snap[lbl] or body in snap[lbl]:
            first = lbl
            break
    specs.append({
        "arm": arm,
        "line_no": lineno,
        "byte_offset": off,
        "byte_len_with_nl": len(line),
        "byte_len_no_nl": len(body),
        "sha256_line_with_nl": hashlib.sha256(line).hexdigest(),
        "sha256_line_no_nl": hashlib.sha256(body).hexdigest(),
        "action": field(obj, "action"),
        "actor": field(obj, "actor"),
        "ref": field(obj, "ref"),
        "summary": field(obj, "summary"),
        "ts": field(obj, "ts"),
        "first_seen_capture_label": first,
        "raw_line_no_nl": body.decode("utf-8", "backslashreplace"),
    })
    off += len(line)

os.makedirs(out_dir, exist_ok=True)
with open(os.path.join(out_dir, f"specimens.{arm}.jsonl"), "w") as fh:
    for s in specs:
        fh.write(json.dumps(s, ensure_ascii=False, sort_keys=True) + "\n")

ALERT_FAMILY = {"alert", "alert-cleared", "throttled", "throttle-cleared"}
alerts = [s for s in specs if s["action"] in ALERT_FAMILY]
with open(os.path.join(out_dir, f"alert-specimens.{arm}.jsonl"), "w") as fh:
    for s in alerts:
        fh.write(json.dumps(s, ensure_ascii=False, sort_keys=True) + "\n")

print(json.dumps({
    "arm": arm,
    "source_events_file": final,
    "source_events_sha256": hashlib.sha256(raw).hexdigest(),
    "source_events_bytes": len(raw),
    "total_specimens": len(specs),
    "alert_family_specimens": len(alerts),
    "alert_family_actions": sorted({s["action"] for s in alerts}),
    "all_actions": sorted({s["action"] for s in specs}),
}, indent=2))
