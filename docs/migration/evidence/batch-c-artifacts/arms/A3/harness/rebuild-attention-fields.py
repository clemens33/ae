#!/usr/bin/env python3
"""Re-derive attention-fields.txt from each case's captured list --json BYTES.

Pure post-processing of an already-captured artifact: the source file's sha256 is recorded
so the derivation is checkable, and nothing is re-run. Emits the session attention triple
and the COMPLETE per-agent object (ref, alias, name, session_id, alive, state, reason) so
a null reason beside a non-null session attention is visible rather than hidden by a
lossy grep.
"""
import hashlib, json, os, sys
roots = sys.argv[1:]
n = 0
for root in roots:
    for case in sorted(os.listdir(root)):
        src = os.path.join(root, case, "out", "list-json.stdout")
        dst = os.path.join(root, case, "attention-fields.txt")
        if not os.path.exists(src):
            continue
        raw = open(src, "rb").read()
        out = ["## attention fields, re-derived from out/list-json.stdout",
               f"source=out/list-json.stdout sha256={hashlib.sha256(raw).hexdigest()} bytes={len(raw)}",
               "derivation=pure post-processing of the captured bytes; nothing was re-run", ""]
        try:
            doc = json.loads(raw.decode())
        except Exception as e:
            out.append(f"unparseable_as_json={e}")
            open(dst, "w").write("\n".join(out) + "\n"); n += 1; continue
        for sess in doc.get("sessions", []):
            out.append(f'session={sess.get("name")}')
            out.append(f'  needs_attention={json.dumps(sess.get("needs_attention"))}')
            out.append(f'  attention={json.dumps(sess.get("attention"))}')
            out.append(f'  attention_rank={json.dumps(sess.get("attention_rank"))}')
            out.append(f'  last_active_epoch={json.dumps(sess.get("last_active_epoch"))}')
            out.append("  agents:")
            for a in sess.get("agents", []):
                out.append("    " + json.dumps(a, sort_keys=True))
        open(dst, "w").write("\n".join(out) + "\n")
        n += 1
print(f"rebuilt {n} attention-fields.txt")
