//! Frozen acceptance contract, private half (#243 P2, plan r3 items 5, 6, 12,
//! 13, 15, 22, 23): both quota DIALOG paths draw a Claude BURN from the stored
//! history, the app reader's history cache holds one read for 60 s per state
//! root, and the history selector the display consumer calls (seam
//! `pace::select_history`, lead ruling on colead I3) picks the run it rates.
//!
//! Oracle: the plan and lead rulings, never the implementation. Fixtures only
//! WRITE; every read is the product's own door. RED tests fail on base at an
//! assertion; the CONTROL passes on base and after.

use std::path::PathBuf;

use super::history::{self, PolicyFacts, Row, Sample, Scope};
use super::pace::{self, Point};
use super::view::{self, Burn};
use super::{AgyCache, HistoryCache, Inputs};

/// 2033-05-18T11:53:20Z, so a few hours never cross a UTC date.
const NOW: i64 = 2_000_030_000;
const HOUR: i64 = 3_600;
const E: i64 = NOW - 60;
const RS: i64 = NOW + 2 * HOUR;
const RW: i64 = NOW + 2 * 86_400;
const INC: &str = "11111111-1111-4111-8111-111111111111";

struct Rig {
    root: PathBuf,
    config: PathBuf,
    sessions: PathBuf,
}

