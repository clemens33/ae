//! Frozen R4 real-route transcripts, from requirements and base 43c76c5d.
//! Only external tmux is scripted; inventory, assembly and classification run.

#![allow(
    clippy::disallowed_methods,
    reason = "frozen acceptance fixtures write their own state and external tmux shim"
)]

use std::fs;
use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;

use super::cli::{OwnedScratch, ae};
use super::support::source_view;
use ae::json::{self, Value};

// Never contacts tmux: every accepted argv is logged, unknown argv fails.
// The second discovery answer contradicts the first in recovery mode.
const TMUX: &str = r#"#!/usr/bin/perl
use strict;
use warnings;
my @a = @ARGV;
shift @a if @a && $a[0] eq '-u';
my ($kind, $server) = splice(@a, 0, 2);
my $key = "$kind:$server";
my $op = shift @a // '';
open my $log, '>>', $ENV{R4_TRACE} or die $!;
if ($op eq 'display-message' && $a[-1] eq '#{socket_path}') {
    print $log "socket\t$key\n";
    exit 1 if $server eq '/r4/entry';
    if ($ENV{R4_MODE} eq 'alias') { print "/r4/live\n"; exit 0; }
    exit 1;
}
if ($op eq 'list-sessions') {
    print $log "sessions\t$key\n";
    close $log;
    my $mode = $ENV{R4_MODE};
    if ($mode eq 'alias' || $mode eq 'panes') {
        if ($server eq '/r4/live') { print "alpha\nbeta\n"; exit 0; }
        exit 1;
    }
    if ($mode eq 'recover' && $server eq '/r4/dead') {
        open my $seen, '<', $ENV{R4_TRACE} or die $!;
        my @seen = grep { $_ eq "sessions\t$key\n" } <$seen>;
        if (@seen > 1) { print "alpha\n"; exit 0; }
    }
    if ($mode eq 'boot' && $server eq '/r4/dead') {
        print STDERR "error connecting to /r4/dead (No such file or directory)\n";
    } else {
        print STDERR "permission denied\n";
    }
    exit 1;
}
if ($op eq 'show-environment') {
    print $log "marker\t$key\t$a[1]\n";
    print "AE_SESSION=1\n";
    exit 0;
}
if ($op eq 'list-panes') {
    print $log "panes\t$key\n";
    exit 1;
}
print STDERR "unexpected tmux argv: $kind $server $op @a\n";
exit 93;
"#;

struct Fixture {
    _scratch: OwnedScratch,
    home: PathBuf,
    bin: PathBuf,
    trace: PathBuf,
}

impl Fixture {
    fn new(tag: &str) -> Self {
        let scratch = OwnedScratch::root("r4", tag);
        let home = scratch.join("state");
        let bin = scratch.join("bin");
        let trace = scratch.join("trace");
        assert!(fs::create_dir_all(&home).is_ok());
        assert!(fs::create_dir_all(&bin).is_ok());
        assert!(fs::write(bin.join("tmux"), TMUX).is_ok());
        assert!(fs::set_permissions(bin.join("tmux"), fs::Permissions::from_mode(0o755)).is_ok());
        Self {
            _scratch: scratch,
            home,
            bin,
            trace,
        }
    }

    fn record(&self, name: &str, kind: &str, server: &str, activity: &str) {
        let dir = self.home.join("sessions").join(name);
        assert!(fs::create_dir_all(&dir).is_ok());
        assert!(fs::write(dir.join("meta"), format!(
            "mode=local\nsession={name}\nseat.main=lead\nprofile.main=idle\ntmux_server_kind={kind}\ntmux_server={server}\n{activity}"
        )).is_ok());
        assert!(fs::write(dir.join("events.jsonl"), "").is_ok());
    }

