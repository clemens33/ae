//! Frozen goquota acceptance: `ae quota` reads `OpenCode` Go usage.
//!
//! Oracle: `.local/brief-goquota-build.md` invariants 1–8, lead rulings Q1–Q5
//! (2026-10-19), and `.local/goquota/spec-r7.md`. Never the code under test.
//!
//! Pins this file freezes:
//! * `[workspace] opencode_go_key_file` is global-only; a project `.ae/config`
//!   key is ignored.
//! * `quota = off` starts no Go network anywhere (watchdog, observe, operator
//!   `ae quota`). Observe never fetches. No Go-scoped profile never fetches.
//! * Key bytes never appear in cache, journal, errors, Debug, or stdout/stderr.
//! * `OpenRouter` / model-less opencode rows stay byte-identical to today's
//!   unsupported hint, pinned by profile (or / bare) after collapsing wrap.
//!   STATUS wraps at 20, so HINT is never one raw line. A `[clients]`
//!   opencode row is a separate Unsupported scope (`source_key` None never
//!   merges; owner None => PROFILES `-`) and is today's product.
//! * 401 / 403 / 429 / timeout notes name the class; `rate-limited` => used
//!   percent >= 100, spelled `100` when the vendor literal is lower.
//! * `launch_cmd::sole_flag_value` is the one lexer; the opencode adapter
//!   stays `ModelSpec::Unobserved`.
//! * `recorded_identity` stays `None` for opencode (named gap).
//! * Watchdog fetch is `go_usage::watchdog_due` (off / not aware / no Go
//!   scope / flying => no); `Flying` Drop-clears on panic.
//! * Rotated/swapped key identity is due inside 300 s; old samples do not
//!   render for the new key.
//! * Tests never touch the network: injected fetch, or `AE_TEST_GO_USAGE_URL`
//!   pointed at a refused loopback.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance owns private scratch files and reads their effects"
)]

use std::fmt::Write as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use ae::go_usage::{Failure, Flying, GoKey, watchdog_due};
use ae::launch_cmd::sole_flag_value;
use ae::quota::{self, Row, Status};
use ae::time::Timestamp;

const SECRET: &str = "SYNTH-GO-KEY-LEAK-TEST-9f3c";
const GO_FLAGS: &[&str] = &["--model", "-m"];
const HINT: &str = "local stats are cost history, not quota";
const ROLLING_RESET: &str = "2026-10-19T17:00:00+00:00";
const WEEKLY_RESET: &str = "2026-10-26T12:00:00Z";
const MONTHLY_RESET: &str = "2026-11-01T00:00:00+00:00";
const NOTE_401: &str = "key rejected (401): check opencode_go_key_file";
const NOTE_403: &str = "no Go subscription (403)";

fn epoch(text: &str) -> i64 {
    Timestamp::parse(text).expect("UTC fixture").epoch()
}

fn now() -> i64 {
    epoch("2026-10-19T12:00:00Z")
}

fn ok_body() -> String {
    format!(
        r#"{{"usage":{{"rolling":{{"status":"ok","percent":12,"resetsAt":"{ROLLING_RESET}"}},"weekly":{{"status":"ok","percent":40,"resetsAt":"{WEEKLY_RESET}"}},"monthly":{{"status":"ok","percent":7,"resetsAt":"{MONTHLY_RESET}"}}}}}}"#
    )
}

fn rate_limited_body() -> String {
    format!(
        r#"{{"usage":{{"rolling":{{"status":"rate-limited","percent":3,"resetsAt":"{ROLLING_RESET}"}},"weekly":{{"status":"ok","percent":40,"resetsAt":"{WEEKLY_RESET}"}},"monthly":{{"status":"ok","percent":7,"resetsAt":"{MONTHLY_RESET}"}}}}}}"#
    )
}

