#!/usr/bin/env python3
"""Re-derive an A6 threshold case's discrimination record from its CAPTURED bytes.

Third time an ad-hoc text filter has silently produced nothing on this batch, so the
extraction is a tested helper now instead of a one-off pattern. The previous version used
sed BRE alternation (\\|), which is a GNU extension: BSD sed matches nothing, so every
reading came back empty and the record said "responsive=no" — the fixture was fine and the
instrument was blind. A filter that matches nothing looks exactly like a subject that
produced nothing.

Reads only files already in the case; records each source's sha256; nothing is re-run.
"""
import hashlib, json, os, sys

C = sys.argv[1]
ASK_EPOCH = int(sys.argv[2]) if len(sys.argv) > 2 else 1755000000

def read(lbl):
    p = os.path.join(C, "out", f"{lbl}_list_json.stdout")
    if not os.path.exists(p):
        return None, None
    raw = open(p, "rb").read()
    try:
        d = json.loads(raw.decode())
        s = d["sessions"][0] if d.get("sessions") else {}
        return (s.get("needs_attention"), s.get("attention"), s.get("attention_rank")), \
               hashlib.sha256(raw).hexdigest()
    except Exception as e:
        return ("UNPARSEABLE", str(e), None), hashlib.sha256(raw).hexdigest()

lo, lo_h = read("age10")
hi, hi_h = read("age100000")
responsive = lo is not None and hi is not None and lo != hi

out = []
out.append("## discrimination record — re-derived from the captured list --json bytes")
out.append("derivation=pure post-processing; every source file's sha256 is recorded below")
out.append("")
out.append("An equality-versus-strictly-past question can only be decided on this fixture if")
out.append("the sensor responds to the age AT ALL. Two controls far either side of the")
out.append("threshold, on the SAME fixture, differing only in the frozen clock:")
out.append(f"  age=10s      needs_attention={lo[0]} attention={lo[1]} rank={lo[2]}   src_sha256={lo_h}")
out.append(f"  age=100000s  needs_attention={hi[0]} attention={hi[1]} rank={hi[2]}   src_sha256={hi_h}")
out.append(f"responsive={'yes' if responsive else 'no'}")
out.append("If responsive=no the boundary triple below cannot discriminate anything and this")
out.append("arm is INCONCLUSIVE for SC-522/523 rather than evidence of either answer.")
out.append("")
out.append("## boundary triple — one fixture, ask ts fixed at epoch %d, three frozen nows" % ASK_EPOCH)
for age in (1799, 1800, 1801):
    v, h = read(f"age{age}")
    out.append(f"  age={age:<6} needs_attention={v[0]} attention={v[1]} rank={v[2]}   src_sha256={h}")
out.append("")
out.append("## threshold taken from the environment — all read at age=1000s")
for v_ in ("unset", "500", "900x"):
    v, h = read(f"env_{v_}")
    out.append(f"  AE_ATTN_REQUEST_SECS={v_:<6} needs_attention={v[0]} attention={v[1]} rank={v[2]}   src_sha256={h}")
out.append("")
out.append("No reading is interpreted here; the values are the record.")
open(os.path.join(C, "discrimination.txt"), "w").write("\n".join(out) + "\n")
print("\n".join(out))
sys.exit(0 if responsive else 3)
