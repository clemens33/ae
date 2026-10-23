//! #252 slice 2: requirements-derived recovery order, not runtime preservation proof.
#![allow(
    clippy::disallowed_methods,
    reason = "private Rig quota, meta and transcript fixtures"
)]

use std::fmt::Write as _;
use std::path::PathBuf;

use ae::autoreseat::{
    Candidate, Choice, Cover, Provenance, Shape, Store, Tier, Window, choose, choose_with, cover,
    listed,
};
use ae::quota::Status;
use ae::tool::ToolKind;

fn shape(tool: ToolKind, pin: &str, store: Store) -> Shape {
    let binary = tool.as_str();
    let account = match (&store, tool) {
        (Store::Proven(path), ToolKind::Claude) => format!("CLAUDE_CONFIG_DIR={} ", path.display()),
        (Store::Proven(path), ToolKind::Codex) => format!("CODEX_HOME={} ", path.display()),
        _ => String::new(),
    };
    Shape {
        words: vec![binary.to_owned(), "--model".to_owned()],
        pin: Some(pin.to_owned()),
        store,
        raw: format!("{account}{binary} --model {pin}"),
        tool,
    }
}

fn account(path: &str) -> Store {
    Store::Proven(PathBuf::from(path))
}

fn profiles(rows: &[(&str, Shape)]) -> Vec<(String, Shape)> {
    rows.iter()
        .map(|(name, shape)| ((*name).to_owned(), shape.clone()))
        .collect()
}

fn names(list: &[(String, Provenance)]) -> Vec<&str> {
    list.iter().map(|(name, _)| name.as_str()).collect()
}

fn reading(used: f64) -> Vec<Window> {
    vec![Window {
        judged: used,
        critical: used >= 90.0,
        long: true,
        observed_at: 100,
        resets_at: Some(1000),
        status: Status::Fresh,
        label: "weekly_all 7d".to_owned(),
    }]
}

fn judge(
    list: &[(String, Provenance)],
    windows: &[(&str, Vec<Window>)],
    room: Option<u8>,
) -> Choice {
    let candidates: Vec<_> = list
        .iter()
        .map(|(name, provenance)| Candidate {
            profile: name.clone(),
            provenance: *provenance,
            configured: true,
            peer_latched: false,
            windows: windows
                .iter()
                .find(|(key, _)| key == name)
                .map_or_else(Vec::new, |(_, rows)| rows.clone()),
            left_at: None,
            left_on_headroom: false,
        })
        .collect();
    if room.is_some() {
        choose_with(&candidates, 100, room)
    } else {
        choose(&candidates, 100)
    }
}

fn claude_family() -> (Shape, Vec<(String, Shape)>) {
    let seat = shape(ToolKind::Claude, "opus", account("/a"));
    let all = profiles(&[
        ("source", seat.clone()),
        ("twin", shape(ToolKind::Claude, "opus", account("/b"))),
        ("kept", shape(ToolKind::Claude, "sonnet", account("/a"))),
    ]);
    (seat, all)
}

#[test]
fn r1_kept_is_first_even_when_account_move_has_a_fresh_below_critical_reading() {
    let (seat, all) = claude_family();
    let list = listed(&[], ("source", &seat), &all);
    assert_eq!(names(&list), ["kept", "twin"]);
    let found = judge(&list, &[("twin", reading(10.0))], None);
    assert_eq!(found.pick, Some(("kept".to_owned(), Tier::Unknown)));
    assert_eq!(found.provenance.word(), ", derived sibling");
}

#[test]
fn r2_measured_codex_keep_prediction_uses_the_same_account() {
    let seat = shape(ToolKind::Codex, "gpt-6.1-sol", account("/a"));
    let all = profiles(&[
        (
            "unproven",
            shape(ToolKind::Codex, "gpt-6-luna", Store::Unknown),
        ),
        ("kept", shape(ToolKind::Codex, "gpt-6-luna", account("/a"))),
    ]);
    let list = listed(&[], ("source", &seat), &all);
    assert_eq!(names(&list), ["kept", "unproven"]);
    assert_eq!(
        judge(&list, &[], None).pick.map(|(name, _)| name),
        Some("kept".to_owned())
    );
}

