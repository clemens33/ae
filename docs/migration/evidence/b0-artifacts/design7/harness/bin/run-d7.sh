#!/opt/homebrew/bin/bash
# B0 Design 7 (SC-511c) arm driver. Captures only.
set -uo pipefail
SB=/tmp/aeb0
D7="$SB/d7"
R="$D7/bin/runners.sh"
S=b0d7
ARMS="$D7/arms"

# key -> family list (from b0-design.md's discriminating-consumer column, mapped
# onto the design's product-layer runner table)
declare -A FAMS=(
  [ts]="list_next archive watchdog"
  [actor]="list_next requests_state watchdog archive"
  [action]="list_next archive requests_state telegram"
  [target]="list_next requests_state archive"
  [ref]="requests_state archive list_next watchdog"
  [summary]="stop list_next archive"
  [actor_slot]="requests_state archive compact"
  [actor_session]="requests_state archive compact"
  [target_slot]="requests_state archive"
  [target_session]="requests_state archive"
  [body_file]="archive compact"
)
ALL_FAMS="list_next watchdog requests_state archive events_tail telegram aewatch compact stop"
CHURN_FAMS="requests_state archive compact"

_mutate() { # <clonedir-root> <op> <key> [newkey|value] [where]
    local root="$1" op="$2" key="$3" x="${4:-}" where="${5:-}"
    local sd="$root/home/.ae/sessions/$S"
    cp -a "$sd/events.jsonl" "$root/events.control.jsonl"
    local -a a=(--in "$root/events.control.jsonl" --out "$root/events.mutated.jsonl"
                --report "$root/mutation.report.txt" --op "$op" --key "$key")
    case "$op" in
        rename) a+=(--newkey "$x") ;;
        insert) a+=(--value "$x" --where "$where") ;;
    esac
    /usr/bin/python3 "$D7/bin/mutate.py" "${a[@]}" > "$root/mutation.selfcheck.txt" 2>&1
    local rc=$?
    printf 'mutate.py rc=%s (0 = every mutated line passed its validity self-check; 3 = harness defect, arm INVALID)\n' "$rc" >> "$root/mutation.selfcheck.txt"
    if (( rc != 0 )); then printf 'ARM INVALID: mutation validity self-check failed\n' > "$root/ARM-INVALID.txt"; return 1; fi
    cp -a "$root/events.mutated.jsonl" "$sd/events.jsonl"
    diff -u "$root/events.control.jsonl" "$root/events.mutated.jsonl" > "$root/mutation.bytediff" || true
    { printf 'control sha256=%s size=%s\n' "$(shasum -a 256 "$root/events.control.jsonl"|awk '{print $1}')" "$(stat -f '%z' "$root/events.control.jsonl")"
      printf 'mutated sha256=%s size=%s\n' "$(shasum -a 256 "$root/events.mutated.jsonl"|awk '{print $1}')" "$(stat -f '%z' "$root/events.mutated.jsonl")"
    } > "$root/mutation.bytes.txt"
    return 0
}

run_family_arm() { # <armid> <family> <op> <key> [x] [where]
    local armid="$1" fam="$2" op="$3" key="$4" x="${5:-}" where="${6:-}"
    local root="$ARMS/$armid/$fam"
    local srv; srv="$(printf 'd7%s' "$(printf '%s' "${armid}${fam}" | shasum -a 256 | cut -c1-10)")"
    "$R" _prep "$root"
    if [[ "$op" != "none" ]]; then _mutate "$root" "$op" "$key" "$x" "$where" || { "$R" _cleanup "$root" "$srv"; return 0; }; fi
    "$R" "fam_$fam" "$root" "$srv"
    "$R" _post "$root"
    "$R" _cleanup "$root" "$srv"
    rm -rf "$root/home" "$root/tmuxtmp"
    printf '%s\n' "$srv" > "$root/tmux-server-name.txt"
}

