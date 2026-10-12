//! Frozen acceptance: reqfilter R1–R5 + lead footer/160-character rulings.
//! Oracle is those requirements; every call reaches the binary's helper route.
#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance fixtures own isolated session journals and tmux sockets"
)]

use super::cli::{OwnedScratch, ae, helper};
use std::path::PathBuf;

struct Fixture {
    root: OwnedScratch,
    dir: PathBuf,
    pane: String,
}

impl Fixture {
    fn new(name: &str, log: &str) -> Self {
        let root = OwnedScratch::root("rqspec", "journal");
        let dir = root.path().join("sessions").join(name);
        std::fs::create_dir_all(&dir).expect("session directory");
        std::fs::write(dir.join("events.jsonl"), log).expect("journal");
        std::fs::write(dir.join("meta"), format!("session={name}\n")).expect("meta");
        std::os::unix::fs::symlink(env!("CARGO_BIN_EXE_ae"), dir.join("requests"))
            .expect("helper link");
        Self {
            root,
            dir,
            pane: String::new(),
        }
    }

    fn identify(&mut self) {
        let socket = self.root.path().join("sock");
        let tmux = |tail: &[&str]| {
            let mut args = vec!["-S".to_owned(), socket.display().to_string()];
            args.extend(tail.iter().map(|s| (*s).to_owned()));
            super::phase2::run_tmux(&args, self.root.path())
        };
        assert!(tmux(&["-f", "/dev/null", "new-session", "-d", "-s", "rqs"]).0);
        let pane = tmux(&["display-message", "-p", "-t", "rqs", "#{pane_id}"]).1;
        let pane = pane.trim().to_owned();
        assert!(tmux(&["set-option", "-p", "-t", &pane, "@ae_slot", "main"]).0);
        assert!(tmux(&["set-option", "-p", "-t", &pane, "@ae_agent", "lead"]).0);
        self.pane = pane;
    }

    fn run(&self, short: bool, tail: &[&str]) -> std::process::Output {
        let mut runner = if short {
            ae()
        } else {
            helper(&self.dir.join("requests"))
        };
        if short {
            runner.args(["@rqs", "requests"]);
        }
        let socket = self.root.path().join("sock");
        let runner = runner
            .env("AE_HOME", self.root.path())
            .args(tail)
            .env("TMUX_PANE", &self.pane)
            .env("TMUX", format!("{},0,0", socket.display()));
        runner.output().expect("requests helper exits")
    }

    fn view(&self, short: bool, tail: &[&str]) -> String {
        let out = self.run(short, tail);
        assert_eq!(out.status.code(), Some(0), "{tail:?}: {out:?}");
        assert!(out.stderr.is_empty(), "{tail:?}: {out:?}");
        let stdout = String::from_utf8(out.stdout).expect("stdout UTF-8");
        assert!(stdout.starts_with("STATUS"), "table header: {stdout}");
        stdout
    }

    fn footer(&self, hidden: usize, mode: &str) -> String {
        // Independent POSIX quoting oracle; includes spaces and apostrophes.
        let link = self.dir.join("requests").display().to_string();
        let path = link.replace('\'', "'\\''");
        format!("{hidden} older closed requests hidden; all of them: '{path}' {mode} --limit 0")
    }
}

fn event(i: usize, action: &str, from: (&str, &str), to: (&str, &str), summary: &str) -> String {
    let (actor, actor_slot) = from;
    let (target, target_slot) = to;
    let summary = summary.replace('\n', "\\n");
    format!(
        r#"{{"ts":"2026-10-01T00:{:02}:00Z","actor":"{actor}","target":"{target}","action":"{action}","ref":"q-{i:02}","actor_slot":"{actor_slot}","actor_session":"rqs","target_slot":"{target_slot}","target_session":"rqs","summary":"{summary}"}}"#,
        59 - i % 60
    ) + "\n"
}

fn add(log: &mut String, i: usize, status: &str, inbox: bool, unrelated: bool) {
    let peer_slot = format!("spawned.{i}");
    let own = [("lead", "main"), ("other", "worker.99")][usize::from(unrelated)];
    let peer = ("worker", peer_slot.as_str());
    let (from, to) = if inbox { (peer, own) } else { (own, peer) };
    log.push_str(&event(i, "ask", from, to, &format!("body-{i}")));
    match status {
        "replied" => log.push_str(&event(i, "reply", to, from, "answer")),
        "cancelled" => log.push_str(&event(i, "cancel", from, to, "withdrawn")),
        "retired" => log.push_str(&event(i, "retire", own, peer, "seat retired")),
        "pending" => {}
        _ => panic!("unknown fixture status"),
    }
}

fn rows(view: &str) -> Vec<(&str, &str)> {
    view.lines()
        .skip(1)
        .filter_map(|line| {
            let mut cells = line.split_whitespace();
            let status = cells.next()?;
            let id = cells.nth(1)?;
            ["pending", "replied", "cancelled", "retired"]
                .contains(&status)
                .then_some((status, id))
        })
        .collect()
}

fn ids(view: &str) -> Vec<&str> {
    rows(view).into_iter().map(|(_, id)| id).collect()
}

