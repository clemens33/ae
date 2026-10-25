//! Frozen acceptance: Settings > Quota live seats (#242 s2). Oracles: plan-r2 519ef605,
//! lead S1-S5 03067308, nav ce61a2e8, GRANT bd5bf3db; `enrich_list_json` stays carried.

use std::path::PathBuf;

use crate::app::model::SettingsTab;
use crate::app::settings::SettingsBodies;
use crate::digest::{AgentEntry, SessionEntry, Status};
use crate::harness_state::HarnessState;
use crate::meta::{Meta, RosterEntry};
use crate::quota::{
    LiveGroup, LiveSeatFact, LiveSeats, RecordedIdentity, live_seat_facts, project_live_seats, view,
};
use crate::theme::Look;
use crate::time::Timestamp;
use crate::tmux::WatchPane;
use crate::tool::ToolKind;

use super::{Paint, settings_body};

const SECTION: &str = "Live seats by account";
const UNKNOWN: &str = "Account unknown";
const HOME_GAP: &str = "home not recorded";
const NOW: i64 = 2_000_000;
const LEAD: &str = "seat.main=lead\nprofile.main=cl\nagent_bin.main=claude\n";
const FRESH: &str = "v2;2000000;60;lead:cl:working:%3:claude:Spec:xhigh:";
const READS: &str = "observe_scopes fleet_rollouts view_held view_stored transport:: Command fs:: File:: observe_session_option SocketPaths";

fn overlay(seats: LiveSeats, kept: Option<&str>, width: u16) -> Vec<String> {
    mixed_body(SettingsTab::Accounts, seats, kept, width)
}

/// The real `settings_body` for one tab of the same fixture: the routing pins
/// render each tab explicitly, never by concatenating bodies.
fn mixed_body(tab: SettingsTab, seats: LiveSeats, kept: Option<&str>, width: u16) -> Vec<String> {
    let window = view::Window {
        name: "Weekly".into(),
        raw: Some("31%".into()),
        counts: None,
        fill: Some(2),
        why: None,
        level: view::Level::Headroom,
        stale: false,
        resets: Some("2d".into()),
        seen: Some("1m".into()),
        burn: None,
    };
    let bodies = SettingsBodies {
        quota: Some(vec![view::Scope {
            title: "claude · cc".into(),
            profiles: "fablex".into(),
            account: None,
            only: None,
            windows: vec![window],
        }]),
        quota_kept: kept.map(str::to_owned),
        live_seats: Some(seats),
        ..SettingsBodies::default()
    };
    let paint = Paint::of(Some(&Look::read("on", "darcula", "on", "on")));
    settings_body(tab, &bodies, paint, width, true)
        .into_iter()
        .map(|(text, _)| text)
        .collect()
}

fn id(tool: ToolKind, source: &str) -> RecordedIdentity {
    RecordedIdentity {
        tool,
        source: PathBuf::from(source),
    }
}

fn fact(session: &str, seat: &str, identity: Option<RecordedIdentity>) -> LiveSeatFact {
    LiveSeatFact {
        session: session.to_owned(),
        seat: seat.to_owned(),
        profile: "fablex".to_owned(),
        tool: ToolKind::Claude,
        identity,
        model: Some("Opus 5.5".to_owned()),
        effort: Some("xhigh".to_owned()),
        activity: HarnessState::Busy,
        declared: Some("done".to_owned()),
        liveness: Some(true),
    }
}

fn seats_in<'a>(groups: impl IntoIterator<Item = &'a LiveGroup>) -> Vec<&'a str> {
    let rows = groups.into_iter().flat_map(|group| &group.rows);
    rows.map(|row| row.session_seat.as_str()).collect()
}

fn roster(meta: &str) -> Vec<RosterEntry> {
    Meta::parse(meta).roster().to_vec()
}

fn listed(status: Status, alive: Option<bool>) -> SessionEntry {
    let mut entry = SessionEntry::new("spec", status);
    entry.agents = vec![AgentEntry {
        reference: "lead".into(),
        alias: "cl".into(),
        name: "lead".into(),
        alive,
        observed: HarnessState::Busy,
        state: Some("done".into()),
        ..AgentEntry::default()
    }];
    entry
}