run_arm() { # <armid> <op> <key> [x] [where] [famlist]
    local armid="$1" op="$2" key="$3" x="${4:-}" where="${5:-}" fams="${6:-}"
    [[ -n "$fams" ]] || fams="${FAMS[$key]:-$ALL_FAMS}"
    mkdir -p "$ARMS/$armid"
    { printf 'arm=%s\nop=%s\nkey=%s\nextra=%s\nwhere=%s\nfamilies=%s\nstarted=%s\n' \
        "$armid" "$op" "$key" "$x" "$where" "$fams" "$(/bin/date -u +%Y-%m-%dT%H:%M:%SZ)"; } > "$ARMS/$armid/ARM.txt"
    local f
    for f in $fams; do
        echo "  [$armid] $f"
        run_family_arm "$armid" "$f" "$op" "$key" "$x" "$where"
    done
    printf 'finished=%s\n' "$(/bin/date -u +%Y-%m-%dT%H:%M:%SZ)" >> "$ARMS/$armid/ARM.txt"
}

run_churn_arm() { # pre/post states: two clones, tmux @ae_agent churn on the post clone
    local armid=churn f
    mkdir -p "$ARMS/$armid"
    { printf 'arm=%s\nconstruction=tmux set-option -p @ae_agent on BOTH panes; @ae_slot and the session untouched\n' "$armid"
      printf 'shape: tests/integration@72c7293:1268-1285\nfamilies=%s\nstates=pre,post\nstarted=%s\n' "$CHURN_FAMS" "$(/bin/date -u +%Y-%m-%dT%H:%M:%SZ)"; } > "$ARMS/$armid/ARM.txt"
    for state in pre post; do
        for f in $CHURN_FAMS; do
            echo "  [churn/$state] $f"
            local root="$ARMS/$armid/$state.$f"
            local srv; srv="$(printf 'd7c%s' "$(printf '%s' "${state}${f}" | shasum -a 256 | cut -c1-9)")"
            "$R" _prep "$root"
            if [[ "$state" == post ]]; then
                "$R" _resume "$root" "$srv" || { "$R" _cleanup "$root" "$srv"; continue; }
                local main help
                main="$("$R" _panes "$root" "$srv" dummy:dummy)"; help="$("$R" _panes "$root" "$srv" dummy2:helper)"
                { printf 'churn applied BEFORE the consumers ran:\n'
                  printf '  tmux set-option -p -t %s @ae_agent churned:lead2\n' "$main"
                  printf '  tmux set-option -p -t %s @ae_agent churned:worker2\n' "$help"
                  printf '  @ae_slot and the session name are NOT touched\n'; } > "$root/churn.controller.txt"
                TMUX_TMPDIR="$root/tmuxtmp" command tmux -L "$srv" set-option -p -t "$main" @ae_agent churned:lead2
                TMUX_TMPDIR="$root/tmuxtmp" command tmux -L "$srv" set-option -p -t "$help" @ae_agent churned:worker2
                TMUX_TMPDIR="$root/tmuxtmp" command tmux -L "$srv" list-panes -a -F '#{pane_id} #{@ae_agent} #{@ae_slot} #{session_name}' > "$root/churn.panes.after.txt" 2>&1
            fi
            "$R" "fam_$f" "$root" "$srv"
            "$R" _post "$root"
            "$R" _cleanup "$root" "$srv"
            rm -rf "$root/home" "$root/tmuxtmp"
        done
    done
    printf 'finished=%s\n' "$(/bin/date -u +%Y-%m-%dT%H:%M:%SZ)" >> "$ARMS/$armid/ARM.txt"
}

run_all() {
    mkdir -p "$ARMS"
    echo "== CONTROL (unmutated clone per family) =="
    run_arm control none "" "" "" "$ALL_FAMS"
    local k
    for k in ts actor action target ref summary actor_slot actor_session target_slot target_session; do
        echo "== $k / remove =="; run_arm "key-${k}-remove" remove "$k"
        echo "== $k / rename =="; run_arm "key-${k}-rename" rename "$k" "${k}_x"
    done
    echo "== body_file (EMPIRICAL EXTENSION lane) =="
    run_arm "ext-body_file-remove" remove body_file
    run_arm "ext-body_file-rename" rename body_file body_file_x
    local w
    for w in first middle last; do
        echo "== additive / $w =="
        run_arm "additive-$w" insert ae_unknown_optional_key "b0-additive-probe-value" "$w" "$ALL_FAMS"
    done
    echo "== churn =="
    run_churn_arm
    echo "ALL D7 ARMS DONE $(/bin/date -u +%Y-%m-%dT%H:%M:%SZ)"
}

"$@"
