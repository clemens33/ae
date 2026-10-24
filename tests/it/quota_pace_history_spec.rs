//! Frozen acceptance contract: `ae quota` PACE reads the stored quota history
//! (#243 P2, plan r3 items 7-17 and 22-23, lead rulings 1d1a6ed5 and 1222faca).
//!
//! Oracle: the plan's rules, never the implementation. A matching stored
//! identity makes PACE the UNION of the rollout points, the stored RAW points
//! and the shown endpoint, judged under the CURRENT policy; a run restarts on a
//! drop over 2.0 pp and at a reset more than 120 s from the endpoint's; the lookback is
//! `(window/5).clamp(30 min, 7 d)`, the needed span stays the legacy third of the
//! 12 h-capped lookback (weekly 4 h); the rate rule is unchanged: judged
//! movement over span, ETA the judged rest at that rate, below 2 pp a bound.
//! RED tests fail on base at an assertion; CONTROL tests pass on base and after.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "fixtures build and inspect process-owned scratch state roots only"
)]

use std::fmt::Write as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use ae::quota::history::{self, PolicyFacts, Row, Sample, Scope};

/// 2033-05-18T11:53:20Z: midday, so a few hours never cross a UTC date.
const NOW: i64 = 2_000_030_000;
const DAY: i64 = 86_400;
const HOUR: i64 = 3_600;
/// The shown endpoint: one minute old, inside the 1 h trust bound.
const E: i64 = NOW - 60;
const RS: i64 = NOW + 2 * HOUR;
const RW: i64 = NOW + 2 * DAY;
/// A version-7 UUID minted one hour before `NOW`, so the rollout lookup finds its day.
const ID: &str = "01d1aadc-f500-7352-ade3-81a786200795";
const INC: [&str; 8] = [
    "11111111-1111-4111-8111-111111111111",
    "22222222-2222-4222-8222-222222222222",
    "33333333-3333-4333-8333-333333333333",
    "44444444-4444-4444-8444-444444444444",
    "55555555-5555-4555-8555-555555555555",
    "66666666-6666-4666-8666-666666666666",
    "77777777-7777-4777-8777-777777777777",
    "88888888-8888-4888-8888-888888888888",
];
const UNKNOWN: &str = r#"{"kind":"unknown"}"#;
/// A window: bucket, qualifier as JSON spells it, minutes.
type W = (&'static str, &'static str, u32);
const WEEKLY: W = ("weekly_all", "null", 10_080);
const SESSION: W = ("session", "null", 300);
const CODEX_WEEK: W = ("codex", "\"pro\"", 10_080);
const CODEX_SHORT: W = ("codex", "\"pro\"", 300);

/// The JSON pace object of one rate, spelled as `--json` spells it.
fn rate(per_h: f64, eta_secs: i64, before_reset: bool) -> String {
    format!(
        r#"{{"kind":"rate","per_h":{per_h},"eta_secs":{eta_secs},"before_reset":{before_reset}}}"#
    )
}

