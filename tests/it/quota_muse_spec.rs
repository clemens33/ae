//! Frozen Muse quota acceptance; oracle: lead brief, plan-r2 rulings, and
//! confirmed probe #3: Muse's workspace must not contain its TMPDIR output root.
//! Synthetic records only. Paid entry is CLI-only, globally opted in, and
//! held across processes; every fake child has a private HOME and PATH.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance owns scratch files and fake children; never operator state"
)]

use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use ae::quota::muse::{self, Cached, Outcome};
use ae::quota::{Row, Status};

const NOW: i64 = 1_800_000_000;
const NOISE: &str = "SYNTH_MUSE_UNRELATED_STREAM_SENTINEL";
const STDERR: &str = "SYNTH_MUSE_STDERR_SENTINEL";
const HOME_ID: &str = "/synthetic/operator/.config/muse";
const PROFILE: &str = "one = muse --model muse-spark-1.3\n";
const CALL: &str = "--call--\nexec\n--json\n--preset\nempty\n--reasoning-effort\nminimal\n--no-session-log\n--no-foreign-personal-context\nReply with exactly: ok\n";

fn report(at: i64, window: &str, weekly: &str, tier: &str) -> String {
    let stamp = at * 1000 + 789;
    let short_reset = (at + 3600) * 1000 + 999;
    let weekly_reset = (at + 7 * 86400) * 1000 + 999;
    format!(
        "{{\"schema_version\":1,\"stream\":{{\"kind\":\"exec.usage\",\"id\":\"synthetic\"}},\"record_type\":\"status\",\"durability\":\"ephemeral\",\"payload_type\":\"run.usage\",\"payload_schema_version\":1,\"payload\":{{\"kind\":\"run_usage\",\"models\":[{{\"model\":\"synthetic\",\"input_tokens\":32100}}],\"unrelated\":\"{NOISE}\",\"allowance\":{{\"tier\":{tier},\"window\":{{\"used_percent\":{window},\"resets_at_ms\":{short_reset}}},\"weekly\":{{\"used_percent\":{weekly},\"resets_at_ms\":{weekly_reset}}},\"as_of_ms\":{stamp},\"source\":\"stream\"}}}}}}\n"
    )
}

fn row<'a>(rows: &'a [Row], bucket: &str) -> &'a Row {
    let found: Vec<_> = rows.iter().filter(|row| row.bucket == bucket).collect();
    assert_eq!(found.len(), 1, "one {bucket} account window: {rows:?}");
    found[0]
}

fn good_cache(attempt_at: i64) -> Cached {
    Cached {
        home: HOME_ID.to_owned(),
        attempt_at,
        outcome: Outcome::Done,
        resets_at: None,
        allowance: Some(
            muse::extract(report(NOW, "12.5", "81", "\"Pro\"").as_bytes()).expect("allowance"),
        ),
    }
}

#[test]
fn exact_status_usage_yields_two_windows_with_vendor_age_and_no_guessed_short_length() {
    let rows = muse::parse(report(NOW, "12.5", "81", "\"Pro\"").as_bytes(), NOW)
        .expect("measured envelope");
    assert_eq!(rows.len(), 2);
    assert_eq!(
        row(&rows, "window"),
        &Row {
            bucket: "window".to_owned(),
            qualifier: Some("Pro".to_owned()),
            window_minutes: None,
            used_percent: Some("12.5".to_owned()),
            resets_at: Some(NOW + 3600),
            observed_at: Some(NOW),
            status: Status::Fresh,
        }
    );
    assert_eq!(
        row(&rows, "weekly"),
        &Row {
            bucket: "weekly".to_owned(),
            qualifier: Some("Pro".to_owned()),
            window_minutes: Some(10080),
            used_percent: Some("81".to_owned()),
            resets_at: Some(NOW + 7 * 86400),
            observed_at: Some(NOW),
            status: Status::Fresh,
        }
    );
}

#[test]
fn stream_noise_never_becomes_allowance_or_persisted_cache_content() {
    let real = report(NOW, "12.5", "81", "null");
    let bytes = format!(
        "not JSON {NOISE}\n{{\"payload_type\":\"run.output.delta\",\"payload\":{{\"text\":\"{NOISE}\"}}}}\n{real}\n"
    );
    assert_eq!(
        muse::parse(bytes.as_bytes(), NOW),
        muse::parse(real.as_bytes(), NOW)
    );
    let cache = good_cache(NOW).render();
    for forbidden in [NOISE, "input_tokens", "models", "synthetic\"}"] {
        assert!(
            !cache.contains(forbidden),
            "unrelated stream data persisted: {cache}"
        );
    }
    assert!(
        !cache.contains("run.usage"),
        "cache contains raw stream envelope: {cache}"
    );
}

#[test]
fn wrong_record_kinds_duplicates_bad_utf8_and_malformed_candidates_are_refused() {
    let good = report(NOW, "12.5", "81", "null");
    for bytes in [
        good.replace("\"record_type\":\"status\"", "\"record_type\":\"event\"")
            .into_bytes(),
        good.replace(
            "\"payload_type\":\"run.usage\"",
            "\"payload_type\":\"run.output.delta\"",
        )
        .into_bytes(),
        good.replacen(
            "\"record_type\":\"status\"",
            "\"record_type\":\"event\",\"record_type\":\"status\"",
            1,
        )
        .into_bytes(),
        good.replacen(
            "\"used_percent\":12.5",
            "\"used_percent\":12.5,\"used_percent\":99",
            1,
        )
        .into_bytes(),
        good.replacen("\"as_of_ms\":", "\"as_of_ms\":123,\"as_of_ms\":", 1)
            .into_bytes(),
        b"{\"payload_type\":\"run.usage\", broken}\n".to_vec(),
        vec![0xff, b'\n'],
    ] {
        assert!(
            muse::parse(&bytes, NOW).is_err(),
            "hostile candidate was accepted: {bytes:?}"
        );
    }
}

