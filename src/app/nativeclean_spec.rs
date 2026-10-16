//! Frozen acceptance cases: nativeclean plan v5 (grouped view session).
//! Independent navigator. Driver installs byte-identically as
//! `src/app/nativeclean_spec.rs`, wired from paneview.rs via
//! `#[cfg(test)] #[path = "nativeclean_spec.rs"] mod nativeclean_spec;`
//! (no app.rs write; sidecompact owns app.rs). Builders below are adopted
//! verbatim; renames need nav sign-off. Pure pins below; e2e contract at the
//! end gates equally. `paneview_spec`: attach test deleted as superseded, its
//! restore test takes the 5-arg real-session form (G5 signed; diff reviewed).
//! Theme ruling: transient view format helper is theme-owned and ships
//! `FORMAT_VERSION` 22; canonical real-session layout unchanged.

use crate::inventory::ServerId;
use crate::meta::Selector;
use crate::{session_launch, tmux};

fn server() -> ServerId {
    ServerId::Selected(Selector::Socket("/tmp/nativeclean-spec.sock".into()))
}

const TOKEN: &str = "4242-1710000000000000000";

/// The return contract does not move: F12 and C-] stay the only return keys,
// the hint names both plus prefix-d, and C-c still reaches the harness.
#[test]
fn return_keys_and_hint_are_unchanged_by_the_clean_view() {
    assert_eq!(tmux::VIEW_RETURN_KEYS, ["F12", "C-]"]);
    for key in tmux::VIEW_RETURN_KEYS {
        assert!(tmux::VIEW_HINT.contains(key));
    }
    assert!(tmux::VIEW_HINT.contains("prefix d"));
    assert!(!tmux::VIEW_RETURN_KEYS.contains(&"C-c"));
}

/// The transient session is ungoverned by ae's grammar: no inventory, fleet,
/// adopter, restore or resume path may claim it.
#[test]
fn view_session_name_is_outside_the_session_grammar() {
    let view = tmux::view_session_name(TOKEN);
    assert_eq!(view, format!("_aeview-{TOKEN}"));
    assert!(!session_launch::name::is_session_name(&view));
    assert!(view.len() <= 128);
}

/// The transient view's bar text is theme-owned: `theme::view_status_format`
/// renders the return hint around the `#{@ae_view_label}` reference the group
/// carries (labels escaped there); tmux.rs only carries the literal. No
/// window list, no real window name, no shell command in the literal.
#[test]
fn view_status_format_is_theme_owned_and_names_no_real_window() {
    let format = crate::theme::view_status_format("F12 or C-] back to ae app");
    assert!(format.contains("F12 or C-] back to ae app"));
    assert!(format.contains("@ae_view_label"));
    assert!(
        !format.contains("window_list"),
        "no window list on the view bar"
    );
    assert!(!format.contains("ae-core"));
    assert!(
        !format.contains("#()"),
        "no shell command in a status literal"
    );
}

#[test]
fn group_creation_is_exact_detached_and_carries_the_theme_line() {
    let view = tmux::view_session_name(TOKEN);
    let line = crate::theme::view_status_format("F12 or C-] back to ae app");
    let args = tmux::view_group_args(&server(), "$7", &view, "demo", "lead", &line);
    let words = args.join(" ");
    assert!(words.contains("new-session"));
    assert!(words.contains("-d"));
    assert!(words.contains(&format!("-s {view}")) || words.contains(&format!("-s={view}")));
    assert!(
        words.contains("-t =$7"),
        "group source is the exact real id"
    );
    assert!(
        words.contains(&line),
        "the theme line rides the group, unmodified"
    );
    assert!(!args.iter().any(|word| word == "-g"));
    assert!(!words.contains("kill-window") && !words.contains("unlink-window"));
}