/// `YYYY-MM-DD` of an epoch, UTC — an independent civil-date oracle.
fn date(epoch: i64) -> String {
    let z = epoch.div_euclid(DAY) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

struct Rig {
    _scratch: super::cli::OwnedScratch,
    root: PathBuf,
    home: PathBuf,
    config: PathBuf,
}

impl Rig {
    fn new(tag: &str) -> Self {
        let scratch = super::cli::OwnedScratch::root("qp", tag);
        let root = std::fs::canonicalize(scratch.path()).expect("canonical scratch");
        for dir in ["home/.codex/sessions", "cc", "sessions"] {
            std::fs::create_dir_all(root.join(dir)).expect("fixture dir");
        }
        Self {
            _scratch: scratch,
            home: root.join("home"),
            config: root.join("config"),
            root,
        }
    }

    /// A Claude scope at `<root>/<dir>`, session 10 % and weekly `weekly` % at E.
    fn claude(tag: &str, extra: &str, weekly: &str) -> Self {
        let rig = Self::new(tag);
        let home = rig.root.join("cc");
        rig.config(&format!(
            "[clients]\ncc = claude config_home={}{extra}\n[profiles]\na = cc\n",
            home.display()
        ));
        Self::cache(&home, weekly, E);
        rig
    }

    fn config(&self, text: &str) {
        std::fs::write(&self.config, text).expect("config");
    }

    fn cache(home: &Path, weekly: &str, observed: i64) {
        let (session, week) = (
            ae::time::Timestamp::from_epoch(RS),
            ae::time::Timestamp::from_epoch(RW),
        );
        std::fs::write(
            home.join(".claude.json"),
            format!(
                "{{\"cachedUsageUtilization\":{{\"fetchedAtMs\":{},\"utilization\":{{\"limits\":[{{\"kind\":\"session\",\"group\":\"session\",\"percent\":10,\"resets_at\":\"{session}\",\"scope\":null}},{{\"kind\":\"weekly_all\",\"group\":\"weekly_all\",\"percent\":{weekly},\"resets_at\":\"{week}\",\"scope\":null}}]}}}}}}\n",
                observed * 1_000
            ),
        )
        .expect("claude cache");
    }

    /// A Codex scope whose one seat's rollout holds `records` (age before E,
    /// short %, weekly %), the newest carrying `extra` rate-limit fields.
    fn codex(tag: &str, records: &[(i64, &str, &str)], extra: &str) -> Self {
        let rig = Self::new(tag);
        rig.config("[clients]\ncx = codex\n[profiles]\np = cx --model sol\n");
        let seat = rig.root.join("sessions/spec");
        std::fs::create_dir_all(&seat).expect("seat session");
        std::fs::write(
            seat.join("meta"),
            format!("schema=2\nseat.main=lead\nprofile.main=p\nharness_session.main={ID}\nagent_bin.main=codex\n"),
        )
        .expect("seat meta");
        let day = rig
            .home
            .join(".codex/sessions")
            .join(date(NOW).replace('-', "/"));
        std::fs::create_dir_all(&day).expect("rollout day");
        let mut text = String::new();
        for (index, (ago, short, weekly)) in records.iter().enumerate() {
            let at = ae::time::Timestamp::from_epoch(E - ago);
            let more = if index + 1 == records.len() {
                extra
            } else {
                ""
            };
            let _ = writeln!(
                text,
                "{{\"timestamp\":\"{at}\",\"type\":\"event_msg\",\"payload\":{{\"type\":\"token_count\",\"rate_limits\":{{\"limit_id\":\"codex\",\"plan_type\":\"pro\",\"primary\":{{\"used_percent\":{short},\"window_minutes\":300,\"resets_at\":{RS}}},\"secondary\":{{\"used_percent\":{weekly},\"window_minutes\":10080,\"resets_at\":{RW}}}{more}}}}}}}"
            );
        }
        std::fs::write(day.join(format!("rollout-test-{ID}.jsonl")), text).expect("rollout");
        rig
    }

    /// The identity the history writer records for the Claude scope at `dir`.
    fn claude_scope(&self, dir: &str) -> Scope {
        let home = self.root.join(dir);
        Scope {
            tool: "claude".to_owned(),
            source: home.join(".claude.json").display().to_string(),
            account: Some(home.display().to_string()),
        }
    }

    fn codex_scope(&self) -> Scope {
        let home = self.home.join(".codex");
        Scope {
            tool: "codex".to_owned(),
            source: home.join("sessions").display().to_string(),
            account: Some(home.display().to_string()),
        }
    }

    /// Plant `samples` as session `name`'s history, incarnation `inc`, one
    /// private day file per UTC date of collection, as the writer lays it out.
    fn store(&self, name: &str, inc: &str, samples: &[Sample]) {
        let dir = self.root.join("sessions").join(name);
        let history = dir.join("quota-history");
        std::fs::create_dir_all(&history).expect("history dir");
        std::fs::set_permissions(&history, std::fs::Permissions::from_mode(0o700))
            .expect("private dir");
        std::fs::write(
            dir.join("meta"),
            format!("session={name}\nsession_id={inc}\n"),
        )
        .expect("history meta");
        let mut days = std::collections::BTreeMap::<String, String>::new();
        for sample in samples {
            let mut sample = sample.clone();
            name.clone_into(&mut sample.producer);
            inc.clone_into(&mut sample.incarnation);
            let line = history::render(&Row::Sample(sample.clone()));
            let file = days.entry(date(sample.collected_at)).or_default();
            file.push_str(&line);
            file.push('\n');
        }
        for (day, text) in days {
            let file = history.join(format!("{day}.jsonl"));
            std::fs::write(&file, text).expect("day file");
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600))
                .expect("private file");
        }
    }

    fn quota(&self, json: bool) -> String {
        let sessions = self.root.join("sessions");
        let inputs = ae::quota::Inputs {
            home: Some(&self.home),
            global: Some(&self.config),
            local: None,
            sessions: Some(&sessions),
            now: NOW,
        };
        let format = if json {
            ae::quota::Format::Json
        } else {
            ae::quota::Format::Table
        };
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = ae::quota::run_as(&inputs, format, &mut out, &mut err).expect("ae quota");
        assert_eq!(code, 0, "{}", String::from_utf8_lossy(&err));
        assert!(err.is_empty(), "no new diagnostic surface: {err:?}");
        String::from_utf8(out).expect("utf-8 quota output")
    }

    /// `ae quota --profiles` at the same instant and inputs as [`Rig::quota`].
    fn profiles(&self) -> String {
        let sessions = self.root.join("sessions");
        let inputs = ae::quota::Inputs {
            home: Some(&self.home),
            global: Some(&self.config),
            local: None,
            sessions: Some(&sessions),
            now: NOW,
        };
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = ae::quota::run_profiles(&inputs, 1 << 20, &mut out, &mut err).expect("view");
        assert_eq!(code, 0, "{}", String::from_utf8_lossy(&err));
        assert!(err.is_empty(), "{err:?}");
        String::from_utf8(out).expect("utf-8 profile view")
    }
}

