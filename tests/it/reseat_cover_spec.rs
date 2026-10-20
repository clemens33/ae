//! Frozen contract: `ae doctor` names every profile that has no auto-reseat
//! exit off its own account, one WARN row per account, and never fails over it.
//!
//! The oracle is the brief — a profile is covered when a candidate leaves its
//! own account — and the fixtures are SYNTHETIC profiles in a scratch `HOME`.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance fixtures inspect only process-owned scratch homes"
)]

use std::path::PathBuf;

struct Rig(super::cli::OwnedScratch);

impl Rig {
    /// A global config with `profiles`, and auto reseat on or off.
    fn new(tag: &str, profiles: &str, on: bool) -> Self {
        let rig = Self(super::cli::OwnedScratch::root("rcv", tag));
        for dir in ["home", "state/sessions", "plain"] {
            std::fs::create_dir_all(rig.0.join(dir)).expect("fixture directories");
        }
        let switch = if on { "auto_reseat = on\n" } else { "" };
        std::fs::write(
            rig.0.join("config"),
            format!(
                "[profiles]\n{profiles}\n[roster]\nlead = sp\n\n[workspace]\nmain = lead\nworkers = \n{switch}\n[auto_reseat]\n"
            ),
        )
        .expect("fixture config");
        rig
    }

    fn config(&self) -> PathBuf {
        self.0.join("config")
    }

    /// Append one `[auto_reseat]` line, the last section of the file.
    fn add(&self, line: &str) {
        let mut text = std::fs::read_to_string(self.config()).expect("config");
        text.push_str(line);
        text.push('\n');
        std::fs::write(self.config(), text).expect("config with the line");
    }

    /// `ae doctor`: (exit code, stdout).
    fn doctor(&self) -> (Option<i32>, String) {
        let output = super::cli::ae()
            .env("HOME", self.0.join("home"))
            .env("AE_HOME", self.0.join("state"))
            .env_remove("CONFIG_FILE")
            .env_remove("CLAUDE_CONFIG_DIR")
            .current_dir(self.0.join("plain"))
            .arg(ae::cli::DOCTOR)
            .arg("--global")
            .arg(self.config())
            .output()
            .expect("doctor child");
        (
            output.status.code(),
            String::from_utf8_lossy(&output.stdout).into_owned(),
        )
    }

    fn home(&self) -> PathBuf {
        std::fs::canonicalize(self.0.join("home")).expect("canonical home")
    }
}

/// `(level, detail)` of every `reseat-cover` row, in order.
fn cover_rows(report: &str) -> Vec<(String, String)> {
    report
        .lines()
        .filter_map(|line| {
            let (level, tail) = line.split_once(char::is_whitespace)?;
            matches!(level, "OK" | "WARN" | "FAIL").then_some(())?;
            let tail = tail.trim_start();
            let (label, detail) = tail.split_once(char::is_whitespace)?;
            (label == "reseat-cover").then(|| (level.to_owned(), detail.trim().to_owned()))
        })
        .collect()
}