/// Attach lands on the view session, asserts destroy-unattached only after
/// the attach, keeps token markers, lands guarded or detaches, moves no real
/// window, sizes no other client down through a shared flag.
#[test]
fn attach_runs_on_the_view_session_and_arms_cleanup_after_itself() {
    let view = tmux::view_session_name(TOKEN);
    let args = tmux::view_attach_args(&server(), "$7", &view, "%12", "11-22");
    let words = args.join(" ");
    let attach = words.find("attach-session").expect("interactive attach");
    assert!(words[attach..].contains(&format!("={view}")));
    let armed = words
        .find("destroy-unattached")
        .expect("cleanup armed on this chain");
    assert!(attach < armed, "armed only after the attach it cleans");
    assert!(words.contains("@ae_view_client") && words.contains("@ae_view_token"));
    let run = words.find("run-shell -C").expect("guarded landing");
    let tail = &words[run..];
    let guard = tail
        .find("if-shell")
        .or_else(|| tail.find("if -F"))
        .expect("membership rechecked");
    let select = tail.find("select-pane").expect("seat selected");
    let detach = tail.find("detach-client").expect("failed landing detaches");
    assert!(guard < select && select < detach);
    assert!(
        !words.contains("active-pane"),
        "client pane stays the window pane"
    );
    assert!(
        !args.iter().any(|word| word == "-f"),
        "no per-client size flag outside the measured set"
    );
    assert!(!args.iter().any(|word| word == "-g"));
}

/// Destruction is one exact name, never a prefix: nothing else can die.
#[test]
fn destroy_names_one_exact_view_session() {
    let view = tmux::view_session_name(TOKEN);
    let args = tmux::view_destroy_args(&server(), &view);
    let words = args.join(" ");
    assert!(words.contains("kill-session"));
    assert!(words.contains(&format!("={view}")));
    assert!(!args.iter().any(|word| word == "-g"));
}

/// Stale sweep parses the token clock out of the name: only a detached view
/// session older than the bound dies; a racing fresh one never does.
#[test]
fn stale_sweep_parses_age_and_spares_a_racing_fresh_session() {
    let now = 1_710_000_060_000_000_000u64;
    assert_eq!(
        tmux::view_session_age_nanos(&tmux::view_session_name(TOKEN)),
        Some(1_710_000_000_000_000_000u64)
    );
    assert_eq!(tmux::view_session_age_nanos("$7"), None);
    assert_eq!(tmux::view_session_age_nanos("_aeview-live"), None);
    assert!(tmux::view_session_expired(
        &tmux::view_session_name(TOKEN),
        now,
        core::time::Duration::from_mins(1)
    ));
    assert!(!tmux::view_session_expired(
        &tmux::view_session_name("4242-1710000059000000000"),
        now,
        core::time::Duration::from_mins(1)
    ));
}

/// Release stays conditional on the owning token, on both sessions.
#[test]
fn marker_release_stays_conditional_on_the_owning_token() {
    for command in [
        tmux::view_release_command("$7", "owner-token"),
        tmux::view_stale_clear_command("$7", "owner-token"),
    ] {
        assert!(command.contains("#{==:#{@ae_view_token},owner-token}"));
        let gate = command.find("#{==:").expect("ownership predicate");
        let unset = command
            .find("set -u")
            .or_else(|| command.find("set-option -u"))
            .expect("marker cleanup");
        assert!(gate < unset);
    }
}

/// Theme ruling: the transient view owns a minimal pure format helper in
/// theme.rs and ships `FORMAT_VERSION` 22 with it; canonical real-session
/// layout unchanged. This pins the ruled value; any further move is deliberate.
#[test]
fn transient_view_format_ships_format_version_22() {
    assert_eq!(crate::theme::FORMAT_VERSION, "22");
}

/// Inside /open moves the proven viewer client into the group: one ordered
/// chain — switch, arm cleanup, markers, captured return, guarded landing. A
/// failed landing runs the return command, NEVER detach-client: inside,
/// detach drops the human terminal.
#[test]
fn inside_switch_orders_handoff_and_never_detaches() {
    let view = tmux::view_session_name(TOKEN);
    let ret = tmux::view_return_command("$1", "@0", "%0", "$7", "11-22")
        .expect("proven ids build a return");
    let args = tmux::view_switch_args(&server(), "$7", &view, "%12", "/dev/ttys0", "11-22", &ret);
    let words = args.join(" ");
    let switch = words.find("switch-client").expect("viewer moves");
    assert!(words[switch..].contains(&format!("={view}")));
    let armed = words.find("destroy-unattached").expect("cleanup armed");
    assert!(switch < armed);
    assert!(words.contains("@ae_view_client") && words.contains("@ae_view_token"));
    assert!(words.contains("@ae_view_return"));
    assert!(
        words.contains("set-option -t $7: @ae_view_return"),
        "F1: the real session carries the captured return"
    );
    let run = words.find("run-shell -C").expect("guarded landing");
    assert!(armed < run);
    let tail = &words[run..];
    let guard = tail
        .find("if-shell")
        .or_else(|| tail.find("if -F"))
        .expect("membership rechecked");
    let select = tail.find("select-pane").expect("seat selected");
    assert!(guard < select);
    assert!(
        !tail.contains("detach-client"),
        "a failed inside landing returns, never detaches"
    );
    assert!(!args.iter().any(|word| word == "-g"));
    assert!(!words.contains("active-pane"));
}