#[test]
fn freshest_observation_wins_independent_of_stream_order_and_equal_conflicts_refuse() {
    let old = report(NOW - 60, "91", "92", "null");
    let fresh = report(NOW, "12.5", "81", "null");
    for bytes in [format!("{old}{fresh}"), format!("{fresh}{old}")] {
        let rows = muse::parse(bytes.as_bytes(), NOW).expect("ordered by as_of_ms");
        assert_eq!(row(&rows, "window").used_percent.as_deref(), Some("12.5"));
        assert_eq!(row(&rows, "window").observed_at, Some(NOW));
    }
    let same = format!("{fresh}{fresh}");
    assert_eq!(
        muse::parse(same.as_bytes(), NOW),
        muse::parse(fresh.as_bytes(), NOW)
    );
    let conflict = format!("{fresh}{}", report(NOW, "99", "99", "null"));
    assert!(muse::parse(conflict.as_bytes(), NOW).is_err());
}

#[test]
fn newest_unusable_values_and_unstamped_candidates_never_resurrect_old_headroom() {
    let old = report(NOW - 60, "1", "2", "null");
    for value in ["-1", "null", "true", "\"17\"", "1e999"] {
        let newest = report(NOW, value, value, "null");
        let rows =
            muse::parse(format!("{newest}{old}").as_bytes(), NOW).expect("latest unknown rows");
        assert!(
            rows.iter()
                .all(|row| row.status == Status::Unknown && row.used_percent.is_none()),
            "{value}: {rows:?}"
        );
    }
    let unstamped = report(NOW, "17", "18", "null").replace(
        &format!("\"as_of_ms\":{}", NOW * 1000 + 789),
        "\"as_of_ms\":null",
    );
    let single = muse::parse(unstamped.as_bytes(), NOW).expect("single unknown observation");
    assert!(
        single
            .iter()
            .all(|row| row.observed_at.is_none() && row.status == Status::Unknown)
    );
    for bytes in [format!("{old}{unstamped}"), format!("{unstamped}{old}")] {
        assert!(
            muse::parse(bytes.as_bytes(), NOW).is_err(),
            "ambiguous order restored old headroom"
        );
    }
}

#[test]
fn future_expired_and_stale_vendor_clocks_keep_the_existing_freshness_boundaries() {
    for (at, now, expected) in [
        (NOW, NOW + 900, Status::Fresh),
        (NOW, NOW + 901, Status::Stale),
        (NOW, NOW + 16 * 60, Status::Stale),
        (NOW + 300, NOW, Status::Unknown),
    ] {
        let rows =
            muse::parse(report(at, "37", "81", "null").as_bytes(), now).expect("clock fixture");
        assert_eq!(row(&rows, "window").status, expected);
        assert_eq!(row(&rows, "window").observed_at, Some(at));
    }
    let expired =
        muse::parse(report(NOW, "37", "81", "null").as_bytes(), NOW + 3600).expect("expired");
    assert_eq!(row(&expired, "window").status, Status::Unknown);
}

#[test]
fn invalid_reset_and_tier_bytes_cannot_supply_a_clock_or_terminal_control() {
    let good = report(NOW, "37", "81", "null");
    let reset = format!("\"resets_at_ms\":{}", (NOW + 3600) * 1000 + 999);
    for value in [
        "-1",
        "null",
        "\"1800003600999\"",
        "1e999",
        "1800003600999.5",
    ] {
        let bytes = good.replacen(&reset, &format!("\"resets_at_ms\":{value}"), 1);
        let rows = muse::parse(bytes.as_bytes(), NOW).expect("unknown reset");
        assert_eq!(row(&rows, "window").resets_at, None);
        assert_eq!(row(&rows, "window").status, Status::Unknown);
    }
    for tier in ["\"bad\\u001b[31m\"", "\"bad\\nline\""] {
        let rows = muse::parse(report(NOW, "37", "81", tier).as_bytes(), NOW)
            .expect("unsafe tier withheld");
        assert!(rows.iter().all(|row| row.qualifier.is_none()));
    }
}

#[test]
fn missing_or_null_allowance_is_empty_and_does_not_invent_two_zero_windows() {
    let good = report(NOW, "37", "81", "null");
    for bytes in [
        good.replace("\"allowance\":", "\"ignored_allowance\":"),
        good.replace(
            "\"allowance\":",
            "\"allowance\":null,\"ignored_allowance\":",
        ),
    ] {
        let rows = muse::parse(bytes.as_bytes(), NOW).expect("no allowance is a named gap");
        assert!(rows.is_empty(), "absent allowance invented rows: {rows:?}");
    }
}