impl Rig {
    /// A Claude scope at `<root>/cc`: session 10 % and weekly 30 % at E.
    fn new(tag: &str) -> Self {
        let root = std::env::temp_dir().join(format!("ae-qp-spec-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("cc")).expect("claude home");
        std::fs::create_dir_all(root.join("sessions")).expect("sessions");
        let root = std::fs::canonicalize(root).expect("canonical root");
        let config = root.join("config");
        std::fs::write(
            &config,
            format!(
                "[clients]\ncc = claude config_home={}\n[profiles]\na = cc\n",
                root.join("cc").display()
            ),
        )
        .expect("config");
        let stamp = |at: i64| crate::time::Timestamp::from_epoch(at);
        std::fs::write(
            root.join("cc/.claude.json"),
            format!(
                "{{\"cachedUsageUtilization\":{{\"fetchedAtMs\":{},\"utilization\":{{\"limits\":[{{\"kind\":\"session\",\"group\":\"session\",\"percent\":10,\"resets_at\":\"{}\",\"scope\":null}},{{\"kind\":\"weekly_all\",\"group\":\"weekly_all\",\"percent\":30,\"resets_at\":\"{}\",\"scope\":null}}]}}}}}}\n",
                E * 1_000,
                stamp(RS),
                stamp(RW)
            ),
        )
        .expect("claude cache");
        Self {
            sessions: root.join("sessions"),
            config,
            root,
        }
    }

    fn inputs(&self, now: i64) -> Inputs<'_> {
        Inputs {
            home: Some(&self.root),
            global: Some(&self.config),
            local: None,
            sessions: Some(&self.sessions),
            now,
        }
    }

    /// Weekly 10 % 20 h and 20 % 10 h before E and session 4 % 40 min before,
    /// as the writer lays them out: a private day file per UTC date.
    fn store(&self, name: &str) {
        let home = self.root.join("cc");
        let scope = Scope {
            tool: "claude".to_owned(),
            source: home.join(".claude.json").display().to_string(),
            account: Some(home.display().to_string()),
        };
        let sample = |bucket: &str, minutes: u32, resets_at: i64, ago: i64, raw: &str| Sample {
            producer: name.to_owned(),
            incarnation: INC.to_owned(),
            collected_at: E - ago + 10,
            scope: scope.clone(),
            bucket: bucket.to_owned(),
            qualifier: None,
            window_minutes: Some(minutes),
            resets_at,
            observed_at: E - ago,
            raw: raw.to_owned(),
            effective: format!("{raw}.000000"),
            derivation: "raw".to_owned(),
            policy: PolicyFacts {
                version: 1,
                manual_resets: None,
                credits: "unreported".to_owned(),
                spend_cap: None,
            },
        };
        let dir = self.sessions.join(name);
        let history = dir.join("quota-history");
        std::fs::create_dir_all(&history).expect("history dir");
        private(&history, 0o700);
        std::fs::write(
            dir.join("meta"),
            format!("session={name}\nsession_id={INC}\n"),
        )
        .expect("meta");
        // 20 h before E is the previous UTC day; the other two are today.
        let line = |s: Sample| history::render(&Row::Sample(s)) + "\n";
        for (day, text) in [
            (
                "2033-05-17",
                line(sample("weekly_all", 10_080, RW, 20 * HOUR, "10")),
            ),
            (
                "2033-05-18",
                line(sample("weekly_all", 10_080, RW, 10 * HOUR, "20"))
                    + &line(sample("session", 300, RS, 40 * 60, "4")),
            ),
        ] {
            let file = history.join(format!("{day}.jsonl"));
            std::fs::write(&file, text).expect("day file");
            private(&file, 0o600);
        }
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn private(path: &std::path::Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).expect("private mode");
}

fn agy() -> AgyCache {
    AgyCache::new(|_| panic!("no paid quota ask in this spec"))
}

/// The BURN of the one Claude scope's window whose name starts `name`.
fn burn(scopes: &[view::Scope], name: &str) -> Burn {
    let windows = &scopes.first().expect("the Claude scope").windows;
    let window = windows.iter().find(|w| w.name.starts_with(name));
    window
        .unwrap_or_else(|| panic!("no {name} window: {windows:?}"))
        .burn
        .clone()
        .expect("a Claude window states its burn")
}

/// The dialog reads through `cache` at `now`, never asking agy.
fn held(rig: &Rig, now: i64, cache: &mut HistoryCache) -> Vec<view::Scope> {
    let (scopes, due) = super::view_held_cached(&rig.inputs(now), &agy(), false, cache);
    assert!(due.is_none(), "no agy scope, no ask due");
    scopes
}

fn weekly_rate() -> Burn {
    // 20 pp over 20 h; 70 pp left at 1 %/h.
    Burn::Rate {
        rate: "1.0%/h ~2d 22h".to_owned(),
        span: "20h00m".to_owned(),
    }
}

/// Only the endpoint: no span yet, against the weekly rule's 4 h.
fn weekly_short() -> Burn {
    Burn::Short {
        seen: "0m".to_owned(),
        need: "4h00m".to_owned(),
    }
}

#[test]
fn both_dialog_paths_draw_claude_burn_from_stored_history() {
    let rig = Rig::new("dialogs");
    rig.store("hist");
    let held = held(&rig, NOW, &mut HistoryCache::default());
    assert_eq!(burn(&held, "Weekly"), weekly_rate());
    // 6 pp over 40 min = 9 %/h; 90 pp left take 10 h.
    let session = Burn::Rate {
        rate: "9.0%/h ~10h00m".to_owned(),
        span: "40m".to_owned(),
    };
    assert_eq!(burn(&held, "Session"), session);
    assert_eq!(
        super::view_cached(&rig.inputs(NOW), &mut agy()),
        held,
        "tmux dialog: same facts"
    );
    let (wrapped, _) = super::view_held(&rig.inputs(NOW), &agy(), false);
    assert_eq!(wrapped, held, "the frozen wrapper reads a fresh cache");
}

#[test]
fn the_history_cache_holds_a_read_for_sixty_seconds_then_sees_a_new_row() {
    let rig = Rig::new("ttl");
    let mut cache = HistoryCache::default();
    assert_eq!(burn(&held(&rig, NOW, &mut cache), "Weekly"), weekly_short());
    rig.store("hist");
    // The empty read is held: 59 s later the new rows are not seen ...
    assert_eq!(
        burn(&held(&rig, NOW + 59, &mut cache), "Weekly"),
        weekly_short()
    );
    // ... though a fresh cache sees them, so the rows are there ...
    let fresh = held(&rig, NOW + 59, &mut HistoryCache::default());
    assert_eq!(burn(&fresh, "Weekly"), weekly_rate());
    // ... and at 60 s the held read expires.
    assert_eq!(
        burn(&held(&rig, NOW + 60, &mut cache), "Weekly"),
        weekly_rate()
    );
}

#[test]
fn a_backward_clock_or_another_state_root_reads_again() {
    let rig = Rig::new("clock");
    let mut cache = HistoryCache::default();
    assert_eq!(
        burn(&held(&rig, NOW + 30, &mut cache), "Weekly"),
        weekly_short()
    );
    rig.store("hist");
    assert_eq!(
        burn(&held(&rig, NOW + 29, &mut cache), "Weekly"),
        weekly_rate()
    );

    let (bare, stored) = (Rig::new("root-a"), Rig::new("root-b"));
    stored.store("hist");
    let mut cache = HistoryCache::default();
    assert_eq!(
        burn(&held(&bare, NOW, &mut cache), "Weekly"),
        weekly_short()
    );
    assert_eq!(
        burn(&held(&stored, NOW + 1, &mut cache), "Weekly"),
        weekly_rate()
    );
    assert_eq!(
        burn(&held(&bare, NOW + 2, &mut cache), "Weekly"),
        weekly_short()
    );
}

#[test]
fn the_decision_observation_carries_no_stored_history() {
    let rig = Rig::new("observe");
    rig.store("hist");
    let (observation, series) = super::observe_full(&rig.inputs(NOW)).expect("observe");
    assert!(
        series.is_empty(),
        "stored history reached the decision path"
    );
    assert!(
        observation.groups.iter().any(|g| g.rows.len() == 2),
        "the Claude rows were read"
    );
}

/// The weekly window: lookback 33 h 36 min, interior spacing a 32nd of it.
const WEEK: u32 = 10_080;

/// One raw point `ago` seconds before E in reset instance `resets_at`.
fn point(ago: i64, used: f64, resets_at: i64) -> Point {
    Point {
        observed_at: E - ago,
        used,
        resets_at,
    }
}

#[test]
fn the_selector_keeps_one_point_per_stamp_and_at_most_32_from_oldest_to_the_endpoint() {
    let end = point(0, 30.0, RW);
    // Every 10 min for 35 h, 0.1 pp apart, each stamp twice (a copy 0.05
    // under), a higher row AT the endpoint's stamp and a newer one after it.
    let mut union = vec![point(-30, 45.0, RW), point(0, 35.0, RW)];
    for step in 1..=210_i32 {
        let used = f64::from(300 - step) / 10.0;
        let ago = i64::from(step) * 600;
        union.extend([point(ago, used, RW), point(ago, used - 0.05, RW)]);
    }
    let kept = pace::select_history(end, &union, WEEK);
    // Wrong: a stored row at or past the shown stamp displaces the endpoint.
    assert_eq!(kept.last(), Some(&end), "{kept:?}");
    // Wrong: thinning from the newest drops the oldest, or a 12 h lookback.
    assert_eq!(kept.first(), Some(&point(201 * 600, 9.9, RW)), "{kept:?}");
    // Wrong: every point is kept.
    assert!(kept.len() <= 32, "{} points: {kept:?}", kept.len());
    // Wrong: copies of one stamp stay apart, or the lower copy wins it.
    for (index, kept_point) in kept.iter().enumerate() {
        let ago = E - kept_point.observed_at;
        if index + 1 < kept.len() {
            let step = i32::try_from(ago / 600).expect("small step");
            assert_eq!(*kept_point, point(ago, f64::from(300 - step) / 10.0, RW));
        }
        let next = kept.get(index + 1).map_or(i64::MAX, |p| p.observed_at);
        assert!(
            kept_point.observed_at < next,
            "one point per stamp: {kept:?}"
        );
    }
    // Wrong: the interior keeps the legacy 1350 s or the 300 s floor spacing.
    let interior = &kept[1..kept.len() - 1];
    assert!(
        interior
            .windows(2)
            .all(|w| w[1].observed_at - w[0].observed_at >= 3_780),
        "{kept:?}"
    );
}

#[test]
fn the_selector_clips_at_another_reset_instance_and_restarts_only_past_two_points_a_step() {
    let end = point(0, 30.0, RW);
    // A reset an hour off is ANOTHER instance; 16.1 -> 14.1 is 2 pp within 1e-9;
    // 25.5 -> 24 -> 22.6 falls 2.9 pp from its peak, at most 1.5 a step.
    let union = [
        point(30 * HOUR, 2.0, RW),
        point(22 * HOUR, 5.0, RW + HOUR),
        point(20 * HOUR, 10.0, RW),
        point(15 * HOUR, 16.1, RW),
        point(12 * HOUR, 14.1, RW),
        point(8 * HOUR, 25.5, RW),
        point(6 * HOUR, 24.0, RW),
        point(4 * HOUR, 22.6, RW),
    ];
    let kept = pace::select_history(end, &union, WEEK);
    // Wrong: filtering the other instance out before finding the boundary,
    // a plain `<= 2`, or a peak or 0.5 pp rule.
    assert_eq!(kept.first(), Some(&point(20 * HOUR, 10.0, RW)), "{kept:?}");
    assert_eq!(kept.last(), Some(&end), "{kept:?}");
    // Wrong: a step over 2 pp INTO the endpoint does not restart.
    let drop = [point(20 * HOUR, 10.0, RW), point(HOUR, 32.1, RW)];
    assert_eq!(pace::select_history(end, &drop, WEEK), [end]);
}

#[test]
fn the_selector_keeps_a_one_second_reset_jitter_and_measures_it_from_the_endpoint() {
    let end = point(0, 30.0, RW);
    // One Codex weekly instance is stamped RW or RW + 1 (measured 2026-10-24):
    // within INSTANCE_SECS (120 s) of the endpoint, so the 20 h run stays whole.
    let jitter: Vec<Point> = (1..=20_i32)
        .map(|hours| {
            let resets_at = if hours % 2 == 0 { RW } else { RW + 1 };
            point(i64::from(hours) * HOUR, f64::from(30 - hours), resets_at)
        })
        .collect();
    let first = |union: &[Point]| pace::select_history(end, union, WEEK).first().copied();
    // Wrong: exact reset equality.
    assert_eq!(first(&jitter), Some(point(20 * HOUR, 10.0, RW)));
    // Wrong: a pairwise chain. 100 s steps reach 200 s from the endpoint's
    // reset 3 h back, past 120 s, so the run starts 2 h back.
    let drift = [
        point(3 * HOUR, 26.0, RW - 200),
        point(2 * HOUR, 27.0, RW - 100),
        point(HOUR, 28.0, RW),
    ];
    assert_eq!(first(&drift), Some(point(2 * HOUR, 27.0, RW - 100)));
    // Wrong: a tolerance past 120 s. 120 s off still joins; 121 s cuts.
    let wide = [
        point(3 * HOUR, 26.0, RW + 121),
        point(2 * HOUR, 27.0, RW + 120),
    ];
    assert_eq!(first(&wide), Some(point(2 * HOUR, 27.0, RW + 120)));
}