fn pane(id: &str) -> WatchPane {
    WatchPane {
        pane_id: id.to_owned(),
        slot: Some("main".to_owned()),
        agent: Some("cl:lead".to_owned()),
        current_command: "claude".to_owned(),
        pane_pid: Some(1),
        observed: "busy".to_owned(),
    }
}

/// The adapter over one running session with seat `lead` alive.
fn adapt(meta: &str, fact: &str, panes: &[WatchPane], same_server: bool) -> Vec<LiveSeatFact> {
    let entry = listed(Status::Running, Some(true));
    let (roster, now) = (roster(meta), Timestamp::from_epoch(NOW));
    live_seat_facts(&entry, &roster, Some(fact), Some(panes), same_server, now)
}

type Cells = (Option<String>, Option<String>, Option<(String, String)>);

/// What `ae list --json` decides for seat `lead` on the same inputs: the oracle.
fn listed_json(meta: &str, fact: &str, panes: &[WatchPane]) -> Cells {
    let mut entry = listed(Status::Running, Some(true));
    let (roster, now) = (roster(meta), Timestamp::from_epoch(NOW));
    crate::session::enrich_list_json(&mut entry, &roster, Some(fact), Some(panes), now);
    let lead = entry.agents.remove(0);
    let account = lead.account.map(|a| (a.tool, a.source));
    (lead.runtime.model, lead.runtime.effort, account)
}

/// The adapter's cells for seat `lead`, spelled as `listed_json` spells them.
fn cells(facts: &[LiveSeatFact]) -> Option<Cells> {
    let lead = facts.iter().find(|fact| fact.seat == "lead")?;
    let account = lead.identity.as_ref().map(|key| {
        let source = key.source.to_str().map(str::to_owned).unwrap_or_default();
        (key.tool.as_str().to_owned(), source)
    });
    Some((lead.model.clone(), lead.effort.clone(), account))
}

fn runtime(facts: &[LiveSeatFact]) -> (Option<&str>, Option<&str>) {
    let lead = facts.iter().find(|fact| fact.seat == "lead");
    let lead = lead.unwrap_or_else(|| panic!("seat lead missing: {facts:?}"));
    (lead.model.as_deref(), lead.effort.as_deref())
}

/// Wrong: activity from the picker verdict (`working`), declared state or
/// profile dropped, runtime read without the one proof.
#[test]
fn a_proven_seat_carries_runtime_and_keeps_both_state_axes() {
    let facts = adapt(LEAD, FRESH, &[pane("%3")], true);
    assert_eq!(runtime(&facts), (Some("Spec"), Some("xhigh")), "{facts:?}");
    assert_eq!(cells(&facts), Some(listed_json(LEAD, FRESH, &[pane("%3")])));
    let lead = &facts[0];
    assert_eq!([&lead.session, &lead.profile], ["spec", "cl"], "{lead:?}");
    assert_eq!(lead.activity, HarnessState::Busy, "observed, not verdict");
    assert_eq!(lead.declared.as_deref(), Some("done"));
    assert_eq!((lead.tool, lead.liveness), (ToolKind::Claude, Some(true)));
}

/// Wrong: name-only or stale trust of the local picker fact; a second copy of
/// the proof that disagrees with `ae list --json` on the same inputs.
#[test]
fn runtime_fails_closed_without_a_fresh_unique_same_server_proof() {
    let one = [pane("%3")];
    let twice = [pane("%3"), pane("%4")];
    let other = [pane("%4")];
    let stale = FRESH.replace("2000000", "1000000");
    let other_profile = FRESH.replace(":cl:", ":old:");
    let v1 = "v1;2000000;60;lead:cl:working:%3";
    let cases: [(&str, &str, &[WatchPane], bool); 7] = [
        ("other server", FRESH, &one, false),
        ("stale fact", &stale, &one, true),
        ("profile mismatch", &other_profile, &one, true),
        ("duplicate slot pane", FRESH, &twice, true),
        ("wrong pane", FRESH, &other, true),
        ("v1 fact", v1, &one, true),
        ("malformed fact", "v2;garbage", &one, true),
    ];
    for (why, fact, panes, same_server) in cases {
        let facts = adapt(LEAD, fact, panes, same_server);
        assert_eq!(runtime(&facts), (None, None), "{why}: {facts:?}");
        assert_eq!(facts[0].declared.as_deref(), Some("done"), "{why}");
        if same_server {
            let oracle = listed_json(LEAD, fact, panes);
            assert_eq!(cells(&facts), Some(oracle), "{why}: list JSON disagrees");
        }
    }
}

