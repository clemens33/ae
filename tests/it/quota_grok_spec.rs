//! Independent acceptance contract: Grok quota from an untrusted shared log.
//! Exercise the public quota command with a scratch HOME and fixed clock.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance fixtures plant vendor logs in process-owned scratch storage"
)]

use super::cli::OwnedScratch;
use std::path::PathBuf;

const NOW: i64 = 1_788_858_600; // 2026-09-08T09:10:00Z
const FRESH: &str = "2026-09-08T09:05:00.000Z";
const OLDER: &str = "2026-09-08T09:00:00.000Z";
const MSG: &str = "billing: fetched credits config";
const SECRET: &str = "PRIVATE_LOG_MARKER_7319";

struct Rig(OwnedScratch);

impl Rig {
    fn new(tag: &str) -> Self {
        let rig = Self(OwnedScratch::root("gq", tag));
        std::fs::write(
            rig.0.path().join("config"),
            "[clients]\ngx = grok\n[profiles]\ng46 = gx --model grok-4.6\n",
        )
        .expect("isolated Grok config");
        rig
    }

    fn log(&self) -> PathBuf {
        self.0.path().join(".grok/logs/unified.jsonl")
    }

    fn plant(&self, bytes: impl AsRef<[u8]>) {
        std::fs::create_dir_all(self.0.path().join(".grok/logs")).expect("log directory");
        std::fs::write(self.log(), bytes).expect("synthetic shared log");
    }

    fn quota(&self) -> String {
        let mut out = Vec::new();
        let mut err = Vec::new();
        let code = ae::quota::run(
            &ae::quota::Inputs {
                home: Some(self.0.path()),
                global: Some(&self.0.path().join("config")),
                local: None,
                sessions: None,
                now: NOW,
            },
            &mut out,
            &mut err,
        )
        .expect("quota command");
        assert_eq!(code, 0);
        assert!(err.is_empty(), "{}", String::from_utf8_lossy(&err));
        let text = String::from_utf8(out).expect("UTF-8 table");
        assert!(!text.contains(SECRET), "shared-log bytes escaped: {text}");
        text
    }
}

fn record(ts: &str, percent: &str, demand: bool) -> String {
    format!(
        "{{\"ts\":\"{ts}\",\"ver\":\"unknown-future-version\",\"msg\":\"{MSG}\",\"ctx\":{{\"config\":{{\"creditUsagePercent\":{percent},\"billingPeriodStart\":\"2026-09-08T09:00:00.000Z\",\"billingPeriodEnd\":\"2026-09-15T09:00:00.000Z\"}},\"onDemandEnabled\":{demand},\"subscriptionTier\":\"super\"}}}}\n"
    )
}

fn compact(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn unsupported(text: &str) {
    assert!(
        compact(text).contains("unsupported (run /usage in grok)"),
        "{text}"
    );
    assert!(
        !text.contains('%'),
        "unsupported must show no window number: {text}"
    );
}

#[test]
fn missing_log_preserves_unsupported_hint() {
    unsupported(&Rig::new("missing").quota());
}

#[test]
fn quota_cli_reads_only_its_scratch_home() {
    let (mut command, scratch) = super::cli::ae_hermetic();
    std::fs::create_dir_all(scratch.path()).expect("CLI scratch HOME");
    let config = scratch.path().join("config");
    std::fs::write(&config, "[profiles]\ng46 = grok --model grok-4.6\n").expect("CLI config");
    let output = command
        .current_dir(scratch.path())
        .env("CONFIG_FILE", &config)
        .arg("quota")
        .output()
        .expect("real quota CLI");
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stderr.is_empty(), "{:?}", output.stderr);
    unsupported(&String::from_utf8(output.stdout).expect("UTF-8 CLI table"));
}

#[test]
fn fresh_credits_show_used_reset_and_observation_age() {
    let rig = Rig::new("fresh");
    rig.plant(record(FRESH, "37.0", false));
    let text = rig.quota();
    // Main 17afa2af: quota.rs:2932-2939 wraps non-weekly_scoped qualifiers;
    // quota.rs:273-276 shows EFFECTIVE as '-' when no reset is declared.
    // Main 2e23ba38: quota/pace.rs:153-166 requires sufficient history;
    // one observation leaves PACE unknown, spelled at pace.rs:203.
    assert!(
        compact(&text).contains("credits (super) 7d 37% - - unknown in 6d 23h 5m ago fresh"),
        "{text}"
    );
    assert!(!text.contains("unsupported"), "{text}");
}