/// The captured return lands the exact app pane iff it is still that pane,
/// else the session with nothing selected; its tail clears the real marker
/// exactly while the token still owns it. Unproven ids build nothing.
#[test]
fn inside_return_lands_captured_or_session_only_and_releases() {
    let ret = tmux::view_return_command("$1", "@0", "%0", "$7", "11-22")
        .expect("proven ids build a return");
    assert!(ret.contains("switch-client") && ret.contains("$1"));
    assert!(ret.contains("@0") && ret.contains("%0"));
    let select = ret.find("select-pane").expect("guarded return select");
    let gate = ret
        .find("if-shell")
        .or_else(|| ret.find("if -F"))
        .expect("still-member guard");
    assert!(gate < select);
    assert!(ret.contains("#{==:#{@ae_view_token},11-22}"));
    let unset = ret.find("set -u").or_else(|| ret.find("set-option -u"));
    assert!(unset.is_some_and(|at| at > gate));
    assert_eq!(
        tmux::view_return_command("hub", "@0", "%0", "$7", "11-22"),
        None
    );
    assert_eq!(
        tmux::view_return_command("$1", "0", "%0", "$7", "11-22"),
        None
    );
    assert_eq!(
        tmux::view_return_command("$1", "@0", "0", "$7", "11-22"),
        None
    );
}

/// The return bind answers the view client from its own marker: return set
/// runs the captured command, unset detaches as outside, anyone else keeps
/// the key. Passthrough stays last.
#[test]
fn return_bind_runs_captured_inside_and_detaches_outside() {
    for key in tmux::VIEW_RETURN_KEYS {
        let words = tmux::view_return_bind_args(&server(), key).join(" ");
        let gate = words.find("@ae_view_client").expect("client gate");
        let branch = words.find("@ae_view_return").expect("inside branch");
        let detach = words.find("detach-client").expect("outside return");
        let passthrough = words
            .find(&format!("send-keys {key}"))
            .expect("other clients keep their key");
        assert!(gate < branch && branch < detach && detach < passthrough);
    }
}

/// The reserved namespace is owned once: every wrapper and every interpreter
/// consults `is_view_session`, so no second classifier can drift.
#[test]
fn view_namespace_is_owned_once_and_bars_the_alias() {
    assert!(tmux::is_view_session("_aeview-9"));
    assert!(tmux::is_view_session(&tmux::view_session_name(TOKEN)));
    assert!(!tmux::is_view_session("real"));
    assert!(!tmux::is_view_session("old name"));
    assert!(!tmux::is_view_session(""));
}

/// Pinned rereads spell `$sid:.%pane` exactly, or nothing is built.
#[test]
fn pinned_targets_spell_session_dot_pane_exactly_or_nothing() {
    assert_eq!(
        tmux::pinned_pane_target("$7", "%12"),
        Some("$7:.%12".to_owned())
    );
    assert_eq!(tmux::pinned_pane_target("real", "%12"), None);
    assert_eq!(tmux::pinned_pane_target("$7", "12"), None);
    assert_eq!(tmux::pinned_pane_target("$7", ""), None);
    assert_eq!(tmux::pinned_pane_target("", "%12"), None);
}

/// Pinning the target proves nothing: tmux answers rc0 with another session
/// for a non-member pane. The membership read names the same session, and
/// the pinned result must equal the mapping, or the wrapper reads None.
#[test]
fn membership_names_same_session_before_any_pinned_read() {
    let args = tmux::view_membership_args(&server(), "$7");
    let words = args.join(" ");
    assert!(words.contains("list-panes"));
    assert!(
        words.contains("=$7"),
        "membership is read off the exact real id"
    );
    assert!(
        words.contains("-s"),
        "membership spans all windows, not the current one"
    );
    assert!(
        words.contains("#{pane_id}"),
        "pane rows are compared exactly"
    );
    assert!(!args.iter().any(|word| word == "-g"));
}