#[test]
fn cache_ttl_uses_attempt_not_vendor_time_and_covers_every_paid_outcome() {
    assert_eq!(muse::TTL_SECS, 1800);
    for outcome in [
        Outcome::Done,
        Outcome::TimedOut,
        Outcome::TooLarge,
        Outcome::Failed,
    ] {
        let mut cache = good_cache(NOW);
        cache.outcome = outcome;
        if cache.outcome != Outcome::Done {
            cache.allowance = None;
        }
        for (at, due) in [
            (NOW - 1, true),
            (NOW, false),
            (NOW + 1799, false),
            (NOW + 1800, true),
            (i64::MAX, true),
        ] {
            assert_eq!(cache.due(HOME_ID, at), due, "{outcome:?} at {at}");
        }
        assert!(cache.due("/other/operator/.config/muse", NOW));
        let saved = cache.render();
        let restored = muse::parse_cache(saved.as_bytes()).expect("own cache record");
        assert_eq!(restored.render(), saved);
        assert_eq!(restored.attempt_at, NOW);
        assert!(!restored.due(HOME_ID, NOW + 1799));
    }
}

#[test]
fn hostile_cache_versions_duplicate_keys_and_bad_types_are_refused() {
    let good = good_cache(NOW).render();
    for bytes in [
        good.replacen("\"v\":1", "\"v\":2", 1),
        good.replacen("\"outcome\":\"ok\"", "\"outcome\":\"unknown\"", 1),
        good.replacen(
            "\"attempt_at\":1800000000",
            "\"attempt_at\":\"1800000000\"",
            1,
        ),
        good.replacen('{', "{\"v\":1,", 1),
        "not-json".to_owned(),
    ] {
        assert_ne!(bytes, good, "mutation must change the input");
        assert!(
            muse::parse_cache(bytes.as_bytes()).is_none(),
            "hostile cache accepted: {bytes}"
        );
    }
}

struct Rig {
    root: super::cli::OwnedScratch,
}

impl Rig {
    fn new(tag: &str, setting: Option<&str>, profiles: &str, body: &str) -> Self {
        let root = super::cli::OwnedScratch::root("quota-muse", tag);
        for dir in ["bin", "tmp", ".config/muse", "state/sessions"] {
            std::fs::create_dir_all(root.join(dir)).expect("private rig directories");
        }
        let setting = setting.map_or(String::new(), |value| format!("muse_quota = {value}\n"));
        std::fs::write(
            root.join("config"),
            format!("[workspace]\n{setting}[profiles]\n{profiles}"),
        )
        .expect("global config");
        std::fs::write(root.join("argv"), "").expect("call receipt");
        let now = i64::try_from(
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .expect("clock")
                .as_secs(),
        )
        .expect("epoch fits");
        std::fs::write(root.join("payload"), report(now, "12.5", "81", "\"Pro\""))
            .expect("synthetic payload");
        let rig = Self { root };
        rig.fake(body);
        rig
    }