/// One stored sample of `scope`: `ago` seconds before E, RAW `raw`, under a
/// plain policy whose stored effective equals raw.
fn at(scope: &Scope, (bucket, qualifier, minutes): W, resets: i64, ago: i64, raw: &str) -> Sample {
    let observed_at = E - ago;
    Sample {
        producer: String::new(),
        incarnation: String::new(),
        collected_at: observed_at + 10,
        scope: scope.clone(),
        bucket: bucket.to_owned(),
        qualifier: (qualifier != "null").then(|| qualifier.trim_matches('"').to_owned()),
        window_minutes: Some(minutes),
        resets_at: resets,
        observed_at,
        raw: raw.to_owned(),
        effective: format!("{:.6}", raw.parse::<f64>().expect("fixture raw")),
        derivation: "raw".to_owned(),
        policy: PolicyFacts {
            version: 1,
            manual_resets: None,
            credits: "unreported".to_owned(),
            spend_cap: None,
        },
    }
}

/// Claude weekly samples: (seconds before E, raw), all in the shown instance.
fn weekly(scope: &Scope, points: &[(i64, &str)]) -> Vec<Sample> {
    points
        .iter()
        .map(|&(ago, raw)| at(scope, WEEKLY, RW, ago, raw))
        .collect()
}

/// The `key` value of `window` in `--json` output.
fn field(json: &str, (bucket, qualifier, minutes): W, key: &str) -> String {
    let head =
        format!(r#"{{"bucket":"{bucket}","qualifier":{qualifier},"window_minutes":{minutes},"#);
    let window = &json[json
        .find(&head)
        .unwrap_or_else(|| panic!("no {head} in {json}"))..];
    let value = &window[window.find(&format!("\"{key}\":")).expect("key") + key.len() + 3..];
    let end = if key == "pace" {
        ",\"resets_at\":"
    } else {
        ","
    };
    value[..value.find(end).expect("value end")].to_owned()
}

fn pace(json: &str, window: W) -> String {
    field(json, window, "pace")
}

#[track_caller]
fn paced(json: &str, window: W, want: &str) {
    assert_eq!(pace(json, window), want, "{json}");
}

#[test]
fn claude_pace_reads_stored_history_past_twelve_hours_in_table_and_json() {
    let rig = Rig::claude("claude", "", "30");
    let scope = rig.claude_scope("cc");
    let mut samples = weekly(&scope, &[(20 * HOUR, "10"), (10 * HOUR, "20")]);
    samples.push(at(&scope, SESSION, RS, 40 * 60, "4"));
    rig.store("hist", INC[0], &samples);
    let json = rig.quota(true);
    // 20 pp over 20 h; 70 pp left at 1 %/h, after the reset in 2 d.
    paced(&json, WEEKLY, &rate(1.0, 252_000, false));
    // 6 pp over 40 min = 9 %/h; 90 pp left take 10 h, after the 2 h reset.
    paced(&json, SESSION, &rate(9.0, 36_000, false));
    assert_eq!(rig.quota(true), json, "inputs.now alone decides the answer");
    let table = rig.quota(false);
    let row = |bucket: &str| {
        table
            .lines()
            .find(|line| line.contains(bucket))
            .unwrap_or_else(|| panic!("no {bucket} row: {table}"))
            .to_owned()
    };
    assert!(row("weekly_all").contains("1.0%/h ~2d 22h"), "{table}");
    assert!(row("session").contains("9.0%/h ~10h00m"), "{table}");
}

#[test]
fn without_matching_history_claude_pace_stays_unknown_and_byte_identical() {
    let rig = Rig::claude("bare", "", "30");
    let (json, table) = (rig.quota(true), rig.quota(false));
    paced(&json, WEEKLY, UNKNOWN);
    paced(&json, SESSION, UNKNOWN);
    // Another account's history, and this account's other tool, join nothing.
    let other = Scope {
        account: Some(rig.root.join("elsewhere").display().to_string()),
        ..rig.claude_scope("cc")
    };
    let mut samples = weekly(&other, &[(20 * HOUR, "10"), (10 * HOUR, "20")]);
    samples.extend(weekly(
        &Scope {
            tool: "codex".to_owned(),
            ..rig.claude_scope("cc")
        },
        &[(20 * HOUR, "10")],
    ));
    rig.store("foreign", INC[0], &samples);
    assert_eq!(rig.quota(true), json);
    assert_eq!(rig.quota(false), table);
}

#[test]
fn manual_resets_judge_both_raw_ends_and_never_the_stored_effective() {
    let rig = Rig::claude("resets", " manual_resets=1", "30");
    let mut samples = weekly(
        &rig.claude_scope("cc"),
        &[(20 * HOUR, "10"), (10 * HOUR, "20")],
    );
    for (sample, effective) in samples.iter_mut().zip(["1.000000", "2.000000"]) {
        sample.effective = effective.to_owned();
        sample.derivation = "resets".to_owned();
        sample.policy.manual_resets = Some(1);
    }
    rig.store("hist", INC[0], &samples);
    let json = rig.quota(true);
    // Judged 30/2 = 15 now, 10/2 = 5 then: 10 pp over 20 h; 85 pp left.
    paced(&json, WEEKLY, &rate(0.5, 612_000, false));
    assert_eq!(field(&json, WEEKLY, "effective_percent"), "15");
}

#[test]
fn policy_changes_and_returns_never_cut_the_raw_slope() {
    let rig = Rig::claude("policy", "", "30");
    let points = [
        (20 * HOUR, "10"),
        (15 * HOUR, "15"),
        (12 * HOUR, "18"),
        (10 * HOUR, "20"),
    ];
    let mut samples = weekly(&rig.claude_scope("cc"), &points);
    samples[0].policy.manual_resets = Some(2);
    samples[0].policy.credits = "unlimited".to_owned();
    samples[0].derivation = "unlimited".to_owned();
    samples[0].effective = "0.000000".to_owned();
    samples[1].policy.spend_cap = Some(true);
    samples[1].derivation = "spend-cap".to_owned();
    samples[1].effective = "100.000000".to_owned();
    samples[2].policy.manual_resets = Some(2);
    samples[2].derivation = "resets".to_owned();
    samples[2].effective = "6.000000".to_owned();
    rig.store("hist", INC[0], &samples);
    let json = rig.quota(true);
    paced(&json, WEEKLY, &rate(1.0, 252_000, false));
}

#[test]
fn codex_history_joins_by_scope_and_current_unlimited_or_spend_cap_states_no_pace() {
    let stored = |rig: &Rig| {
        let scope = rig.codex_scope();
        let samples: Vec<_> = [(20 * HOUR, "10"), (10 * HOUR, "20")]
            .iter()
            .map(|&(ago, raw)| at(&scope, CODEX_WEEK, RW, ago, raw))
            .collect();
        rig.store("hist", INC[0], &samples);
    };
    let one = [(0, "10", "30")];
    let rig = Rig::codex("codex", &one, "");
    stored(&rig);
    let json = rig.quota(true);
    paced(&json, CODEX_WEEK, &rate(1.0, 252_000, false));
    for (tag, extra) in [
        (
            "unlimited",
            r#","credits":{"has_credits":true,"unlimited":true,"balance":"0"}"#,
        ),
        ("spendcap", r#","spend_control_reached":true"#),
    ] {
        let rig = Rig::codex(tag, &one, extra);
        stored(&rig);
        let json = rig.quota(true);
        assert_eq!(pace(&json, CODEX_WEEK), "null", "{tag}: {json}");
        assert_eq!(pace(&json, CODEX_SHORT), "null", "{tag}: {json}");
    }
}

#[test]
fn one_stored_point_keeps_a_mature_rollout_rate_and_its_oldest_point() {
    // Twelve rollout records 22.5 min apart reach 4 h 7.5 min back: weekly
    // 10 -> 21 is 11 pp over 4.125 h; 79 pp left take 29 h 37.5 min, before
    // the reset in 2 d. A wider history spacing must not drop the oldest.
    let records: Vec<(i64, String)> = (0..12)
        .rev()
        .map(|step| (step * 1_350, format!("{}", 21 - step)))
        .collect();
    let records: Vec<(i64, &str, &str)> = records
        .iter()
        .map(|(ago, weekly)| (*ago, "10", weekly.as_str()))
        .collect();
    let expected = rate(2.7, 106_650, true);
    let rig = Rig::codex("mature", &records, "");
    let alone = rig.quota(true);
    assert_eq!(pace(&alone, CODEX_WEEK), expected, "{alone}");
    let scope = rig.codex_scope();
    rig.store("hist", INC[0], &[at(&scope, CODEX_WEEK, RW, 1_800, "19.5")]);
    let joined = rig.quota(true);
    assert_eq!(pace(&joined, CODEX_WEEK), expected, "{joined}");
}

#[test]
fn history_joins_the_rollout_points_and_one_stored_point_switches_the_drop_rule() {
    // Rollout 10 % 5 h back, stored 18 and 22 within it: 20 pp over 5 h; 70 pp
    // left take 17.5 h. Stored points REPLACING the rollout leave 3 h: no rate.
    let rig = Rig::codex("union", &[(5 * HOUR, "10", "10"), (0, "10", "30")], "");
    let scope = rig.codex_scope();
    let stored = [(3 * HOUR, "18"), (2 * HOUR, "22")];
    let samples: Vec<_> = stored
        .iter()
        .map(|&(ago, raw)| at(&scope, CODEX_WEEK, RW, ago, raw))
        .collect();
    rig.store("hist", INC[0], &samples);
    let json = rig.quota(true);
    paced(&json, CODEX_WEEK, &rate(4.0, 63_000, true));
    // A rollout-only 1 pp dip restarts the legacy run, leaving 1 h: unknown.
    let records = [
        (8 * HOUR, "10", "10"),
        (2 * HOUR, "10", "25"),
        (HOUR, "10", "24"),
        (0, "10", "30"),
    ];
    let rig = Rig::codex("switch", &records, "");
    paced(&rig.quota(true), CODEX_WEEK, UNKNOWN);
    // ONE matching stored point puts the rollout under the 2 pp rule: the dip
    // is a correction, 20 pp over 8 h; 70 pp left take 28 h.
    let point = at(&rig.codex_scope(), CODEX_WEEK, RW, 7 * HOUR, "12");
    rig.store("hist", INC[0], &[point]);
    let json = rig.quota(true);
    paced(&json, CODEX_WEEK, &rate(2.5, 100_800, true));
}

type Case<'a> = (&'a str, &'a [(i64, &'a str)], String);