/// Collapse table wrap. STATUS is capped at 20, so HINT splits across lines
/// (`unsupported (local` / `stats are cost` / `history, not quota)`).
fn collapsed(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Profile `name` row plus continuation lines until the next line whose first
/// column is non-blank, then [`collapsed`].
fn profile_block(text: &str, name: &str) -> Option<String> {
    let lines: Vec<&str> = text.lines().collect();
    let start = lines
        .iter()
        .position(|line| line.split_whitespace().next() == Some(name))?;
    let mut end = start + 1;
    while end < lines.len() {
        let first_blank = lines[end].chars().next().is_none_or(char::is_whitespace);
        if !first_blank {
            break;
        }
        end += 1;
    }
    Some(collapsed(&lines[start..end].join("\n")))
}

/// Today's `[clients] opencode = opencode` row: owner None, `source_key` None,
/// so `absorb_resolved` never merges it. `profiles_label` spells empty as `-`.
fn clients_only_unsupported(text: &str) -> bool {
    profile_block(text, "-").is_some_and(|row| row.contains("unsupported"))
}

/// Resets far in the future so a child `ae quota` (wall clock) still reads Fresh.
fn live_body() -> String {
    r#"{"usage":{"rolling":{"status":"ok","percent":12,"resetsAt":"2099-01-01T00:00:00Z"},"weekly":{"status":"ok","percent":40,"resetsAt":"2099-01-08T00:00:00Z"},"monthly":{"status":"ok","percent":7,"resetsAt":"2099-02-01T00:00:00Z"}}}"#.to_owned()
}

struct Rig {
    root: super::cli::OwnedScratch,
    config: PathBuf,
    local_config: PathBuf,
    state: PathBuf,
    key_path: PathBuf,
}

impl Rig {
    fn new(tag: &str, config: &str) -> Self {
        let root = super::cli::OwnedScratch::root("gq", tag);
        let state = root.join("state");
        std::fs::create_dir_all(state.join("quota")).expect("state quota dir");
        std::fs::create_dir_all(root.join(".ae")).expect("project overlay dir");
        let config_path = root.join("config");
        std::fs::write(&config_path, config).expect("global config");
        Self {
            config: config_path,
            local_config: root.join(".ae/config"),
            state,
            key_path: root.join("go-usage.key"),
            root,
        }
    }

    fn path(&self) -> &Path {
        self.root.path()
    }

    fn state(&self) -> &Path {
        &self.state
    }

    fn key_path(&self) -> &Path {
        &self.key_path
    }

    fn cache_path(&self) -> PathBuf {
        self.state().join("quota/opencode-go.json")
    }

    fn write_key(&self, body: &str, mode: u32) {
        let path = self.key_path();
        std::fs::write(path, body).expect("key file");
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).expect("key mode");
    }

    fn global_with_key(tag: &str, profiles: &str) -> Self {
        let rig = Self::new(
            tag,
            &format!(
                "[workspace]\nopencode_go_key_file = $HOME/go-usage.key\n\
                 [clients]\nopencode = opencode\n\
                 [profiles]\n{profiles}"
            ),
        );
        rig.write_key(&format!("{SECRET}\n"), 0o600);
        rig
    }

    /// Operator `ae quota` in a child, with this scratch as HOME / `AE_HOME`.
    fn table(&self) -> String {
        let output = super::cli::ae()
            .env("HOME", self.path())
            .env("AE_HOME", self.state())
            .env("CONFIG_FILE", &self.config)
            .current_dir(self.path())
            .arg("quota")
            .output()
            .expect("hermetic ae quota");
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        let text = String::from_utf8(output.stdout).expect("UTF-8 table");
        let err = String::from_utf8_lossy(&output.stderr);
        assert!(!text.contains(SECRET), "key leaked on stdout: {text}");
        assert!(!err.contains(SECRET), "key leaked on stderr: {err}");
        text
    }

    fn refresh(
        &self,
        now: i64,
        fetch: impl FnOnce(&GoKey) -> Result<Vec<u8>, Failure>,
    ) -> ae::go_usage::Refreshed {
        ae::go_usage::refresh_with(self.state(), self.key_path(), now, fetch)
    }

    fn cache_exists(&self) -> bool {
        self.cache_path().is_file()
    }
}

fn assert_no_secret(bytes: &[u8]) {
    assert!(
        !bytes.windows(SECRET.len()).any(|w| w == SECRET.as_bytes()),
        "key bytes present in dump"
    );
}

fn row<'a>(rows: &'a [Row], bucket: &str) -> &'a Row {
    rows.iter()
        .find(|row| row.bucket == bucket)
        .unwrap_or_else(|| panic!("missing bucket {bucket}"))
}

fn product_src(name: &str) -> String {
    std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(name)).expect(name)
}

// --- parser ---------------------------------------------------------------