#[test]
fn default_keeps_every_pending_and_newest_ten_closed_by_opening_append_order() {
    let mut log = String::new();
    for i in 0..28 {
        let status = if (2..15).contains(&i) {
            ["replied", "cancelled", "retired"][i % 3]
        } else {
            "pending"
        };
        add(&mut log, i, status, false, false);
    }
    // Closing an OLD opening last does not make it a new request.
    let peer = ("worker", "spawned.2");
    log.push_str(&event(2, "reply", peer, ("lead", "main"), "late answer"));
    let fx = Fixture::new("rqs", &log);
    for short in [false, true] {
        let view = fx.view(short, &["all"]);
        let expected: Vec<_> = (0..28)
            .filter(|i| !(2..5).contains(i))
            .map(|i| format!("q-{i:02}"))
            .collect();
        assert_eq!(ids(&view), expected, "chronological retained requests");
        let pending = rows(&view).iter().filter(|(s, _)| *s == "pending").count();
        assert_eq!(pending, 15);
        for status in ["replied", "cancelled", "retired"] {
            assert!(rows(&view).iter().any(|(s, _)| *s == status), "{status}");
        }
        assert_eq!(view.lines().last(), Some(fx.footer(3, "all").as_str()));
        assert_eq!(view.lines().count(), 27, "header + 25 rows + one footer");
    }
}

#[test]
fn identity_modes_filter_before_limit_and_default_mode_remains_mine() {
    for mode in [None, Some("mine"), Some("inbox")] {
        let mut log = String::new();
        let inbox = mode == Some("inbox");
        for i in 0..14 {
            let status = if i == 13 { "pending" } else { "replied" };
            add(&mut log, i, status, inbox, false);
        }
        for i in 20..38 {
            let status = if i == 37 { "pending" } else { "replied" };
            add(&mut log, i, status, inbox, true);
        }
        let mut fx = Fixture::new("rqs", &log);
        fx.identify();
        let tail: Vec<_> = mode.into_iter().collect();
        let view = fx.view(false, &tail);
        let expected: Vec<_> = (3..14).map(|i| format!("q-{i:02}")).collect();
        assert_eq!(ids(&view), expected);
        let footer = fx.footer(3, mode.unwrap_or("mine"));
        assert_eq!(view.lines().last(), Some(footer.as_str()));
        if mode.is_none() {
            let limited = fx.view(true, &["--limit", "2"]);
            assert_eq!(ids(&limited), ["q-11", "q-12", "q-13"]);
            assert_eq!(limited.lines().last(), Some(fx.footer(11, "mine").as_str()));
        }
    }
}

#[test]
fn custom_limits_and_zero_change_only_closed_rows() {
    let mut log = String::new();
    for i in 0..13 {
        let status = if matches!(i, 0 | 12) {
            "pending"
        } else {
            "cancelled"
        };
        add(&mut log, i, status, false, false);
    }
    let fx = Fixture::new("rqs", &log);
    for (limit, start, hidden) in [("1", 11, 10), ("7", 5, 4), ("0", 1, 0), ("99", 1, 0)] {
        let view = fx.view(true, &["all", "--limit", limit]);
        let expected: Vec<_> = std::iter::once(0)
            .chain(start..13)
            .map(|i| format!("q-{i:02}"))
            .collect();
        assert_eq!(ids(&view), expected, "limit {limit}");
        if hidden == 0 {
            assert_eq!(view.lines().count(), 14);
        } else {
            assert_eq!(view.lines().last(), Some(fx.footer(hidden, "all").as_str()));
        }
    }
}

#[test]
fn footer_appears_only_above_ten_and_quotes_legacy_path() {
    for count in [0, 9, 10, 11] {
        let mut log = String::new();
        for i in 0..count {
            add(&mut log, i, "replied", false, false);
        }
        let fx = Fixture::new("legacy space's", &log);
        let view = fx.view(false, &["all"]);
        assert_eq!(rows(&view).len(), count.min(10));
        let lines = 1 + count.min(10) + usize::from(count > 10);
        assert_eq!(view.lines().count(), lines);
        if count > 10 {
            assert_eq!(view.lines().last(), Some(fx.footer(1, "all").as_str()));
        }
    }
}

#[test]
fn excerpts_keep_short_text_and_clip_long_unicode_to_159_chars_plus_ellipsis() {
    for summary in [
        "é".repeat(160),
        "é".repeat(161),
        format!("first\n{}", "x".repeat(190)),
    ] {
        let from = ("lead", "main");
        let log = event(0, "ask", from, ("worker", "spawned.0"), &summary);
        let fx = Fixture::new("rqs", &log);
        let view = fx.view(false, &["all"]);
        let folded = summary.replace('\n', " ");
        let expected = if folded.chars().count() <= 160 {
            folded
        } else {
            folded.chars().take(159).collect::<String>() + "…"
        };
        assert_eq!(view.lines().count(), 2, "one request stays one line");
        let row = view.lines().nth(1).expect("request row");
        assert_eq!(&row[89..], expected, "bounded excerpt");
        assert!(row[89..].chars().count() <= 160);
    }
}

#[test]
fn malformed_limit_and_unknown_options_are_usage_errors_before_identity() {
    let fx = Fixture::new("rqs", "");
    for tail in [
        vec!["--unknown"],
        vec!["--limit"],
        vec!["all", "--limit", "-1"],
        vec!["all", "--limit", "x"],
        vec!["all", "--limit", "99999999999999999999999999999"],
        vec!["all", "--limit", "1", "--limit", "2"],
    ] {
        let out = fx.run(true, &tail);
        assert_eq!(out.status.code(), Some(2), "{tail:?}: {out:?}");
        assert!(out.stdout.is_empty(), "usage writes no table: {tail:?}");
        assert!(!out.stderr.is_empty(), "usage explains refusal: {tail:?}");
    }
    let out = fx.run(false, &["--limit", "0"]);
    assert_eq!(
        out.status.code(),
        Some(1),
        "valid limit supplies no identity: {out:?}"
    );
    assert!(out.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("could not detect current agent identity")
    );
}