#[test]
fn a_run_restarts_only_past_two_points_including_at_the_live_endpoint() {
    let cases: [Case<'_>; 5] = [
        // A 1 pp correction stays one run, and so does 16.1 -> 14.1: binary64
        // reads it 2.0000000000000018, past a plain `<= 2` but inside 1e-9.
        (
            "one",
            &[(20 * HOUR, "10"), (12 * HOUR, "21"), (10 * HOUR, "20")],
            rate(1.0, 252_000, false),
        ),
        (
            "two",
            &[(20 * HOUR, "10"), (12 * HOUR, "16.1"), (10 * HOUR, "14.1")],
            rate(1.0, 252_000, false),
        ),
        // 2.5 pp restarts at 23, 10 h back: 7 pp over 10 h.
        (
            "over",
            &[(20 * HOUR, "10"), (12 * HOUR, "25.5"), (10 * HOUR, "23")],
            rate(0.7, 360_000, false),
        ),
        // The live endpoint 30 under a stored 31.5 is a correction ...
        (
            "live",
            &[(20 * HOUR, "10"), (HOUR, "31.5")],
            rate(1.0, 252_000, false),
        ),
        // ... and under a stored 33 a restart, leaving no span.
        (
            "drop",
            &[(20 * HOUR, "10"), (HOUR, "33")],
            UNKNOWN.to_owned(),
        ),
    ];
    for (tag, points, expected) in cases {
        let rig = Rig::claude(tag, "", "30");
        rig.store("hist", INC[0], &weekly(&rig.claude_scope("cc"), points));
        let json = rig.quota(true);
        assert_eq!(pace(&json, WEEKLY), expected, "{tag}: {json}");
    }
}

