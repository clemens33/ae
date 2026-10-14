//! Frozen contract: `ae doctor` names the two silent misconfigurations of a
//! fleet — a Claude quota scope nothing feeds, and a config file ae reads
//! beyond the global one — as report rows, and never fails over either.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance fixtures inspect only process-owned scratch homes"
)]

use std::path::{Path, PathBuf};

const RESET: i64 = 4_070_908_800; // 2099-01-01 UTC

struct Rig(super::cli::OwnedScratch);

impl Rig {
    fn new(tag: &str, clients: &str) -> Self {
        let rig = Self(super::cli::OwnedScratch::root("dfs", tag));
        for dir in ["home/.claude", "state/sessions", "plain", "overridden/.ae"] {
            std::fs::create_dir_all(rig.0.join(dir)).expect("fixture directories");
        }
        std::fs::write(
            rig.0.join("config"),
            format!(
                "[clients]\n{clients}\n[profiles]\nbasep = base --model opus\n\n[roster]\nlead = basep\n\n[workspace]\nmain = lead\nworkers = \n"
            ),
        )
        .expect("fixture config");
        std::fs::write(
            rig.0.join("overridden/.ae/config"),
            "[workspace]\nworkers = \n",
        )
        .expect("fixture override");
        rig
    }

    fn command(&self) -> super::cli::Runner {
        let mut command = super::cli::ae();
        command
            .env("HOME", self.0.join("home"))
            .env("AE_HOME", self.0.join("state"))
            .env_remove("CONFIG_FILE")
            .env_remove("CLAUDE_CONFIG_DIR");
        command
    }

    /// A SYNTHETIC statusline payload into this rig's scratch state only.
    fn feed(&self) {
        let payload = format!(
            r#"{{"rate_limits":{{"five_hour":{{"used_percentage":12,"resets_at":{RESET}}},"seven_day":{{"used_percentage":34,"resets_at":{RESET}}}}}}}"#
        );
        let path = self.0.join("stdin");
        std::fs::write(&path, payload).expect("statusline payload");
        let output = self
            .command()
            .current_dir(self.0.path())
            .arg("_quota-feed")
            .stdin(std::fs::File::open(path).expect("statusline stdin"))
            .output()
            .expect("feed child");
        assert_eq!(output.status.code(), Some(0), "{output:?}");
    }

    /// The one record the feed kept, so a test can age it.
    fn record(&self) -> PathBuf {
        let mut records = std::fs::read_dir(self.0.join("state/quota-feed"))
            .expect("feed directory")
            .map(|entry| entry.expect("record entry").path());
        let one = records.next().expect("one feed record");
        assert!(records.next().is_none(), "exactly one feed record");
        one
    }

    /// Re-stamp the record as received `age` seconds ago.
    fn age_record(&self, age: i64) {
        let path = self.record();
        let body = std::fs::read_to_string(&path).expect("record");
        let now = ae::time::Timestamp::now().epoch();
        let aged: Vec<String> = body
            .lines()
            .map(|line| {
                if line.starts_with("received ") {
                    format!("received {}", now - age)
                } else {
                    line.to_owned()
                }
            })
            .collect();
        std::fs::write(&path, aged.join("\n") + "\n").expect("aged record");
    }

    /// `ae doctor` run from `dir`: (exit code, stdout).
    fn doctor_in(&self, dir: &Path) -> (Option<i32>, String) {
        let output = self
            .command()
            .current_dir(dir)
            .arg(ae::cli::DOCTOR)
            .arg("--global")
            .arg(self.0.join("config"))
            .output()
            .expect("doctor child");
        (
            output.status.code(),
            String::from_utf8_lossy(&output.stdout).into_owned(),
        )
    }

    fn doctor(&self) -> (Option<i32>, String) {
        self.doctor_in(&self.0.join("plain"))
    }
}