/// The alias mapping is exactly one `$digits` line; anything else is None.
#[test]
fn view_of_reads_exactly_one_session_id_line() {
    assert_eq!(tmux::view_of_value("$12\n"), Some("$12".to_owned()));
    assert_eq!(tmux::view_of_value(""), None);
    assert_eq!(tmux::view_of_value("real\n"), None);
    assert_eq!(tmux::view_of_value("$12\n$13\n"), None);
}

/// The alias is stamped with its real session in the creation chain: a chain
/// failure aborts the view and destroys the exact name, never a half-marked
/// alias that later reads resolve.
#[test]
fn group_creation_marks_the_alias_with_its_real_session() {
    let view = tmux::view_session_name(TOKEN);
    let line = crate::theme::view_status_format("F12 or C-] back to ae app");
    let args = tmux::view_group_args(&server(), "$7", &view, "demo", "lead", &line);
    let words = args.join(" ");
    let create = words.find("new-session").expect("creation opens");
    let mark = words.find("@ae_view_of").expect("alias stamped");
    assert!(create < mark, "stamped in the creation chain");
    assert!(words[mark..].contains("$7"), "mapping names the real id");
}

/// r3: the kill door chains nothing in argv. `kill_session_args` stays one
/// exact verb for both spellings; the sweep lives in `transport::stop_views`
/// over list-windows reads, decided purely and killed by exact view id.
#[test]
fn kill_args_stay_a_single_exact_verb() {
    let id = tmux::kill_session_args(&server(), "$7").join(" ");
    assert!(!id.contains("run-shell"), "no sweep in argv");
    assert!(id.contains("kill-session") && id.contains("=$7"));
    let name = tmux::kill_session_args(&server(), "demo").join(" ");
    assert!(!name.contains("run-shell"), "creation path keeps one verb");
    assert!(name.contains("kill-session") && name.contains("=demo"));
}

/// Snapshot reads exactly the candidate session's windows; the holder listing
/// reads every session wide. Both fail closed upstream on read failure.
#[test]
fn holder_reads_are_exact_and_window_wide() {
    let words = tmux::view_window_snapshot_args(&server(), "$7").join(" ");
    assert!(words.contains("list-windows"));
    assert!(words.contains("=$7"));
    assert!(words.contains("#{window_id}"));
    let args = tmux::view_holder_listing_args(&server());
    let words = args.join(" ");
    assert!(words.contains("list-windows"));
    assert!(words.contains("-a"));
    assert!(words.contains("#{window_id}"));
    assert!(words.contains("#{session_id}"));
    assert!(!args.iter().any(|word| word == "-g"));
}

/// The pure holder decision: a view name holding a snapshot window dies, as
/// does a view name mapped to the real id even with its windows unlinked;
/// anything else is left alone. An unknown (empty) snapshot selects nothing
/// and refuses upstream: no kills of any kind, real never killed.
#[test]
fn holder_decision_kills_only_views_of_the_real_session() {
    let w = vec!["@0".to_owned(), "@1".to_owned()];
    let rows = vec![
        (
            "$9".to_owned(),
            "_aeview-1".to_owned(),
            "@0".to_owned(),
            String::new(),
        ),
        (
            "$9".to_owned(),
            "_aeview-9".to_owned(),
            "@1".to_owned(),
            String::new(),
        ),
        (
            "$9".to_owned(),
            "_aeview-7".to_owned(),
            "@9".to_owned(),
            "$7".to_owned(),
        ),
        (
            "$8".to_owned(),
            "_aeview-2".to_owned(),
            "@9".to_owned(),
            String::new(),
        ),
        (
            "$8".to_owned(),
            "_aeview-3".to_owned(),
            "@9".to_owned(),
            "$8".to_owned(),
        ),
        (
            "$3".to_owned(),
            "other".to_owned(),
            "@1".to_owned(),
            String::new(),
        ),
    ];
    assert_eq!(
        tmux::views_holding(&w, "$7", &rows),
        vec!["$9".to_owned()],
        "one kill per holder id; foreign, linked and mapped-elsewhere kept"
    );
    assert!(tmux::views_holding(&[], "$7", &rows).is_empty());
    assert!(tmux::views_holding(&w, "$7", &[]).is_empty());
}