#[test]
fn idle_log_is_stale_and_keeps_its_actual_age() {
    let rig = Rig::new("stale");
    rig.plant(record("2026-09-08T07:05:00.000Z", "37", false));
    let text = rig.quota();
    // Main 2e23ba38: quota/pace.rs:396-405 refuses pace after 60 minutes;
    // PACE is '-', while USED and the actual stale age stay visible.
    assert!(
        compact(&text).contains("37% - - - in 6d 23h 2h05m ago stale"),
        "{text}"
    );
    assert!(!text.contains(" fresh"), "{text}");
}

#[test]
fn newest_timestamp_wins_even_when_older_record_is_last() {
    let rig = Rig::new("order");
    rig.plant(format!(
        "{}{}",
        record(FRESH, "37", false),
        record(OLDER, "91", true)
    ));
    let text = rig.quota();
    assert!(
        compact(&text).contains("37% - - unknown in 6d 23h 5m ago fresh"),
        "{text}"
    );
    assert!(!text.contains("91%"), "{text}");
    assert!(
        !text.contains("on-demand spend enabled"),
        "older demand flag escaped: {text}"
    );
}

#[test]
fn newest_unusable_percentage_never_resurrects_an_older_number() {
    for (tag, percent) in [("null", "null"), ("negative", "-1"), ("overflow", "1e999")] {
        let rig = Rig::new(tag);
        rig.plant(format!(
            "{}{}",
            record(FRESH, percent, false),
            record(OLDER, "91", false)
        ));
        unsupported(&rig.quota());
    }
    let rig = Rig::new("absent");
    let absent = record(FRESH, "null", false).replace("\"creditUsagePercent\":null,", "");
    rig.plant(format!("{absent}{}", record(OLDER, "91", false)));
    unsupported(&rig.quota());
}

#[test]
fn future_at_inclusive_five_minute_boundary_is_unknown_without_numbers_or_age() {
    let rig = Rig::new("future");
    rig.plant(format!(
        "{}{}",
        record("2026-09-08T09:15:00.000Z", "37", false),
        record(OLDER, "91", false)
    ));
    let text = rig.quota();
    assert!(text.contains("unknown"), "{text}");
    assert!(!text.contains('%'), "future numbers escaped: {text}");
    assert!(
        !text.contains("ago"),
        "future stamp must state no age: {text}"
    );
    assert!(
        !text.contains("in 6d"),
        "future reset must remain blank: {text}"
    );
}

#[test]
fn shared_log_noise_and_malformed_candidates_do_not_blind_quota() {
    let rig = Rig::new("noise");
    let mut bytes = format!(
        "{{\"prompt\":\"{SECRET}\"}}\n{{malformed {MSG} {SECRET}\n{}{{\"msg\":\"prompt mentions {MSG}\",\"ts\":\"2026-09-08T09:09:00.000Z\",\"secret\":\"{SECRET}\"}}\n",
        record(FRESH, "37", false)
    ).into_bytes();
    bytes.extend_from_slice(&[0xff, b'\n']);
    bytes.extend_from_slice(format!("{MSG} {SECRET}\n").as_bytes());
    let huge = format!(
        "{{\"msg\":\"{MSG}\",\"secret\":\"{}\"}}\n",
        SECRET.repeat(4_000)
    );
    bytes.extend_from_slice(huge.as_bytes());
    rig.plant(bytes);
    let text = rig.quota();
    assert!(
        compact(&text).contains("37% - - unknown in 6d 23h 5m ago fresh"),
        "{text}"
    );
}

#[test]
fn candidate_prefilter_accepts_spacing_and_key_order_changes() {
    let rig = Rig::new("spacing");
    let spaced = record(FRESH, "37", false)
        .replace(':', ": ")
        .replace(',', ", ")
        .replace("{\"ts\":", "{\"ignored\": true, \"ts\":");
    rig.plant(spaced);
    let text = rig.quota();
    assert!(
        compact(&text).contains("37% - - unknown in 6d 23h 5m ago fresh"),
        "{text}"
    );
}

#[test]
fn no_billing_candidate_preserves_unsupported_without_echoing_noise() {
    let rig = Rig::new("empty");
    rig.plant(format!(
        "{{\"msg\":\"prompt\",\"text\":\"{SECRET}\"}}\nnot JSON\n"
    ));
    unsupported(&rig.quota());
}