/// `(level, label, detail)` of every report row, in order.
fn rows(report: &str) -> Vec<(String, String, String)> {
    report
        .lines()
        .filter_map(|line| {
            let (level, tail) = line.split_once(char::is_whitespace)?;
            matches!(level, "OK" | "WARN" | "FAIL").then_some(())?;
            let tail = tail.trim_start();
            let (label, detail) = tail.split_once(char::is_whitespace).unwrap_or((tail, ""));
            Some((level.to_owned(), label.to_owned(), detail.trim().to_owned()))
        })
        .collect()
}

/// Rows about a quota scope: the ones that name the feed command.
fn feed_rows(report: &str) -> Vec<(String, String, String)> {
    rows(report)
        .into_iter()
        .filter(|(_, _, detail)| detail.contains("_quota-feed"))
        .collect()
}

fn failures(report: &str) -> String {
    report
        .lines()
        .rev()
        .find(|line| line.starts_with("Summary:"))
        .and_then(|line| line.split(',').next())
        .unwrap_or_default()
        .to_owned()
}

#[test]
fn a_claude_scope_nothing_feeds_is_one_warning_row_naming_the_scope_and_the_fix() {
    let rig = Rig::new("nofeed", "base = claude");
    let (_, report) = rig.doctor();
    let found = feed_rows(&report);
    assert_eq!(found.len(), 1, "exactly one feed row: {report}");
    let (level, label, detail) = &found[0];
    assert_eq!(level, "WARN", "{report}");
    assert!(
        label.contains("base") || detail.contains("base"),
        "the row names the scope (the client label): {report}"
    );
    assert!(
        detail.contains("no reading") || detail.contains("no quota"),
        "the row says the reading is absent: {report}"
    );
}

#[test]
fn a_fresh_feed_leaves_no_row_and_the_command_exit_is_not_touched() {
    let rig = Rig::new("fed", "base = claude");
    let (bare_code, bare) = rig.doctor();
    rig.feed();
    let (fed_code, fed) = rig.doctor();
    assert!(
        feed_rows(&fed).is_empty(),
        "a fresh feed says nothing: {fed}"
    );
    assert_eq!(
        (bare_code, failures(&bare)),
        (fed_code, failures(&fed)),
        "the row costs the exit code and the failure count nothing"
    );
}

#[test]
fn a_feed_older_than_the_bound_is_named_with_its_age() {
    let rig = Rig::new("aged", "base = claude");
    rig.feed();
    rig.age_record(2 * 3600);
    let (_, report) = rig.doctor();
    let found = feed_rows(&report);
    assert_eq!(found.len(), 1, "{report}");
    assert_eq!(found[0].0, "WARN", "{report}");
    assert!(found[0].2.contains("2h"), "the age is stated: {report}");
    rig.age_record(30 * 60);
    let (_, report) = rig.doctor();
    assert!(
        feed_rows(&report).is_empty(),
        "half an hour is inside the bound: {report}"
    );
}

#[test]
fn a_client_ae_cannot_read_says_nothing_new() {
    let rig = Rig::new("opencode", "base = opencode");
    let (_, report) = rig.doctor();
    assert!(feed_rows(&report).is_empty(), "{report}");
}

#[test]
fn a_project_override_beside_the_caller_is_named_by_path() {
    let rig = Rig::new("override", "base = claude");
    let work = rig.0.join("overridden");
    let (_, report) = rig.doctor_in(&work);
    let want = std::fs::canonicalize(&work)
        .expect("canonical work dir")
        .join(".ae/config");
    let found: Vec<_> = rows(&report)
        .into_iter()
        .filter(|(_, label, _)| label == "local-config")
        .collect();
    assert_eq!(found.len(), 1, "one extra file, one row: {report}");
    assert_eq!(found[0].0, "WARN", "{report}");
    assert!(
        found[0].2.contains(&want.display().to_string()),
        "the row carries the path ae reads: {report}"
    );
}

#[test]
fn a_work_dir_with_no_override_adds_no_config_row() {
    let rig = Rig::new("plain", "base = claude");
    let (_, report) = rig.doctor();
    assert!(
        rows(&report)
            .iter()
            .all(|(_, label, _)| label != "local-config"),
        "no extra file, no row: {report}"
    );
}