/// Every profile the WARN rows name, in row order.
fn named(rows: &[(String, String)]) -> Vec<String> {
    rows.iter()
        .filter(|(level, _)| level == "WARN")
        .flat_map(|(_, detail)| {
            detail
                .split_once("this account: ")
                .map(|(_, tail)| tail.split(" — ").next().unwrap_or_default())
                .unwrap_or_default()
                .split(", ")
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>()
        })
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

const SHARED: &str = "sp = muse --model spark\nwo = muse --model wonder\nso = codex --model sol\n";

#[test]
fn with_auto_reseat_off_the_row_reads_skipped() {
    let rig = Rig::new("off", SHARED, false);
    let (_, report) = rig.doctor();
    assert_eq!(
        cover_rows(&report),
        [("OK".to_owned(), "skipped (auto_reseat off)".to_owned())],
        "{report}"
    );
}

#[test]
fn profiles_with_no_exit_off_their_account_are_one_warn_row_per_account() {
    let rig = Rig::new("on", SHARED, true);
    let (_, report) = rig.doctor();
    let codex = rig.home().join(".codex");
    assert_eq!(
        cover_rows(&report),
        [
            (
                "WARN".to_owned(),
                "muse · shared: 2 profile(s) have no exit off this account: sp, wo — add under [auto_reseat], e.g. sp = so".to_owned()
            ),
            (
                "WARN".to_owned(),
                format!(
                    "codex · {}: 1 profile(s) have no exit off this account: so — add under [auto_reseat], e.g. so = sp, wo",
                    codex.display()
                )
            ),
        ],
        "{report}"
    );
}

/// `[auto_reseat]` is keyed by profile: the printed line covers the profile it
/// names and no neighbour, so the fleet is covered one line at a time.
#[test]
fn each_printed_line_covers_its_profile_alone_until_the_row_reads_ok() {
    let rig = Rig::new("loop", SHARED, true);
    let mut left = named(&cover_rows(&rig.doctor().1));
    assert_eq!(left, ["sp", "wo", "so"]);
    for _ in 0..4 {
        let (_, report) = rig.doctor();
        let rows = cover_rows(&report);
        if rows.len() == 1 && rows[0].0 == "OK" {
            assert_eq!(
                rows[0].1, "all 3 profile(s) have an exit off their own account",
                "{report}"
            );
            assert!(left.is_empty(), "{left:?}");
            return;
        }
        let line = rows
            .iter()
            .find_map(|(_, detail)| detail.split_once("e.g. ").map(|(_, line)| line.to_owned()))
            .expect("a WARN row carries the line to add");
        let covered = line.split(" = ").next().expect("a profile").to_owned();
        rig.add(&line);
        let now = named(&cover_rows(&rig.doctor().1));
        left.retain(|name| *name != covered);
        assert_eq!(
            now, left,
            "adding `{line}` drops {covered} and no neighbour on its account"
        );
    }
    let (_, report) = rig.doctor();
    assert_eq!(
        cover_rows(&report),
        [(
            "OK".to_owned(),
            "all 3 profile(s) have an exit off their own account".to_owned()
        )],
        "{report}"
    );
}

/// The same config with a row for EVERY profile reads covered at once.
#[test]
fn a_row_for_every_named_profile_reads_ok() {
    let rig = Rig::new("allrows", SHARED, true);
    for line in ["sp = so", "wo = so", "so = sp"] {
        rig.add(line);
    }
    let (_, report) = rig.doctor();
    assert_eq!(
        cover_rows(&report),
        [(
            "OK".to_owned(),
            "all 3 profile(s) have an exit off their own account".to_owned()
        )],
        "{report}"
    );
}

/// One row covers the profile it names and not its neighbour on the account.
#[test]
fn one_row_drops_that_name_and_the_count_by_one() {
    let rig = Rig::new("onerow", SHARED, true);
    let codex = rig.home().join(".codex");
    rig.add("sp = so");
    let (_, report) = rig.doctor();
    assert_eq!(
        cover_rows(&report),
        [
            (
                "WARN".to_owned(),
                "muse · shared: 1 profile(s) have no exit off this account: wo — add under [auto_reseat], e.g. wo = so".to_owned()
            ),
            (
                "WARN".to_owned(),
                format!(
                    "codex · {}: 1 profile(s) have no exit off this account: so — add under [auto_reseat], e.g. so = sp, wo",
                    codex.display()
                )
            ),
        ],
        "{report}"
    );
}

#[test]
fn a_command_ae_cannot_name_is_unproven_and_the_row_costs_the_exit_code_nothing() {
    let rig = Rig::new(
        "unproven",
        "sp = /bin/sh -c :\nso = codex --model sol\n",
        true,
    );
    let (on_code, on) = rig.doctor();
    let rows = cover_rows(&on);
    let own = rows
        .iter()
        .find(|(_, detail)| detail.contains("their own account is not provable"))
        .unwrap_or_else(|| panic!("a row names the unprovable account: {on}"));
    assert_eq!(own.0, "WARN");
    assert!(own.1.contains(": sp"), "{own:?}");
    assert!(
        own.1
            .contains("1 profile(s) cannot prove an exit off this account — their own account is not provable: sp"),
        "an unprovable account is its own row, never a no-exit claim: {own:?}"
    );
    assert!(
        rows.iter().any(|(level, detail)| level == "WARN"
            && detail.contains("have no exit off this account: so")),
        "a profile that is provably uncovered keeps its own row: {on}"
    );
    let off = Rig::new(
        "unproven-off",
        "sp = /bin/sh -c :\nso = codex --model sol\n",
        false,
    );
    let (off_code, off_report) = off.doctor();
    assert_eq!(
        (on_code, failures(&on)),
        (off_code, failures(&off_report)),
        "a WARN row costs the exit code and the failure count nothing"
    );
}