    fn run(&self, mode: &str, boot: &str) -> (Value, Vec<String>) {
        assert!(fs::write(&self.trace, "").is_ok());
        let real_path = std::env::var("PATH").unwrap_or_else(|why| panic!("test PATH: {why}"));
        let out = ae()
            .env("HOME", &self.home)
            .env("AE_HOME", &self.home)
            .env("CONFIG_FILE", self.home.join("config"))
            .env("TMUX_TMPDIR", &self.home)
            .env("AE_TMUX_SERVER_KIND", "socket")
            .env("AE_TMUX_SERVER", "/r4/entry")
            .env("AE_TEST_BOOT_TIME", boot)
            .env("R4_MODE", mode)
            .env("R4_TRACE", &self.trace)
            .env("PATH", format!("{}:{real_path}", self.bin.display()))
            .args(["list", "--all", "--json"])
            .output()
            .unwrap_or_else(|why| panic!("real list process: {why}"));
        assert_eq!(
            out.status.code(),
            Some(0),
            "stdout={} stderr={}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        let document = json::parse(String::from_utf8_lossy(&out.stdout).trim())
            .unwrap_or_else(|why| panic!("list JSON: {why:?}"));
        let mut trace: Vec<String> = fs::read_to_string(&self.trace)
            .unwrap_or_else(|why| panic!("external read transcript: {why}"))
            .lines()
            .map(str::to_owned)
            .collect();
        // Entry asks its own selected server before the fleet read. Assert
        // that setup observation separately, then compare the complete fleet
        // transcript. No world-read observation is filtered or discarded.
        assert_eq!(
            trace.first().map(String::as_str),
            Some("socket\t-S:/r4/entry")
        );
        trace.remove(0);
        (document, trace)
    }
}

fn status<'a>(document: &'a Value, name: &str) -> &'a str {
    let Some(Value::Arr(rows)) = document.get("sessions") else {
        panic!("sessions array")
    };
    rows.iter()
        .find(|row| row.get_str("name") == Some(name))
        .and_then(|row| row.get_str("status"))
        .unwrap_or_else(|| panic!("named row status: {name}"))
}

#[test]
fn r4_world_aliases_share_one_discovery_and_keep_the_recorded_spelling() {
    // Mutation: skip deduplication or lose cache equivalence in reconciliation.
    let f = Fixture::new("alias");
    f.record("alpha", "socket", "/r4/live", "");
    f.record("beta", "name", "ae", "");
    let (document, trace) = f.run("alias", "invalid");
    assert_eq!(status(&document, "alpha"), "running");
    assert_eq!(status(&document, "beta"), "running");
    assert_eq!(document.get("inventory_complete"), Some(&Value::Bool(true)));
    assert_eq!(
        trace,
        [
            "socket\t-S:/r4/live",
            "socket\t-L:ae",
            "sessions\t-S:/r4/live",
            "marker\t-S:/r4/live\t=alpha",
            "marker\t-S:/r4/live\t=beta",
            "panes\t-S:/r4/live",
            "panes\t-L:ae",
        ]
    );
}

#[test]
fn r4_world_panes_are_asked_once_for_two_running_records_on_one_server() {
    // Mutation: remove observed_runtimes' per-server asked lookup.
    let f = Fixture::new("panes");
    f.record("alpha", "socket", "/r4/live", "");
    f.record("beta", "socket", "/r4/live", "");
    let (document, trace) = f.run("panes", "invalid");
    assert_eq!(status(&document, "alpha"), "running");
    assert_eq!(status(&document, "beta"), "running");
    assert_eq!(
        trace,
        [
            "socket\t-S:/r4/live",
            "socket\t-L:ae",
            "sessions\t-S:/r4/live",
            "marker\t-S:/r4/live\t=alpha",
            "marker\t-S:/r4/live\t=beta",
            "sessions\t-L:ae",
            "panes\t-S:/r4/live",
        ]
    );
}

#[test]
fn r4_world_failed_discovery_cannot_recover_between_inventory_and_classification() {
    // Mutations: second FleetDiscovery for classify; bypass settled failure.
    let f = Fixture::new("recover");
    f.record("alpha", "socket", "/r4/dead", "");
    let (document, trace) = f.run("recover", "invalid");
    assert_eq!(status(&document, "alpha"), "unknown");
    assert_eq!(
        document.get("inventory_complete"),
        Some(&Value::Bool(false))
    );
    assert_eq!(
        trace,
        [
            "socket\t-S:/r4/dead",
            "socket\t-L:ae",
            "sessions\t-S:/r4/dead",
            "sessions\t-L:ae"
        ]
    );
}

#[test]
fn r4_world_only_unrecorded_default_failure_is_optional() {
    // Mutation: always optional, or never optional, for -L ae.
    for recorded in [false, true] {
        let f = Fixture::new(if recorded { "recorded" } else { "optional" });
        if recorded {
            f.record("alpha", "name", "ae", "");
        }
        let (document, trace) = f.run("fail", "invalid");
        assert_eq!(
            document.get("inventory_complete"),
            Some(&Value::Bool(!recorded))
        );
        if recorded {
            assert_eq!(status(&document, "alpha"), "unknown");
        }
        assert_eq!(trace, ["socket\t-L:ae", "sessions\t-L:ae"]);
    }
}

