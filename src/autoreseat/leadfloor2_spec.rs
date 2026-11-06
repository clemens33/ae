//! #309 part B independent acceptance. Oracle: brief human/lead rulings.
//! Config grammar: driver plan R1 plus lead-rulings.md, accepted before implementation.
//! Tests exercise existing APIs so base RED must be behavioral, not a compiler error.

#![allow(
    clippy::disallowed_methods,
    clippy::format_push_string,
    reason = "acceptance fixtures own and remove only private scratch directories"
)]

use super::*;
use crate::autoreseat::{self as ar, PairGuard};
use crate::tool::Provider;
use std::sync::atomic::{AtomicU64, Ordering};

const NOW: i64 = 1_800_000_000;
const OLD_REASON: &str = "a lead seat never changes model unasked";
static NEXT: AtomicU64 = AtomicU64::new(0);

const FLOOR: &str = "auto_reseat_lead_floor = 3\n";
const TIERS: &str = "[model_tiers]\nsmall = haiku, 1\nmedium = sonnet, 3\nlarge = opus, 4\n";

struct Fixture {
    root: PathBuf,
    dir: PathBuf,
    bytes: Vec<u8>,
    slot: String,
    layout: String,
    config: String,
}

impl Fixture {
    fn new(layout: &str, slot: &str, floor: &str, tiers: &str, twin: bool) -> Self {
        let root = std::env::temp_dir().join(format!(
            "ae-leadfloor2-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(root.join("sessions/check")).expect("private session");
        for account in ["own", "away", "twin"] {
            std::fs::create_dir_all(root.join(account)).expect("private account");
        }
        let root = root.canonicalize().expect("canonical private root");
        let dir = root.join("sessions/check");
        let command = |account: &str, model: &str| {
            format!(
                "CLAUDE_CONFIG_DIR={} claude --model {model}",
                root.join(account).display()
            )
        };
        let mut config = format!(
            "[profiles]\nsource = \"{}\"\nlow = \"{}\"\nequal = \"{}\"\nhigh = \"{}\"\n",
            command("own", "opus"),
            command("own", "haiku"),
            command("away", "sonnet"),
            command("away", "fable")
        );
        if twin {
            config.push_str(&format!("twin = \"{}\"\n", command("twin", "opus")));
        }
        config.push_str(&format!(
            "[workspace]\nauto_reseat = all\nauto_reseat_grace_secs = 0\n{floor}{tiers}"
        ));
        let file = root.join("config");
        std::fs::write(&file, &config).expect("private config");
        let bytes = format!(
            "schema=2\nsession=check\nconfig={}\norigin={}\nwork_dir={}\nlayout={layout}\nseat.{slot}=subject\nprofile.{slot}=source\nagent_bin.{slot}=claude\nconfig_home.{slot}={}\n",
            file.display(), root.display(), root.display(), root.join("own").display()
        ).into_bytes();
        std::fs::write(dir.join("meta"), &bytes).expect("private meta");
        Self {
            root,
            dir,
            bytes,
            slot: slot.to_owned(),
            layout: layout.to_owned(),
            config,
        }
    }

    fn rewrite(&mut self, from: &str, to: &str) {
        assert!(
            self.config.contains(from),
            "replacement must change fixture"
        );
        self.config = self.config.replace(from, to);
        std::fs::write(self.root.join("config"), &self.config).expect("update private config");
    }

    fn judged(&self, declared: &[String]) -> Vec<Candidate> {
        let meta = crate::meta::Meta::parse(std::str::from_utf8(&self.bytes).expect("utf8"));
        let row = meta
            .roster()
            .iter()
            .find(|row| row.slot == self.slot)
            .expect("subject");
        let list = listed(&self.dir, &self.bytes, row, declared);
        candidates(
            &list,
            |profile| crate::reseat::resolved(&self.dir, profile),
            (&[], &[]),
            None,
            &[],
            NOW,
        )
    }

    fn only(&mut self, profiles: &[&str]) {
        let mut in_profiles = false;
        self.config = self
            .config
            .lines()
            .filter(|line| {
                if line.starts_with('[') {
                    in_profiles = *line == "[profiles]";
                }
                !in_profiles
                    || !line.contains(" = ")
                    || line.starts_with("source = ")
                    || profiles
                        .iter()
                        .any(|name| line.starts_with(&format!("{name} = ")))
            })
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        std::fs::write(self.root.join("config"), &self.config)
            .expect("restrict candidate universe");
    }

    fn choose(&self, declared: &[&str], room: Option<u8>) -> Plan {
        self.paired(declared, room, None)
    }

    fn paired(&self, declared: &[&str], room: Option<u8>, pair: Option<&PairGuard>) -> Plan {
        let names: Vec<String> = declared.iter().map(|name| (*name).to_owned()).collect();
        let mut text = format!(
            "{}\n[workspace]\nauto_reseat_at = {}\n",
            self.config,
            room.map_or_else(|| "off".to_owned(), |at| at.to_string())
        );
        if !names.is_empty() {
            text.push_str(&format!("[auto_reseat]\nsource = {}\n", names.join(", ")));
        }
        let raw = ar::settings_in(&self.root.join("config"), &text);
        let settings = raw.seat_view(self.layout == "lead-pair", &self.slot);
        let seat = ar::Seat {
            session: "check",
            slot: &self.slot,
            agent: "subject",
            profile: "source",
            orchestrator: false,
        };
        let key = Timestamp::from_epoch(NOW - 100);
        let action = if room.is_some() {
            "auto-reseat-headroom"
        } else {
            "limit"
        };
        let event = crate::events::Event::parse_line(&format!(
            "{{\"ts\":\"{key}\",\"actor\":\"watchdog\",\"action\":\"{action}\",\"target\":\"subject\",\"target_slot\":\"{}\",\"target_session\":\"check\",\"summary\":\"headroom 95% weekly_all 7d\"}}", self.slot
        )).expect("due episode");
        let sight = Sight {
            pane: ar::Pane {
                frame: ar::Frame::Clear,
                human_prompt: false,
                client_input: None,
            },
            limited: room.is_none(),
            near: true,
        };
        plan_paired(
            &settings,
            &seat,
            &sight,
            &ar::StartRead::default(),
            &[event],
            |declared, _| self.judged(declared),
            pair,
            Timestamp::from_epoch(NOW),
        )
    }

    fn settings(&self) -> Settings {
        ar::settings_in(&self.root.join("config"), &self.config)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn picked(plan: &Plan) -> Option<&str> {
    match plan {
        Plan::Attempt { to, .. } => Some(to),
        _ => None,
    }
}

fn reason(plan: &Plan, candidate: &str, model: &str) -> String {
    let Plan::Refuse { class, why, .. } = plan else {
        panic!("expected refusal: {plan:?}");
    };
    assert_eq!(*class, CauseClass::NoCandidate);
    for word in ["no usable candidate", candidate, model, "tier"] {
        assert!(why.contains(word), "missing {word:?}: {why}");
    }
    why.clone()
}

#[test]
fn leadfloor2_equal_and_stronger_siblings_replace_the_blanket_bar_on_both_leads() {
    for slot in ["main", "worker.0"] {
        for room in [None, Some(95)] {
            let fixture = Fixture::new("lead-pair", slot, FLOOR, TIERS, false);
            let choice = fixture.choose(&[], room);
            assert_eq!(
                picked(&choice),
                Some("equal"),
                "{slot} {room:?}: {choice:?}"
            );
            assert!(
                matches!(&choice, Plan::Attempt { note, .. } if note.contains("derived sibling"))
            );
            let mut stronger = Fixture::new("lead-pair", slot, FLOOR, TIERS, false);
            stronger.rewrite("sonnet, 3", "sonnet, 4");
            assert_eq!(picked(&stronger.choose(&[], room)), Some("equal"));
        }
    }
}

#[test]
fn leadfloor2_declared_rows_cannot_lower_a_lead_but_keep_their_preference_order() {
    for slot in ["main", "worker.0"] {
        for room in [None, Some(95)] {
            let mut fixture = Fixture::new("lead-pair", slot, FLOOR, TIERS, false);
            let choice = fixture.choose(&["low", "equal"], room);
            assert_eq!(picked(&choice), Some("equal"), "{slot}: {choice:?}");
            assert!(
                matches!(&choice, Plan::Attempt { note, .. } if note.contains("low")),
                "{choice:?}"
            );
            fixture.only(&["low"]);
            let refused = fixture.choose(&["low"], room);
            assert_eq!(picked(&refused), None, "{refused:?}");
            let why = reason(&refused, "low", "haiku");
            assert!(why.contains('1') && why.contains('3'), "{why}");
        }
    }
}

#[test]
fn leadfloor2_unknown_model_tiers_fail_closed_and_name_the_model() {
    for slot in ["main", "worker.0"] {
        let mut fixture = Fixture::new("lead-pair", slot, FLOOR, TIERS, false);
        fixture.only(&["high"]);
        let choice = fixture.choose(&["high"], None);
        assert_eq!(picked(&choice), None, "{choice:?}");
        let why = reason(&choice, "high", "fable");
        assert!(why.contains("missing") || why.contains("no tier"), "{why}");
        let mut unknown = Fixture::new("lead-pair", slot, FLOOR, TIERS, false);
        unknown.only(&["low"]);
        unknown.rewrite("--model haiku", "");
        let choice = unknown.choose(&["low"], None);
        assert_eq!(picked(&choice), None, "{choice:?}");
        let why = reason(&choice, "low", "model");
        assert!(
            why.contains("unknown") || why.contains("unproven") || why.contains("unread"),
            "{why}"
        );
    }
}

#[test]
fn leadfloor2_model_tiers_follow_the_model_across_profile_names() {
    let mut fixture = Fixture::new("lead-pair", "main", FLOOR, TIERS, false);
    fixture.rewrite("--model haiku", "--model sonnet");
    let choice = fixture.choose(&["low"], None);
    assert_eq!(
        picked(&choice),
        Some("low"),
        "a profile label is not a model tier: {choice:?}"
    );
    fixture.rewrite("--model sonnet", "--model haiku");
    assert_eq!(picked(&fixture.choose(&["equal"], None)), None);
}

#[test]
fn leadfloor2_twin_keeps_its_model_even_when_that_model_has_no_tier() {
    for slot in ["main", "worker.0"] {
        let fixture = Fixture::new(
            "lead-pair",
            slot,
            FLOOR,
            "[model_tiers]\nsmall = haiku, 1\n",
            true,
        );
        let choice = fixture.choose(&[], None);
        assert_eq!(picked(&choice), Some("twin"), "{choice:?}");
        assert!(matches!(&choice, Plan::Attempt { note, .. } if note.contains("derived twin")));
    }
}

#[test]
fn leadfloor2_floor_is_limited_to_the_two_lead_pair_slots() {
    for (layout, slot) in [
        ("vertical", "main"),
        ("vertical", "worker.0"),
        ("lead-pair", "worker.1"),
        ("lead-pair", "spawned.0"),
    ] {
        let fixture = Fixture::new(layout, slot, FLOOR, TIERS, false);
        for room in [None, Some(95)] {
            assert_eq!(picked(&fixture.choose(&[], room)), Some("low"));
            assert_eq!(picked(&fixture.choose(&["low"], room)), Some("low"));
        }
    }
}

#[test]
fn leadfloor2_tiers_without_a_floor_preserve_part_a_exactly() {
    for slot in ["main", "worker.0"] {
        for tiers in ["", TIERS] {
            let fixture = Fixture::new("lead-pair", slot, "", tiers, false);
            let choice = fixture.choose(&[], None);
            assert_eq!(picked(&choice), None);
            let Plan::Refuse { why, .. } = choice else {
                panic!("expected refusal");
            };
            assert_eq!(
                why,
                format!(
                    "no-candidate: refused: no usable candidate: low, derived sibling ({OLD_REASON}), equal, derived sibling ({OLD_REASON}), high, derived sibling ({OLD_REASON})"
                )
            );
            assert_eq!(picked(&fixture.choose(&["low"], None)), Some("low"));
        }
    }
}

#[test]
fn leadfloor2_capability_and_pair_provider_guards_both_apply() {
    let mut fixture = Fixture::new(
        "lead-pair",
        "main",
        FLOOR,
        "[model_tiers]\nsmall = haiku, 1\nother = gpt-6.1-sol, 3\n",
        false,
    );
    fixture.rewrite("claude --model sonnet", "codex --model gpt-6.1-sol");
    let judged = fixture.judged(&["equal".to_owned()]);
    let pair = PairGuard {
        partner: Some("peer".to_owned()),
        partner_serves: Some(Provider::OpenAI),
        seat_serves: Some(Provider::Anthropic),
        destinations: judged
            .iter()
            .map(|c| {
                (
                    c.profile.clone(),
                    Some(if c.profile == "equal" {
                        Provider::OpenAI
                    } else {
                        Provider::Anthropic
                    }),
                )
            })
            .collect(),
    };
    let choice = fixture.paired(&["equal"], None, Some(&pair));
    assert_eq!(picked(&choice), None);
    assert!(
        matches!(&choice, Plan::Refuse { class: CauseClass::PairProvider, why, .. }
        if why.contains("partner's provider")),
        "{choice:?}"
    );
    fixture.only(&["low"]);
    let collapsed = PairGuard {
        partner_serves: Some(Provider::Anthropic),
        ..pair
    };
    let choice = fixture.paired(&["low"], None, Some(&collapsed));
    assert_eq!(
        picked(&choice),
        None,
        "collapsed fallback cannot waive capability floor: {choice:?}"
    );
}

#[test]
fn leadfloor2_floor_accepts_unsigned_boundary_values_and_refuses_unusable_values() {
    for (floor, expected) in [("0", Some("equal")), ("3", Some("equal")), ("255", None)] {
        let mut fixture = Fixture::new(
            "lead-pair",
            "main",
            &format!("auto_reseat_lead_floor = {floor}\n"),
            TIERS,
            false,
        );
        fixture.only(&["equal"]);
        assert_eq!(
            picked(&fixture.choose(&["equal"], None)),
            expected,
            "floor {floor}"
        );
    }
    for row in [
        "bad",
        "-1",
        "+3",
        "256",
        "3.0",
        "\"\"",
        "",
        "3\nauto_reseat_lead_floor = 3",
    ] {
        let mut fixture = Fixture::new(
            "lead-pair",
            "main",
            &format!("auto_reseat_lead_floor = {row}\n"),
            TIERS,
            false,
        );
        fixture.only(&["equal"]);
        let choice = fixture.choose(&["equal"], None);
        let Plan::Refuse { why, .. } = choice else {
            panic!("unusable floor {row:?} admitted: {choice:?}");
        };
        assert!(
            why.contains("auto_reseat_lead_floor") && why.contains("unusable"),
            "{why}"
        );
        assert!(
            fixture
                .settings()
                .notes
                .iter()
                .any(|n| n.contains("auto_reseat_lead_floor")),
            "{row:?}"
        );
    }
}

#[test]
fn leadfloor2_malformed_tier_rows_name_the_problem_and_never_authorize_a_model() {
    for row in [
        "small = haiku",
        "small = haiku, +3",
        "small = haiku, 256",
        "small = haiku, -1",
        "small = , 3",
        "small =",
        "bad.alias = haiku, 3",
    ] {
        let mut fixture = Fixture::new(
            "lead-pair",
            "main",
            FLOOR,
            &format!("[model_tiers]\n{row}\n"),
            false,
        );
        fixture.only(&["low"]);
        assert_eq!(picked(&fixture.choose(&["low"], None)), None, "{row}");
        assert!(
            !fixture.settings().notes.is_empty(),
            "malformed row must be observable: {row}"
        );
    }
    let mut fixture = Fixture::new(
        "lead-pair",
        "main",
        FLOOR,
        "[model_tiers]\nsame = haiku, 1\nsame = haiku, 3\n",
        false,
    );
    fixture.only(&["low"]);
    assert_eq!(
        picked(&fixture.choose(&["low"], None)),
        Some("low"),
        "later alias wins"
    );
}

#[test]
fn leadfloor2_family_version_matching_and_version_specificity_follow_existing_pin_rule() {
    for (model, rows, passes) in [
        ("claude-sonnet-5-5", "family = sonnet, 3", true),
        (
            "claude-sonnet-5-5-20261106",
            "version = sonnet-5.5, 3",
            true,
        ),
        ("claude-sonnet-5-5", "version = sonnet-5-4, 3", false),
        ("sonnet", "version = claude-sonnet-5-5, 3", false),
        (
            "claude-sonnet-5-5",
            "family = sonnet, 4\nversion = sonnet-5-5, 1",
            false,
        ),
        (
            "claude-sonnet-5-5",
            "family = sonnet, 1\nversion = sonnet-5-5, 3",
            true,
        ),
    ] {
        let mut fixture = Fixture::new(
            "lead-pair",
            "main",
            FLOOR,
            &format!("[model_tiers]\n{rows}\n"),
            false,
        );
        fixture.only(&["equal"]);
        fixture.rewrite("--model sonnet", &format!("--model {model}"));
        assert_eq!(
            picked(&fixture.choose(&["equal"], None)),
            passes.then_some("equal"),
            "{model}: {rows}"
        );
    }
}

#[test]
fn leadfloor2_exact_adapter_keeps_dotted_and_provider_model_spelling_exact() {
    for pin in ["gpt-6.1-sol", "vendor/model"] {
        for row_pin in [pin.to_owned(), format!("{pin}-extra")] {
            let mut fixture = Fixture::new(
                "lead-pair",
                "main",
                FLOOR,
                &format!("[model_tiers]\nmodel = {row_pin}, 3\n"),
                false,
            );
            fixture.only(&["low"]);
            fixture.rewrite("claude --model haiku", &format!("codex --model {pin}"));
            assert_eq!(
                picked(&fixture.choose(&["low"], None)),
                (pin == row_pin).then_some("low"),
                "{pin} vs {row_pin}"
            );
        }
    }
}

#[test]
fn leadfloor2_conflicting_version_rows_cannot_expose_a_permissive_family_row() {
    for rows in [
        "family = sonnet, 4\none = sonnet-5-5, 1\ntwo = sonnet-5-5, 3",
        "two = sonnet-5-5, 3\nfamily = sonnet, 4\none = sonnet-5-5, 1",
        "one = sonnet-5-5, 1\ntwo = claude-sonnet-5-5, 3\nfamily = sonnet, 4",
        "one = sonnet-5-5, 1\ntwo = sonnet-5-5, 3\nthree = sonnet-5-5, 3\nfamily = sonnet, 4",
    ] {
        let mut fixture = Fixture::new(
            "lead-pair",
            "main",
            FLOOR,
            &format!("[model_tiers]\n{rows}\n"),
            false,
        );
        fixture.only(&["equal"]);
        fixture.rewrite("--model sonnet", "--model claude-sonnet-5-5");
        let choice = fixture.choose(&["equal"], None);
        let why = reason(&choice, "equal", "claude-sonnet-5-5");
        for word in ["one", "two", "disagree"] {
            assert!(why.contains(word), "{why}");
        }
    }
}

#[test]
fn leadfloor2_same_model_declared_exemption_requires_two_readable_equal_models() {
    for floor in [FLOOR, "auto_reseat_lead_floor = bad\n"] {
        for model in ["opus", ""] {
            let mut fixture = Fixture::new("lead-pair", "main", floor, "", false);
            fixture.only(&["low"]);
            fixture.rewrite(
                "--model haiku",
                if model.is_empty() { "" } else { "--model opus" },
            );
            if model.is_empty() {
                fixture.rewrite("--model opus", "");
            }
            let choice = fixture.choose(&["low"], None);
            assert_eq!(
                picked(&choice),
                (!model.is_empty()).then_some("low"),
                "{choice:?}"
            );
        }
    }
}

#[test]
fn leadfloor2_urgent_start_waives_waits_but_keeps_the_floor() {
    for slot in ["main", "worker.0"] {
        for (tier, allowed) in [(1, false), (3, true)] {
            let mut fixture = Fixture::new(
                "lead-pair",
                slot,
                FLOOR,
                &format!("[model_tiers]\nsmall = haiku, {tier}\n"),
                false,
            );
            fixture.only(&["low"]);
            let raw = ar::settings_in(
                &fixture.root.join("config"),
                &format!(
                    "{}\n[workspace]\nauto_reseat_at = off\nauto_reseat_grace_secs = 600\n",
                    fixture.config
                ),
            );
            let settings = raw.seat_view(true, slot);
            let seat = ar::Seat {
                session: "check",
                slot,
                agent: "subject",
                profile: "source",
                orchestrator: false,
            };
            let now = Timestamp::from_epoch(NOW);
            let event = crate::events::Event::parse_line(&format!(
                "{{\"ts\":\"{now}\",\"actor\":\"watchdog\",\"action\":\"auto-reseat-headroom\",\"target\":\"subject\",\"target_slot\":\"{slot}\",\"target_session\":\"check\",\"summary\":\"exhausted start\"}}"
            )).expect("urgent opener");
            let start = ar::StartRead {
                settled: Some(NOW),
                exhausted: Some(ar::Window {
                    judged: 100.0,
                    critical: true,
                    long: false,
                    observed_at: NOW,
                    resets_at: Some(NOW + 3600),
                    status: crate::quota::Status::Fresh,
                    label: "session 5h".to_owned(),
                }),
                brief_armed: false,
                quiet: true,
            };
            let sight = Sight {
                pane: ar::Pane {
                    frame: ar::Frame::Unproven,
                    human_prompt: false,
                    client_input: Some(NOW - 60),
                },
                limited: false,
                near: true,
            };
            let plan = plan_with(
                &settings,
                &seat,
                &sight,
                &start,
                &[event],
                |declared, _| fixture.judged(declared),
                now,
            );
            assert_eq!(picked(&plan), allowed.then_some("low"), "{slot}: {plan:?}");
            if !allowed {
                reason(&plan, "low", "haiku");
            }
        }
    }
}

#[test]
fn leadfloor2_proven_return_origin_is_exempt_even_without_a_tier_but_keeps_pair_guard() {
    for slot in ["main", "worker.0"] {
        for floor in [FLOOR, "auto_reseat_lead_floor = bad\n"] {
            let mut fixture = Fixture::new("lead-pair", slot, floor, "", false);
            fixture.only(&["low"]);
            // This declared cross-provider model is the proven pre-chain human choice.
            fixture.rewrite("claude --model haiku", "codex --model gpt-6.1-sol");
            let event = |offset, action: &str, reference: &str, summary: &str| {
                crate::events::Event::parse_line(&format!(
                    "{{\"ts\":\"{}\",\"actor\":\"watchdog\",\"action\":\"{action}\",\"target\":\"subject\",\"target_slot\":\"{slot}\",\"target_session\":\"check\",\"ref\":\"{reference}\",\"summary\":\"{summary}\"}}",
                    Timestamp::from_epoch(NOW + offset)
                )).expect("return-chain record")
            };
            let events = vec![
                event(-100, "limit", "", ""),
                event(
                    -99,
                    ar::ATTEMPT_ACTION,
                    &Timestamp::from_epoch(NOW - 100).to_string(),
                    "",
                ),
                event(
                    -98,
                    "reseat",
                    "",
                    &format!("reseated subject (pane %1, slot {slot}) to source, seed submitted"),
                ),
                event(-97, ar::DONE_ACTION, "low", "moved"),
                event(-90, "auto-reseat-return", "", "return origin"),
            ];
            let chain = ar::chain(&events, "check", slot, "subject").expect("product-valid chain");
            assert_eq!(chain.origin, "low");
            let open =
                ar::return_episode(&events, "check", slot, "subject").expect("return episode");
            let home = Home {
                origin: chain.origin,
                newest: chain.newest,
                flapped: false,
                ready: Some(ar::Window {
                    judged: 10.0,
                    critical: false,
                    long: true,
                    observed_at: NOW,
                    resets_at: Some(NOW + 3600),
                    status: crate::quota::Status::Fresh,
                    label: "weekly_all 7d".to_owned(),
                }),
            };
            let raw = fixture.settings();
            let settings = raw.seat_view(true, slot);
            let seat = ar::Seat {
                session: "check",
                slot,
                agent: "subject",
                profile: "source",
                orchestrator: false,
            };
            let sight = Sight {
                pane: ar::Pane {
                    frame: ar::Frame::Clear,
                    human_prompt: false,
                    client_input: None,
                },
                limited: false,
                near: false,
            };
            let plan = plan_return(
                &settings,
                (&seat, &sight),
                &open,
                Some(&home),
                |origin| fixture.judged(&[origin.to_owned()]),
                None,
                Timestamp::from_epoch(NOW),
            );
            assert_eq!(picked(&plan), Some("low"), "{plan:?}");
            let pair = PairGuard {
                partner: Some("peer".to_owned()),
                partner_serves: Some(Provider::OpenAI),
                seat_serves: Some(Provider::Anthropic),
                destinations: vec![("low".to_owned(), Some(Provider::OpenAI))],
            };
            let plan = plan_return(
                &settings,
                (&seat, &sight),
                &open,
                Some(&home),
                |origin| fixture.judged(&[origin.to_owned()]),
                Some(&pair),
                Timestamp::from_epoch(NOW),
            );
            assert!(
                matches!(
                    plan,
                    Plan::Refuse {
                        class: CauseClass::PairProvider,
                        ..
                    }
                ),
                "{plan:?}"
            );
        }
    }
}
