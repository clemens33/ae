#!/opt/homebrew/bin/bash
# L-DISCRIM shared helpers. Every arm here exists because an earlier arm could not
# FAIL the claim it was cited for, so each one carries an explicit ability-to-fail
# control and writes ARM-INVALID when that control does not hold.
set -uo pipefail
source /tmp/aelx/lib/arm.sh
ARTROOT=/Users/ckriech/projects/clemens33/ae-rust/docs/migration/evidence/l-artifacts

l_use_v3() { cp /tmp/aelx/instr3/ae "$R/b/ae"; chmod 0755 "$R/b/ae"; }

led() { # <checkpoint> <field> <value...>
    local cp="$1" f="$2"; shift 2
    printf '%s\t%s\t%s\t%s\n' "$(date -u '+%Y-%m-%dT%H:%M:%SZ')" "$cp" "$f" "$*" >>"$R/cap/ledger.tsv"
    return 0
}

darmtxt() { # <arm> <ids> <why-this-arm-exists> <construction> [extra...]
    local arm="$1" ids="$2" why="$3" con="$4"; shift 4
    { printf 'arm\t%s\nsection\tL-DISCRIM\n' "$arm"
      printf 'roster_ids\t%s\n' "$ids"
      printf 'exists_because\t%s\n' "$why"
      printf 'construction\t%s\n' "$con"
      printf 'hook_patch_version\t%s\n' "${PATCHV:-none (frozen, unmodified)}"
      printf 'binary.sha256\t%s\n' "$(l_sha "$R/b/ae")"
      local x; for x in "$@"; do printf '%s\n' "$x"; done
    } >"$R/cap/ARM.txt"
}

dsnap() { # <label>
    local l="$1"
    l_manifest "$R/h/.ae" "$R/cap/$l.aehome.tsv"
    l_manifest "$R/h/.ae/sessions" "$R/cap/$l.sessions.tsv"
    l_manifest "$R/h/.ae/archive" "$R/cap/$l.archive.tsv"
    l_tmuxsnap "$SOCK" "$R/cap/$l.tmux.txt"
    return 0
}

# The unmodelled fake, so helper delivery is never gated by a readiness sensor.
d_config() { # <root>
    { printf '[agents]\n'
      printf 'grok = "grok %s/b/fake-tool.sh"\n' "$R"
      printf '\n[workspace]\nmain = grok\nlayout = vertical\n'
    } >"$R/h/.ae/config"
    cp /tmp/aelx/lib/t100-fake.sh "$R/b/fake-tool.sh"
    return 0
}

# A pane the real helpers accept as the agent. Killed by the caller when done.
d_plant_pane() { # <session>
    local sess="$1" row mainpane agent slot newp
    row="$(/opt/homebrew/bin/tmux -S "$SOCK" list-panes -a -F '#{pane_id} #{@ae_agent} #{@ae_slot}' | awk '$2!="" && $2!~/^_/{print;exit}')"
    read -r mainpane agent slot <<<"$row"
    newp="$(/opt/homebrew/bin/tmux -S "$SOCK" new-window -d -t "$sess:" -c "$R/w" -P -F '#{pane_id}')"
    /opt/homebrew/bin/tmux -S "$SOCK" set-option -p -t "$newp" @ae_agent "$agent"
    /opt/homebrew/bin/tmux -S "$SOCK" set-option -p -t "$newp" @ae_slot "$slot"
    printf '%s' "$newp"
}

d_pane_run() { # <pane> <label> <cmd-string>
    local p="$1" l="$2" c="$3"
    /opt/homebrew/bin/tmux -S "$SOCK" send-keys -t "$p" -l -- "$c > $R/cap/$l.stdout 2> $R/cap/$l.stderr; echo \$? > $R/cap/$l.rc"
    /opt/homebrew/bin/tmux -S "$SOCK" send-keys -t "$p" Enter
    printf '%s\n' "$c" >"$R/cap/$l.invocation"
    local i=0; while (( i < 400 )); do [[ -s "$R/cap/$l.rc" ]] && return 0; sleep 0.1; i=$((i+1)); done
    return 1
}