#[test]
fn a_run_clips_at_the_reset_instance_and_never_joins_another_identity() {
    let rig = Rig::claude("identity", "", "30");
    let scope = rig.claude_scope("cc");
    // 2 % in this instance, then a point of another instance, then this one
    // again: the run begins after the change, at 10 % 20 h back.
    let mut samples = vec![
        at(&scope, WEEKLY, RW, 30 * HOUR, "2"),
        at(&scope, WEEKLY, RW + HOUR, 22 * HOUR, "5"),
    ];
    samples.extend(weekly(&scope, &[(20 * HOUR, "10"), (10 * HOUR, "20")]));
    // 0 % 15 h back, inside the kept run: any identity joined wrongly drops
    // 10 pp there, restarts the run and reads 30 pp over 15 h, 2.0 %/h.
    let other = |change: &dyn Fn(&mut Sample)| {
        let mut sample = at(&scope, WEEKLY, RW, 15 * HOUR, "0");
        change(&mut sample);
        sample
    };
    samples.extend([
        other(&|s| s.scope.account = None),
        other(&|s| s.scope.source = rig.root.join("other/.claude.json").display().to_string()),
        other(&|s| s.scope.tool = "codex".to_owned()),
        other(&|s| s.window_minutes = Some(300)),
        other(&|s| s.qualifier = Some("x".to_owned())),
        other(&|s| s.bucket = "weekly_scoped".to_owned()),
    ]);
    rig.store("hist", INC[0], &samples);
    let json = rig.quota(true);
    paced(&json, WEEKLY, &rate(1.0, 252_000, false));
}