#[test]
fn parse_ok_body_yields_rolling_weekly_monthly_with_stated_lengths() {
    let parsed = ae::quota::go::parse(ok_body().as_bytes(), now(), now());
    assert!(parsed.notes.is_empty(), "{:?}", parsed.notes);
    assert_eq!(parsed.rows.len(), 3);
    let rolling = row(&parsed.rows, "rolling");
    assert_eq!(rolling.window_minutes, Some(300));
    assert_eq!(rolling.used_percent.as_deref(), Some("12"));
    assert_eq!(rolling.qualifier, None);
    assert_eq!(rolling.status, Status::Fresh);
    assert_eq!(rolling.observed_at, Some(now()));
    assert_eq!(rolling.resets_at, Some(epoch("2026-10-19T17:00:00Z")));
    assert!(!quota::proven_long(rolling.window_minutes));
    let weekly = row(&parsed.rows, "weekly");
    assert_eq!(weekly.window_minutes, Some(10_080));
    assert_eq!(weekly.used_percent.as_deref(), Some("40"));
    assert!(quota::proven_long(weekly.window_minutes));
    let monthly = row(&parsed.rows, "monthly");
    assert_eq!(
        monthly.window_minutes, None,
        "calendar month length unstated"
    );
    assert_eq!(monthly.used_percent.as_deref(), Some("7"));
    assert!(!quota::proven_long(monthly.window_minutes));
}

#[test]
fn rate_limited_status_floors_used_percent_at_100_and_notes_the_bucket() {
    let parsed = ae::quota::go::parse(rate_limited_body().as_bytes(), now(), now());
    let rolling = row(&parsed.rows, "rolling");
    let used: f64 = rolling
        .used_percent
        .as_deref()
        .expect("percent")
        .parse()
        .expect("numeric");
    assert!(
        used >= 100.0,
        "rate-limited must read at/over the limit: {used}"
    );
    assert_eq!(
        rolling.used_percent.as_deref(),
        Some("100"),
        "vendor 3 floors to the spelling 100"
    );
    assert!(
        parsed
            .notes
            .iter()
            .any(|note| note.contains("opencode go rolling: vendor says rate-limited")),
        "{:?}",
        parsed.notes
    );
}

#[test]
fn offset_plus0200_converts_to_utc_epoch() {
    let body = r#"{"usage":{"rolling":{"status":"ok","percent":1,"resetsAt":"2026-10-19T15:00:00+02:00"},"weekly":{"status":"ok","percent":1,"resetsAt":"2026-10-26T12:00:00Z"},"monthly":{"status":"ok","percent":1,"resetsAt":"2026-11-01T00:00:00Z"}}}"#;
    let parsed = ae::quota::go::parse(body.as_bytes(), now(), now());
    assert_eq!(
        row(&parsed.rows, "rolling").resets_at,
        Some(epoch("2026-10-19T13:00:00Z"))
    );
}

#[test]
fn fractional_zulu_resets_at_is_accepted() {
    let body = r#"{"usage":{"rolling":{"status":"ok","percent":1,"resetsAt":"2026-10-19T17:00:00.5Z"},"weekly":{"status":"ok","percent":1,"resetsAt":"2026-10-26T12:00:00Z"},"monthly":{"status":"ok","percent":1,"resetsAt":"2026-11-01T00:00:00Z"}}}"#;
    let parsed = ae::quota::go::parse(body.as_bytes(), now(), now());
    assert_eq!(
        row(&parsed.rows, "rolling").resets_at,
        Some(epoch("2026-10-19T17:00:00Z"))
    );
}

#[test]
fn unknown_status_drops_that_window_and_names_the_bucket() {
    let body = r#"{"usage":{"rolling":{"status":"weird","percent":1,"resetsAt":"2026-10-19T17:00:00Z"},"weekly":{"status":"ok","percent":1,"resetsAt":"2026-10-26T12:00:00Z"},"monthly":{"status":"ok","percent":1,"resetsAt":"2026-11-01T00:00:00Z"}}}"#;
    let parsed = ae::quota::go::parse(body.as_bytes(), now(), now());
    assert!(parsed.rows.iter().all(|row| row.bucket != "rolling"));
    assert_eq!(parsed.rows.len(), 2);
    assert!(
        parsed.notes.iter().any(|note| note.contains("rolling")),
        "{:?}",
        parsed.notes
    );
}

