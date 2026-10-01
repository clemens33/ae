//! Driver pins for the restore predicate, the beat and the ledger trim. The
//! black-box contract is `restore_spec.rs`, written by another agent.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "fixtures build and inspect real directories with expect on the fixture I/O; \
              the boundary is about what PRODUCT code may reach"
)]

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Output;
use std::time::{Duration, SystemTime};

use ae::meta::{Selector, ServerSelector};
use ae::restore::{
    Decision, Fact, Guard, Ledger, Proof, WINDOW_SECS, judge, ledger, order, pins_clean_stop,
};
use ae::session::SessionRead;
use ae::tmux::Evidence;
use ae::watchdog_glue::{beat_modified, beat_path, touch_beat};

use super::cli::{OwnedScratch, ae};
use super::phase2::run_tmux;

fn fact(name: &str, on: &str, launched: i64, ledger: Ledger, beat: Option<i64>) -> Fact {
    Fact {
        name: name.to_owned(),
        server: ServerSelector::Positive(Selector::Name(on.to_owned())),
        live: false,
        launched: Evidence::At(launched),
        ledger,
        beat: beat.map_or(Evidence::Silent, Evidence::At),
    }
}

#[test]
fn the_crashed_cohort_restores_and_every_other_class_is_named_and_skipped() {
    use Decision::{Abandoned, Damaged, Live, NoBeat, NoServer, Restore, Stopped, Unlaunched};
    use Ledger::Clear;
    let newest = 90_000;
    let mut live = fact("live", "s", 50_000, Clear, Some(newest + 500));
    live.live = true;
    let mut unlaunched = fact("unlaunched", "s", 0, Clear, Some(newest));
    unlaunched.launched = Evidence::Silent;
    let mut nowhere = fact("nowhere", "s", 1000, Clear, Some(newest));
    nowhere.server = ServerSelector::Missing;
    let facts = [
        fact("cohort-max", "s", 1000, Clear, Some(newest)),
        fact("edge", "s", 1000, Clear, Some(newest - WINDOW_SECS)),
        fact(
            "abandoned",
            "s",
            1000,
            Clear,
            Some(newest - WINDOW_SECS - 1),
        ),
        fact("stopped", "s", 1000, Ledger::Stopped, Some(newest + 400)),
        fact("damaged", "s", 1000, Ledger::Damaged, Some(newest + 300)),
        fact("unwatched", "s", 1000, Clear, None),
        // A beat from before the current launch belongs to an earlier run.
        fact("prior-run", "s", newest + 100, Clear, Some(newest)),
        live,
        unlaunched,
        nowhere,
        fact("other-server", "t", 1000, Clear, Some(1500)),
    ];
    // The live, stopped and damaged beats are all newer than `newest` and none
    // of them may move the window: only unstopped, unlive, readable beats do.
    assert_eq!(
        judge(&facts, |_| Proof::Absent),
        [
            Restore, Restore, Abandoned, Stopped, Damaged, NoBeat, NoBeat, Live, Unlaunched,
            NoServer, Restore
        ]
    );
}

type Line = (&'static str, i64, &'static str);

fn scratch_with(tag: &str, lines: &[Line], junk: bool) -> (OwnedScratch, Ledger) {
    let scratch = OwnedScratch::root("rt", tag);
    let mut body = String::new();
    for (action, at, summary) in lines {
        let ts = ae::time::Timestamp::from_epoch(*at);
        let _ = writeln!(
            body,
            "{{\"ts\":\"{ts}\",\"actor\":\"human\",\"action\":\"{action}\",\"summary\":\"{summary}\"}}"
        );
    }
    if junk {
        body.push_str("{not json\n");
    }
    let wrote = std::fs::write(scratch.join("events.jsonl"), body);
    wrote.unwrap_or_else(|why| panic!("fixture ledger: {why}"));
    let read = SessionRead::open(&scratch).ok();
    let verdict = ledger(read.as_ref(), 1000);
    (scratch, verdict)
}