#[test]
fn eight_producers_are_one_point_and_an_equal_time_keeps_the_higher_raw() {
    let rig = Rig::claude("producers", "", "30");
    let scope = rig.claude_scope("cc");
    for (index, inc) in INC.iter().enumerate() {
        let oldest = if index == 7 { "12" } else { "10" };
        rig.store(
            &format!("p{index}"),
            inc,
            &weekly(&scope, &[(20 * HOUR, oldest), (10 * HOUR, "20")]),
        );
    }
    let json = rig.quota(true);
    // 18 pp over 20 h; 70 pp left at 0.9 %/h.
    paced(&json, WEEKLY, &rate(0.9, 280_000, false));
}

#[test]
fn the_shown_endpoint_stays_the_winner_and_history_never_resurrects_a_stale_one() {
    let rig = Rig::claude("winner", "", "30");
    let scope = rig.claude_scope("cc");
    let mut samples = weekly(&scope, &[(20 * HOUR, "10"), (10 * HOUR, "20")]);
    // Newer than the shown row, still before NOW: never the endpoint.
    samples.push(at(&scope, WEEKLY, RW, -30, "45"));
    // A higher stored raw AT the shown stamp: the shown 30 % still wins it.
    samples.push(at(&scope, WEEKLY, RW, 0, "35"));
    rig.store("hist", INC[0], &samples);
    let json = rig.quota(true);
    paced(&json, WEEKLY, &rate(1.0, 252_000, false));
    assert_eq!(field(&json, WEEKLY, "used_percent"), "30");
    assert_eq!(field(&json, WEEKLY, "observed_at"), E.to_string());
    // Two hours old, the endpoint is past the trust bound: no pace at all.
    Rig::cache(&rig.root.join("cc"), "30", NOW - 2 * HOUR);
    let json = rig.quota(true);
    paced(&json, WEEKLY, "null");
}

