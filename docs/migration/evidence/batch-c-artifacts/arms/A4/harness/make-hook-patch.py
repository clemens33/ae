#!/usr/bin/env python3
"""Build the ONE hook-only patch over an exact 72c7293 copy of `ae`.

Hooks are NO-OPS unless AE_HOOK names them: the guard is the first statement and
returns 0 before any side effect, so an inactive hook writes nothing, prints nothing
and changes no exit status. An ACTIVE hook only records that it was reached and blocks
until the controller releases it — it never performs the mutation itself.
"""
import hashlib, subprocess, sys

SRC = "/private/tmp/claude-501/-Users-ckriech-projects-clemens33-ae-rust/347d2089-7268-421d-8188-8924e246bbf0/scratchpad/frozen/ae"
DST = "/tmp/aecx/hooked/ae"
DIFF = "/tmp/aecx/hooked/hook.patch"

FUNC = '''# --- BATCH C INSTRUMENTATION: the ONE hook-only patch (cluster-plan global rule) ---
# A hook is a NO-OP unless AE_HOOK names it: the guard is the first statement and
# returns 0 before any side effect, so an INACTIVE hook writes nothing, prints nothing,
# touches no file and cannot change an exit status. An ACTIVE hook only records that it
# was reached and BLOCKS until the controller releases it — the controller, never the
# hook, performs the named writer-shaped mutation.
_ae_hook() {
    [[ "${AE_HOOK:-}" == "$1" ]] || return 0
    local _d="${AE_HOOK_DIR:-}"
    [[ -n "$_d" ]] || return 0
    printf '%s\\t%s\\t%s\\n' "$1" "$(date -u +%FT%TZ)" "$$" >>"${_d}/hook.log"
    : >"${_d}/${1}.reached"
    while [[ ! -e "${_d}/${1}.release" ]]; do sleep 0.05; done
    printf '%s\\t%s\\t%s\\n' "${1}-RELEASED" "$(date -u +%FT%TZ)" "$$" >>"${_d}/hook.log"
    return 0
}

'''

EDITS = [
    # (anchor, insertion, mode) mode: 'before' | 'after'
    ("list_ae_sessions() {\n", FUNC, "before"),
    # D01 — running-session site, immediately after meta_blob is read
    ('            [[ -n "$sess_dir" && -f "${sess_dir}/meta" ]] && meta_blob="$(<"${sess_dir}/meta")"\n'
     '            local origin mode workdir\n',
     '            _ae_hook H_LIST_META_CAPTURED\n', "middle"),
    # D02 — after the reversed request scan completes, before row emission
    ('    done < <(_ae_tac "$file" 2>/dev/null || true)\n'
     '    local i status summary\n',
     '    _ae_hook H_REQUEST_SCAN_COMPLETE\n', "middle"),
    # D04b — after best-candidate resolution, before the exact recheck
    ('    if [[ -z "$best_name" ]]; then\n'
     '        echo "ae next: no running session needs attention." >&2\n'
     '        return 1\n'
     '    fi\n',
     '\n    _ae_hook H_NEXT_SELECTED\n', "after"),
    # D04b — after the successful exact recheck, before the final focus call
    ('        echo "ae next: \'$best_name\' disappeared before attach." >&2\n'
     '        return 1\n'
     '    fi\n',
     '    _ae_hook H_NEXT_RECHECKED\n', "after"),
]

src = open(SRC).read()
out = src
for anchor, ins, mode in EDITS:
    n = out.count(anchor)
    if n != 1:
        sys.exit(f"ANCHOR NOT UNIQUE ({n}): {anchor[:70]!r}")
    if mode == "before":
        out = out.replace(anchor, ins + anchor, 1)
    elif mode == "after":
        out = out.replace(anchor, anchor + ins, 1)
    else:  # middle: split the two-line anchor and insert between
        a, b = anchor.split("\n", 1)
        out = out.replace(anchor, a + "\n" + ins + b, 1)

import os
os.makedirs("/tmp/aecx/hooked", exist_ok=True)
open(DST, "w").write(out)
os.chmod(DST, 0o755)
d = subprocess.run(["diff", "-u", SRC, DST], capture_output=True, text=True).stdout
open(DIFF, "w").write(d)
print("source_sha256   =", hashlib.sha256(open(SRC,'rb').read()).hexdigest())
print("hooked_sha256   =", hashlib.sha256(open(DST,'rb').read()).hexdigest())
print("patch_sha256    =", hashlib.sha256(d.encode()).hexdigest())
print("patch_lines     =", len(d.splitlines()))
print("added_lines     =", sum(1 for l in d.splitlines() if l.startswith('+') and not l.startswith('+++')))
print("removed_lines   =", sum(1 for l in d.splitlines() if l.startswith('-') and not l.startswith('---')))