#[test]
fn the_ledger_fails_closed_and_counts_only_stops_since_the_launch() {
    const CLEAN: &str = "stopped: verified gone on its recorded server";
    const FAILED: &str = "FAILED: unavailable";
    let req = |at| ("stop-request", at, "stop requested");
    let cases: [(&[Line], Ledger); 13] = [
        (&[("stop-result", 999, CLEAN)], Ledger::Clear),
        (&[("stop-result", 1000, CLEAN)], Ledger::Stopped),
        (&[("stop-result", 1001, "already stopped")], Ledger::Stopped),
        (&[("stop-result", 1001, FAILED)], Ledger::Clear),
        (&[req(1001)], Ledger::Stopped),
        (&[req(900)], Ledger::Clear),
        (&[req(1001), ("stop-result", 1002, FAILED)], Ledger::Clear),
        (
            &[req(1001), ("stop-result", 1002, FAILED), req(1003)],
            Ledger::Stopped,
        ),
        // Seconds are the clock's grain; the LEDGER's order decides inside one.
        (&[req(1001), ("stop-result", 1001, FAILED)], Ledger::Clear),
        (&[("stop-result", 1001, FAILED), req(1001)], Ledger::Stopped),
        (
            &[req(1001), ("stop-result", 1001, FAILED), req(1001)],
            Ledger::Stopped,
        ),
        // A memo that QUOTES a clean stop is not one.
        (&[("memo", 1500, CLEAN)], Ledger::Clear),
        (&[("stop-result", 998, CLEAN), req(999)], Ledger::Clear),
    ];
    for (index, (lines, want)) in cases.into_iter().enumerate() {
        let (_scratch, got) = scratch_with(&format!("l{index}"), lines, false);
        assert_eq!(got, want, "case {index}: {lines:?}");
    }
    // One unreadable line may have been the stop: unknown, not clear.
    let (_scratch, got) = scratch_with("junk", &[("stop-result", 999, CLEAN)], true);
    assert_eq!(got, Ledger::Damaged);
    assert_eq!(ledger(None, 1000), Ledger::Damaged, "an unreadable ledger");
}

#[test]
fn a_proof_gap_is_named_and_a_present_name_never_moves_the_window() {
    use Decision::{Live, NoBeat, Restore, Unproven};
    let clear = Ledger::Clear;
    let top = 90_000;
    let facts = [
        fact("gap", "s", 1000, clear, Some(top)),
        fact("older", "s", 1000, clear, Some(top - WINDOW_SECS - 100)),
        fact("taken", "s", 1000, clear, Some(top + 1000)),
        fact("bare", "s", 1000, clear, None),
    ];
    let mut asked = Vec::new();
    let got = judge(&facts, |index| {
        asked.push(index);
        match index {
            0 => Proof::Unknown,
            2 => Proof::Present,
            _ => Proof::Absent,
        }
    });
    // The gap's and the taken name's beats are the freshest, yet neither
    // evicts `older`; only a session past every other gate is ever proved.
    assert_eq!(got, [Unproven, Restore, Live, NoBeat]);
    assert_eq!(asked, [0, 1, 2]);
}

#[allow(
    clippy::expect_used,
    reason = "a fixture builder: a failed write fails the test"
)]
fn saved(tag: &str, launched: i64, beat: Option<i64>, lines: &[Line], junk: bool) -> OwnedScratch {
    let (scratch, _) = scratch_with(tag, lines, junk);
    let meta = "tmux_server_kind=socket\ntmux_server=/tmp/guard.sock\n";
    std::fs::write(scratch.join("meta"), meta).expect("meta");
    std::fs::write(scratch.join(".launch-attempt"), format!("{launched}\n")).expect("stamp");
    if let Some(at) = beat {
        touch_beat(&scratch).expect("beat");
        let when = SystemTime::UNIX_EPOCH + Duration::from_secs(u64::try_from(at).expect("epoch"));
        std::fs::File::options()
            .write(true)
            .open(beat_path(&scratch))
            .and_then(|file| file.set_modified(when))
            .expect("beat mtime");
    }
    scratch
}