#[test]
fn a_long_dense_history_reaches_its_full_lookback() {
    let rig = Rig::claude("dense", "", "30");
    // A point every 10 min, 0.1 pp apart, 35 h back; the 33 h 36 min lookback
    // keeps 9.9 % at 33 h 30 min: 20.1 pp over 33.5 h is 0.6 %/h.
    let points: Vec<(i64, String)> = (1..=210)
        .map(|step| {
            (
                step * 600,
                format!("{}.{}", (300 - step) / 10, (300 - step) % 10),
            )
        })
        .collect();
    let points: Vec<(i64, &str)> = points
        .iter()
        .map(|(ago, raw)| (*ago, raw.as_str()))
        .collect();
    rig.store("hist", INC[0], &weekly(&rig.claude_scope("cc"), &points));
    let json = rig.quota(true);
    paced(&json, WEEKLY, &rate(0.6, 420_000, false));
}

/// Every path under `dir` with its length and modification time.
fn listing(dir: &Path) -> Vec<(PathBuf, u64, std::time::SystemTime)> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(next) = stack.pop() {
        let meta = std::fs::symlink_metadata(&next).expect("listed node");
        if meta.is_dir() {
            stack.extend(
                std::fs::read_dir(&next)
                    .expect("dir")
                    .map(|e| e.expect("entry").path()),
            );
        }
        found.push((next, meta.len(), meta.modified().expect("mtime")));
    }
    found.sort();
    found
}

#[test]
fn damaged_history_is_skipped_read_only_beside_a_usable_session() {
    let rig = Rig::claude("damaged", "", "30");
    let (json, table) = (rig.quota(true), rig.quota(false));
    let scope = rig.claude_scope("cc");
    let good = weekly(&scope, &[(20 * HOUR, "10")]);
    // Malformed lines and a torn tail; a foreign incarnation; no session id.
    rig.store("bad", INC[1], &good);
    let bad = rig.root.join("sessions/bad/quota-history");
    let bad = bad.join(format!("{}.jsonl", date(E - 20 * HOUR + 10)));
    std::fs::write(&bad, "{not json\n{\"v\":1}\n{\"torn").expect("damage");
    rig.store("alien", INC[2], &good);
    std::fs::write(
        rig.root.join("sessions/alien/meta"),
        format!("session_id={}\n", INC[3]),
    )
    .expect("alien");
    rig.store("anon", INC[4], &good);
    std::fs::write(rig.root.join("sessions/anon/meta"), "session=anon\n").expect("anon");
    let before = listing(&rig.root.join("sessions"));
    assert_eq!(rig.quota(true), json, "unusable history is no history");
    assert_eq!(rig.quota(false), table);
    assert_eq!(
        listing(&rig.root.join("sessions")),
        before,
        "the read writes nothing"
    );
    rig.store("good", INC[5], &good);
    let json = rig.quota(true);
    paced(&json, WEEKLY, &rate(1.0, 252_000, false));
}

