//! Frozen contract: a statusline feeds quota silently, under its own Claude scope.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance fixtures inspect only process-owned scratch homes"
)]

use std::io::Seek as _;
use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
use std::path::{Path, PathBuf};

const RESET: i64 = 4_070_908_800; // 2099-01-01 UTC

struct Rig(super::cli::OwnedScratch);
impl Rig {
    fn new(tag: &str) -> Self {
        let rig = Self(super::cli::OwnedScratch::root("qf", tag));
        for dir in ["home/.claude", "home/.claude-mic", "state/sessions"] {
            std::fs::create_dir_all(rig.0.join(dir)).expect("fixture directories");
        }
        std::fs::write(
            rig.0.join("config"),
            concat!(
                "[clients]\nbase = claude manual_resets=1\n",
                "mic = claude config_home=$HOME/.claude-mic\n",
                "[profiles]\nbasep = base --model opus\n",
                "explicitp = CLAUDE_CONFIG_DIR=$HOME/.claude claude --model opus\nmicp = mic --model opus\n"
            ),
        )
        .expect("fixture config");
        rig
    }
    fn command(&self) -> super::cli::Runner {
        let mut command = super::cli::ae();
        command
            .env("HOME", self.0.join("home"))
            .env("AE_HOME", self.0.join("state"))
            .env("CONFIG_FILE", self.0.join("config"))
            .env_remove("CLAUDE_CONFIG_DIR")
            .current_dir(self.0.path());
        command
    }
    fn feed(&self, explicit: Option<&Path>, bytes: &[u8]) {
        self.feed_argv(explicit, bytes, &[]);
    }
    fn feed_argv(&self, explicit: Option<&Path>, bytes: &[u8], tail: &[&str]) {
        let path = self.0.join("stdin");
        std::fs::write(&path, bytes).expect("statusline payload");
        let mut input = std::fs::File::open(path).expect("statusline stdin");
        let mut command = self.command();
        if let Some(dir) = explicit {
            command.env("CLAUDE_CONFIG_DIR", dir);
        }
        let output = command
            .arg("_quota-feed")
            .args(tail)
            .stdin(input.try_clone().expect("shared input offset"))
            .output()
            .expect("feed child");
        assert_eq!(
            input.stream_position().expect("consumed bytes"),
            bytes.len() as u64,
            "every feed path must drain stdin to EOF"
        );
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert!(
            output.stdout.is_empty() && output.stderr.is_empty(),
            "{output:?}"
        );
    }
    fn record(&self, explicit: Option<&Path>) -> PathBuf {
        let home = std::fs::canonicalize(explicit.unwrap_or(&self.0.join("home")))
            .expect("canonical fixture home");
        let scope = format!("scope {}\n", home.join(".claude.json").display());
        std::fs::read_dir(self.0.join("state/quota-feed"))
            .expect("feed directory")
            .map(|entry| entry.expect("record entry").path())
            .find(|path| std::fs::read_to_string(path).is_ok_and(|body| body.contains(&scope)))
            .expect("one record for the exact scope")
    }
    fn at(&self, now: i64) -> String {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        assert_eq!(
            ae::quota::run(
                &ae::quota::Inputs {
                    home: Some(&self.0.join("home")),
                    global: Some(&self.0.join("config")),
                    local: None,
                    sessions: Some(&self.0.join("state/sessions")),
                    now,
                },
                &mut out,
                &mut err
            )
            .expect("quota read"),
            0
        );
        assert!(err.is_empty(), "{err:?}");
        String::from_utf8(out).expect("quota table")
    }
}