#[test]
fn the_lock_time_guard_rereads_the_disk_and_fails_closed() {
    type Case<'a> = (Option<i64>, &'a [Line], bool, Option<&'a str>);
    let guard = Guard {
        launched: 1000,
        cutoff: 5000,
        server: Selector::Socket("/tmp/guard.sock".into()),
    };
    let req = ("stop-request", 1500, "stop requested");
    let clean = (
        "stop-result",
        1500,
        "stopped: verified gone on its recorded server",
    );
    let failed = ("stop-result", 1600, "FAILED: no");
    let cases: [Case; 8] = [
        (Some(6000), &[], false, None),
        (Some(5000), &[], false, None),
        (Some(4999), &[], false, Some("beat")),
        (None, &[], false, Some("beat")),
        (Some(6000), &[clean], false, Some("stopped")),
        (Some(6000), &[req], false, Some("stopped")),
        (Some(6000), &[req, failed], false, None),
        (Some(6000), &[], true, Some("ledger unreadable")),
    ];
    for (index, (beat, lines, junk, want)) in cases.into_iter().enumerate() {
        let dir = saved(&format!("g{index}"), 1000, beat, lines, junk);
        let got = guard.refuse(&dir);
        assert!(
            got.map(str::to_owned).is_some() == want.is_some()
                && want.is_none_or(|word| got.is_some_and(|why| why.contains(word))),
            "case {index}: {got:?} vs {want:?}"
        );
    }
    let dir = saved("gmove", 1000, Some(6000), &[], false);
    assert_eq!(guard.refuse(&dir), None);
    std::fs::write(dir.join(".launch-attempt"), "1001\n").expect("relaunch");
    assert!(guard.refuse(&dir).is_some_and(|why| why.contains("launch")));
    std::fs::write(dir.join(".launch-attempt"), "1000\n").expect("restamp");
    let moved = "tmux_server_kind=socket\ntmux_server=/tmp/elsewhere.sock\n";
    std::fs::write(dir.join("meta"), moved).expect("moved server");
    assert!(guard.refuse(&dir).is_some_and(|why| why.contains("server")));
    std::fs::remove_file(dir.join("meta")).expect("rm meta");
    assert!(guard.refuse(&dir).is_some_and(|why| why.contains("record")));
}

#[test]
fn restore_order_is_the_fleet_order_then_the_name() {
    let place = |name: &str| {
        ["zed", "amy"]
            .iter()
            .position(|n| *n == name)
            .unwrap_or(usize::MAX)
    };
    let names = ["bob", "amy", "cat", "zed"].map(str::to_owned).to_vec();
    assert_eq!(order(names, place), ["zed", "amy", "bob", "cat"]);
}

fn mtime(path: &Path) -> SystemTime {
    std::fs::symlink_metadata(path)
        .and_then(|meta| meta.modified())
        .unwrap_or_else(|why| panic!("mtime: {why}"))
}

#[test]
fn the_beat_is_an_empty_file_refreshed_by_exclusive_temp_and_never_written_through_a_link() {
    let scratch = OwnedScratch::root("rt", "beat");
    let dir: &Path = &scratch;
    assert_eq!(beat_modified(dir), Evidence::Silent);
    touch_beat(dir).expect("first beat");
    let beat = beat_path(dir);
    assert_eq!(std::fs::metadata(&beat).expect("beat").len(), 0);
    let old = SystemTime::now() - Duration::from_hours(2);
    std::fs::File::options()
        .write(true)
        .open(&beat)
        .and_then(|file| file.set_modified(old))
        .expect("aged beat");
    touch_beat(dir).expect("refresh");
    assert!(mtime(&beat) > old + Duration::from_hours(1));
    assert!(matches!(beat_modified(dir), Evidence::At(_)));
    let leftovers = std::fs::read_dir(dir)
        .expect("dir")
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_string_lossy().contains(".tmp."))
        .count();
    assert_eq!(leftovers, 0, "a staged temp was left behind");

    // A link planted AT the beat is replaced, never followed.
    let target = scratch.join("target");
    std::fs::write(&target, "keep").expect("target");
    std::fs::remove_file(&beat).expect("rm beat");
    std::os::unix::fs::symlink(&target, &beat).expect("planted link");
    assert_eq!(beat_modified(dir), Evidence::Unreadable, "a link is damage");
    let before = mtime(&target);
    touch_beat(dir).expect("replace the link");
    assert!(std::fs::symlink_metadata(&beat).expect("beat").is_file());
    assert_eq!(std::fs::read_to_string(&target).expect("target"), "keep");
    assert_eq!(mtime(&target), before, "the link's target was touched");

    // A link planted at the predictable TEMP name is refused, never truncated.
    let staged = dir.join(format!(".watchdog-beat.tmp.{}", std::process::id()));
    std::os::unix::fs::symlink(&target, &staged).expect("planted temp");
    assert!(touch_beat(dir).is_err(), "a taken temp name must refuse");
    assert_eq!(std::fs::read_to_string(&target).expect("target"), "keep");

    // A directory where the beat belongs is damage.
    std::fs::remove_file(&staged).expect("rm temp");
    std::fs::remove_file(&beat).expect("rm beat");
    std::fs::create_dir(&beat).expect("dir beat");
    assert_eq!(beat_modified(dir), Evidence::Unreadable);
}