#[test]
fn r3_proven_other_account_precedes_an_unknown_account_sibling() {
    let seat = shape(ToolKind::Claude, "opus", account("/a"));
    let all = profiles(&[
        ("unknown", shape(ToolKind::Claude, "haiku", Store::Unknown)),
        ("account", shape(ToolKind::Claude, "sonnet", account("/b"))),
    ]);
    let list = listed(&[], ("source", &seat), &all);
    assert_eq!(names(&list), ["account", "unknown"]);
    assert_eq!(
        judge(&list, &[("unknown", reading(10.0))], None)
            .pick
            .map(|(name, _)| name),
        Some("account".to_owned())
    );
}

#[test]
fn r4_same_step_keeps_twin_then_sibling_and_original_profile_order() {
    let seat = shape(ToolKind::Claude, "opus", account("/a"));
    let all = profiles(&[
        ("sibling", shape(ToolKind::Claude, "sonnet", account("/b"))),
        ("z", shape(ToolKind::Claude, "opus", account("/c"))),
        ("a", shape(ToolKind::Claude, "opus", account("/d"))),
        ("alias", shape(ToolKind::Claude, "opus", account("/c"))),
    ]);
    let list = listed(&[], ("source", &seat), &all);
    assert_eq!(names(&list), ["z", "a", "sibling"]);
    assert_eq!(
        judge(&list, &[], None).pick.map(|(name, _)| name),
        Some("z".to_owned())
    );
    assert_eq!(
        judge(&list, &[("sibling", reading(10.0))], None)
            .pick
            .map(|(name, _)| name),
        Some("sibling".to_owned()),
        "tier remains first within one step"
    );
}

#[test]
fn r5_critical_kept_loses_to_every_noncritical_account_move() {
    let (seat, all) = claude_family();
    let list = listed(&[], ("source", &seat), &all);
    for windows in [
        vec![("kept", reading(95.0))],
        vec![("kept", reading(95.0)), ("twin", reading(10.0))],
    ] {
        assert_eq!(
            judge(&list, &windows, None).pick.map(|(name, _)| name),
            Some("twin".to_owned())
        );
    }
    assert_eq!(
        judge(
            &list,
            &[("kept", reading(95.0)), ("twin", reading(96.0))],
            None
        )
        .pick
        .map(|(name, _)| name),
        Some("kept".to_owned()),
        "step decides when both are critical"
    );
}

#[test]
fn r6_declared_priority_tier_order_and_record_bytes_do_not_change() {
    let (seat, all) = claude_family();
    let declared = ["twin".to_owned()];
    let list = listed(&declared, ("source", &seat), &all);
    assert_eq!(names(&list), ["twin", "kept"]);
    let found = judge(
        &list,
        &[("twin", reading(95.0)), ("kept", reading(10.0))],
        None,
    );
    assert_eq!(found.pick.map(|(name, _)| name), Some("twin".to_owned()));
    assert_eq!(found.provenance.word(), "");
    let list = listed(
        &["kept".to_owned(), "twin".to_owned()],
        ("source", &seat),
        &all,
    );
    assert_eq!(
        judge(&list, &[("twin", reading(10.0))], None)
            .pick
            .map(|(name, _)| name),
        Some("twin".to_owned()),
        "declared rows keep their existing tier rule"
    );
}

#[test]
fn r7_unusable_kept_falls_through_for_both_limit_and_headroom() {
    let (seat, all) = claude_family();
    let list = listed(&[], ("source", &seat), &all);
    for (used, room) in [(100.0, None), (95.0, Some(95))] {
        let found = judge(
            &list,
            &[("kept", reading(used)), ("twin", reading(10.0))],
            room,
        );
        assert_eq!(found.pick.map(|(name, _)| name), Some("twin".to_owned()));
        assert!(
            found
                .skipped
                .iter()
                .any(|(name, _)| name == "kept, derived sibling")
        );
    }
}

#[test]
fn r8_go_route_is_only_available_when_declared_and_cover_stays_bare() {
    let seat = shape(
        ToolKind::OpenCode,
        "opencode/deepseek-v4.1-flash-free",
        Store::Shared,
    );
    // The rule follows the existing adapter route, not a magic profile suffix.
    let all = profiles(&[(
        "custom-paid",
        shape(
            ToolKind::OpenCode,
            "opencode-go/glm-5.3-flash",
            Store::Shared,
        ),
    )]);
    assert!(listed(&[], ("source", &seat), &all).is_empty());
    assert_eq!(cover(&[], ("source", &seat), &all), Cover::Bare);
    let list = listed(&["custom-paid".to_owned()], ("source", &seat), &all);
    assert_eq!(names(&list), ["custom-paid"]);
    assert_eq!(list[0].1.word(), "");
}