fn payload(session: &str, week: &str) -> Vec<u8> {
    format!(r#"{{"private":"STATUSLINE_SECRET","rate_limits":{{"five_hour":{{"used_percentage":{session},"resets_at":{RESET}}},"seven_day":{{"used_percentage":{week},"resets_at":{RESET}}}}}}}"#).into_bytes()
}
fn received(path: &Path) -> i64 {
    std::fs::read_to_string(path)
        .expect("record")
        .lines()
        .nth(2)
        .expect("received line")
        .strip_prefix("received ")
        .expect("received prefix")
        .parse()
        .expect("received epoch")
}
fn compact(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}
fn cache(path: &Path, at: i64) {
    std::fs::write(path, format!(r#"{{"cachedUsageUtilization":{{"fetchedAtMs":{},"utilization":{{"limits":[{{"kind":"session","percent":12,"resets_at":"2099-01-01T00:00:00Z"}},{{"kind":"weekly_all","percent":18,"resets_at":"2099-01-01T00:00:00Z"}},{{"kind":"weekly_scoped","percent":31,"scope":{{"model":{{"display_name":"Fable"}}}},"resets_at":"2099-01-01T00:00:00Z"}}]}}}}}}"#, at * 1000)).expect("vendor cache");
}

#[test]
fn fixture_configuration_is_valid_before_feed() {
    let rig = Rig::new("config");
    let text = rig.at(1_788_858_600);
    assert!(
        text.contains("basep") && text.contains("explicitp") && text.contains("micp"),
        "{text}"
    );
}

#[test]
fn feed_is_silent_private_and_the_existing_effective_derivation_reads_it() {
    let rig = Rig::new("live");
    rig.feed(None, &payload("42.5", "84"));
    let path = rig.record(None);
    let body = std::fs::read_to_string(&path).expect("record");
    assert!(body.starts_with("ae-quota-feed 1\nscope "));
    assert!(
        body.contains(&format!("window session 42.5 {RESET}\n")),
        "{body}"
    );
    assert!(
        !body.contains("STATUSLINE_SECRET"),
        "unrelated stdin must not persist"
    );
    assert_eq!(
        std::fs::metadata(&path)
            .expect("record mode")
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(
        std::fs::metadata(path.parent().expect("parent"))
            .expect("directory mode")
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    let text = compact(&rig.at(received(&path)));
    assert!(text.contains("session 5h 42.5% 21.2% x1"), "{text}");
    assert!(text.contains("weekly_all 7d 84% 42% x1"), "{text}");
    let at = received(&path);
    assert!(rig.at(at + 901).contains("stale"));
    let expired = rig.at(RESET);
    assert!(
        !expired.contains("42.5%") && expired.contains("statusline"),
        "{expired}"
    );
    assert!(
        !rig.0.join("home/.claude.json").exists(),
        "feed must not create vendor state"
    );
}

#[test]
fn implicit_explicit_and_canonical_alias_scopes_do_not_cross_accounts() {
    let rig = Rig::new("scopes");
    let default = rig.0.join("home/.claude");
    let mic = rig.0.join("home/.claude-mic");
    let alias = rig.0.join("home/alias");
    std::os::unix::fs::symlink(&mic, &alias).expect("scope alias");
    rig.feed(None, &payload("42", "43"));
    rig.feed(Some(&default), &payload("65", "66"));
    rig.feed(Some(&mic), &payload("72", "73"));
    rig.feed(Some(&alias), &payload("74", "75"));
    let path = rig.record(None);
    assert_eq!(
        std::fs::read_dir(rig.0.join("state/quota-feed"))
            .expect("records")
            .count(),
        3
    );
    assert_eq!(rig.record(Some(&alias)), rig.record(Some(&mic)));
    let text = rig.at(received(&path));
    for (profile, percent) in [("basep", "42%"), ("explicitp", "65%"), ("micp", "74%")] {
        let line = text
            .lines()
            .find(|line| line.contains(profile))
            .expect("scope row");
        assert!(line.contains(percent), "{profile}: {text}");
    }
}

#[test]
fn newer_timestamp_wins_per_window_and_model_scoped_cache_survives() {
    let rig = Rig::new("merge");
    rig.feed(None, &payload("79.999", "84"));
    let path = rig.record(None);
    let now = received(&path);
    assert!(
        std::fs::read_to_string(&path)
            .expect("precise record")
            .contains(&format!("window session 79.999 {RESET}\n"))
    );
    let vendor = rig.0.join("home/.claude.json");
    for (delta, expected) in [(-10, "79.99"), (0, "12%"), (10, "12%")] {
        cache(&vendor, now + delta);
        let text = rig.at(now);
        let line = text
            .lines()
            .find(|line| line.contains("session") && line.contains("5h"))
            .expect("session row");
        assert!(line.contains(expected), "delta={delta}: {text}");
        assert!(text.contains("Fable") && text.contains("31%"), "{text}");
    }
    cache(&vendor, now - 10);
    let before = std::fs::read(&vendor).expect("vendor bytes");
    rig.feed(None, &payload("81", "85"));
    assert_eq!(std::fs::read(&vendor).expect("vendor bytes"), before);
}

#[test]
fn unchanged_payload_is_throttled_but_changes_and_sixty_seconds_refresh() {
    let rig = Rig::new("throttle");
    let sample = payload("42", "43");
    rig.feed(None, &sample);
    let path = rig.record(None);
    let body = std::fs::read_to_string(&path).expect("record");
    let inode = std::fs::metadata(&path).expect("metadata").ino();
    rig.feed(None, &sample);
    assert_eq!(std::fs::read_to_string(&path).expect("record"), body);
    assert_eq!(std::fs::metadata(&path).expect("metadata").ino(), inode);
    let at = received(&path);
    std::fs::write(
        &path,
        body.replace(
            &format!("received {at}\n"),
            &format!("received {}\n", at - 60),
        ),
    )
    .expect("aged record");
    rig.feed(None, &sample);
    assert!(
        received(&path) >= at,
        "60 seconds must refresh observation time"
    );
    rig.feed(None, &payload("44", "45"));
    assert!(
        std::fs::read_to_string(&path)
            .expect("record")
            .contains("window session 44 ")
    );
}

#[test]
fn absent_malformed_over_cap_and_invalid_numbers_leave_last_record_untouched() {
    let rig = Rig::new("invalid");
    rig.feed(None, &payload("42", "43"));
    let path = rig.record(None);
    let before = std::fs::read(&path).expect("last good record");
    let huge = vec![b' '; 2 * 1024 * 1024];
    for bytes in [
        b"{}".as_slice(),
        b"{\"rate_limits\":null}",
        b"{",
        &huge,
        &payload("-1", "101"),
        &payload("\"42\"", "null"),
        b"{\"rate_limits\":{\"five_hour\":{\"used_percentage\":42,\"resets_at\":1.5}}}",
    ] {
        rig.feed(None, bytes);
        assert_eq!(std::fs::read(&path).expect("held record"), before);
    }
    for dir in [Path::new(""), Path::new("relative")] {
        rig.feed(Some(dir), &payload("99", "99"));
        assert_eq!(std::fs::read(&path).expect("held record"), before);
    }
    rig.feed_argv(None, &payload("99", "99"), &["unexpected"]);
    assert_eq!(std::fs::read(&path).expect("held record"), before);
}

#[test]
fn hostile_record_damage_never_becomes_quota_and_does_not_hide_the_cache() {
    let rig = Rig::new("hostile");
    rig.feed(None, &payload("97", "98"));
    let path = rig.record(None);
    let good = std::fs::read_to_string(&path).expect("record");
    let now = received(&path);
    cache(&rig.0.join("home/.claude.json"), now - 10);
    for damaged in [
        good.replace("ae-quota-feed 1", "ae-quota-feed 2"),
        good.replace("scope /", "scope /other/"),
        format!("{good}extra\n"),
        good.replace("window session 97", "window session 101"),
        "x".repeat(1025),
    ] {
        std::fs::write(&path, damaged).expect("hostile record");
        let text = rig.at(now);
        assert!(text.contains("12%") && !text.contains("97%"), "{text}");
    }
    std::fs::remove_file(&path).expect("remove planted record");
    let outside = rig.0.join("outside");
    std::fs::write(&outside, &good).expect("outside record");
    std::os::unix::fs::symlink(&outside, &path).expect("hostile record link");
    assert!(!rig.at(now).contains("97%"));
    std::fs::remove_file(&path).expect("remove link");
    std::fs::remove_dir(path.parent().expect("feed dir")).expect("empty feed dir");
    let outside_dir = rig.0.join("outside-dir");
    std::fs::create_dir(&outside_dir).expect("outside directory");
    std::fs::write(outside_dir.join(path.file_name().expect("filename")), &good)
        .expect("outside record");
    std::os::unix::fs::symlink(&outside_dir, path.parent().expect("feed dir"))
        .expect("hostile directory link");
    assert!(!rig.at(now).contains("97%"));
    rig.feed(None, &payload("99", "99"));
    assert_eq!(
        std::fs::read_to_string(outside_dir.join(path.file_name().expect("filename")))
            .expect("outside record"),
        good
    );
}
