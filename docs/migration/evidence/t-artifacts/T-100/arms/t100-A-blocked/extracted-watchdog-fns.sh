_watchdog_quiet_hash () 
{ 
    local buf="$1";
    printf '%s' "$buf" | awk '
        function submit_hdr(l) { return l ~ /^[[:space:]]*[›❯][[:space:]]+⟦ae:msg from watchdog⟧[[:space:]]*$/ }
        function indented(l)   { return l ~ /^[[:space:]][[:space:]]/ }
        function raw_nudge(l) {
            return l ~ /^(Session goal: .*\. )?Status check: if you have more work, continue\. Otherwise declare your state so I stop nudging: .*\/state <waiting-user\|blocked\|done> "<reason>"[[:space:]]*$/
        }
        function raw_env(l)  { return l ~ /^⟦ae:msg from watchdog⟧[[:space:]]*$/ }
        function is_echo(l) {
            if (l ~ /^[[:space:]]*└[[:space:]]+Marked [^ ]+ (working|waiting-user|blocked|done)([:.].*)?$/) return 1
            # The CAPTURED status grammar, not "anything mentioning output:". The loose
            # version swallowed ordinary assistant prose that happened to contain
            # "output: Marked …" — deaf in the yield direction, which is the failure
            # this filter must never introduce. Bytes from the capture:
            #   ⏺ SP [HH:MM] SP Done SP U+2014 SP output: SP Marked …
            if (l ~ /^⏺ \[[0-9][0-9]:[0-9][0-9]\] Done — output: Marked [^ ]+ (working|waiting-user|blocked|done)([:.].*)?$/) return 1
            if (l ~ /^Marked [^ ]+ (working|waiting-user|blocked|done)([:.].*)?$/) return 1
            return 0
        }
        {
            # Inside a rendered nudge block: swallow its indented body.
            if (in_block) { if (indented($0)) next; in_block = 0 }
            # A held raw envelope is only dropped when the raw nudge follows it.
            if (held != "") {
                if (raw_nudge($0)) { held = ""; next }
                print held; held = ""
            }
            if (submit_hdr($0)) { in_block = 1; next }   # rendered, both modeled TUIs
            if (raw_env($0))    { held = $0; next }      # unmodeled pane: pair form
            if (raw_nudge($0))  next                     # unmodeled / legacy watchdog
            if (is_echo($0))    next
            print
        }
        END { if (held != "") print held }
    ' | _ae_md5 | cut -d' ' -f1;
    return 0
}
_watchdog_capture_pane () 
{ 
    tmux capture-pane -p -J -S -40 -E - -t "$1" 2> /dev/null
}