    fn fake(&self, body: &str) {
        let script = format!(
            r#"#!/bin/sh
case "$0" in */*) rig_root=${{0%/*}}/.. ;; *) exit 98 ;; esac
printf '%s\n' --call-- "$@" >> "$rig_root/argv"
printf '%s\n' "$HOME" > "$rig_root/child-home"
cwd=$(pwd -P) || exit 98
tmp=$(CDPATH= cd "$TMPDIR" && pwd -P) || exit 98
printf '%s\n' "$cwd" > "$rig_root/child-cwd"
/bin/ls -A . > "$rig_root/child-cwd-entries" || exit 98
/bin/ls -ldn . > "$rig_root/child-cwd-mode" || exit 98
# Reproduce Muse's observed refusal before it can make a model call.
case "$tmp/" in
    "$cwd/"*) printf '%s\n' SYNTH_MUSE_WORKSPACE_CONTAINS_TMPDIR >&2; exit 1 ;;
esac
{body}
"#
        );
        let path = self.root.join("bin/muse");
        std::fs::write(&path, script).expect("owned fake, never a helper symlink");
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
            .expect("fake executable");
    }

    fn run(&self) -> String {
        let output = super::cli::ae()
            .env("HOME", self.root.path())
            .env("AE_HOME", self.root.join("state"))
            .env("CONFIG_FILE", self.root.join("config"))
            .env("PATH", self.root.join("bin"))
            .env("TMPDIR", self.root.join("tmp"))
            .current_dir(self.root.path())
            .arg("quota")
            .output()
            .expect("hermetic quota child");
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        if !self.argv().is_empty() {
            self.assert_child_cwd();
        }
        let text = String::from_utf8(output.stdout).expect("quota table UTF-8");
        assert!(text.starts_with("PROFILES"), "{text}");
        for forbidden in [NOISE, STDERR] {
            assert!(!text.contains(forbidden), "vendor text escaped: {text}");
        }
        text
    }

    /// Child-time receipts survive removal of the workspace. Checking them
    /// after every called fake covers success, nonzero, invalid output and cap.
    fn assert_child_cwd(&self) {
        let cwd = std::fs::read_to_string(self.root.join("child-cwd")).expect("cwd receipt");
        let cwd = PathBuf::from(cwd.trim());
        let temp = std::fs::canonicalize(self.root.join("tmp")).expect("canonical temp root");
        assert!(
            cwd.is_absolute() && cwd != temp && cwd.starts_with(&temp),
            "Muse workspace must be a strict temp descendant: cwd={}, temp={}",
            cwd.display(),
            temp.display()
        );
        let mode = std::fs::read_to_string(self.root.join("child-cwd-mode"))
            .expect("existing directory mode at child entry");
        assert!(
            mode.starts_with("drwx------"),
            "private 0700 directory: {mode}"
        );
        assert_eq!(
            std::fs::read_to_string(self.root.join("child-cwd-entries"))
                .expect("directory entries at child entry"),
            "",
            "Muse workspace must exist and be empty when the child starts"
        );
        assert!(
            std::fs::symlink_metadata(&cwd)
                .is_err_and(|why| why.kind() == std::io::ErrorKind::NotFound),
            "temporary Muse workspace survived quota return: {}",
            cwd.display()
        );
    }

    fn argv(&self) -> String {
        std::fs::read_to_string(self.root.join("argv")).expect("argv receipt")
    }
    fn cache(&self) -> PathBuf {
        self.root.join("state/quota/muse.json")
    }
}

const SUCCESS: &str = "exec /bin/cat \"$rig_root/payload\"";

fn words(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn default_off_explicit_off_and_invalid_global_values_never_start_muse() {
    for (tag, setting) in [
        ("default", None),
        ("off", Some("off")),
        ("bad", Some("true")),
        ("case", Some("ON")),
        ("empty", Some("")),
    ] {
        let rig = Rig::new(tag, setting, PROFILE, SUCCESS);
        let text = rig.run();
        assert!(rig.argv().is_empty(), "opt-in was absent: {setting:?}");
        assert!(!text.contains('%'), "off invented usage: {text}");
        if matches!(setting, Some("true" | "ON" | "")) {
            let text = words(&text);
            assert!(text.contains("muse_quota") && text.contains("on"), "{text}");
        } else {
            assert!(
                words(&text).contains("no verified local quota source"),
                "{text}"
            );
        }
    }
}

#[test]
fn local_config_cannot_opt_in_or_override_global_opt_in() {
    for (tag, global, local, expected) in
        [("local-on", "off", "on", 0), ("local-off", "on", "off", 1)]
    {
        let rig = Rig::new(tag, Some(global), PROFILE, SUCCESS);
        std::fs::create_dir(rig.root.join(".ae")).expect("local config directory");
        std::fs::write(
            rig.root.join(".ae/config"),
            format!("[workspace]\nmuse_quota = {local}\n"),
        )
        .expect("local ruling");
        let _ = rig.run();
        assert_eq!(
            rig.argv().matches("--call--").count(),
            expected,
            "global={global}, local={local}"
        );
    }
}

#[test]
fn explicit_cli_pins_argv_home_cwd_tier_age_and_one_account_across_models() {
    let profiles = "spark = muse --model muse-spark-1.3\nwonder = muse --model muse-wonder-full-1.0-contributor --contributor-mode on\n";
    let rig = Rig::new("shared", Some("on"), profiles, SUCCESS);
    let text = rig.run();
    assert_eq!(rig.argv(), CALL);
    assert_eq!(
        std::fs::read_to_string(rig.root.join("child-home"))
            .expect("HOME receipt")
            .trim(),
        rig.root.path().to_string_lossy()
    );
    let text = words(&text);
    for expected in [
        "spark", "wonder", "window", "weekly", "Pro", "12.5%", "81%", "7d", "0m ago", "fresh",
    ] {
        assert!(text.contains(expected), "missing {expected}: {text}");
    }
    assert!(!text.contains("5h"), "window duration was guessed: {text}");
    assert_eq!(
        text.matches("12.5%").count(),
        1,
        "duplicated account usage: {text}"
    );
}

#[test]
fn rewritten_home_unknown_variable_and_unrelated_scope_never_start_muse() {
    for (tag, profile) in [
        ("other-home", "one = HOME=/synthetic-other muse\n"),
        ("variable", "one = HOME=$MUSE_ACCEPTANCE_UNKNOWN muse\n"),
        ("other-tool", "one = gemini\n"),
    ] {
        let rig = Rig::new(tag, Some("on"), profile, SUCCESS);
        let _ = rig.run();
        assert!(
            rig.argv().is_empty(),
            "ineligible profile asked ambient account: {profile}"
        );
    }
}

#[test]
fn independent_cli_process_reuses_success_without_refreshing_cache_age_or_saving_noise() {
    let rig = Rig::new("held", Some("on"), PROFILE, SUCCESS);
    assert!(rig.run().contains("12.5%"));
    let saved = std::fs::read(rig.cache()).expect("persisted answer");
    assert!(!String::from_utf8_lossy(&saved).contains(NOISE));
    let permissions = std::fs::metadata(rig.cache())
        .expect("private cache mode")
        .permissions()
        .mode();
    assert_eq!(permissions & 0o777, 0o600);
    rig.fake("exit 7");
    assert!(rig.run().contains("12.5%"));
    assert_eq!(rig.argv(), CALL, "second process paid again inside TTL");
    assert_eq!(
        std::fs::read(rig.cache()).expect("unchanged held answer"),
        saved
    );
}

#[test]
fn independent_cli_process_holds_paid_failures_for_the_same_ttl() {
    let body = format!("printf '%s\\n' {STDERR} >&2\nexit 7");
    let rig = Rig::new("failed-held", Some("on"), PROFILE, &body);
    let first = words(&rig.run());
    assert!(
        first.contains("read-error") && first.contains("no usable answer"),
        "{first}"
    );
    let saved = std::fs::read(rig.cache()).expect("failure is held");
    rig.fake(SUCCESS);
    let second = words(&rig.run());
    assert!(
        second.contains("read-error") && second.contains("no usable answer"),
        "{second}"
    );
    assert_eq!(rig.argv(), CALL);
    assert_eq!(
        std::fs::read(rig.cache()).expect("held failure unchanged"),
        saved
    );
}

#[test]
fn muse_rows_use_the_shared_manual_reset_derivation() {
    let rig = Rig::new("derived", Some("on"), PROFILE, SUCCESS);
    std::fs::write(rig.root.join("config"), "[workspace]\nmuse_quota = on\n[clients]\nmuse = muse manual_resets=1\n[profiles]\none = muse --model muse-spark-1.3\n").expect("declared manual reset");
    let text = words(&rig.run());
    assert!(
        text.contains("81% 40.5% x1"),
        "81 percent over two allowances must derive 40.5 percent: {text}"
    );
    assert_eq!(rig.argv(), CALL);
}

#[test]
fn oversized_stdout_is_a_named_failure_and_is_held_without_echoing_vendor_bytes() {
    let rig = Rig::new("oversize", Some("on"), PROFILE, SUCCESS);
    std::fs::write(rig.root.join("payload"), vec![b'x'; 256 * 1024 + 1])
        .expect("bounded-output fixture");
    let text = words(&rig.run());
    assert!(
        text.contains("read-error") && text.contains("no usable answer"),
        "{text}"
    );
    assert!(!text.contains('%'));
    let _ = rig.run();
    assert_eq!(
        rig.argv(),
        CALL,
        "an oversized paid attempt must be held too"
    );
}

#[test]
fn ttl_expiry_reprobes_but_vendor_age_remains_independent_of_hold_time() {
    let rig = Rig::new("expired", Some("on"), PROFILE, SUCCESS);
    assert!(rig.run().contains("12.5%"));
    let mut cached =
        muse::parse_cache(&std::fs::read(rig.cache()).expect("held answer")).expect("parse cache");
    cached.attempt_at -= 1800;
    std::fs::write(rig.cache(), cached.render()).expect("age attempt, not observation");
    let now = i64::try_from(
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("clock")
            .as_secs(),
    )
    .expect("epoch");
    std::fs::write(
        rig.root.join("payload"),
        report(now - 16 * 60, "37", "83", "null"),
    )
    .expect("old vendor observation");
    let text = words(&rig.run());
    assert!(
        text.contains("37%") && text.contains("stale") && text.contains("16m"),
        "held TTL faked freshness: {text}"
    );
    assert_eq!(rig.argv(), format!("{CALL}{CALL}"));
}

#[test]
fn no_store_and_lock_contention_never_launch_an_uncacheable_paid_probe() {
    let rig = Rig::new("no-store", Some("on"), PROFILE, SUCCESS);
    std::fs::write(rig.root.join("state/quota"), "blocked directory")
        .expect("unwritable destination shape");
    let text = words(&rig.run());
    assert!(text.contains("writable state root"), "{text}");
    assert!(rig.argv().is_empty());
    let rig = Rig::new("busy", Some("on"), PROFILE, SUCCESS);
    std::fs::create_dir(rig.root.join("state/quota")).expect("quota directory");
    let _guard = ae::store::lock(&rig.root.join("state/quota/.muse.lock"), Duration::ZERO)
        .expect("another process owns lock");
    let text = words(&rig.run());
    assert!(text.contains("asking muse"), "{text}");
    assert!(rig.argv().is_empty());
}

#[test]
fn paid_answer_survives_a_later_cache_publish_failure_with_a_named_gap() {
    let rig = Rig::new("unrecorded", Some("on"), PROFILE, SUCCESS);
    std::fs::create_dir_all(rig.cache()).expect("directory blocks only atomic publication");
    let text = words(&rig.run());
    assert!(
        text.contains("12.5%") && text.contains("81%"),
        "paid answer lost: {text}"
    );
    assert!(
        text.contains("writable state root"),
        "unrecorded answer not named: {text}"
    );
    assert_eq!(rig.argv(), CALL, "no immediate retry");
}

#[test]
fn missing_binary_and_unusable_vendor_output_name_the_failure_without_usage() {
    let rig = Rig::new("absent", Some("on"), PROFILE, SUCCESS);
    std::fs::remove_file(rig.root.join("bin/muse")).expect("empty PATH");
    let text = words(&rig.run());
    assert!(
        text.contains("muse") && text.contains("not found"),
        "{text}"
    );
    assert!(rig.argv().is_empty());
    rig.fake(SUCCESS);
    assert!(
        rig.run().contains("12.5%"),
        "an absent executable cannot hold a paid-failure TTL"
    );
    assert_eq!(rig.argv(), CALL);
    for (tag, body) in [("garbage", "printf '%s\\n' not-json"), ("empty", "exit 0")] {
        let rig = Rig::new(tag, Some("on"), PROFILE, body);
        let text = words(&rig.run());
        assert!(
            text.contains("read-error") && text.contains("no usable answer"),
            "{text}"
        );
        assert!(!text.contains('%'), "failure invented usage: {text}");
        assert_eq!(rig.argv(), CALL);
    }
}

/// Structural complement to CLI receipts: the background and Settings paths
/// may read the held cache, but cannot acquire a route to the paid entry.
#[test]
fn observers_settings_watchdog_and_reseat_never_name_the_paid_muse_leg() {
    let quota = include_str!("../../src/quota.rs");
    for (start, end) in [
        ("pub(crate) fn observe(", "pub(crate) fn view_of("),
        ("pub(crate) fn view_of(", "Read configured client scopes"),
    ] {
        let body = quota
            .split_once(start)
            .expect("known reader boundary")
            .1
            .split_once(end)
            .expect("known reader end")
            .0;
        for forbidden in [
            "run_muse_quota",
            "muse_usage::refresh",
            "muse_usage::refresh_with",
        ] {
            assert!(!body.contains(forbidden), "{start} reached {forbidden}");
        }
    }
    for (name, body) in [
        ("watchdog", include_str!("../../src/watchdog_daemon.rs")),
        ("reseat", include_str!("../../src/autoreseat.rs")),
        ("app loader", include_str!("../../src/app/loader.rs")),
    ] {
        for forbidden in [
            "run_muse_quota",
            "muse_usage::refresh",
            "muse_usage::refresh_with",
        ] {
            assert!(!body.contains(forbidden), "{name} reached {forbidden}");
        }
    }
}

/// Unlike per-reader name checks, this asks the whole product tree who can
/// reach the paid entry. A renamed wrapper is another caller and must fail.
#[test]
fn the_only_paid_muse_caller_is_inside_the_cli_run_path() {
    fn sources(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).expect("product source directory") {
            let path = entry.expect("source entry").path();
            if path.is_dir() {
                sources(&path, out);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                out.push(path);
            }
        }
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut paths = Vec::new();
    sources(&root.join("src"), &mut paths);
    assert!(paths.len() > 30, "tree-wide scan must see the product");
    let mut paid_references = Vec::new();
    let mut transport_callers = Vec::new();
    for path in paths {
        let code = std::fs::read_to_string(&path).expect("product Rust source");
        let code: String = code.split_whitespace().collect();
        let relative = path.strip_prefix(root).expect("product path").to_path_buf();
        // Includes a function imported under another name: the import is
        // still a reference, even when the eventual call uses the alias.
        for (at, _) in code.match_indices("muse_usage::") {
            let suffix = &code[at + "muse_usage::".len()..];
            let direct = suffix.starts_with("refresh");
            let grouped = suffix.starts_with('{')
                && suffix
                    .split_once('}')
                    .is_some_and(|(names, _)| names.contains("refresh"));
            assert!(
                !suffix.starts_with('*'),
                "glob import hides a paid caller: {}",
                relative.display()
            );
            if direct || grouped {
                paid_references.push((relative.clone(), at));
            }
        }
        assert!(
            !code.contains("muse_usageas"),
            "module alias hides a paid caller: {}",
            relative.display()
        );
        if code.contains("run_muse_quota") {
            assert!(
                matches!(
                    relative.to_str(),
                    Some("src/transport.rs" | "src/muse_usage.rs")
                ),
                "new route around paid-entry cache/consent: {}",
                relative.display()
            );
            if relative == std::path::Path::new("src/muse_usage.rs") {
                for _ in code.match_indices("run_muse_quota") {
                    transport_callers.push(relative.clone());
                }
            }
        }
    }
    assert_eq!(
        paid_references.len(),
        1,
        "one product reference to paid refresh (including imports): {paid_references:?}"
    );
    assert_eq!(paid_references[0].0, std::path::Path::new("src/quota.rs"));
    assert_eq!(
        transport_callers.len(),
        1,
        "only the held-cache module may call transport"
    );
    let quota: String = include_str!("../../src/quota.rs")
        .split_whitespace()
        .collect();
    let run = quota.find("pubfnrun(").expect("CLI quota entry");
    let next = quota[run..].find("fnrun_with(").map_or(run, |at| run + at);
    let end = quota[next..]
        .find("fngo_refresh(")
        .map(|at| next + at)
        .expect("next helper closes the CLI run implementation");
    assert!(
        (run..end).contains(&paid_references[0].1),
        "paid entry escaped CLI quota::run/run_with"
    );
}

// Frozen musediag acceptance: lead's measured-shape, cache-invariant and
// limit-row rulings. Every account, request ID and timestamp here is synthetic.
const MUSEDIAG_ID: &str = "00000000-0000-4000-8000-000000000000";
const MUSEDIAG_PROFILES: &str =
    "synthetic_a = muse --model synthetic-a\nsynthetic_b = muse --model synthetic-b\n";
const MUSEDIAG_RESET: i64 = 1_893_456_000; // 2030-01-01T00:00:00Z
const MUSEDIAG_TIME: &str = "2030-01-01T00:00:00Z";
const MUSEDIAG_LIMIT: &str = concat!(
    "{\"record_type\":\"event\",\"payload_type\":\"run.terminal.failed\",",
    "\"payload\":{\"kind\":\"run_terminal\",\"terminal\":\"failed\",",
    "\"reason\":\"API error 429 [request_id=00000000-0000-4000-8000-000000000000]: ",
    "Subscription quota exhausted. Your usage window resets at ",
    "2030-01-01T00:00:00Z. (rate_limit_error)\",\"text\":\"\"}}\n"
);

fn musediag_held(outcome: &str, extra: &str) -> String {
    format!(r#"{{"v":1,"home":"{HOME_ID}","attempt_at":1800000000,"outcome":"{outcome}"{extra}}}"#)
}

fn musediag_cell(text: &str, column: &str, next: &str) -> String {
    let mut lines = text.lines();
    let header = lines.next().expect("quota header");
    let start = header.find(column).expect("named column");
    let end = header.find(next).expect("next column");
    // Header is ASCII; row positions count characters because SCOPE has a dot.
    lines
        .next()
        .expect("one Muse row")
        .chars()
        .skip(start)
        .take(end - start)
        .collect::<String>()
        .trim()
        .to_owned()
}

#[test]
fn measured_muse_limit_survives_transport_shows_reset_and_is_held_without_request_ids() {
    let rig = Rig::new(
        "md-limit",
        Some("on"),
        MUSEDIAG_PROFILES,
        "/bin/cat \"$rig_root/payload\"; exit 1",
    );
    let now = i64::try_from(
        SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .expect("fixture clock")
            .as_secs(),
    )
    .expect("epoch fits");
    let reset = now + 3600;
    let reset_text = ae::time::Timestamp::from_epoch(reset).to_string();
    let stream = format!(
        "not-json {NOISE}\n{{\"record_type\":\"status\",\"payload_type\":\"run.usage\",\"payload\":{{\"allowance\":null}}}}\n{}",
        MUSEDIAG_LIMIT.replace(MUSEDIAG_TIME, &reset_text)
    );
    std::fs::write(rig.root.join("payload"), stream).expect("synthetic failed stream");
    let first = rig.run();
    assert_eq!(musediag_cell(&first, "BUCKET", "WINDOW"), "limit");
    assert_eq!(musediag_cell(&first, "WINDOW", "USED"), "-");
    assert_eq!(musediag_cell(&first, "USED", "EFFECTIVE"), "100%");
    assert!(
        musediag_cell(&first, "RESETS", "OBSERVED").starts_with("in "),
        "reset must occupy RESETS, not merely hint text: {first}"
    );
    assert!(
        words(&first).contains("muse usage limit reached"),
        "{first}"
    );
    assert!(words(&first).contains("limit-derived"), "{first}");
    for profile in ["synthetic_a", "synthetic_b"] {
        assert!(
            first.contains(profile),
            "limit row lost model correlation: {first}"
        );
    }
    assert_eq!(
        first.matches("muse ·").count(),
        1,
        "one shared account: {first}"
    );
    assert!(!first.contains("read-error"), "{first}");
    let saved = std::fs::read(rig.cache()).expect("held limit");
    let saved_text = std::str::from_utf8(&saved).expect("UTF-8 normalized cache");
    let cache = ae::json::parse(saved_text).expect("cache JSON");
    assert_eq!(cache.get_str("outcome"), Some("limit"));
    assert_eq!(cache.get("resets_at"), Some(&ae::json::Value::Num(reset)));
    assert!(cache.get("allowance").is_none());
    for forbidden in [MUSEDIAG_ID, "request_id", "run.terminal.failed", NOISE] {
        assert!(
            !saved_text.contains(forbidden),
            "cache retained {forbidden}"
        );
        assert!(!first.contains(forbidden), "display retained {forbidden}");
    }
    assert_eq!(rig.argv(), CALL);
    rig.fake("exit 7");
    let second = rig.run();
    assert_eq!(musediag_cell(&second, "BUCKET", "WINDOW"), "limit");
    assert_eq!(musediag_cell(&second, "USED", "EFFECTIVE"), "100%");
    assert_eq!(std::fs::read(rig.cache()).expect("same held bytes"), saved);
    assert_eq!(rig.argv(), CALL, "a second process paid again inside TTL");
}

#[test]
fn other_muse_errors_and_malformed_limit_envelopes_keep_the_original_exit_status() {
    let wrong = [
        MUSEDIAG_LIMIT.replace("API error 429", "API error 500"),
        MUSEDIAG_LIMIT.replace("Subscription quota exhausted.", "Rate limit exceeded."),
        MUSEDIAG_LIMIT.replace("Subscription quota exhausted.", "Authentication failed."),
        MUSEDIAG_LIMIT.replace(
            "Your usage window resets at ",
            "Your weekly limit resets at ",
        ),
        MUSEDIAG_LIMIT.replace(MUSEDIAG_TIME, "2030-02-30T00:00:00Z"),
        MUSEDIAG_LIMIT.replace(MUSEDIAG_TIME, "2030-01-01T00:00:00.000Z"),
        MUSEDIAG_LIMIT.replace(MUSEDIAG_TIME, "2030-01-01T00:00:00+00:00"),
        MUSEDIAG_LIMIT.replace(MUSEDIAG_TIME, ""),
        MUSEDIAG_LIMIT.replace("\"terminal\":\"failed\"", "\"terminal\":\"completed\""),
        MUSEDIAG_LIMIT.replace("\"record_type\":\"event\"", "\"record_type\":\"status\""),
        MUSEDIAG_LIMIT.replace("run.terminal.failed", "task.lifecycle.failed"),
        MUSEDIAG_LIMIT.replace("API error 429", "untrusted prefix API error 429"),
        MUSEDIAG_LIMIT.replace("(rate_limit_error)", "(rate_limit_error) extra"),
        MUSEDIAG_LIMIT.replace("(rate_limit_error)", "(authentication_error)"),
        MUSEDIAG_LIMIT.replace("\"reason\":", "\"text\":"),
        MUSEDIAG_LIMIT.replacen('{', "{\"record_type\":\"status\",", 1),
        MUSEDIAG_LIMIT.replace("\"terminal\":", "\"terminal\":\"completed\",\"terminal\":"),
        MUSEDIAG_LIMIT.replace("\"reason\":", "\"reason\":\"unrelated\",\"reason\":"),
        MUSEDIAG_LIMIT.replace("\"payload\":{", "\"payload\":{},\"payload\":{"),
        MUSEDIAG_LIMIT.replacen('{', "{broken,", 1),
        format!("plain error, not a JSON event: {MUSEDIAG_TIME}"),
    ];
    for (index, payload) in wrong.into_iter().enumerate() {
        assert_ne!(
            payload, MUSEDIAG_LIMIT,
            "fixture mutation must change input"
        );
        let code = [1, 7, 255][index % 3];
        let rig = Rig::new(
            &format!("md-bad{index}"),
            Some("on"),
            PROFILE,
            &format!("/bin/cat \"$rig_root/payload\"; exit {code}"),
        );
        std::fs::write(rig.root.join("payload"), payload).expect("synthetic other error");
        let text = words(&rig.run());
        assert!(text.contains("read-error"), "case {index}: {text}");
        assert!(
            text.contains(&format!("exited {code}")),
            "case {index}: {text}"
        );
        assert!(
            !text.contains("100%"),
            "unmeasured limit invented usage: {text}"
        );
        let saved = std::fs::read_to_string(rig.cache()).expect("held exit status");
        let cache = ae::json::parse(&saved).expect("cache JSON");
        assert_eq!(
            cache.get_str("outcome"),
            Some(format!("exit-{code}").as_str())
        );
        assert!(cache.get("resets_at").is_none());
        assert!(!saved.contains(MUSEDIAG_ID));
    }
}

#[test]
fn non_utf8_nonzero_muse_stdout_preserves_exit_code_and_drops_vendor_bytes() {
    let rig = Rig::new(
        "md-binary",
        Some("on"),
        PROFILE,
        "printf '\\377\\376'; exit 7",
    );
    let text = words(&rig.run());
    assert!(
        text.contains("read-error") && text.contains("exited 7"),
        "{text}"
    );
    let saved = std::fs::read_to_string(rig.cache()).expect("normalized exit record");
    let cache = ae::json::parse(&saved).expect("cache JSON");
    assert_eq!(cache.get_str("outcome"), Some("exit-7"));
    assert!(cache.get("resets_at").is_none());
}

#[test]
fn exit_zero_muse_allowance_keeps_its_two_windows_even_beside_failed_event_text() {
    let rig = Rig::new("md-ok", Some("on"), PROFILE, SUCCESS);
    let allowance = std::fs::read_to_string(rig.root.join("payload")).expect("allowance");
    std::fs::write(
        rig.root.join("payload"),
        format!("{MUSEDIAG_LIMIT}{allowance}"),
    )
    .expect("exit-zero stream");
    let text = words(&rig.run());
    for expected in ["window", "weekly", "12.5%", "81%", "Pro"] {
        assert!(text.contains(expected), "{text}");
    }
    let saved = std::fs::read_to_string(rig.cache()).expect("held allowance");
    let cache = ae::json::parse(&saved).expect("cache JSON");
    assert_eq!(cache.get_str("outcome"), Some("ok"));
    assert!(cache.get("allowance").is_some());
    assert!(cache.get("resets_at").is_none());
    assert!(!saved.contains(MUSEDIAG_ID));
    assert!(
        !text.contains("100%") && !text.contains("limit reached"),
        "{text}"
    );
}

#[test]
fn a_limit_cache_round_trips_only_its_reset_and_uses_the_unchanged_attempt_ttl() {
    let input = musediag_held("limit", &format!(",\"resets_at\":{MUSEDIAG_RESET}"));
    let held = muse::parse_cache(input.as_bytes()).expect("limit with a valid reset");
    let rendered = held.render();
    let value = ae::json::parse(&rendered).expect("rendered cache");
    assert_eq!(value.get_str("outcome"), Some("limit"));
    assert_eq!(
        value.get("resets_at"),
        Some(&ae::json::Value::Num(MUSEDIAG_RESET))
    );
    assert!(value.get("allowance").is_none());
    let restored = muse::parse_cache(rendered.as_bytes()).expect("read own limit");
    assert_eq!(restored, held);
    for (at, due) in [
        (NOW - 1, true),
        (NOW, false),
        (NOW + 1799, false),
        (NOW + 1800, true),
    ] {
        assert_eq!(restored.due(HOME_ID, at), due, "at {at}");
    }
    assert!(restored.due("/synthetic/another-muse-home", NOW));
    // Old v1 failure and successful allowance records remain readable.
    assert!(muse::parse_cache(musediag_held("exit-1", "").as_bytes()).is_some());
    let old_success = good_cache(NOW);
    assert_eq!(
        muse::parse_cache(old_success.render().as_bytes()),
        Some(old_success)
    );
}

#[test]
fn only_a_limit_outcome_may_carry_one_valid_integral_reset_in_the_cache() {
    for extra in [
        "",
        ",\"resets_at\":null",
        ",\"resets_at\":0",
        ",\"resets_at\":-1",
        ",\"resets_at\":253402300800",
        ",\"resets_at\":1893456000.5",
        ",\"resets_at\":\"1893456000\"",
        ",\"resets_at\":true",
        ",\"resets_at\":99999999999999999999",
        ",\"resets_at\":1893456000,\"resets_at\":1893456001",
        ",\"resets_at\":1893456000,\"allowance\":{}",
    ] {
        let input = musediag_held("limit", extra);
        assert!(muse::parse_cache(input.as_bytes()).is_none(), "{input}");
    }
    for outcome in [
        "exit-1",
        "failed",
        "timeout",
        "unparsed",
        "not-utf8",
        "too-large",
    ] {
        let input = musediag_held(outcome, ",\"resets_at\":1893456000");
        assert!(muse::parse_cache(input.as_bytes()).is_none(), "{input}");
    }
    let success_with_reset =
        good_cache(NOW)
            .render()
            .replacen('{', "{\"resets_at\":1893456000,", 1);
    assert!(muse::parse_cache(success_with_reset.as_bytes()).is_none());
    for reset in [1_i64, 253_402_300_799] {
        let input = musediag_held("limit", &format!(",\"resets_at\":{reset}"));
        assert!(
            muse::parse_cache(input.as_bytes()).is_some(),
            "valid boundary {reset}"
        );
    }
}
