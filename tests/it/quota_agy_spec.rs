//! Independent acceptance contract: only an explicit quota command may run agy.
//! Every child has a scratch HOME and a PATH containing only the fixture shim.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance fixtures write owned scratch files, never operator state"
)]

use std::os::unix::fs::PermissionsExt as _;
use std::time::{Duration, Instant};

const AGY: &str = "[profiles]\nagy-one = agy\n";
const TWO_AGY: &str = "[profiles]\nagy-one = agy --model gemini\nagy-two = agy --model claude\n";
const PAYLOAD: &str = r#"{
  "status":"SUCCESS", "num_turns":0, "response":"VENDOR_RESPONSE_SENTINEL",
  "command":{"name":"usage","data":{"groups":[
    {"name":"Gemini Models","buckets":[
      {"id":"gemini-5h","window":"5h","remaining_fraction":0.75,"reset_time":"2099-01-01T00:00:00Z"},
      {"id":"gemini-weekly","window":"weekly","remaining_fraction":1,"reset_time":"2099-01-02T00:00:00Z"}]},
    {"name":"Claude and GPT models","buckets":[
      {"id":"3p-5h","window":"5h","remaining_fraction":0.5,"reset_time":"2099-01-03T00:00:00Z"},
      {"id":"3p-weekly","window":"weekly","remaining_fraction":0,"reset_time":"2099-01-04T00:00:00Z"}]}
  ]}}
}"#;

struct Rig {
    root: super::cli::OwnedScratch,
}

impl Rig {
    fn new(tag: &str, config: &str, version: &str, quota_body: &str) -> Self {
        let root = super::cli::OwnedScratch::root("qa", tag);
        std::fs::create_dir(root.join("bin")).expect("scratch PATH");
        std::fs::create_dir_all(root.join(".gemini/antigravity-cli")).expect("scratch agy home");
        std::fs::write(root.join("config"), config).expect("scratch config");
        std::fs::write(root.join("payload"), PAYLOAD).expect("canned quota");
        std::fs::write(root.join("argv"), "").expect("empty argv receipt");
        // Generated at test time, never a checked-in shell file. Exact argv is
        // part of the oracle: a stray prompt or argument must fail loudly.
        let shim = format!(
            "#!/bin/sh\n\
             case \"$0\" in */*) agy_root=${{0%/*}}/.. ;; *) exit 98 ;; esac\n\
             agy_payload=\"$agy_root/payload\"\n\
             agy_pid=\"$agy_root/pid\"\n\
             printf '%s\\n' \"$*\" >> \"$agy_root/argv\"\n\
             if [ \"$#\" -eq 1 ] && [ \"$1\" = --version ]; then\n\
               printf '%s\\n' '{version}'\n\
               exit 0\n\
             fi\n\
             if [ \"$#\" -ne 4 ] || [ \"$1\" != -p ] || [ \"$2\" != /quota ] || \
                [ \"$3\" != --output-format ] || [ \"$4\" != json ]; then\n\
               printf '%s\\n' UNEXPECTED_ARGV >&2\n\
               exit 97\n\
             fi\n\
             {quota_body}\n"
        );
        let path = root.join("bin/agy");
        std::fs::write(&path, shim).expect("agy shim");
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
            .expect("executable shim");
        Self { root }
    }

    fn run(&self) -> String {
        let output = super::cli::ae()
            .env("HOME", self.root.path())
            .env("AE_HOME", self.root.join("state"))
            .env("CONFIG_FILE", self.root.join("config"))
            .env("PATH", self.root.join("bin"))
            .current_dir(self.root.path())
            .arg("quota")
            .output()
            .expect("hermetic ae quota child");
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert!(output.stderr.is_empty(), "{output:?}");
        let text = String::from_utf8(output.stdout).expect("UTF-8 quota table");
        assert!(text.starts_with("PROFILES"), "{text}");
        assert!(
            text.contains("OBSERVED") && text.contains("STATUS"),
            "{text}"
        );
        assert!(!text.contains("VENDOR_RESPONSE_SENTINEL"), "{text}");
        assert!(!text.contains("VENDOR_STDERR_SENTINEL"), "{text}");
        text
    }