# Build a parent archive whose handover and pending counts are BOTH NON-ZERO and
# DISTINCT — the whole point of these arms, because a 0 cannot be told apart from a
# lost value. Producer-derived: the real memo, spawn and ask helpers do the writing.
# Sets PARENT_UUID, PARENT_H, PARENT_P. Returns non-zero if the counts did not land.
d_make_counted_parent() { # <session> <handovers> <asks>
    local sess="$1" nh="$2" na="$3"
    d_config "$R"
    { l_mkrepo "$R"; } >/dev/null 2>&1
    HOOKS=""; BLOCK=""; l_arm_env
    AE_CWD="$R/w"; WDIR="$R/w"
    l_ae 0launch --local "$sess"
    sleep 4
    l_arm_preflight "$sess" || return 1
    local META="$R/h/.ae/sessions/$sess"
    local AP; AP="$(d_plant_pane "$sess")"; sleep 1
    AGENT_REF="$(/opt/homebrew/bin/tmux -S "$SOCK" list-panes -a -F '#{pane_id} #{@ae_agent}' | awk '$2!="" && $2!~/^_/{print $2; exit}')"
    led fixture planted.pane "$AP"
    led fixture agent.ref "$AGENT_REF"
    # The ask targets the session's OWN main agent. A spawned second agent was tried
    # first and rejected as a fixture: `spawn` reported the brief undelivered and the
    # spawned pane fell back to a shell, so `ask` then refused to send to it and no
    # request event was written at all. Both refusals are recorded in the D2 first
    # attempt. The main agent's pane is live, so the real ask helper accepts it.
    local i
    for ((i = 1; i <= nh; i++)); do
        d_pane_run "$AP" "0memo$i" "$META/memo add --topic handover discriminator fixture handover entry $i" \
            || led fixture "memo$i" "NO EXIT STATUS IN BOUND"
        sleep 1
    done
    for ((i = 1; i <= na; i++)); do
        d_pane_run "$AP" "0ask$i" "$META/ask $AGENT_REF discriminator fixture question $i" \
            || led fixture "ask$i" "NO EXIT STATUS IN BOUND"
        sleep 2
    done
    /opt/homebrew/bin/tmux -S "$SOCK" kill-window -t "$AP" 2>/dev/null
    sleep 1
    cp -p "$META/memo.tsv" "$R/cap/fixture.memo.tsv" 2>/dev/null
    cp -p "$META/events.jsonl" "$R/cap/fixture.events.jsonl" 2>/dev/null
    PARENT_UUID="$(grep '^session_id=' "$META/meta" | head -1 | cut -d= -f2-)"
    l_ae 0end end -f "$sess"
    sleep 2
    local AM="$R/h/.ae/archive/$PARENT_UUID/meta"
    [[ -f "$AM" ]] || return 1
    cp -p "$AM" "$R/cap/parent-archive-meta.txt"
    cp -p "$R/h/.ae/archive/$PARENT_UUID/digest.md" "$R/cap/parent-archive-digest.md"
    PARENT_H="$(grep '^handover_count=' "$AM" | cut -d= -f2-)"
    PARENT_P="$(grep '^pending_request_count=' "$AM" | cut -d= -f2-)"
    led fixture parent.uuid "$PARENT_UUID"
    led fixture parent.handover_count "$PARENT_H"
    led fixture parent.pending_request_count "$PARENT_P"
    return 0
}

# The ability-to-fail precondition shared by D1 and D2: both counts non-zero AND
# distinct from each other, so a lost value cannot hide behind a default.
d_assert_counts_discriminating() { # <h> <p>
    local h="$1" p="$2"
    led fixture counts.nonzero "$( [[ "$h" != 0 && "$p" != 0 ]] && echo YES || echo NO )"
    led fixture counts.distinct "$( [[ "$h" != "$p" ]] && echo YES || echo NO )"
    [[ "$h" != 0 && "$p" != 0 && "$h" != "$p" ]]
}

# Mutate an archive's two counts CONSISTENTLY in meta and digest so the tree stays
# valid. Named byte diffs, mode preserved.
d_set_counts() { # <archive-dir> <handover> <pending> <label>
    local d="$1" h="$2" p="$3" lbl="$4"
    cp -p "$d/meta" "$R/cap/$lbl.meta.before.txt"
    cp -p "$d/digest.md" "$R/cap/$lbl.digest.before.md"
    l_rewrite_preserving_mode "$d/meta" "s/^handover_count=.*\$/handover_count=${h}/"
    l_rewrite_preserving_mode "$d/meta" "s/^pending_request_count=.*\$/pending_request_count=${p}/"
    l_rewrite_preserving_mode "$d/digest.md" "s/^## Handover (.*)\$/## Handover (${h})/"
    l_rewrite_preserving_mode "$d/digest.md" "s/^## Unresolved requests (.*)\$/## Unresolved requests (${p})/"
    cp -p "$d/meta" "$R/cap/$lbl.meta.after.txt"
    cp -p "$d/digest.md" "$R/cap/$lbl.digest.after.md"
    diff -u "$R/cap/$lbl.meta.before.txt" "$R/cap/$lbl.meta.after.txt" >"$R/cap/$lbl.meta.diff" 2>&1
    diff -u "$R/cap/$lbl.digest.before.md" "$R/cap/$lbl.digest.after.md" >"$R/cap/$lbl.digest.diff" 2>&1
    { printf 'mutation\tthe archive handover and pending counts are set to %s and %s in BOTH meta and digest.md, so the tree stays internally consistent and the frozen validator still accepts it\n' "$h" "$p"
      printf 'mode.preserved\tyes (temp + chmod-to-original-mode + rename)\n'
    } >"$R/cap/$lbl.mutation.txt"
    return 0
}

# Read a child's recorded lineage tuple BY EXACT KEY.
d_child_tuple() { # <session> <out>
    local s="$1" out="$2"
    local m="$R/h/.ae/sessions/$s/meta"
    { if [[ -f "$m" ]]; then
        printf '# the child recorded lineage tuple, BY EXACT KEY, from %s\n' "$m"
        grep -n '^parent_archive_id=\|^parent_archive_handover_count=\|^parent_archive_pending_count=\|^session_id=\|^session_id_origin=' "$m" 2>&1
        printf '\n# od of each lineage line\n'
        grep '^parent_archive_handover_count=\|^parent_archive_pending_count=' "$m" | od -c
        printf '\n# the whole meta, verbatim\n'; cat "$m"
      else printf '(no meta at %s — the child does not exist)\n' "$m"; fi
    } >"$out" 2>&1
    return 0
}