/// Orphan selection needs namespace AND uuid together: a real session row
/// spoofing the uuid is kept, and an empty uuid selects nothing.
#[test]
fn orphan_selection_needs_namespace_and_uuid_together() {
    let rows = vec![
        ("$9".to_owned(), "_aeview-1".to_owned(), "uuid-1".to_owned()),
        ("$8".to_owned(), "_aeview-2".to_owned(), "other".to_owned()),
        ("$7".to_owned(), "real".to_owned(), "uuid-1".to_owned()),
        ("$6".to_owned(), "other".to_owned(), "uuid-1".to_owned()),
    ];
    assert_eq!(
        tmux::orphan_view_ids("uuid-1", &rows),
        vec!["$9".to_owned()],
        "spoofed real sessions and other-uuid views are kept"
    );
    assert!(tmux::orphan_view_ids("", &rows).is_empty());
    assert!(tmux::orphan_view_ids("uuid-1", &[]).is_empty());
}

/// The namespace has one spelling across match pattern and predicate.
#[test]
fn alias_match_uses_the_owned_namespace_spelling() {
    assert_eq!(tmux::VIEW_SESSION_PREFIX, "_aeview-");
    assert_eq!(tmux::VIEW_OF_OPTION, "@ae_view_of");
    assert!(tmux::is_view_session(&format!(
        "{}9",
        tmux::VIEW_SESSION_PREFIX
    )));
}

// The consumer decides, never the door bool alone: kill_verified maps
// Absent+Clear to success and Absent+Held/Unreadable to a refusing
// not-stopped with state preserved. Table pinned at the live consumer
// (e2e 15); the one-line hunk is reviewed, not re-derived here.

/// Original identity survives grouping: the fleet-pane interpreter drops rows
/// in the reserved `_aeview-` namespace, however tmux orders the duplicate
/// grouped rows, so `session_of` (first match wins) can only resolve to a
/// real session. The filter is namespace-exact, NOT a grammar gate: retained
/// pre-grammar sessions stay usable per the migration rule. Live tmux-order
/// proof rides the e2e below.
#[test]
fn pane_rows_from_synthetic_sessions_never_reach_identity() {
    let hostile = "real | %1 | lead\n_aeview-9 | %1 | \nold name | %2 | \n";
    let rows = tmux::interpret_fleet_panes(true, hostile).expect("parsed rows");
    assert!(
        rows.iter()
            .any(|row| row.session == "real" && row.pane == "%1")
    );
    assert!(
        rows.iter()
            .any(|row| row.session == "old name" && row.pane == "%2"),
        "retained pre-grammar sessions keep their rows: {rows:?}"
    );
    assert!(
        rows.iter().all(|row| !row.session.starts_with("_aeview-")),
        "reserved view aliases never reach identity: {rows:?}"
    );
    assert_eq!(tmux::interpret_fleet_panes(false, hostile), None);
}