    fn argv(&self) -> String {
        std::fs::read_to_string(self.root.join("argv")).expect("shim argv receipt")
    }

    fn payload(&self, value: &str) {
        std::fs::write(self.root.join("payload"), value).expect("quota scenario");
    }
}

fn assert_call(rig: &Rig) {
    assert_eq!(rig.argv(), "--version\n-p /quota --output-format json\n");
}

#[test]
fn explicit_quota_renders_four_live_windows_with_resets_and_observed_age() {
    let rig = Rig::new("live", AGY, "1.2.14", "exec /bin/cat \"$agy_payload\"");
    let text = rig.run();
    for (bucket, window, used) in [
        ("gemini-5h", "5h", "25%"),
        ("gemini-weekly", "7d", "0%"),
        ("3p-5h", "5h", "50%"),
        ("3p-weekly", "7d", "100%"),
    ] {
        let line = text
            .lines()
            .find(|line| line.contains(bucket))
            .expect("bucket row");
        let words: Vec<_> = line.split_whitespace().collect();
        for expected in [window, used, "in", "0m", "ago", "fresh"] {
            assert!(
                words.contains(&expected),
                "{bucket}: missing {expected}: {text}"
            );
        }
        assert_eq!(text.matches(bucket).count(), 1, "{text}");
    }
    // Read the public column, including continuation lines: model-family
    // names may wrap independently of the numeric cells beside them.
    let header = text.lines().next().expect("table header");
    let bucket_start = header.find("BUCKET").expect("bucket column");
    let bucket_width = header.find("WINDOW").expect("window column") - bucket_start;
    let buckets = text
        .lines()
        .skip(1)
        .map(|line| {
            line.chars()
                .skip(bucket_start)
                .take(bucket_width)
                .collect::<String>()
                .trim()
                .to_owned()
        })
        .collect::<Vec<_>>()
        .join(" ");
    assert!(buckets.contains("Gemini Models"), "{text}");
    assert!(buckets.contains("Claude and GPT models"), "{text}");
    assert_call(&rig);
}

#[test]
fn old_or_unparseable_versions_never_receive_a_quota_prompt() {
    for (tag, version) in [
        ("old", "1.1.9"),
        ("floor", "1.1.10"),
        ("trick", "1.1.9-1.1.11"),
        ("pre", "1.1.11-rc1"),
        ("junk", "release 1.2.14"),
    ] {
        let rig = Rig::new(tag, AGY, version, "exit 96");
        let text = rig.run();
        assert!(text.contains("unsupported"), "{text}");
        // Notes wrap; whitespace normalization preserves the required words.
        let normalized = text.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(normalized.contains("agy 1.1.11+ required"), "{text}");
        assert_eq!(rig.argv(), "--version\n", "unsafe prompt for {version}");
    }
}

#[test]
fn minimum_supported_version_runs_quota() {
    let rig = Rig::new("min", AGY, "1.1.11", "exec /bin/cat \"$agy_payload\"");
    assert!(rig.run().contains("gemini-5h"));
    assert_call(&rig);
}

/// Preservation pin: main already renders unsupported when agy is absent.
#[test]
fn absent_agy_preserves_the_table_and_an_unsupported_hint() {
    let rig = Rig::new("absent", AGY, "1.2.14", "exit 96");
    std::fs::remove_file(rig.root.join("bin/agy")).expect("empty PATH");
    let text = rig.run();
    assert!(
        text.contains("unsupported") && text.contains("agy"),
        "{text}"
    );
    assert!(text.contains("/quota"), "{text}");
    assert!(rig.argv().is_empty());
}

#[test]
fn a_deadline_returns_truncated_instead_of_waiting_for_the_vendor() {
    let rig = Rig::new(
        "hang",
        AGY,
        "1.2.14",
        "printf '%s\\n' \"$$\" > \"$agy_pid\"\nexec /bin/sleep 30",
    );
    let started = Instant::now();
    let text = rig.run();
    assert!(
        started.elapsed() >= Duration::from_secs(8),
        "vendor was refused before the promised 8 s deadline"
    );
    assert!(
        started.elapsed() < Duration::from_secs(15),
        "deadline was not enforced"
    );
    assert!(text.contains("truncated"), "{text}");
    assert!(
        text.contains("deadline") || text.contains("timed out"),
        "{text}"
    );
    let pid: u32 = std::fs::read_to_string(rig.root.join("pid"))
        .expect("shim reached quota call")
        .trim()
        .parse()
        .expect("shim process id");
    let processes = ae::procs::snapshot().expect("existing ps door must succeed");
    assert!(
        processes.iter().all(|process| process.pid != pid),
        "timed-out child {pid} survived or was not reaped"
    );
    assert_call(&rig);
}