#[test]
fn on_demand_note_does_not_replace_the_full_window_percentage() {
    let rig = Rig::new("demand");
    rig.plant(record(FRESH, "100.0", true));
    let text = rig.quota();
    assert!(
        compact(&text).contains("100% - - unknown in 6d 23h 5m ago fresh"),
        "{text}"
    );
    assert_eq!(
        text.matches("on-demand spend enabled: a full window may not block work")
            .count(),
        1,
        "{text}"
    );
    let rig = Rig::new("nodemand");
    rig.plant(record(FRESH, "100.0", false));
    assert!(!rig.quota().contains("on-demand spend enabled"));
}

#[test]
fn declared_manual_reset_uses_the_same_effective_headroom_rule() {
    let rig = Rig::new("reset");
    std::fs::write(
        rig.0.path().join("config"),
        "[clients]\ngx = grok manual_resets=1\n[profiles]\ng46 = gx --model grok-4.6\n",
    )
    .expect("declared reset");
    rig.plant(record(FRESH, "96", false));
    let text = rig.quota();
    assert!(
        compact(&text).contains("96% 48% x1 - unknown in 6d 23h 5m ago fresh"),
        "{text}"
    );
}

#[test]
fn tail_read_keeps_a_complete_record_inside_the_last_512_kib() {
    let rig = Rig::new("tail");
    let mut bytes = vec![b'x'; 550 * 1024];
    bytes.push(b'\n');
    bytes.extend_from_slice(record(FRESH, "37", false).as_bytes());
    bytes.extend(std::iter::repeat_n(b'x', 300 * 1024));
    bytes.push(b'\n');
    rig.plant(bytes);
    let text = rig.quota();
    assert!(
        compact(&text).contains("37% - - unknown in 6d 23h 5m ago fresh"),
        "{text}"
    );
}

#[test]
fn record_cut_by_the_tail_cap_is_dropped_without_older_fallback() {
    let rig = Rig::new("cut");
    let candidate = record(FRESH, "37", false);
    // Tail begins with valid JSON, but the byte before it is not a newline.
    // The original first line is noise; its suffix must not become evidence.
    let padding = 512 * 1024 - candidate.len() - 1;
    rig.plant(format!(
        "{}{candidate}{}\n",
        "x".repeat(70 * 1024),
        "x".repeat(padding)
    ));
    unsupported(&rig.quota());
}

#[test]
fn missing_period_end_is_unsupported_without_older_fallback() {
    let rig = Rig::new("noend");
    let newest = record(FRESH, "37", false).replace(
        "\"billingPeriodEnd\":\"2026-09-15T09:00:00.000Z\"",
        "\"billingPeriodEnd\":null",
    );
    rig.plant(format!("{newest}{}", record(OLDER, "91", false)));
    unsupported(&rig.quota());
}

#[test]
fn implausible_period_is_unknown_without_window_numbers() {
    let rig = Rig::new("period");
    rig.plant(
        record(FRESH, "37", false).replace("2026-09-15T09:00:00.000Z", "2026-12-15T09:00:00.000Z"),
    );
    let text = rig.quota();
    assert!(text.contains("unknown"), "{text}");
    assert!(
        !text.contains('%'),
        "implausible window stated usage: {text}"
    );
    assert!(
        text.contains("5m ago"),
        "observation age remains honest: {text}"
    );
}

#[test]
fn rejected_tier_bytes_never_reach_a_cell_or_note() {
    let rig = Rig::new("tier");
    rig.plant(
        record(FRESH, "37", false).replace("\"super\"", &format!("\"{SECRET}/\\u001b[31m\"")),
    );
    let text = rig.quota();
    assert!(
        compact(&text).contains("credits 7d 37% - - unknown in 6d 23h 5m ago fresh"),
        "{text}"
    );
}

#[test]
#[cfg(unix)]
fn symlinked_log_is_refused_without_using_its_window() {
    let rig = Rig::new("symlink");
    std::fs::create_dir_all(rig.0.path().join(".grok/logs")).expect("log directory");
    let outside = rig.0.path().join("untrusted-target");
    std::fs::write(&outside, record(FRESH, "37", false)).expect("symlink target");
    std::os::unix::fs::symlink(&outside, rig.log()).expect("symlinked vendor log");
    let text = rig.quota();
    assert!(!text.contains('%'), "symlinked window was consumed: {text}");
    assert!(text.contains("read-error"), "{text}");
}