#[test]
fn r9_reordering_never_adds_effort_only_other_binary_or_other_tool_candidates() {
    let (seat, mut all) = claude_family();
    let mut effort = seat.clone();
    effort.raw.push_str(" --effort low");
    effort
        .words
        .extend(["--effort".to_owned(), "low".to_owned()]);
    let mut binary = shape(ToolKind::Claude, "haiku", account("/a"));
    binary.words[0] = "/other/claude".to_owned();
    binary.raw = "CLAUDE_CONFIG_DIR=/a /other/claude --model haiku".to_owned();
    all.extend(profiles(&[
        ("effort-only", effort),
        ("binary", binary),
        (
            "other-tool",
            shape(ToolKind::Codex, "gpt-6-luna", account("/c")),
        ),
    ]));
    assert_eq!(
        names(&listed(&[], ("source", &seat), &all)),
        ["kept", "twin"]
    );
}

#[test]
fn r10_unknown_store_never_earns_a_keep_prediction_or_alias_dedup() {
    let seat = shape(ToolKind::Claude, "opus", Store::Unknown);
    let all = profiles(&[
        ("first", shape(ToolKind::Claude, "sonnet", Store::Unknown)),
        ("second", shape(ToolKind::Claude, "sonnet", Store::Unknown)),
    ]);
    let list = listed(&[], ("source", &seat), &all);
    assert_eq!(names(&list), ["first", "second"]);
    assert_eq!(
        judge(&list, &[("second", reading(10.0))], None)
            .pick
            .map(|(name, _)| name),
        Some("second".to_owned()),
        "same seed step, so fresh tier wins"
    );
}