#[test]
fn percent_string_or_missing_usage_or_bad_resets_at_never_invents_a_number() {
    for body in [
        r#"{"usage":{"rolling":{"status":"ok","percent":"hot","resetsAt":"2026-10-19T17:00:00Z"}}}"#,
        r#"{"nope":true}"#,
        r#"{"usage":{"rolling":{"status":"ok","percent":1,"resetsAt":"tomorrow"}}}"#,
        r"not json",
        r"[]",
    ] {
        let parsed = ae::quota::go::parse(body.as_bytes(), now(), now());
        assert!(
            parsed
                .rows
                .iter()
                .all(|row| row.used_percent.is_none() || row.bucket != "rolling"),
            "invented a number from {body}: {:?}",
            parsed.rows
        );
        assert!(!parsed.notes.is_empty(), "hostile {body} needs a note");
    }
}

#[test]
fn extra_junk_beside_usage_is_ignored_when_windows_are_readable() {
    let mut body = ok_body();
    body.insert_str(1, r#""junk":{"nested":[1,2,3]},"#);
    let parsed = ae::quota::go::parse(body.as_bytes(), now(), now());
    assert_eq!(parsed.rows.len(), 3);
}

#[test]
fn all_three_unreadable_is_one_read_error_placeholder_plus_a_note() {
    let parsed = ae::quota::go::parse(br#"{"usage":{}}"#, now(), now());
    assert_eq!(parsed.rows.len(), 1);
    assert_eq!(parsed.rows[0].status, Status::ReadError);
    assert_eq!(parsed.rows[0].bucket, "-");
    assert!(!parsed.notes.is_empty());
}

// --- sole_flag_value (no adapter change) ---------------------------------

#[test]
fn sole_flag_value_reads_model_without_changing_the_opencode_adapter() {
    assert_eq!(
        sole_flag_value("opencode --model opencode-go/deepseek-v4.1-flash", GO_FLAGS).as_deref(),
        Some("opencode-go/deepseek-v4.1-flash")
    );
    assert_eq!(
        sole_flag_value("opencode --model=opencode-go/glm", GO_FLAGS).as_deref(),
        Some("opencode-go/glm")
    );
    assert_eq!(
        sole_flag_value("opencode -m opencode-go/x", GO_FLAGS).as_deref(),
        Some("opencode-go/x")
    );
    assert_eq!(
        sole_flag_value("opencode --model openrouter/foo", GO_FLAGS).as_deref(),
        Some("openrouter/foo")
    );
    assert_eq!(sole_flag_value("opencode", GO_FLAGS), None);
    assert_eq!(
        sole_flag_value(
            "opencode --model opencode-go/a --model opencode-go/b",
            GO_FLAGS
        ),
        None,
        "ambiguous"
    );
    assert_eq!(sole_flag_value("opencode --model", GO_FLAGS), None);
    let tool_src = product_src("src/tool.rs");
    let block = tool_src
        .split("const OPENCODE:")
        .nth(1)
        .expect("OPENCODE adapter")
        .split("\nconst ")
        .next()
        .expect("adapter block");
    assert!(
        block.contains("model: ModelSpec::Unobserved"),
        "adapter must stay Unobserved so drift replay stays off"
    );
    assert!(
        !block.contains("Replayable") && !block.contains("ReportOnly"),
        "OPENCODE model spec changed: {block}"
    );
    assert!(
        block.contains("source: QuotaSource::Unsupported"),
        "OPENCODE quota source stays Unsupported: {block}"
    );
}

// --- recorded_identity named gap ----------------------------------------

#[test]
fn recorded_identity_stays_none_for_an_opencode_roster_seat() {
    let src = product_src("src/quota.rs");
    assert!(
        src.contains("QuotaSource::GrokLog | QuotaSource::Unsupported => return None"),
        "named gap: opencode stays on the Unsupported arm; no checkpoint ask / throttle / headroom reseat"
    );
}

// --- Failure is Copy -----------------------------------------------------

#[test]
fn go_failure_is_copy_and_cannot_hold_the_key() {
    fn assert_copy<T: Copy + Send + 'static>() {}
    assert_copy::<Failure>();
}

// --- refresh / cache / custody -------------------------------------------

#[test]
fn gokey_debug_redacts_and_refresh_never_writes_the_secret() {
    let rig = Rig::global_with_key("redact", "go = opencode --model opencode-go/x\n");
    let mut saw_debug = String::new();
    let refreshed = rig.refresh(now(), |key| {
        write!(&mut saw_debug, "{key:?}").expect("debug");
        Ok(ok_body().into_bytes())
    });
    assert!(refreshed.fetched);
    assert!(
        !saw_debug.contains(SECRET),
        "Debug leaked the key: {saw_debug}"
    );
    assert_eq!(saw_debug, "GoKey(<redacted>)");
    let cache = std::fs::read(rig.cache_path()).expect("cache");
    assert_no_secret(&cache);
    let text = String::from_utf8_lossy(&cache);
    assert!(text.contains("\"v\":1") || text.contains("\"v\": 1"));
    assert!(!text.contains(SECRET));
}

#[test]
fn no_key_configured_never_writes_a_cache_and_matches_today() {
    let rig = Rig::new(
        "nokey",
        "[clients]\nopencode = opencode\n[profiles]\nopencode = opencode\n",
    );
    let text = collapsed(&rig.table());
    assert!(
        text.contains("unsupported") && text.contains(HINT),
        "{text}"
    );
    assert!(!rig.cache_exists(), "opt-in: no key, no cache write");
}

#[test]
fn local_config_key_is_ignored_and_does_not_fetch() {
    let rig = Rig::new(
        "localkey",
        "[clients]\nopencode = opencode\n[profiles]\ngo = opencode --model opencode-go/x\n",
    );
    std::fs::write(
        &rig.local_config,
        "[workspace]\nopencode_go_key_file = $HOME/go-usage.key\n",
    )
    .expect("local overlay");
    rig.write_key(SECRET, 0o600);
    let text = rig.table();
    assert!(text.contains("unsupported"), "{text}");
    assert!(
        !rig.cache_exists(),
        "project .ae/config must not vote; a fetch attempt writes the cache"
    );
}

#[test]
fn mode_0644_refuses_with_chmod_600_and_does_not_fetch() {
    let rig = Rig::global_with_key("insecure", "go = opencode --model opencode-go/x\n");
    rig.write_key(SECRET, 0o644);
    let calls = AtomicUsize::new(0);
    let refreshed = rig.refresh(now(), |_| {
        calls.fetch_add(1, Ordering::SeqCst);
        Ok(ok_body().into_bytes())
    });
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(!refreshed.fetched);
    assert_eq!(refreshed.outcome, Some(Failure::KeyInsecure));
    let text = rig.table();
    assert!(text.contains("chmod 600"), "{text}");
    assert!(text.contains("go-usage.key"), "path named: {text}");
    assert!(!text.contains(SECRET), "{text}");
    assert!(
        !rig.cache_exists()
            || !std::fs::read(rig.cache_path())
                .is_ok_and(|bytes| String::from_utf8_lossy(&bytes).contains("\"outcome\":\"ok\"")),
        "insecure key must not produce an ok fetch"
    );
}

#[test]
fn fifo_or_directory_key_is_refused_without_fetch() {
    let rig = Rig::global_with_key("notreg", "go = opencode --model opencode-go/x\n");
    let path = rig.key_path();
    std::fs::remove_file(path).expect("clear key");
    std::fs::create_dir(path).expect("dir where a key belongs");
    let calls = AtomicUsize::new(0);
    let refreshed = rig.refresh(now(), |_| {
        calls.fetch_add(1, Ordering::SeqCst);
        Ok(ok_body().into_bytes())
    });
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(refreshed.outcome, Some(Failure::KeyNotRegular));
}

#[test]
fn non_printable_or_overlong_or_empty_key_is_refused() {
    for (tag, body, want) in [
        ("empty", "  \n".to_owned(), Failure::KeyEmpty),
        ("ctrl", format!("{SECRET}\x01"), Failure::KeyNotPrintable),
        ("long", "a".repeat(4097), Failure::KeyTooLong),
    ] {
        let rig = Rig::global_with_key(tag, "go = opencode --model opencode-go/x\n");
        rig.write_key(&body, 0o600);
        let calls = AtomicUsize::new(0);
        let refreshed = rig.refresh(now(), |_| {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok(vec![])
        });
        assert_eq!(calls.load(Ordering::SeqCst), 0, "{tag} fetched");
        assert_eq!(refreshed.outcome, Some(want), "{tag}");
    }
}

#[test]
fn fresh_cache_same_key_identity_skips_fetch() {
    let rig = Rig::global_with_key("fresh", "go = opencode --model opencode-go/x\n");
    let first = rig.refresh(now(), |_| Ok(ok_body().into_bytes()));
    assert!(first.fetched);
    let calls = AtomicUsize::new(0);
    let second = rig.refresh(now() + 60, |_| {
        calls.fetch_add(1, Ordering::SeqCst);
        Ok(vec![])
    });
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(!second.fetched);
}

#[test]
fn stale_cache_past_300s_fetches_once() {
    let rig = Rig::global_with_key("stale", "go = opencode --model opencode-go/x\n");
    assert!(rig.refresh(now(), |_| Ok(ok_body().into_bytes())).fetched);
    let calls = AtomicUsize::new(0);
    let second = rig.refresh(now() + 301, |_| {
        calls.fetch_add(1, Ordering::SeqCst);
        Ok(ok_body().into_bytes())
    });
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(second.fetched);
}

#[test]
fn lock_held_skips_fetch() {
    let rig = Rig::global_with_key("locked", "go = opencode --model opencode-go/x\n");
    let lock_path = rig.state().join("quota/.opencode-go.lock");
    std::fs::create_dir_all(lock_path.parent().expect("dir")).expect("quota dir");
    let lock = std::fs::File::create(&lock_path).expect("lock file");
    lock.lock().expect("hold lock");
    let calls = AtomicUsize::new(0);
    let refreshed = rig.refresh(now(), |_| {
        calls.fetch_add(1, Ordering::SeqCst);
        Ok(ok_body().into_bytes())
    });
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(!refreshed.fetched);
    drop(lock);
}

#[test]
fn failure_keeps_last_good_sample_advances_attempt_at_and_names_the_class() {
    let rig = Rig::global_with_key("keep", "go = opencode --model opencode-go/x\n");
    assert!(rig.refresh(now(), |_| Ok(ok_body().into_bytes())).fetched);
    for (offset, fail, slug) in [
        (301, Failure::Unauthorized, "unauthorized"),
        (602, Failure::NotEntitled, "not-entitled"),
        (903, Failure::Throttled, "throttled"),
        (1204, Failure::Timeout, "timeout"),
    ] {
        let refreshed = rig.refresh(now() + offset, |_| Err(fail));
        assert_eq!(refreshed.outcome, Some(fail), "{slug}");
        let cache = ae::quota::go::parse_cache(&std::fs::read(rig.cache_path()).expect("cache"))
            .expect("cache parses");
        assert_eq!(cache.attempt_at, now() + offset, "{slug}");
        assert!(!cache.samples.is_empty(), "{slug}: last good dropped");
        assert_eq!(cache.outcome, slug, "{slug}");
        assert_no_secret(&std::fs::read(rig.cache_path()).expect("cache"));
    }
}

#[test]
fn body_over_64_kib_is_too_large_and_is_not_parsed_as_usage() {
    let rig = Rig::global_with_key("huge", "go = opencode --model opencode-go/x\n");
    let huge = vec![b'x'; 64 * 1024 + 1];
    let refreshed = rig.refresh(now(), |_| Ok(huge));
    assert_eq!(refreshed.outcome, Some(Failure::TooLarge));
}

// --- operator table ------------------------------------------------------

#[test]
fn go_profile_plus_key_plus_fresh_cache_shows_three_windows() {
    let rig = Rig::global_with_key(
        "windows",
        "go = opencode --model opencode-go/deepseek-v4.1-flash\n",
    );
    let wall = Timestamp::now().epoch();
    assert!(rig.refresh(wall, |_| Ok(live_body().into_bytes())).fetched);
    let text = rig.table();
    assert!(
        text.contains("rolling") && text.contains("5h") && text.contains("12%"),
        "{text}"
    );
    assert!(
        text.contains("weekly") && text.contains("7d") && text.contains("40%"),
        "{text}"
    );
    assert!(
        text.contains("monthly") && text.contains("7%"),
        "monthly bucket+percent: {text}"
    );
    let monthly = text
        .lines()
        .find(|line| line.contains("monthly"))
        .expect("monthly row");
    let words: Vec<_> = monthly.split_whitespace().collect();
    assert!(
        words.contains(&"-"),
        "WINDOW for unstated monthly length is -: {monthly}"
    );
    let go = profile_block(&text, "go").expect("go profile row");
    assert!(
        !go.contains(HINT) && !go.contains("unsupported"),
        "Go scope must not keep the unsupported hint: {go}"
    );
    assert!(
        clients_only_unsupported(&text),
        "bare [clients] row is today's product (PROFILES -): {text}"
    );
}

#[test]
fn openrouter_on_the_same_client_stays_byte_identical_to_today() {
    let rig = Rig::global_with_key(
        "router",
        "go = opencode --model opencode-go/x\n\
         or = opencode --model openrouter/foo\n\
         bare = opencode\n",
    );
    let wall = Timestamp::now().epoch();
    assert!(rig.refresh(wall, |_| Ok(live_body().into_bytes())).fetched);
    let text = rig.table();
    let or = profile_block(&text, "or").expect("or profile row");
    let bare = profile_block(&text, "bare").expect("bare profile row");
    let go = profile_block(&text, "go").expect("go profile row");
    assert!(
        or.contains("unsupported") && or.contains(HINT),
        "OpenRouter stays unsupported: {or}"
    );
    assert!(
        bare.contains("unsupported") && bare.contains(HINT),
        "model-less stays unsupported: {bare}"
    );
    assert!(
        !go.contains("unsupported"),
        "Go must not stay unsupported: {go}"
    );
    assert!(
        clients_only_unsupported(&text),
        "bare [clients] opencode row is today's product (PROFILES -); do not hide it: {text}"
    );
}

#[test]
fn two_go_profiles_on_two_labels_are_one_scope() {
    let rig = Rig::new(
        "twolabels",
        "[workspace]\nopencode_go_key_file = $HOME/go-usage.key\n\
         [clients]\noc = opencode\noc2 = opencode\n\
         [profiles]\ngoalpha = oc --model opencode-go/x\ngobeta = oc2 --model opencode-go/y\n",
    );
    rig.write_key(SECRET, 0o600);
    let wall = Timestamp::now().epoch();
    assert!(rig.refresh(wall, |_| Ok(live_body().into_bytes())).fetched);
    let text = rig.table();
    assert_eq!(
        text.matches("rolling").count(),
        1,
        "one Go account, one rolling row: {text}"
    );
    assert!(
        text.contains("goalpha") && text.contains("gobeta"),
        "both profiles on the one scope PROFILES cell: {text}"
    );
}

#[test]
fn no_go_profile_does_not_fetch_even_with_a_key() {
    let rig = Rig::global_with_key("nogo", "or = opencode --model openrouter/foo\n");
    let text = collapsed(&rig.table());
    assert!(
        text.contains("unsupported") && text.contains(HINT),
        "{text}"
    );
    assert!(
        !rig.cache_exists(),
        "key without a Go-scoped profile must not hit the network"
    );
}

#[test]
fn model_opencode_go_without_slash_is_not_a_go_scope() {
    let rig = Rig::global_with_key("noslash", "go = opencode --model opencode-go\n");
    let text = rig.table();
    assert!(text.contains("unsupported"), "{text}");
    assert!(
        !rig.cache_exists(),
        "opencode-go without a slash is not opencode-go/<model>"
    );
}

#[test]
fn quota_off_writes_no_go_cache() {
    let rig = Rig::new(
        "off",
        "[workspace]\nquota = off\nopencode_go_key_file = $HOME/go-usage.key\n\
         [clients]\nopencode = opencode\n\
         [profiles]\ngo = opencode --model opencode-go/x\n",
    );
    rig.write_key(SECRET, 0o600);
    let _text = rig.table();
    assert!(
        !rig.cache_exists(),
        "quota=off => zero Go network; a fetch attempt writes the cache"
    );
}

#[test]
fn failure_notes_name_401_403_429_and_timeout_and_keep_last_good_rows() {
    // One Rig per class. A second refresh at the same wall is not due
    // (spec §5: now-attempt_at in 0..300), so a shared Rig would leave
    // iter 2+ with outcome from iter 1 forever. table() is a child on
    // the wall clock: the fail's attempt_at must sit inside 0..300 of
    // that clock or the child is due and overwrites via loopback :9.
    let wall = Timestamp::now().epoch();
    for (tag, fail, needle) in [
        ("notes-401", Failure::Unauthorized, NOTE_401),
        ("notes-403", Failure::NotEntitled, NOTE_403),
        ("notes-429", Failure::Throttled, "429"),
        ("notes-timeout", Failure::Timeout, "timeout"),
    ] {
        let rig = Rig::global_with_key(tag, "go = opencode --model opencode-go/x\n");
        assert!(
            rig.refresh(wall.saturating_sub(301), |_| Ok(live_body().into_bytes()))
                .fetched,
            "{tag}"
        );
        let refreshed = rig.refresh(wall, |_| Err(fail));
        assert_eq!(refreshed.outcome, Some(fail), "{needle}");
        let text = rig.table();
        assert!(text.contains(needle), "missing {needle}: {text}");
        assert!(text.contains("12%"), "last good dropped: {text}");
        assert!(!text.contains(SECRET), "{text}");
    }
}

// --- observe stays network-free ------------------------------------------

#[test]
fn observe_never_calls_refresh_in_quota_rs() {
    let src = product_src("src/quota.rs");
    let product = src
        .split_once("#[cfg(test)]\nmod tests {")
        .map_or(src.as_str(), |(m, _)| m);
    assert!(
        !product.contains("opencode.ai"),
        "quota.rs must not name the vendor URL"
    );
    // observe / observe_full / observe_scopes sit before `pub fn run`.
    // They read the cache only. `run_with` may call go_usage::refresh_with
    // inline (operator path, quota on).
    let observe = product
        .split_once("pub(crate) fn observe(")
        .expect("observe")
        .1
        .split_once("pub fn run(")
        .expect("run after observe")
        .0;
    assert!(
        !observe.contains("go_usage::refresh"),
        "observe family must not fetch; run_with may: {observe}"
    );
}

#[test]
fn watchdog_due_is_the_one_fetch_decision() {
    assert!(!watchdog_due(false, true, true, false), "quota=off");
    assert!(!watchdog_due(true, false, true, false), "not quota-aware");
    assert!(!watchdog_due(true, true, false, false), "no Go scope");
    assert!(!watchdog_due(true, true, true, true), "flying");
    assert!(
        watchdog_due(true, true, true, false),
        "quota on, aware, Go scope, idle"
    );
}

#[test]
fn flying_guard_clears_when_the_closure_panics() {
    let flag = AtomicBool::new(false);
    let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _guard = Flying::arm(&flag);
        assert!(flag.load(Ordering::SeqCst));
        panic!("go-flight-test");
    }));
    assert!(panicked.is_err());
    assert!(
        !flag.load(Ordering::SeqCst),
        "Drop must clear flying so a panic cannot stick the flag"
    );
}