#[test]
fn resume_retention_keeps_only_the_newest_clean_stop_when_the_window_dropped_it() {
    let scratch = OwnedScratch::root("rt", "trim");
    let store = ae::store::open(&scratch);
    let clean = |n: u32| {
        format!(
            "{{\"ts\":\"2026-09-30T10:00:0{n}Z\",\"actor\":\"human\",\"action\":\"stop-result\",\"summary\":\"already stopped\"}}\n"
        )
    };
    let failed = "{\"ts\":\"2026-09-30T10:01:00Z\",\"actor\":\"human\",\"action\":\"stop-result\",\"summary\":\"FAILED: no\"}\n";
    let other = "{\"ts\":\"2026-09-30T10:02:00Z\",\"actor\":\"a\",\"action\":\"state\",\"summary\":\"done\"}\n";
    let path = scratch.join("events.jsonl");
    let run = |body: String, keep: usize| {
        std::fs::write(&path, body).expect("ledger");
        store.retain_events(keep, pins_clean_stop);
        std::fs::read_to_string(&path).expect("trimmed")
    };
    // The newest clean stop fell out of the window: it comes back, alone.
    let dropped = run(
        format!("{}{}{failed}{}", clean(1), clean(2), other.repeat(5)),
        4,
    );
    assert_eq!(dropped, format!("{}{}", clean(2), other.repeat(4)));
    // A clean stop still inside the window needs no pin; nor does a ledger
    // with none at all — that stays byte for byte what retention always did.
    let inside = format!("{}{}{}", clean(1), other.repeat(3), clean(2));
    assert_eq!(
        run(inside.clone(), 4),
        format!("{}{}", other.repeat(3), clean(2))
    );
    assert_eq!(
        run(format!("{failed}{}", other.repeat(6)), 4),
        other.repeat(4)
    );
    assert_eq!(
        run(inside, 10),
        format!("{}{}{}", clean(1), other.repeat(3), clean(2))
    );
}

/// The source files under `src/` whose text contains `needle`.
#[allow(
    clippy::expect_used,
    reason = "a source scan: an unreadable src tree fails the test"
)]
fn callers_of(needle: &str) -> Vec<String> {
    let mut callers = Vec::new();
    let mut stack = vec![Path::new(env!("CARGO_MANIFEST_DIR")).join("src")];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("src").filter_map(Result::ok) {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if std::fs::read_to_string(&path).is_ok_and(|text| text.contains(needle)) {
                let name = path.file_name().expect("name");
                callers.push(name.to_string_lossy().into_owned());
            }
        }
    }
    callers
}

#[test]
fn a_beat_under_a_non_directory_is_damage_not_absence() {
    let scratch = OwnedScratch::root("rt", "beat-enotdir");
    let plain = scratch.join("plain");
    std::fs::write(&plain, b"").unwrap_or_else(|why| panic!("fixture file: {why}"));
    assert_eq!(
        beat_modified(&plain),
        Evidence::Unreadable,
        "ENOTDIR is not NotFound"
    );
}

#[test]
fn only_the_watchdog_cycle_writes_the_beat() {
    let callers = callers_of("watchdog_glue::touch_beat(");
    assert_eq!(callers, ["watchdog_daemon.rs"], "the beat has one writer");
}

#[test]
fn only_the_restore_loop_starts_a_guarded_launch() {
    let callers = callers_of("session_launch::run_restore(");
    assert_eq!(callers, ["restore.rs"], "a restore launch has one caller");
}

/// A crashed `saved` on its RECORDED server whose recorded profile the config
/// has since dropped, and a live non-ae `saved` on the TARGET server a bare run
/// proposes: the one place the recorded-vs-proposed preflight choice shows.
struct Split {
    scratch: OwnedScratch,
    home: PathBuf,
    project: PathBuf,
    recorded: PathBuf,
    target: PathBuf,
}

impl Split {
    fn new(tag: &str) -> Self {
        let mut scratch = OwnedScratch::root("rt", tag);
        let (home, project) = (scratch.join("state"), scratch.join("project"));
        let (recorded, target) = (scratch.join("s1"), scratch.join("s2"));
        scratch.add_tmux_server(recorded.clone());
        scratch.add_tmux_server(target.clone());
        std::fs::create_dir_all(&home).expect("state root");
        std::fs::create_dir_all(&project).expect("project");
        let config = "[profiles]\nidle = \"sleep 600\"\n[roster]\nlead = idle\n[workspace]\nmain = lead\nlayout = vertical\n";
        std::fs::write(home.join("config"), config).expect("config");
        Self {
            scratch,
            home,
            project,
            recorded,
            target,
        }
    }