// Real selection path: the account-wide row binds every candidate, while an
// Opus-only row must not block a same-account Sonnet candidate.
#[allow(
    clippy::too_many_lines,
    reason = "one real selector scenario with both quota scopes"
)]
fn real_selection(tag: &str, scope: &str, chosen: &str) {
    use super::seat_relaunch::Rig;
    use ae::autoreseat::{ATTEMPT_ACTION, DONE_ACTION};
    use ae::events::Event;
    use ae::time::Timestamp;
    use std::time::{Duration, Instant};

    {
        let rig = Rig::new(tag);
        let root =
            std::fs::canonicalize(&rig.scratch).unwrap_or_else(|why| panic!("scratch: {why}"));
        assert!(std::fs::create_dir_all(root.join("home")).is_ok());
        let command = |home: &str, model: &str| {
            format!(
                "CLAUDE_CONFIG_DIR={} {} {} --model {model}",
                root.join(home).display(),
                root.join("tools/claude").display(),
                root.join("claude.pl").display()
            )
        };
        let mut config = String::from("[profiles]\n");
        for (name, home, model) in [
            ("source", "home-a", "opus"),
            ("twin", "home-b", "opus"),
            ("kept", "home-a", "sonnet"),
        ] {
            writeln!(config, "fake-{name} = \"{}\"", command(home, model))
                .unwrap_or_else(|why| panic!("config: {why}"));
        }
        config.push_str("[workspace]\nmain = lead\nlayout = vertical\nauto_reseat = on\nauto_reseat_grace_secs = 0\nauto_reseat_at = off\n");
        assert!(std::fs::write(root.join("config"), config).is_ok());
        rig.seat_rows("spawned.0", "scout", "source", "claude");
        let pane = rig.new_pane("spawned.0", "scout");
        rig.start(&pane, "spawned.0", "claude");
        let id = "12345678-1234-4234-8234-123456789abc";
        let mut meta = rig
            .meta()
            .lines()
            .filter(|line| {
                ![
                    "harness_session.spawned.0=",
                    "config_home.spawned.0=",
                    "config_home_base.spawned.0=",
                    "work_dir=",
                ]
                .iter()
                .any(|prefix| line.starts_with(prefix))
            })
            .fold(String::new(), |mut text, line| {
                writeln!(text, "{line}").unwrap_or_else(|why| panic!("meta row: {why}"));
                text
            });
        writeln!(
            meta,
            "work_dir={}\nharness_session.spawned.0={id}\nconfig_home.spawned.0={}",
            root.join("work").display(),
            root.join("home-a").display()
        )
        .unwrap_or_else(|why| panic!("meta: {why}"));
        assert!(std::fs::write(rig.dir.join("meta"), meta).is_ok());
        let transcript = root
            .join("home-a/projects")
            .join(ae::carry::project_key(&root.join("work")))
            .join(format!("{id}.jsonl"));
        assert!(
            std::fs::create_dir_all(
                transcript
                    .parent()
                    .unwrap_or_else(|| panic!("transcript parent"))
            )
            .is_ok()
        );
        assert!(
            std::fs::write(
                &transcript,
                b"{\"type\":\"user\"}\n{\"type\":\"assistant\"}\n"
            )
            .is_ok()
        );
        let now = Timestamp::now();
        for (home, used, qualifier) in [("home-a", 100, scope), ("home-b", 10, "null")] {
            let (kind, qualifier) = if qualifier == "null" {
                ("weekly_all", "null".to_owned())
            } else {
                (
                    "weekly_scoped",
                    format!("{{\"model\":{{\"display_name\":\"{qualifier}\"}}}}"),
                )
            };
            let cache = format!(
                "{{\"cachedUsageUtilization\":{{\"fetchedAtMs\":{},\"utilization\":{{\"limits\":[{{\"kind\":\"{kind}\",\"group\":\"{kind}\",\"percent\":{used},\"resets_at\":\"{}\",\"scope\":{qualifier}}}]}}}}}}\n",
                now.epoch() * 1000,
                Timestamp::from_epoch(now.epoch() + 86400)
            );
            assert!(std::fs::create_dir_all(root.join(home)).is_ok());
            assert!(std::fs::write(root.join(home).join(".claude.json"), cache).is_ok());
        }
        rig.mark_limited(&pane);
        let key = Timestamp::from_epoch(now.epoch() - 10);
        let limit = format!(
            "{{\"ts\":\"{key}\",\"actor\":\"watchdog\",\"action\":\"limit\",\"target\":\"scout\",\"target_slot\":\"spawned.0\",\"target_session\":\"{}\"}}\n",
            rig.session
        );
        assert!(std::fs::write(rig.dir.join("events.jsonl"), limit).is_ok());
        let result = super::cli::ae()
            .args([&format!("@{}", rig.session), "send", "scout", "auto reseat"])
            .env("HOME", root.join("home"))
            .env("AE_HOME", &root)
            .env("CONFIG_FILE", root.join("config"))
            .env("TMUX", format!("{},0,0", rig.sock.display()))
            .env("TMUX_PANE", &rig.main_pane)
            .env("AE_SENDER_OVERRIDE", "watchdog")
            .env("_AE_EVENT_ACTION", ATTEMPT_ACTION)
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("CODEX_HOME")
            .output()
            .unwrap_or_else(|why| panic!("trigger: {why}"));
        assert!(
            result.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&result.stderr),
            rig.events()
        );
        let deadline = Instant::now() + Duration::from_secs(90);
        loop {
            let done = rig
                .events()
                .lines()
                .filter_map(|line| Event::parse_line(line).ok())
                .any(|event| event.action == DONE_ACTION);
            if done {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "move timed out: {}",
                rig.events()
            );
            std::thread::sleep(Duration::from_millis(100));
        }
        assert_eq!(
            rig.meta_row("profile.spawned.0"),
            chosen,
            "{}",
            rig.events()
        );
        assert_eq!(
            rig.meta_row("harness_session.spawned.0"),
            id,
            "conversation remains exact"
        );
        let expected = if chosen == "fake-kept" {
            "fake-kept, derived sibling"
        } else {
            "fake-twin, derived twin"
        };
        assert!(
            rig.events().contains(expected),
            "legacy provenance: {}",
            rig.events()
        );
    }
}

#[test]
fn r11_real_model_scoped_limit_selects_kept() {
    real_selection("laddermodel", "Opus", "fake-kept");
}

#[test]
fn r12_real_account_wide_limit_skips_kept_and_selects_account() {
    real_selection("ladderwide", "null", "fake-twin");
}