/// Wrong: the UI joins by profile/tool/home lexically; the other-server gate
/// also drops the durable account; agy/Grok/Muse get a same-tool guess.
#[test]
fn the_account_is_the_one_recorded_join_and_survives_another_server() {
    // Under the lane's own TMPDIR, which the lane removes.
    let dir = std::env::temp_dir().join(format!("ae-sjn-{}", std::process::id()));
    let home = dir.join("home");
    std::fs::create_dir_all(&home).unwrap_or_else(|why| panic!("{why}"));
    let meta = format!("{LEAD}config_home.main={}\n", home.display());
    let want = crate::quota::recorded_identity(&roster(&meta)[0]);
    assert!(want.is_some(), "fixture: a recorded claude home joins");
    let facts = adapt(&meta, FRESH, &[pane("%3")], false);
    let got = facts.first().map(|fact| fact.identity.clone());
    assert_eq!(got, Some(want), "durable account kept: {facts:?}");
    let same = adapt(&meta, FRESH, &[pane("%3")], true);
    assert_eq!(cells(&same), Some(listed_json(&meta, FRESH, &[pane("%3")])));
    for binary in ["agy", "grok", "muse"] {
        let meta = meta.replace("agent_bin.main=claude", &format!("agent_bin.main={binary}"));
        let facts = adapt(&meta, FRESH, &[pane("%3")], true);
        let got = facts.first().map(|fact| fact.identity.clone());
        assert_eq!(got, Some(None), "{binary}: {facts:?}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Wrong: stopped or dead seats listed; unknown liveness read as live.
#[test]
fn liveness_is_running_and_alive_only_with_unknown_kept_apart() {
    let cases = [
        (Status::Running, Some(true), Some(true)),
        (Status::Running, None, None),
        (Status::Running, Some(false), Some(false)),
        (Status::Stopped, None, Some(false)),
        (Status::Stopped, Some(true), Some(false)),
        (Status::Unknown, None, None),
    ];
    for (status, alive, want) in cases {
        let entry = listed(status, alive);
        let now = Timestamp::from_epoch(NOW);
        let facts = live_seat_facts(&entry, &roster(LEAD), None, None, true, now);
        let got: Vec<_> = facts.iter().map(|fact| fact.liveness).collect();
        assert_eq!(got, [want], "{status:?} {alive:?}: {facts:?}");
    }
}

/// Superseded by human b717ce77 (dedicated Accounts tab): quota windows live on
/// Quota without a seat section, the seat snapshot lives on Accounts without
/// quota windows. Replaces #242 s2 `live_seats_section_follows_existing_quota_windows`
/// with the per-tab separation pins; receipt names both tests.
#[test]
fn quota_and_accounts_tabs_do_not_mix_bodies() {
    let quota = mixed_body(
        SettingsTab::Quota,
        project_live_seats(&[], &[], false),
        None,
        80,
    );
    assert!(
        quota.iter().any(|row| row.contains("Weekly"))
            && quota
                .iter()
                .all(|row| !row.contains(SECTION) && !row.contains(":lead")),
        "{quota:?}"
    );
    let key = id(ToolKind::Claude, "/acct/.claude.json");
    let accounts = mixed_body(
        SettingsTab::Accounts,
        project_live_seats(&[fact("s", "lead", Some(key.clone()))], &[key], false),
        None,
        80,
    );
    assert!(
        accounts.iter().any(|row| row.contains("s:lead"))
            && accounts.iter().all(|row| !row.contains("Weekly")),
        "{accounts:?}"
    );
}

/// Wrong: None keys merged into a configured scope of the same tool.
#[test]
fn none_identity_never_joins_a_configured_scope_of_the_same_tool() {
    let key = id(ToolKind::Claude, "/acct/a/.claude.json");
    let facts = [
        fact("s", "lead", Some(key.clone())),
        fact("s", "ghost", None),
    ];
    let seats = project_live_seats(&facts, &[key], false);
    let typed = seats.groups.iter().find(|group| group.key.is_some());
    let unknown = seats.groups.iter().find(|group| group.key.is_none());
    assert_eq!(typed.map(|g| seats_in([g])), Some(vec!["s:lead"]));
    assert_eq!(unknown.map(|g| seats_in([g])), Some(vec!["s:ghost"]));
    assert!(unknown.is_some_and(|g| g.heading.contains(UNKNOWN)));
}

/// Wrong: join on tool/label/cache; reuse `home not recorded` for claude/codex.
#[test]
fn agy_grok_muse_render_account_unknown_with_home_not_recorded() {
    for tool in [ToolKind::Agy, ToolKind::Grok, ToolKind::Muse] {
        let seat = LiveSeatFact {
            tool,
            ..fact("r", "lead", None)
        };
        let seats = project_live_seats(&[seat], &[id(tool, "/configured/same-tool")], false);
        assert_eq!(seats.groups.len(), 1, "{tool:?} joined a scope: {seats:?}");
        let group = &seats.groups[0];
        assert!(group.key.is_none() && group.heading.contains(UNKNOWN));
        assert_eq!(group.gap.as_deref(), Some(HOME_GAP), "{tool:?}");
        assert_eq!(seats_in(&seats.groups), ["r:lead"], "{tool:?}");
        let text = overlay(seats, None, 80).join("\n");
        assert!(text.contains(UNKNOWN), "{text}");
        assert!(text.contains(HOME_GAP), "{text}");
    }
    for tool in [ToolKind::Claude, ToolKind::Codex] {
        let seat = LiveSeatFact {
            tool,
            ..fact("s", "lead", None)
        };
        let seats = project_live_seats(&[seat], &[], false);
        let group = seats.groups.iter().find(|group| group.key.is_none());
        assert!(group.is_some(), "{tool:?} {UNKNOWN}: {seats:?}");
        let gap = group.and_then(|g| g.gap.as_deref());
        assert_ne!(gap, Some(HOME_GAP), "{tool:?}");
        let text = overlay(seats, None, 80).join("\n");
        assert!(text.contains(UNKNOWN), "{text}");
        assert!(!text.contains(HOME_GAP), "{text}");
    }
}

/// Wrong: group by a clipped label (`settings_cell` / `view::Scope.title`, 36),
/// merge into the first configured group, or ignore configured key order.
#[test]
fn equal_clipped_labels_with_different_keys_stay_separate() {
    let (pre, post) = ("a".repeat(20), "b".repeat(20));
    let key = |mid: &str| {
        id(
            ToolKind::Claude,
            &format!("/{pre}/{mid}/{post}/.claude.json"),
        )
    };
    let (a, b) = (key("one"), key("two"));
    let clip = |key: &RecordedIdentity| {
        crate::event_text::display_cell(&key.source.display().to_string(), 36)
    };
    assert_eq!(clip(&a), clip(&b), "fixture: clipped labels collide");
    let facts = [
        fact("s2", "lead", Some(b.clone())),
        fact("s1", "lead", Some(a.clone())),
    ];
    let seats = project_live_seats(&facts, &[a.clone(), b.clone()], false);
    let got: Vec<_> = (seats.groups.iter())
        .map(|g| (g.key.clone(), seats_in([g])))
        .collect();
    let want = [(Some(a), vec!["s1:lead"]), (Some(b), vec!["s2:lead"])];
    assert_eq!(got, want, "each seat in its own key, configured order");
}

/// Wrong: unproved runtime invented; activity replaced by the declaration.
#[test]
fn rows_keep_observed_and_declared_apart_and_spell_unproved_runtime_as_question() {
    let key = id(ToolKind::Claude, "/acct/.claude.json");
    let unproved = LiveSeatFact {
        model: None,
        effort: None,
        ..fact("s", "lead", Some(key.clone()))
    };
    let seats = project_live_seats(
        &[unproved, fact("s", "two", Some(key.clone()))],
        &[key],
        false,
    );
    let rows = seats.groups.first().map(|group| group.rows.clone());
    let rows = rows.unwrap_or_else(|| panic!("no group: {seats:?}"));
    let got: Vec<[&str; 5]> = (rows.iter())
        .map(|r| {
            [
                &r.session_seat,
                &r.model,
                &r.effort,
                &r.activity,
                &r.declared,
            ]
            .map(|c| &c[..])
        })
        .collect();
    assert!(
        got.contains(&["s:lead", "?", "?", "busy", "done"]),
        "{got:?}"
    );
    assert!(
        got.contains(&["s:two", "Opus 5.5", "xhigh", "busy", "done"]),
        "{got:?}"
    );
}

/// Wrong: count every roster; drop unconfigured accounts; list unknown
/// liveness as live; one row per account dropping the second session.
#[test]
fn membership_live_only_unknown_counted_unconfigured_kept() {
    let known = id(ToolKind::Claude, "/cfg/.claude.json");
    let extra = id(ToolKind::Claude, "/other/.claude.json");
    let with = |session: &str, key: &RecordedIdentity, liveness| LiveSeatFact {
        liveness,
        ..fact(session, "lead", Some(key.clone()))
    };
    let facts = [
        with("a", &known, Some(true)),
        with("one", &known, Some(true)),
        with("dead", &known, Some(false)),
        with("maybe", &known, None),
        with("b", &extra, Some(true)),
    ];
    let seats = project_live_seats(&facts, std::slice::from_ref(&known), false);
    let mut names = seats_in(&seats.groups);
    names.sort_unstable();
    assert_eq!(names, ["a:lead", "b:lead", "one:lead"], "{seats:?}");
    let group = |key: &RecordedIdentity| seats.groups.iter().find(|g| g.key.as_ref() == Some(key));
    let counts = group(&known).map(|g| (g.rows.len(), g.unknown_liveness));
    assert_eq!(counts, Some((2, 1)), "two rows, one unknown: {seats:?}");
    assert!(group(&extra).is_some(), "unconfigured kept: {seats:?}");
    let text = overlay(seats, None, 80).join("\n").to_ascii_lowercase();
    assert!(text.contains("liveness unknown"), "{text}");
    assert!(!text.contains("dead:lead"), "{text}");
    assert!(!text.contains("maybe:lead"), "{text}");
}

/// Wrong: incompleteness hidden, or recovered by a second read.
#[test]
fn an_incomplete_snapshot_says_so() {
    let key = id(ToolKind::Claude, "/acct/.claude.json");
    let seats = project_live_seats(&[fact("s", "lead", Some(key.clone()))], &[key], true);
    assert!(seats.incomplete && !seats.groups.is_empty(), "{seats:?}");
    let text = overlay(seats, None, 80).join("\n").to_ascii_lowercase();
    assert!(text.contains("incomplete"), "{text}");
    assert!(text.contains("s:lead"), "{text}");
}

/// Wrong: byte or char clipping (wide profile), raw escapes, identity dropped
/// to fit, a clipped row with no visible omission, or the value dropped wide.
#[test]
fn narrow_width_clips_by_cell_neutralizes_and_keeps_session_seat() {
    let key = id(ToolKind::Claude, "/acct/.claude.json");
    let row = LiveSeatFact {
        model: Some("\u{1b}[31mred-model-name-that-is-long".into()),
        profile: "宽宽宽宽宽宽宽宽宽宽".into(),
        ..fact("longsession", "spawned.1", Some(key.clone()))
    };
    let wide = overlay(
        project_live_seats(
            std::slice::from_ref(&row),
            std::slice::from_ref(&key),
            false,
        ),
        None,
        160,
    );
    let width = 40;
    let rows = overlay(project_live_seats(&[row], &[key], false), None, width);
    let line = rows
        .iter()
        .find(|row| row.contains("longsession:spawned.1"));
    let line = line.unwrap_or_else(|| panic!("session:seat clipped: {rows:?}"));
    assert!(!line.contains('\u{1b}'), "control: {line:?}");
    let cjk = |ch: char| 1 + usize::from(('\u{4e00}'..='\u{9fff}').contains(&ch));
    let cells: usize = line.chars().map(cjk).sum();
    let budget = usize::from(width - super::LEFT);
    assert!(cells <= budget, "{cells} cells: {line:?}");
    let cut = line.contains('…') || line.contains("...");
    assert!(cut, "omission: {line:?}");
    let shown = wide
        .iter()
        .find(|row| row.contains("longsession:spawned.1"));
    let shown = shown.unwrap_or_else(|| panic!("wide row: {wide:?}"));
    assert!(shown.contains("red-model-name-that-is-long"), "{shown:?}");
    assert!(
        shown.contains("宽宽宽宽宽宽宽宽宽宽"),
        "full profile: {shown:?}"
    );
    assert!(!shown.contains('\u{1b}'), "{shown:?}");
}

/// Wrong: the projection rereads quota caches/rollouts or opens a door; the
/// grant wording drifts.
#[test]
fn the_grant_stands_and_the_projection_reads_nothing() {
    let quota = include_str!("../../quota.rs");
    let grant = "GRANT ae-20261024T143648Z-bd5bf3db: the existing join owner quota::recorded_identity -> run::canonical_config_home may run its fs::canonicalize + symlink_metadata probes, on the LOADER thread only, at most once per proved seat per fleet projection, through the unchanged owner. Still forbidden: file content/cache/session scans, a second quota observation, tmux/process/network reads, new doors, UI-thread I/O, any lexical/second join.";
    assert!(quota.contains(grant), "GRANT wording missing from quota.rs");
    for name in ["live_seat_facts", "project_live_seats"] {
        let start = quota.find(&format!("pub(crate) fn {name}(")).unwrap_or(0);
        let end = quota[start..]
            .find("\n}")
            .map_or(quota.len(), |at| start + at);
        assert!(start > 0, "{name} missing");
        for needle in READS.split(' ') {
            let body = &quota[start..end];
            assert!(!body.contains(needle), "{name} reads via {needle}");
        }
    }
}

/// Product part of an app source: everything before its terminal test module.
fn product(source: &str) -> &str {
    source
        .find("#[cfg(test)]\nmod tests {")
        .map_or(source, |at| &source[..at])
}

/// Wrong: draw/app/settings compute facts (UI-thread probes); the loader reads
/// quota, the world or the picker a second time for keys or facts.
#[test]
fn only_the_loader_builds_live_seat_facts_from_its_one_set_of_reads() {
    let loader = product(include_str!("../loader.rs"));
    for (needle, want) in [
        ("crate::quota::view_", 1),
        ("world_read(", 1),
        ("observe_picker_sessions", 1),
        ("observe_scopes", 0),
        ("fleet_rollouts", 0),
        ("view_stored", 0),
    ] {
        assert_eq!(loader.matches(needle).count(), want, "loader {needle}");
    }
    // Owner amendment (lead-spec-r1-read 103bf2c5): no process, Muse or quota
    // leg beyond the base count (base 559d3873: 12, 0, 0, 1).
    let legs = [
        ("transport::", 12),
        ("muse_usage::", 0),
        ("quota::run", 0),
        ("quota::ask", 1),
    ];
    for (needle, base) in legs {
        assert!(loader.matches(needle).count() <= base, "loader {needle}");
    }
    // Every other product path this slice may change (plan-r2 path budget).
    let others = [
        ("app.rs", include_str!("../../app.rs")),
        ("draw.rs", include_str!("../draw.rs")),
        ("settings.rs", include_str!("../settings.rs")),
    ];
    for (name, source) in others {
        for call in ["live_seat_facts(", "project_live_seats("] {
            assert!(!product(source).contains(call), "{name} {call}");
        }
    }
    let calls = loader.matches("live_seat_facts(").count();
    assert_eq!(calls, 1, "the loader builds the facts");
}
