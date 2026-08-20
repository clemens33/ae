#!/opt/homebrew/bin/bash
# L-END arm: handover (SC-830, SC-831).
set -uo pipefail
source /tmp/aelx/lib/arm.sh

l_arm_begin L-END handover frozen
l_config "$R" claude
{ l_mkrepo "$R"; } >/dev/null 2>&1
HOOKS=""; BLOCK=""; l_arm_env
AE_CWD="$R/w"; WDIR="$R/w"
l_ae 1launch --local hv1
sleep 3
l_arm_preflight hv1 || { printf 'PREFLIGHT-FAILED\n' >"$R/cap/ARM-INVALID.txt"; l_arm_end; exit 1; }
META="$R/h/.ae/sessions/hv1"

reqstate() { # <label>
    { printf '## requests all\n'; env -i "${AE_ENV[@]}" "$META/requests" all 2>&1
      printf '\n## requests inbox\n'; env -i "${AE_ENV[@]}" "$META/requests" inbox 2>&1
      printf '\n## events.jsonl byte count\t%s\n' "$(stat -f '%z' "$META/events.jsonl" 2>/dev/null || echo '-')"
      printf '## memo.tsv byte count\t%s\n' "$(stat -f '%z' "$META/memo.tsv" 2>/dev/null || echo '-')"
    } >"$R/cap/requests.$1.txt" 2>&1
    cp "$META/events.jsonl" "$R/cap/events.$1.jsonl" 2>/dev/null || : >"$R/cap/events.$1.jsonl"
    [[ -d "$META/messages" ]] && l_manifest "$META/messages" "$R/cap/messages.$1.tsv"
    return 0
}

reqstate 0pre
l_snap 0pre

# ── SC-831: a handover under a SHORTENED bound ────────────────────────────
BOUND=8
l_arm_env "AE_COMPACT_HANDOVER_SECS=$BOUND"
l_ae 2compact-bounded compact -f hv1
sleep 1
reqstate 1at-expiry
l_snap 1at-expiry
l_manifest "$META" "$R/cap/1at-expiry.sessiondir.tsv"

# ── SC-830: the same session compacted with --digest-only ─────────────────
reqstate 2before-digest-only
l_arm_env
l_ae 3compact-digest-only compact -f --digest-only hv1
sleep 2
# the source session is gone if the boundary was crossed; capture whatever is there
reqstate 3after-digest-only 2>/dev/null || true
l_snap 3post
{ printf '# archive dirs\n'; ls -1 "$R/h/.ae/archive" 2>&1
  printf '\n# session dirs\n'; ls -1 "$R/h/.ae/sessions" 2>&1
  printf '\n# archived events.jsonl (request rows), per archive\n'
  for e in "$R"/h/.ae/archive/*/events.jsonl; do [[ -e "$e" ]] || continue
    printf '=== %s ===\n' "$e"; cat "$e"; done
  printf '\n# archived digest.md, per archive\n'
  for d in "$R"/h/.ae/archive/*/digest.md; do [[ -e "$d" ]] || continue
    printf '=== %s ===\n' "$d"; cat "$d"; done
} >"$R/cap/post-archive.txt" 2>&1

{ printf 'arm\thandover\nsection\tL-END\n'
  printf 'roster_ids\tSC-830 SC-831\n'
  printf 'fixture\t--local family, renamed-interpreter fake tool that accepts real sends\n'
  printf 'construction\tcompact -f runs first under a SHORTENED handover bound (AE_COMPACT_HANDOVER_SECS=%s) with no reply and no handover memo supplied; the same session is then compacted with --digest-only\n' "$BOUND"
  printf 'handover_bound_sec\t%s\n' "$BOUND"
  printf 'compact_bounded_rc\t%s\n' "$(cat "$R/cap/2compact-bounded.rc")"
  printf 'compact_digest_only_rc\t%s\n' "$(cat "$R/cap/3compact-digest-only.rc")"
  printf 'inconclusive_discipline\ta bounded wait that expires is recorded as the product'"'"'s own reported outcome plus the state at expiry; no absence is inferred\n'
} >"$R/cap/ARM.txt"
l_arm_end
echo DONE