    fn run(&self, server: &Path, args: &[&str]) -> Output {
        ae().env_remove("TMUX")
            .env_remove("TMUX_PANE")
            .env("HOME", &self.scratch)
            .env("AE_HOME", &self.home)
            .env("CONFIG_FILE", self.home.join("config"))
            .env("TMUX_TMPDIR", &self.scratch)
            .env("AE_TMUX_SERVER_KIND", "socket")
            .env("AE_TMUX_SERVER", server)
            .env("AE_NO_AUTOSTART", "1")
            .current_dir(&self.project)
            .args(args)
            .output()
            .expect("real ae entry")
    }

    fn tmux(&self, server: &Path, words: &[&str]) -> (bool, String) {
        let mut argv = vec!["-S".to_owned(), server.display().to_string()];
        argv.extend(words.iter().map(|word| (*word).to_owned()));
        run_tmux(&argv, &self.scratch)
    }

    /// `saved` launched on the recorded server, aged into a crashed cohort, its
    /// server killed, and its recorded profile renamed to one the config lacks.
    fn crash_saved_with_a_dropped_profile(&self) {
        let launched = self.run(&self.recorded, &["--local", "saved", "--no-attach"]);
        assert!(launched.status.success(), "{launched:?}");
        let dir = self.home.join("sessions").join("saved");
        let now = ae::time::Timestamp::now().epoch();
        std::fs::write(dir.join(".launch-attempt"), format!("{}\n", now - 600)).expect("stamp");
        let beat = std::fs::File::create(dir.join(".watchdog-beat")).expect("beat");
        let at =
            SystemTime::UNIX_EPOCH + Duration::from_secs(u64::try_from(now - 60).expect("epoch"));
        beat.set_modified(at).expect("beat mtime");
        let (ok, pid) = self.tmux(&self.recorded, &["display-message", "-p", "#{pid}"]);
        assert!(ok, "recorded server pid");
        let _ = self.tmux(
            &self.recorded,
            &["run-shell", &format!("kill -KILL {}", pid.trim())],
        );
        assert!(
            !self.tmux(&self.recorded, &["list-sessions"]).0,
            "server gone"
        );
        let meta = std::fs::read_to_string(dir.join("meta")).expect("meta");
        let dropped: Vec<String> = meta
            .lines()
            .map(|line| match line.split_once('=') {
                Some((key, _)) if key.starts_with("profile.") => format!("{key}=vanished"),
                Some((ae::migrate::KEY, _)) => {
                    format!("{}={}", ae::migrate::KEY, ae::migrate::CURRENT - 1)
                }
                _ => line.to_owned(),
            })
            .collect();
        std::fs::write(dir.join("meta"), dropped.join("\n") + "\n").expect("meta rewrite");
        let held = self.tmux(
            &self.target,
            &["-f", "/dev/null", "new-session", "-d", "-s", "saved"],
        );
        assert!(held.0, "a live non-ae saved on the target: {}", held.1);
    }
}

#[test]
fn a_restore_preflights_its_recorded_server_not_the_one_a_live_namesake_sits_on() {
    let split = Split::new("route-restore");
    split.crash_saved_with_a_dropped_profile();
    let meta = split.home.join("sessions").join("saved").join("meta");
    let before = std::fs::read(&meta).expect("meta before");
    let bare = split.run(&split.target, &["--no-attach"]);
    let err = String::from_utf8_lossy(&bare.stderr);
    assert!(
        err.contains("ae: restore failed saved:") && err.contains("vanished"),
        "the dropped profile refuses the restore: {err}"
    );
    assert_eq!(
        std::fs::read(&meta).expect("meta after"),
        before,
        "the refusal came before the migration chain wrote the meta: {err}"
    );
}

#[test]
fn an_ordinary_launch_preflights_the_proposed_server_where_its_namesake_lives() {
    let split = Split::new("route-ordinary");
    split.crash_saved_with_a_dropped_profile();
    let launch = split.run(&split.target, &["saved", "--no-attach"]);
    let err = String::from_utf8_lossy(&launch.stderr);
    assert!(
        err.contains("exists but is not an ae session") && !err.contains("vanished"),
        "the live namesake is the refusal: {err}"
    );
}