// ---------------------------------------------------------------------------
// Live-proof contract (private socket, ordinary client + view client;
// before/after PNG + text each; the pane-id row read is list-clients -F
// CLIENT_FORMAT, the upstream pane mapping the digest leaves to this proof):
//
// 1. chrome-gone: view client, 1 line, no window list / ae-core; names
//    session + seat + "F12 or C-] back to ae app". Inside-tmux mirror
//    unchanged and never claimed clean.
// 2. other-client-untouched: second client's status bytes + current window
//    byte-identical before, during, after (state the exact read).
// 3. typing-reaches-pane: echo proof; view client's row pane_id == %target.
// 4. return: F12 and C-] in view detach; same keys from other client reach
//    the pane; view session gone after return, detach, child failure.
// 5. guarded landing: pane not member -> detach, nothing selected.
// 6. real-session-death (watcher A): kill real -> view session killed within
//    the stated bound via the exact destroy args; view client detached, never
//    switched elsewhere; no harness orphan past the bound.
// 7. handoff-exclusion: closed by construction (three-field reader drops
//    rankless rows, fleet_listing_row drops non-grammar names); live proof
//    kills a session with a view open and shows no client ever lands in
//    _aeview-*. No lifecycle change needed.
// 8. stale sweep: crashed view leaves a detached _aeview-*; next view setup
//    removes it iff older than 60 s; a fresh racing one survives. A failed
//    make_view chain destroys the exact view it may have part-created
//    (Owned-equivalent inside and out); injected chain failure proves
//    refusal plus a clean census.
// 9. resize step + draft/selection survival on return (existing path).
// 10. endurance: 100 open/return cycles (outside), then no `_aeview-*` in
//     list-sessions, both view markers unset, real session intact.
// 11. invisibility sweep with a view open: ae list, list --all, picker rows,
//     fleet strip text, adopter writes, inventory dirs — none names _aeview-*
//     (picker rank+grammar gates and fleet_rows rank gate by construction);
//     prove @ae_attn_rank absent on the view session: grammar alone does not
//     protect fleet_rows, rank absence does.
// 12. inside /open: existing entry proof unchanged; switch into the group on
//     the proven invoking client (mover is the only chooser); clean bar; F12
//     runs the captured return (exact app pane iff still-member, else session
//     with nothing selected, marker released); failed landing runs return,
//     never detach; target-gone lands session-only; Here tie refuses with the
//     tie notice and zero moves; marker Busy uses exact view-name ownership
//     (client session equals view_session_name of the marker token), so a
//     client in an unrelated view reads Stale and reopen proceeds (L5);
//     click and v stay read-only mirror; console/chat /open diff-empty;
//     App::open hunk only after sidecompact handoff, landed after rebase in
//     this branch.
// 13. crash matrix, victim and signal named per case; Drop/watch threads die
//     with the process, so no case claims in-process cleanup after death.
//     App-only case (kill -9 exactly the ae app PID, attach child untouched)
//     is MEASURED on slice: orphan survives reparented and attached, shell
//     owns the foreground so F12/prefix-d do not reach it, slice markers stay
//     stale; base marker/F12 state unmeasured, no parity claimed there.
//     Recipe is kill the orphan client pid or close the terminal, which fires
//     the armed chain; this covers the pre-stop orphan only. Once ae stop
//     runs, item 15 governs: a confirmed stop with surviving harnesses cannot
//     be recorded, and no operator recipe bounds such an outcome. Server
//     readv stall seen once on slice; base freeze unobserved, O_NONBLOCK
//     account a hypothesis.
//     Attach-client kill and prefix-d detach must destroy via the chain;
//     real-session kill goes through watcher A within its bound.
// 14. alias-identity sweep during a view: send/ask/review identity, monitor
//     own-session, session_of, writer-lease holder, migration identity all
//     resolve the real session, including a valid seat in a non-current
//     window (membership spans -s, positive case); malformed view_of
//     (unset/empty/multi-line/non-$digits), non-member pane, stale real id,
//     and a pinned result that names a view or mismatches the mapping all
//     read None/refused, never the alias; pane-keyed recognition (deliver
//     deferral, viewing input) unchanged and still pane-joined.
// 15. stop safeguard (r3, consumer decides): pre-sweep kills views holding
//     snapshot windows or mapped to the real id before the real kill; any
//     read/kill failure refuses with the real never killed, so the existing
//     name verify reports present. An unreadable snapshot selects nothing and
//     kills nothing. Post-sweep up to 3 rounds; the one-line kill_verified
//     hunk maps Absent+Clear to success and Absent+Held/Unreadable to a
//     refusing not-stopped with state preserved. Live, all executed with exit
//     code, message and record asserted (never prose-only): orphan-then-stop
//     destroys the alias, harness gone, client exits; a foreign alias
//     grouping a DIFFERENT live session is kept; no-alias case unchanged;
//     Held-refusal retry executed and asserted per absent path: singular must
//     exit 1 with stop-result record wording and stderr message asserted,
//     stop-all must not count satisfied exit 0, end must not archive+delete,
//     while an alias-held harness lives. Narrow absent-path guards in stop-all
//     and end are owner-authorized; pair judges the minimal fix from trace.
//     Stop-all and end guards asserted with exit, message and record;
//     singular ordinary/confirmed assert unchanged exit-1 with record wording
//     and message asserted, no guard required: singular neither counts
//     satisfied nor removes state;
//     alias carries @ae_view_uuid == real uuid (live option read),
//     non-canonical uuid makes no view, spoofed real rows are kept. Tampered same-user mapping stays a named residual.
//     Listing failure with the server proven absent by existing strict absence
//     evidence means proven-empty and proceeds: last-session stop and
//     dead-server stop succeed; malformed or ambiguous stays refusal. No new
//     probe mechanism.
