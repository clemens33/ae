#!/usr/bin/env python3
"""Produce the ONE hook-only patch for SC-1301 and the hooked copy of frozen ae.

Admissibility (cluster-plan's global rule): an exact 72c7293 copy plus ONE hook-only patch,
patch and hashes recorded in the run manifest; the INACTIVE hook must be byte/rc/file/tmux
equivalent to the unmodified control; an ACTIVE hook only blocks or emits — the CONTROLLER
performs any mutation.

Three hook points, one per writer, because the three writers do not share a boundary:
  AH_META_TEMP_COMPLETE      ae_meta_set, after the temp file is written and BEFORE the
                             rename (ae:14141) — the atomic writer's only window
  AH_SPAWN_BETWEEN_APPENDS   _cmd_spawn, between two of its OWN appends (ae:11939/11940) —
                             a real partial-generation window the frozen writer produces
  AH_CAPTURE_APPEND_DONE     start_capture_session_id, after its single append (ae:2073) —
                             there is no mid-write window here, so this point exists only
                             to sequence the controller, never to claim a writer tear

usage: make-h-hook-patch.py <frozen-ae> <out-hooked> <out-patch>
"""
import difflib, hashlib, os, sys

src, out_hooked, out_patch = sys.argv[1], sys.argv[2], sys.argv[3]
orig = open(src, encoding="utf-8", errors="surrogateescape").read().split("\n")
lines = list(orig)

HOOK_FN = '''_ae_hook() { # <point> — GUARD FIRST: with AE_HOOK unset this is a no-op and returns 0
    [[ -n "${AE_HOOK:-}" ]] || return 0
    [[ "${AE_HOOK}" == "$1" ]] || return 0
    printf '%s\\t%s\\n' "$1" "$(/bin/date -u +%s)" >>"${AE_HOOK_MARK:-/dev/null}"
    if [[ -n "${AE_HOOK_WAIT:-}" ]]; then
        local _n=0
        while [[ ! -e "${AE_HOOK_WAIT}" ]] && (( _n < 120 )); do sleep 0.5; _n=$((_n + 1)); done
    fi
    return 0
}
'''

def insert_after(pred, text, once=True):
    for i, l in enumerate(lines):
        if pred(l):
            lines.insert(i + 1, text)
            return i + 1
    raise SystemExit(f"anchor not found: {pred}")

# the hook function itself, defined early enough for every caller
insert_after(lambda l: l.startswith("AE_VERSION="), HOOK_FN)

# 1. atomic writer: split the `>"$tmp" && mv` so the hook sits between them
for i, l in enumerate(lines):
    if l.strip() == "' \"$meta_file\" >\"$tmp\" && mv \"$tmp\" \"$meta_file\"":
        indent = l[:len(l) - len(l.lstrip())]
        lines[i] = (f"{indent}' \"$meta_file\" >\"$tmp\" || return 1\n"
                    f"{indent}_ae_hook AH_META_TEMP_COMPLETE\n"
                    f"{indent}mv \"$tmp\" \"$meta_file\"")
        break
else:
    raise SystemExit("atomic writer anchor not found")

# 2. _cmd_spawn: between two of its own appends
for i, l in enumerate(lines):
    if 'echo "agent.${slot}=${alias_name}:${spawn_name}:${session_id}" >>"$meta_dir/meta"' in l:
        indent = l[:len(l) - len(l.lstrip())]
        lines.insert(i + 1, f"{indent}_ae_hook AH_SPAWN_BETWEEN_APPENDS")
        break
else:
    raise SystemExit("spawn anchor not found")

# 3. start_capture_session_id: after its single append
for i, l in enumerate(lines):
    if 'echo "launch_time.${slot}=$(date +%s)" >>"$meta_dir/meta"' in l:
        indent = l[:len(l) - len(l.lstrip())]
        lines.insert(i + 1, f"{indent}_ae_hook AH_CAPTURE_APPEND_DONE")
        break
else:
    raise SystemExit("capture anchor not found")

# the helper-side writer is emitted into generated helpers by `declare -f`, so the hook
# must ride the same emission list or the helper calls a function it does not have
# The helper-side writer is emitted into every generated _lib by `declare -f`. The hook it
# now calls must ride the SAME emission list or a generated helper calls a function it does
# not have — the D02 discovery from batch C, applied before it costs a run.
for i, l in enumerate(lines):
    if "ae_meta_get ae_meta_set ae_meta_unset" in l:
        lines[i] = l.replace("ae_meta_get ae_meta_set ae_meta_unset",
                             "_ae_hook ae_meta_get ae_meta_set ae_meta_unset")
        break
else:
    raise SystemExit("declare -f emission list anchor not found")

hooked = "\n".join(lines)
open(out_hooked, "w", encoding="utf-8", errors="surrogateescape").write(hooked)
# The hooked copy must be EXECUTABLE. Without this it is a file ae's own AE_PATH guard
# refuses — "ae not found (expected at ...)" — which is how the spawn cut failed to arm:
# a real product guard, reporting correctly, about an instrument that was never runnable.
os.chmod(out_hooked, 0o755)
diff = list(difflib.unified_diff(orig, lines, fromfile="ae@72c7293", tofile="ae@72c7293+hook", lineterm=""))
open(out_patch, "w").write("\n".join(diff) + "\n")
added = sum(1 for d in diff if d.startswith("+") and not d.startswith("+++"))
removed = sum(1 for d in diff if d.startswith("-") and not d.startswith("---"))
print(f"hooked_sha256={hashlib.sha256(hooked.encode('utf-8', 'surrogateescape')).hexdigest()}")
print(f"patch_sha256={hashlib.sha256(open(out_patch,'rb').read()).hexdigest()}")
print(f"lines_added={added} lines_removed={removed}")