#[test]
fn watchdog_refresh_is_gated_on_quota_aware_and_is_single_flight() {
    let src = product_src("src/watchdog_daemon.rs");
    let product = src
        .split_once("#[cfg(test)]\nmod tests {")
        .map_or(src.as_str(), |(m, _)| m);
    assert!(
        product.contains("watchdog_due"),
        "daemon must call go_usage::watchdog_due; the table owns the gate"
    );
    assert!(
        product.contains("thread::spawn") || product.contains("thread::Builder"),
        "single-flight Go refresh is a detached std::thread"
    );
}

#[test]
fn rotated_key_is_due_within_300s_and_old_samples_do_not_render() {
    let rig = Rig::global_with_key("rotate", "go = opencode --model opencode-go/x\n");
    // Fail at wall so table() (child wall clock) sees attempt_at in 0..300
    // and does not overwrite via loopback :9. t = wall-61 is still inside
    // the 300 s bound: without identity change this refresh would skip.
    let wall = Timestamp::now().epoch();
    let t = wall.saturating_sub(61);
    assert!(rig.refresh(t, |_| Ok(live_body().into_bytes())).fetched);
    rig.write_key(&format!("{SECRET}-ROT\n"), 0o600);
    let calls = AtomicUsize::new(0);
    let second = rig.refresh(t + 61, |_| {
        calls.fetch_add(1, Ordering::SeqCst);
        Err(Failure::Unauthorized)
    });
    assert_eq!(calls.load(Ordering::SeqCst), 1, "rotated key must be due");
    assert!(second.fetched);
    assert_eq!(second.outcome, Some(Failure::Unauthorized));
    let text = rig.table();
    assert!(
        !text.contains("12%"),
        "old-key percent must not render under the new key: {text}"
    );
    assert!(!text.contains(SECRET), "{text}");
}

#[test]
fn third_locked_agent_site_is_go_usage() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/go_usage.rs");
    let text = std::fs::read_to_string(&path).expect("src/go_usage.rs exists");
    assert!(text.contains("Agent::new_with_config"));
    assert!(text.contains("https_only"));
    assert!(text.contains("max_redirects"));
}

#[test]
fn hermetic_cli_pins_go_usage_url_to_refused_loopback() {
    let src = product_src("tests/it/cli.rs");
    assert!(
        src.contains("AE_TEST_GO_USAGE_URL"),
        "isolated env must pin the Go URL"
    );
    assert!(
        src.contains("http://127.0.0.1:9/"),
        "default must be a refused loopback, never opencode.ai"
    );
}