#[test]
fn a_symlinked_config_home_joins_its_canonical_history() {
    let rig = Rig::claude("link", "", "30");
    std::os::unix::fs::symlink(rig.root.join("cc"), rig.root.join("link")).expect("link");
    rig.config(&format!(
        "[clients]\ncc = claude config_home={}\n[profiles]\na = cc\n",
        rig.root.join("link").display()
    ));
    rig.store(
        "hist",
        INC[0],
        &weekly(
            &rig.claude_scope("cc"),
            &[(20 * HOUR, "10"), (10 * HOUR, "20")],
        ),
    );
    let json = rig.quota(true);
    paced(&json, WEEKLY, &rate(1.0, 252_000, false));
}

/// The text of the item `header` opens, through its closing column-0 brace.
fn item<'a>(text: &'a str, header: &str) -> &'a str {
    let start = text.find(header).unwrap_or_else(|| panic!("no {header}"));
    let body = &text[start..];
    &body[..body.find("\n}\n").expect("item end")]
}

#[test]
fn no_decision_path_reads_quota_history() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let read = |file: &str| std::fs::read_to_string(src.join(file)).expect("source");
    let reads = ["read_fleet", "HistoryCache", "view_held_cached"];
    for file in [
        "watchdog_daemon.rs",
        "autoreseat.rs",
        "autoreseat/leg.rs",
        "effort.rs",
        "effort/leg.rs",
        "doctor.rs",
    ] {
        let text = read(file);
        for word in reads {
            assert!(!text.contains(word), "{file} reaches {word}");
        }
    }
    let quota = read("quota.rs");
    for header in [
        "pub(crate) fn observe(",
        "pub(crate) fn observe_full(",
        "fn observe_scopes(",
        "fn observe_scopes_budget(",
    ] {
        let body = item(&quota, header);
        for word in reads {
            assert!(!body.contains(word), "{header} reaches {word}");
        }
    }
}

/// The PACE cell `ae quota --profiles` draws on the `bucket` row of `window`.
fn profile_pace(view: &str, bucket: &str, window: &str) -> String {
    let line = view
        .lines()
        .find(|line| line.contains(&format!(" | {bucket} | {window} | ")))
        .unwrap_or_else(|| panic!("no {bucket} {window} row: {view}"));
    let cell = line.split(" | ").nth(6).expect("PACE cell");
    cell.trim().to_owned()
}

#[test]
fn the_profile_view_draws_the_history_pace_ae_quota_draws() {
    // Lead ruling I1: one PACE truth. The Claude rig of the first test.
    let rig = Rig::claude("profiles", "", "30");
    let scope = rig.claude_scope("cc");
    let mut samples = weekly(&scope, &[(20 * HOUR, "10"), (10 * HOUR, "20")]);
    samples.push(at(&scope, SESSION, RS, 40 * 60, "4"));
    rig.store("hist", INC[0], &samples);
    let (view, table) = (rig.profiles(), rig.quota(false));
    for (bucket, window, cell) in [
        ("weekly_all", "7d", "1.0%/h ~2d 22h"),
        ("session", "5h", "9.0%/h ~10h00m"),
    ] {
        assert_eq!(profile_pace(&view, bucket, window), cell, "{view}");
        assert!(table.contains(cell), "{table}");
    }
    // Codex: one stored point switches a rollout-only UNKNOWN to a rate.
    let records = [
        (8 * HOUR, "10", "10"),
        (2 * HOUR, "10", "25"),
        (HOUR, "10", "24"),
        (0, "10", "30"),
    ];
    let rig = Rig::codex("profswitch", &records, "");
    let point = at(&rig.codex_scope(), CODEX_WEEK, RW, 7 * HOUR, "12");
    rig.store("hist", INC[0], &[point]);
    paced(&rig.quota(true), CODEX_WEEK, &rate(2.5, 100_800, true));
    let cell = profile_pace(&rig.profiles(), "codex (pro)", "7d");
    assert!(cell.starts_with("2.5%/h ~1d 4h"), "{cell}");
    assert!(rig.quota(false).contains(&cell), "{cell}");
}