#[test]
fn r4_world_boot_absence_requires_all_activity_before_a_known_nonfuture_boot_and_missing_socket() {
    // Mutations: weaken &&, accept a gap, or re-ask classification independently.
    // Old epoch constants deliberately avoid the wall-clock boundary.
    for (tag, mode, boot, activity, second, stopped, probes) in [
        ("proof", "boot", "200", "started=100\n", None, true, 2),
        (
            "no-boot",
            "boot",
            "invalid",
            "started=100\n",
            None,
            false,
            1,
        ),
        (
            "future",
            "boot",
            "9223372036854775807",
            "started=100\n",
            None,
            false,
            1,
        ),
        ("late", "boot", "200", "started=300\n", None, false, 1),
        ("gap", "boot", "200", "", None, false, 1),
        (
            "one-late",
            "boot",
            "200",
            "started=100\n",
            Some("started=300\n"),
            false,
            1,
        ),
        (
            "no-socket-proof",
            "fail",
            "200",
            "started=100\n",
            None,
            false,
            2,
        ),
    ] {
        let f = Fixture::new(tag);
        f.record("alpha", "socket", "/r4/dead", activity);
        if let Some(activity) = second {
            f.record("beta", "socket", "/r4/dead", activity);
        }
        let (document, trace) = f.run(mode, boot);
        assert_eq!(
            status(&document, "alpha"),
            if stopped { "stopped" } else { "unknown" },
            "{tag}: {trace:?}"
        );
        assert_eq!(
            document.get("inventory_complete"),
            Some(&Value::Bool(stopped)),
            "{tag}"
        );
        let mut expected = vec![
            "socket\t-S:/r4/dead",
            "socket\t-L:ae",
            "sessions\t-S:/r4/dead",
        ];
        if probes == 2 {
            expected.push("sessions\t-S:/r4/dead");
        }
        expected.push("sessions\t-L:ae");
        assert_eq!(
            trace, expected,
            "{tag}: boot proof is one discovery answer shared by both phases"
        );
    }
}

#[test]
fn r4_world_clock_read_positions_and_observation_order_stay_explicit() {
    // Lexical tripwire, not proof about arbitrary renamed/indirect calls.
    // Mutation: hoist/merge either clock, reorder scan/discovery/runtime reads.
    // Read every production source through the fail-closed shared source view.
    let product = source_view::product();
    let bodies: Vec<_> = product
        .files
        .iter()
        .filter_map(|source| {
            let (file, text) = (&source.path, &source.text);
            let start = text.find("fn world_read(")?;
            let rest = &text[start..];
            let open = rest.find('{').expect("world_read body");
            let blank = source_view::blank_literals(file, rest).expect("valid source");
            let mut depth = 0;
            let mut end = None;
            for (i, byte) in blank.bytes().enumerate().skip(open) {
                match byte {
                    b'{' => depth += 1,
                    b'}' => {
                        depth -= 1;
                        if depth == 0 {
                            end = Some(i);
                            break;
                        }
                    }
                    _ => {}
                }
            }
            Some(rest[..=end.expect("closed world_read")].to_owned())
        })
        .collect();
    assert_eq!(
        bodies.len(),
        1,
        "world_read must be found exactly once across production files"
    );
    // R4c deliberately hides resolver injection behind live(). Both spellings
    // are the same one-cache construction at this seam; no other call moves.
    let body = bodies[0].replace("SocketPaths::live(", "SocketPaths::asking(");
    let calls = [
        "read_gate(",
        "inventory::durable_records_with(",
        "inventory::entitled_servers(",
        "SocketPaths::asking(",
        ".deduplicated(",
        "inventory::RecordedOn::of(",
        "doors::boot_time(",
        "time::Timestamp::now()",
        "inventory::take_entitled(",
        "liveness::classify(",
        "AFTER_CLASSIFY.with(",
        "observed_runtimes(",
        "listing::Presentation::enter(",
        "time::Timestamp::now()",
    ];
    let mut transcript = Vec::new();
    for call in calls {
        for (at, _) in body.match_indices(call) {
            if !transcript
                .iter()
                .any(|(known, text)| *known == at && *text == call)
            {
                transcript.push((at, call));
            }
        }
    }
    transcript.sort_by_key(|(at, _)| *at);
    assert_eq!(
        transcript
            .into_iter()
            .map(|(_, call)| call)
            .collect::<Vec<_>>(),
        calls
    );
}