#[test]
fn vendor_failure_or_garbage_is_read_error_without_echoing_stderr() {
    for (tag, body) in [
        ("fail", "printf '%s\\n' VENDOR_STDERR_SENTINEL >&2\nexit 7"),
        (
            "garbage",
            "printf '%s\\n' VENDOR_STDERR_SENTINEL >&2\nprintf '%s\\n' not-json",
        ),
    ] {
        let rig = Rig::new(tag, AGY, "1.2.14", body);
        assert!(rig.run().contains("read-error"));
        assert_call(&rig);
    }
}

#[test]
fn a_reported_agent_turn_is_refused() {
    let rig = Rig::new("turn", AGY, "1.2.14", "exec /bin/cat \"$agy_payload\"");
    rig.payload(&PAYLOAD.replace("\"num_turns\":0", "\"num_turns\":1"));
    let text = rig.run();
    assert!(text.contains("read-error"), "{text}");
    assert!(
        !text.contains("gemini-5h"),
        "an agent turn cannot supply quota: {text}"
    );
    assert_call(&rig);
}

#[test]
fn two_profiles_on_one_login_share_one_live_call() {
    let rig = Rig::new(
        "shared",
        TWO_AGY,
        "1.2.14",
        "exec /bin/cat \"$agy_payload\"",
    );
    let text = rig.run();
    assert!(
        text.contains("agy-one") && text.contains("agy-two"),
        "{text}"
    );
    assert_eq!(text.matches("gemini-5h").count(), 1, "{text}");
    assert_call(&rig);
}

/// Preservation pin: unrelated scopes must keep their existing local-only path.
#[test]
fn non_agy_scopes_do_not_start_any_agy_process() {
    let rig = Rig::new("other", "[profiles]\nother = gemini\n", "1.2.14", "exit 96");
    let text = rig.run();
    assert!(text.contains("gemini"), "{text}");
    assert!(rig.argv().is_empty());
}

#[test]
fn an_empty_successful_report_has_unknown_windows() {
    let rig = Rig::new("empty", AGY, "1.2.14", "exec /bin/cat \"$agy_payload\"");
    rig.payload(
        r#"{"status":"SUCCESS","num_turns":0,"command":{"name":"usage","data":{"groups":[]}}}"#,
    );
    let text = rig.run();
    assert!(text.contains("unknown"), "{text}");
    assert!(
        !text.contains("read-error") && !text.contains("unsupported"),
        "{text}"
    );
    assert_call(&rig);
}

#[test]
fn oversized_stdout_is_refused_without_echoing_vendor_bytes() {
    let rig = Rig::new("large", AGY, "1.2.14", "exec /bin/cat \"$agy_payload\"");
    rig.payload(&format!(
        "VENDOR_RESPONSE_SENTINEL{}",
        "x".repeat(64 * 1024)
    ));
    let text = rig.run();
    assert!(text.contains("read-error"), "{text}");
    assert_call(&rig);
}

/// Structural safety pin complements the driver's in-crate observe runtime pin.
/// Neither local observer may gain a path to the on-demand executor.
#[test]
fn local_observers_never_reference_the_on_demand_leg_or_run_wrapper() {
    let source = include_str!("../../src/quota.rs");
    for (start, end) in [
        ("pub(crate) fn observe(", "pub(crate) fn observe_full("),
        (
            "pub(crate) fn observe_full(",
            "pub(crate) fn quota_dialog_rows(",
        ),
    ] {
        let body = source
            .split_once(start)
            .expect("local observer")
            .1
            .split_once(end)
            .expect("observer boundary")
            .0;
        for forbidden in ["run_agy_quota", "run_with(", "spawn_until("] {
            assert!(!body.contains(forbidden), "{start} references {forbidden}");
        }
    }
}
