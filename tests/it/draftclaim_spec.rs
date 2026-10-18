//! Frozen draftclaim acceptance (multi-window S1): a kept line belongs to
//! the ONE writing instance that kept it. Oracle: brief invariants 1-7 in
//! `.local/brief-draftclaim.md` and DESIGN-r3 section S1 — never the code
//! under test. Real apps, a real owner chat, real kernel locks on a
//! private tmux server; nothing on the judged path is mocked.
//!
//! The contract this file freezes, beyond the brief's invariants:
//! * Wording fragments: an offer draws "kept line from an earlier window",
//!   "may already have been sent", "Enter restores" and "Esc discards";
//!   a filled kept line (own or accepted) draws the existing
//!   "Kept line, maybe already sent" warning.
//! * Offers and own restores are judged ONLY at an acquisition (an app
//!   entering writing into an empty composer, an owner chat's promotion),
//!   one orphan per acquisition; a nonempty composer is offered nothing.
//! * The claim is taken when the offer is shown: while the claimant
//!   process lives, no other consumer is offered those bytes.
//! * At a pending offer, Enter fills (never sends), Esc discards durably,
//!   and any other typed key neither composes, submits nor discards.
//! * An instance's own kept record precedes every orphan: while it holds
//!   bytes, its owner restores it (with the warning) and is offered none.
//! * Records are discovered on disk as regular files named
//!   `console.draft` (legacy) or `console.draft.<instance>`, never a
//!   `.lock` sibling; a planted plain `console.draft` is the legacy
//!   orphan of invariant 5.
//!
//! `controls_…` passes on the unfixed base by design: it proves the rig,
//! so a RED below is the S1 behaviour and not a broken fixture.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance owns private terminal fixtures and reads their effects"
)]

use crate::app_mode::writing;
use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use ae::events::Event;

const UUID: &str = "0199c0de-cccc-4890-abcd-ef0123456789";
const WAIT: Duration = Duration::from_secs(20);
const WRITER: &str = ".console-writer.lock";

/// The offer's first fragment (invariant 3: another window's kept line).
pub(crate) const OFFER_HEAD: &str = "kept line from an earlier window";
/// The offer's fill fragment: Enter fills the composer only.
pub(crate) const OFFER_FILL: &str = "Enter restores";
/// The offer's discard fragment.
pub(crate) const OFFER_DROP: &str = "Esc discards";
/// The offer's duplicate-send warning fragment.
pub(crate) const OFFER_WARN: &str = "may already have been sent";
/// The filled kept line's warning, unchanged from before S1.
pub(crate) const KEPT_WARN: &str = "Kept line, maybe already sent";

/// Whether `screen` shows the S1 orphan offer with all four fragments.
pub(crate) fn offer_shown(screen: &str) -> bool {
    screen.contains(OFFER_HEAD)
        && screen.contains(OFFER_FILL)
        && screen.contains(OFFER_DROP)
        && screen.contains(OFFER_WARN)
}

/// The kept draft bodies in session directory `dir`, sorted: every
/// nonempty regular file named `console.draft` or `console.draft.*`
/// except a `.lock` sibling. Symlinks are never followed, so a planted
/// link is not a record this helper can see — matching the store's own
/// non-following reads.
pub(crate) fn draft_bodies(dir: &Path) -> Vec<Vec<u8>> {
    let mut bodies: Vec<Vec<u8>> = fs::read_dir(dir)
        .expect("session dir is listable")
        .filter_map(Result::ok)
        .filter(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            let lock = Path::new(&name)
                .extension()
                .is_some_and(|ext| ext == "lock");
            name.starts_with("console.draft") && !lock
        })
        .filter(|entry| {
            fs::symlink_metadata(entry.path()).is_ok_and(|meta| meta.file_type().is_file())
        })
        .filter_map(|entry| fs::read(entry.path()).ok())
        .filter(|body| !body.is_empty())
        .collect();
    bodies.sort();
    bodies
}

/// No instance kept a draft in `dir` — the S1 form of the old
/// `!console.draft.exists()` witnesses.
pub(crate) fn assert_no_kept_draft(dir: &Path) {
    assert_eq!(
        draft_bodies(dir),
        Vec::<Vec<u8>>::new(),
        "no draft record holds bytes in {}",
        dir.display()
    );
}

/// Exactly `want` (order-free) are the kept draft bodies in `dir`.
pub(crate) fn assert_kept_drafts(dir: &Path, want: &[&[u8]]) {
    let mut sorted: Vec<Vec<u8>> = want.iter().map(|body| body.to_vec()).collect();
    sorted.sort();
    assert_eq!(
        draft_bodies(dir),
        sorted,
        "kept draft records in {}",
        dir.display()
    );
}

struct Rig {
    tool: super::deliver::Rig,
    root: PathBuf,
    socket: PathBuf,
    home: String,
}

impl Rig {
    fn new(tag: &str) -> Self {
        let tool = super::deliver::Rig::new(tag, "codex", 0);
        let root = tool
            .dir
            .parent()
            .expect("sessions")
            .parent()
            .expect("root")
            .to_owned();
        let home = tool
            .dir
            .file_name()
            .expect("name")
            .to_string_lossy()
            .into_owned();
        let socket = root.join("sock");
        let rig = Self {
            tool,
            root,
            socket,
            home,
        };
        rig.meta(UUID, "colead");
        fs::write(rig.tool.dir.join("events.jsonl"), "").expect("empty journal");
        fs::write(rig.root.join("config"), "[workspace]\nchat = app\n").expect("private config");
        fs::write(
            rig.tool.dir.join(".launch-attempt"),
            ae::time::Timestamp::now().epoch().to_string(),
        )
        .expect("launch stamp");
        for (option, value) in [
            ("@ae_session_uuid", UUID),
            ("@ae_look", "off"),
            ("@ae_motion", "off"),
            ("@ae_main_pane", rig.tool.pane.as_str()),
            ("default-size", "160x45"),
        ] {
            rig.tmux(&["set-option", "-t", &rig.home, option, value]);
        }
        rig.tmux(&[
            "set-option",
            "-p",
            "-t",
            &rig.tool.pane,
            "@ae_agent",
            "lead",
        ]);
        // This is an independent kernel witness, not an app/store lock helper.
        drop(rig.lease());
        rig
    }

    fn meta(&self, uuid: &str, peer: &str) {
        fs::write(self.tool.dir.join("meta"), format!(
            "session={}\nmode=local\nsession_id={uuid}\nlayout=lead-pair\ntmux_server_kind=socket\ntmux_server={}\nseat.main=lead\nagent_bin.main=codex\nlaunch_id.main=tok-rig\nseat.worker.0={peer}\nagent_bin.worker.0=codex\nlaunch_id.worker.0=tok-peer\n",
            self.home, self.socket.display()
        )).expect("private lead pair");
    }

    fn tmux(&self, tail: &[&str]) -> String {
        let mut args = vec!["-S".to_owned(), self.socket.display().to_string()];
        args.extend(tail.iter().map(|arg| (*arg).to_owned()));
        let (ok, out) = super::phase2::run_tmux(&args, &self.root);
        assert!(ok, "private tmux {tail:?}: {out}");
        out
    }

    fn console(&self, chat: bool) -> String {
        let gates = self.root.join("gates-owner");
        fs::create_dir(&gates).expect("owner read gates");
        for (key, value) in [
            ("HOME", self.root.display().to_string()),
            ("AE_TEST_APP_READ_GATE", gates.display().to_string()),
        ] {
            self.tmux(&["set-environment", "-t", &self.home, key, &value]);
        }
        for key in ["CLAUDE_CONFIG_DIR", "CODEX_HOME"] {
            self.tmux(&["set-environment", "-r", "-t", &self.home, key]);
        }
        fs::write(
            self.root.join("config"),
            if chat {
                "[workspace]\nchat = chat\n"
            } else {
                "[workspace]\nchat = app\n"
            },
        )
        .expect("console choice");
        let out = super::cli::ae()
            .env("AE_HOME", &self.root)
            .env("HOME", &self.root)
            .env("CONFIG_FILE", self.root.join("config"))
            .env("AE_TMUX_SERVER_KIND", "socket")
            .env("AE_TMUX_SERVER", &self.socket)
            .env("TMUX", format!("{},1,0", self.socket.display()))
            .env("TMUX_PANE", &self.tool.pane)
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("CODEX_HOME")
            .arg("_console")
            .output()
            .expect("private owner console");
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let pane = self
            .tmux(&[
                "list-panes",
                "-s",
                "-t",
                &self.home,
                "-F",
                "#{pane_id}|#{@ae_console}",
            ])
            .lines()
            .find_map(|row| {
                let (pane, stamp) = row.split_once('|')?;
                (stamp == UUID).then(|| pane.to_owned())
            })
            .expect("stamped console");
        self.wait(&pane, "GUARD console ready", |s| {
            if chat {
                s.contains("to lead>")
            } else {
                s.contains("Enter writes")
            }
        });
        pane
    }

    /// A real terminal, with both caller tmux variables explicitly removed.
    /// tmux supplies only the fixture PTY; ae resolves the recorded server.
    fn outside(&self, name: &str) -> String {
        let gates = self.root.join(format!("gates-{name}"));
        fs::create_dir(&gates).expect("private gates");
        let args = vec![
            "new-window".to_owned(),
            "-d".into(),
            "-P".into(),
            "-F".into(),
            "#{pane_id}".into(),
            "-t".into(),
            format!("={}:", self.home),
            "-n".into(),
            name.into(),
            "env".into(),
            "-u".into(),
            "TMUX".into(),
            "-u".into(),
            "TMUX_PANE".into(),
            "-u".into(),
            "CLAUDE_CONFIG_DIR".into(),
            "-u".into(),
            "CODEX_HOME".into(),
            format!("AE_HOME={}", self.root.display()),
            format!("HOME={}", self.root.display()),
            format!("CONFIG_FILE={}", self.root.join("config").display()),
            "AE_TMUX_SERVER_KIND=socket".into(),
            format!("AE_TMUX_SERVER={}", self.socket.display()),
            format!("AE_TEST_APP_READ_GATE={}", gates.display()),
            env!("CARGO_BIN_EXE_ae").into(),
            "app".into(),
            self.home.clone(),
        ];
        let refs: Vec<_> = args.iter().map(String::as_str).collect();
        let pane = self.tmux(&refs).trim().to_owned();
        assert!(pane.starts_with('%'), "fixture app pane: {pane}");
        self.wait(&pane, "GUARD outside app drew home", |s| {
            s.contains("▣ 1 ") && s.contains(&self.home)
        });
        pane
    }

    fn screen(&self, pane: &str) -> String {
        self.tmux(&["capture-pane", "-p", "-t", pane])
    }

    fn wait(&self, pane: &str, why: &str, met: impl Fn(&str) -> bool) -> String {
        let until = Instant::now() + WAIT;
        loop {
            let screen = self.screen(pane);
            if met(&screen) {
                return screen;
            }
            assert!(Instant::now() < until, "{why}; screen:\n{screen}");
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    /// Poll the on-disk kept drafts until they are exactly `want`.
    fn wait_bodies(&self, want: &[&[u8]]) {
        let mut sorted: Vec<Vec<u8>> = want.iter().map(|body| body.to_vec()).collect();
        sorted.sort();
        let until = Instant::now() + WAIT;
        loop {
            if draft_bodies(&self.tool.dir) == sorted {
                return;
            }
            assert!(
                Instant::now() < until,
                "kept drafts never became {sorted:?}: {:?}",
                draft_bodies(&self.tool.dir)
            );
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    /// Poll until no kept draft record holds `body`. A Sent outcome's
    /// last act is clearing the record its own admission kept, after the
    /// journal already holds the ask — so once the journal is proven,
    /// absence proves the finish ran. The screen's "sent" word proves
    /// neither: it is a substring of a body like `dc-b-sent`, drawn
    /// before the finish clears anything.
    fn wait_body_gone(&self, body: &[u8]) {
        let until = Instant::now() + WAIT;
        loop {
            if !draft_bodies(&self.tool.dir).iter().any(|kept| kept == body) {
                return;
            }
            assert!(
                Instant::now() < until,
                "the sent ask's own record was never cleared: {:?}",
                draft_bodies(&self.tool.dir)
            );
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    fn raw(&self, pane: &str, text: &str) {
        self.tmux(&["send-keys", "-t", pane, "-l", "--", text]);
    }

    fn key(&self, pane: &str, key: &str) {
        self.tmux(&["send-keys", "-t", pane, key]);
    }

    /// Enter writing where NO offer and no restore are expected to fill.
    fn enter(&self, pane: &str) {
        self.raw(pane, "i");
        self.wait(pane, "S1 plain entry acquires write mode", writing);
        self.free();
    }

    /// Enter writing where the S1 orphan offer must be shown, unfilled.
    fn enter_offering(&self, pane: &str, line: &str) {
        self.raw(pane, "i");
        let screen = self.wait(pane, "S1 orphan is offered, not auto-filled", |s| {
            offer_shown(s)
        });
        assert!(
            !self.draft_row(&screen).contains(line),
            "S1 offer fills nothing before Enter"
        );
        self.free();
    }

    fn escape(&self, pane: &str) {
        self.key(pane, "Escape");
        self.wait(pane, "Esc returns to browsing", |s| {
            s.contains("Enter writes")
        });
        self.free();
    }

    /// Leave writing however many Escapes the mode takes (an offer's Esc
    /// discards first); ends browsing with the lease free.
    fn browse(&self, pane: &str) {
        for _ in 0..3 {
            if self.screen(pane).contains("Enter writes") {
                self.free();
                return;
            }
            self.key(pane, "Escape");
            std::thread::sleep(Duration::from_millis(100));
        }
        self.escape(pane);
    }

    /// Kill the app/chat process in `pane` (a crash: no cleanup runs) and
    /// wait until the pane is gone, so kernel locks are released.
    fn kill(&self, pane: &str) {
        self.tmux(&["kill-window", "-t", pane]);
        let until = Instant::now() + WAIT;
        loop {
            let panes = self.tmux(&["list-panes", "-s", "-t", &self.home, "-F", "#{pane_id}"]);
            if !panes.lines().any(|row| row == pane) {
                return;
            }
            assert!(Instant::now() < until, "killed pane {pane} never went away");
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    /// The row under the home composer's address row: where its draft shows.
    fn draft_row<'a>(&self, screen: &'a str) -> &'a str {
        let rows: Vec<&str> = screen.lines().collect();
        let address = rows
            .iter()
            .rposition(|row| row.contains(&format!("to {}", self.home)))
            .expect("home composer address row");
        let row = *rows.get(address + 1).expect("the draft row under it");
        row.split_once('│').map_or(row, |(_, draft)| draft)
    }

    fn typed(&self, pane: &str, text: &str) {
        self.raw(pane, text);
        self.wait(pane, "GUARD fresh draft bytes drawn", |s| {
            self.draft_row(s).contains(text)
        });
    }

    /// Clear the composer memory (^U). The disk record is NOT judged here.
    fn clear(&self, pane: &str, text: &str) {
        self.key(pane, "C-u");
        self.wait(pane, "GUARD composer memory cleared", |s| {
            !self.draft_row(s).contains(text)
        });
    }

    fn asks(&self) -> Vec<Event> {
        self.tool
            .events()
            .lines()
            .map(|row| Event::parse_line(row).expect("fixture event"))
            .filter(|e| e.actor == "console:local" && e.action == "ask")
            .collect()
    }

    /// Accept a shown offer with one fresh Enter: it must FILL `line`
    /// behind the kept-line warning and send nothing by itself.
    fn accept_offer(&self, pane: &str, line: &str) {
        let before = self.asks().len();
        self.key(pane, "Enter");
        self.wait(
            pane,
            "S1 Enter on the offer fills behind the warning",
            |s| self.draft_row(s).contains(line) && s.contains(KEPT_WARN),
        );
        assert_eq!(
            self.asks().len(),
            before,
            "S1 the filling Enter never submits"
        );
    }

    fn submit(&self, pane: &str, text: &str, count: usize) {
        self.key(pane, "Enter");
        self.wait(pane, "one literal ask confirmed", |s| {
            s.contains("sent") && self.asks().len() == count
        });
        let asks = self.asks();
        let ask = asks.last().expect("new ask");
        assert_eq!(ask.summary.as_deref(), Some(text), "literal journal body");
        assert_eq!(ask.target.as_deref(), Some("lead"));
        assert!(
            self.tool.submitted().contains(text),
            "real harness submitted the body"
        );
        // Only now may a caller judge the on-disk records: this ask's
        // own finish has run, so whatever records remain are final.
        self.wait_body_gone(text.as_bytes());
    }

    fn lease(&self) -> File {
        let file = OpenOptions::new()
            .append(true)
            .create(true)
            .open(self.tool.dir.join(WRITER))
            .expect("regular lease");
        file.try_lock().expect("fixture acquires free kernel lease");
        file
    }

    fn free(&self) {
        let until = Instant::now() + WAIT;
        loop {
            if !self.tool.dir.join(WRITER).exists() {
                return;
            }
            let file = OpenOptions::new()
                .append(true)
                .open(self.tool.dir.join(WRITER))
                .expect("lease exists");
            match file.try_lock() {
                Ok(()) => return,
                Err(TryLockError::WouldBlock) => {}
                Err(why) => panic!("fixture lease witness: {why}"),
            }
            assert!(
                Instant::now() < until,
                "exit failed to release kernel lease"
            );
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    /// Plant the legacy unstamped kept file (invariant 5's orphan).
    fn kept(&self, bytes: &str) {
        fs::write(self.tool.dir.join("console.draft"), bytes).expect("legacy kept file");
    }

    /// Point the fixture's lead stamp back at the fake TUI after a
    /// missing-target failure, so a later send can succeed.
    fn fix_target(&self) {
        self.tmux(&[
            "set-option",
            "-p",
            "-t",
            &self.tool.pane,
            "@ae_agent",
            "lead",
        ]);
    }

    fn swallow_submit(&self) {
        let args = [
            "respawn-pane".to_owned(),
            "-k".into(),
            "-t".into(),
            self.tool.pane.clone(),
            "perl".into(),
            self.root.join("faketui.pl").display().to_string(),
            self.root.join("received").display().to_string(),
            self.root.join("enters").display().to_string(),
            "400".into(),
            "codex".into(),
            "0".into(),
            "swallow".into(),
            self.root.join("control").display().to_string(),
        ];
        self.tmux(&args.iter().map(String::as_str).collect::<Vec<_>>());
        self.wait(
            &self.tool.pane,
            "GUARD empty modelled fake restarted",
            |s| s.contains("fake tui transcript") && s.contains("fake-model"),
        );
    }

    /// One real failed send of `text`: Unknown (no ask record) or
    /// Uncertain (pasted, submit swallowed). The writer's own kept
    /// record must hold `text` afterwards, and only that.
    fn failed_send(&self, pane: &str, text: &str, uncertain: bool) {
        self.typed(pane, text);
        if uncertain {
            self.swallow_submit();
        } else {
            // A real tracked-path refusal with no ask record gives Unknown.
            self.tmux(&[
                "set-option",
                "-p",
                "-t",
                &self.tool.pane,
                "@ae_agent",
                "missing-target",
            ]);
        }
        self.key(pane, "Enter");
        self.wait(pane, "GUARD real failed outcome", |s| {
            s.contains(if uncertain {
                "uncertain: check the lead pane"
            } else {
                "no record of it"
            })
        });
        assert!(
            draft_bodies(&self.tool.dir)
                .iter()
                .any(|body| body == text.as_bytes()),
            "the writer's own record holds the failed line: {:?}",
            draft_bodies(&self.tool.dir)
        );
        assert!(
            self.tool.submitted().is_empty(),
            "fake harness confirms no submit"
        );
    }

    fn entries(&self) -> BTreeSet<String> {
        fs::read_dir(&self.tool.dir)
            .expect("session dir is listable")
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect()
    }
}

/// Control: the rig's plain and failed paths behave on ANY build, so the
/// REDs below name S1 and not a broken fixture.
#[test]
fn controls_plain_and_failed_sends_behave_as_today() {
    let rig = Rig::new("dccontrol");
    let pane = rig.console(false);
    rig.enter(&pane);
    rig.typed(&pane, "dc-control-sent");
    rig.submit(&pane, "dc-control-sent", 1);
    assert_no_kept_draft(&rig.tool.dir);
    rig.typed(&pane, "dc-control-failed");
    rig.tmux(&[
        "set-option",
        "-p",
        "-t",
        &rig.tool.pane,
        "@ae_agent",
        "missing-target",
    ]);
    rig.key(&pane, "Enter");
    rig.wait(&pane, "GUARD control failed outcome", |s| {
        s.contains("no record of it")
    });
    assert_eq!(rig.asks().len(), 1, "the failure recorded no ask");
    assert_kept_drafts(&rig.tool.dir, &[b"dc-control-failed"]);
}

/// Invariants 1, 2, 6: window A's uncertain line stays A's while A lives
/// (B is offered and filled with NOTHING), and becomes B's offer only
/// once A's process is gone.
#[test]
fn a_live_windows_kept_line_is_never_offered_in_another_window_but_is_once_it_dies() {
    let rig = Rig::new("dclive");
    let a = rig.outside("win-a");
    rig.enter(&a);
    rig.failed_send(&a, "dc-live-kept", true);
    rig.clear(&a, "dc-live-kept");
    rig.escape(&a);
    let b = rig.outside("win-b");
    rig.enter(&b);
    let screen = rig.screen(&b);
    assert!(
        !offer_shown(&screen),
        "S1 a live instance's record is never offered: {screen}"
    );
    assert!(
        !rig.draft_row(&screen).contains("dc-live-kept"),
        "S1 a live instance's line is never filled elsewhere"
    );
    rig.escape(&b);
    rig.kill(&a);
    rig.enter_offering(&b, "dc-live-kept");
    rig.accept_offer(&b, "dc-live-kept");
    assert_eq!(
        rig.asks().len(),
        1,
        "only A's own unconfirmed ask exists; recovery sent nothing"
    );
}

/// Invariants 3, 5: the legacy orphan is offered, the acquiring chunk's
/// own Enter cannot accept it, plain typing at the offer does nothing,
/// a fresh Enter fills only, and the next fresh Enter sends.
#[test]
fn enter_on_an_offer_fills_only_and_the_next_fresh_enter_sends() {
    let rig = Rig::new("dcenter");
    rig.kept("dc-legacy-line");
    let pane = rig.outside("win-a");
    // One literal chunk: the Enter that rode in with the acquiring `i`
    // predates the offer and must not count as accepting it.
    rig.raw(&pane, "i\r");
    let screen = rig.wait(&pane, "S1 legacy orphan is offered", offer_shown);
    assert!(
        !rig.draft_row(&screen).contains("dc-legacy-line"),
        "S1 the acquiring Enter never fills or sends"
    );
    assert!(rig.asks().is_empty());
    rig.raw(&pane, "zzz");
    std::thread::sleep(Duration::from_millis(500));
    let screen = rig.screen(&pane);
    assert!(
        offer_shown(&screen),
        "S1 typing at the offer leaves it standing: {screen}"
    );
    assert!(
        !rig.draft_row(&screen).contains("dc-legacy-line")
            && !rig.draft_row(&screen).contains("zzz"),
        "S1 typing at the offer neither fills nor composes"
    );
    assert!(rig.asks().is_empty() && rig.tool.submitted().is_empty());
    assert_kept_drafts(&rig.tool.dir, &[b"dc-legacy-line"]);
    rig.accept_offer(&pane, "dc-legacy-line");
    assert!(
        rig.tool.submitted().is_empty(),
        "the fill submitted nothing"
    );
    rig.submit(&pane, "dc-legacy-line", 1);
    rig.wait_bodies(&[]);
}

/// Invariants 3, 4, 5: Esc at the offer discards durably — the bytes are
/// gone from disk, no ask exists, and no later entry offers them again.
#[test]
fn escape_on_an_offer_discards_the_line_durably() {
    let rig = Rig::new("dcesc");
    rig.kept("dc-discard-line");
    let pane = rig.outside("win-a");
    rig.enter_offering(&pane, "dc-discard-line");
    rig.key(&pane, "Escape");
    rig.wait_bodies(&[]);
    assert!(rig.asks().is_empty() && rig.tool.submitted().is_empty());
    rig.browse(&pane);
    rig.enter(&pane);
    let screen = rig.screen(&pane);
    assert!(
        !rig.draft_row(&screen).contains("dc-discard-line"),
        "S1 a discarded line never comes back"
    );
    // A fresh Enter on the empty composer has no offer to accept and no
    // line to send: the ask count is the proof either way.
    rig.key(&pane, "Enter");
    std::thread::sleep(Duration::from_millis(500));
    assert!(rig.asks().is_empty(), "nothing was left to recover or send");
    assert_no_kept_draft(&rig.tool.dir);
}

/// Invariant 4 (simultaneous recovery): of two browsing windows exactly
/// one recovers the orphan. The loser, entering after the winner claimed
/// and browsed, is offered nothing while the winner lives — and gets the
/// offer only after the winner's process dies.
#[test]
fn only_one_window_recovers_an_orphan_and_a_second_waits_for_the_claimant_to_die() {
    let rig = Rig::new("dcone");
    let a = rig.outside("win-a");
    rig.enter(&a);
    rig.failed_send(&a, "dc-one-line", false);
    rig.clear(&a, "dc-one-line");
    rig.escape(&a);
    rig.kill(&a);
    rig.fix_target();
    let b = rig.outside("win-b");
    rig.enter_offering(&b, "dc-one-line");
    rig.accept_offer(&b, "dc-one-line");
    rig.escape(&b);
    let c = rig.outside("win-c");
    rig.enter(&c);
    let screen = rig.screen(&c);
    assert!(
        !offer_shown(&screen),
        "S1 a claimed record is refused to a second window: {screen}"
    );
    assert!(
        !rig.draft_row(&screen).contains("dc-one-line"),
        "S1 the second window never fills the claimed line"
    );
    rig.escape(&c);
    rig.kill(&b);
    rig.enter_offering(&c, "dc-one-line");
    rig.accept_offer(&c, "dc-one-line");
    assert!(
        rig.asks().is_empty(),
        "recovery between windows sent nothing"
    );
    rig.submit(&c, "dc-one-line", 1);
    rig.wait_bodies(&[]);
}

/// Invariant 4 (claimant crash): bytes stay offerable when the claimant
/// dies undecided at the offer, and again when the next claimant dies
/// after the fill but before any send.
#[test]
fn a_claimant_that_crashes_offered_or_filled_leaves_the_line_offerable() {
    let rig = Rig::new("dccrash");
    rig.kept("dc-crash-line");
    let b1 = rig.outside("win-b1");
    rig.enter_offering(&b1, "dc-crash-line");
    rig.kill(&b1);
    rig.free();
    let b2 = rig.outside("win-b2");
    rig.enter_offering(&b2, "dc-crash-line");
    rig.accept_offer(&b2, "dc-crash-line");
    rig.kill(&b2);
    rig.free();
    let c = rig.outside("win-c");
    rig.enter_offering(&c, "dc-crash-line");
    rig.accept_offer(&c, "dc-crash-line");
    assert!(rig.asks().is_empty(), "two crashes sent nothing");
    assert_kept_drafts(&rig.tool.dir, &[b"dc-crash-line"]);
}

/// Invariants 2, 4 (several orphans): two dead instances leave two
/// lines. They are offered one per acquisition — never both inside one
/// writing — in any order, until each is resolved exactly once.
#[test]
fn several_orphans_are_offered_one_per_acquisition_until_all_are_resolved() {
    let rig = Rig::new("dcmany");
    let a1 = rig.outside("win-a1");
    rig.enter(&a1);
    rig.failed_send(&a1, "dc-orphan-one", false);
    rig.clear(&a1, "dc-orphan-one");
    rig.escape(&a1);
    let a2 = rig.outside("win-a2");
    rig.enter(&a2);
    let screen = rig.screen(&a2);
    assert!(
        !offer_shown(&screen) && !rig.draft_row(&screen).contains("dc-orphan-one"),
        "S1 the second live window starts clean: {screen}"
    );
    rig.failed_send(&a2, "dc-orphan-two", false);
    rig.clear(&a2, "dc-orphan-two");
    rig.escape(&a2);
    // The own-record assertion inside `failed_send` judged each window's
    // send in isolation; together both records now sit on disk.
    rig.wait_bodies(&[b"dc-orphan-one", b"dc-orphan-two"]);
    rig.kill(&a1);
    rig.kill(&a2);
    rig.fix_target();
    let b = rig.outside("win-b");
    rig.raw(&b, "i");
    let screen = rig.wait(&b, "S1 first orphan is offered", offer_shown);
    assert!(
        !rig.draft_row(&screen).contains("dc-orphan-one")
            && !rig.draft_row(&screen).contains("dc-orphan-two"),
        "S1 an offer fills nothing"
    );
    rig.key(&b, "Enter");
    let screen = rig.wait(&b, "S1 first offer fills behind the warning", |s| {
        s.contains(KEPT_WARN)
            && (rig.draft_row(s).contains("dc-orphan-one")
                || rig.draft_row(s).contains("dc-orphan-two"))
    });
    let first = if rig.draft_row(&screen).contains("dc-orphan-one") {
        "dc-orphan-one"
    } else {
        "dc-orphan-two"
    };
    let other = if first == "dc-orphan-one" {
        "dc-orphan-two"
    } else {
        "dc-orphan-one"
    };
    assert!(rig.asks().is_empty(), "the fill sent nothing");
    rig.submit(&b, first, 1);
    let screen = rig.screen(&b);
    assert!(
        !rig.draft_row(&screen).contains(other),
        "S1 no second orphan is filled inside the same writing"
    );
    rig.wait_bodies(&[other.as_bytes()]);
    rig.escape(&b);
    rig.raw(&b, "i");
    rig.wait(&b, "S1 next acquisition offers without filling", |s| {
        writing(s) && offer_shown(s) && rig.draft_row(s).trim().is_empty()
    });
    rig.free();
    // This process already carries the first offer's notice, so the
    // second offer is proven by what Enter does, not by the fragments.
    rig.accept_offer(&b, other);
    rig.submit(&b, other, 2);
    rig.wait_bodies(&[]);
    let asks = rig.asks();
    let mut summaries: Vec<&str> = asks
        .iter()
        .map(|ask| ask.summary.as_deref().expect("summary"))
        .collect();
    summaries.sort_unstable();
    assert_eq!(
        summaries,
        ["dc-orphan-one", "dc-orphan-two"],
        "each orphan was recovered and sent exactly once"
    );
}

/// Invariant 1 (own restore keeps the warning) + precedence: a window
/// with its OWN kept line on disk restores it on re-entry — filled at
/// once, behind the unchanged warning, with no offer — even while a
/// legacy orphan also waits. Only once the own line is sent (clearing
/// the own record alone) is the orphan offered.
#[test]
fn an_own_kept_line_restores_with_the_warning_and_precedes_any_orphan() {
    let rig = Rig::new("dcown");
    let pane = rig.outside("win-a");
    rig.enter(&pane);
    rig.failed_send(&pane, "dc-own-line", false);
    rig.clear(&pane, "dc-own-line");
    rig.escape(&pane);
    rig.kept("dc-orphan-legacy");
    rig.fix_target();
    rig.raw(&pane, "i");
    let screen = rig.wait(&pane, "S1 own line restored with the warning", |s| {
        rig.draft_row(s).contains("dc-own-line") && s.contains(KEPT_WARN)
    });
    assert!(
        !offer_shown(&screen),
        "S1 own restore needs no consent offer: {screen}"
    );
    assert!(
        !rig.draft_row(&screen).contains("dc-orphan-legacy"),
        "S1 the orphan never displaces the own line"
    );
    assert!(rig.asks().is_empty(), "a restore never submits");
    rig.submit(&pane, "dc-own-line", 1);
    rig.wait_bodies(&[b"dc-orphan-legacy"]);
    rig.escape(&pane);
    rig.enter_offering(&pane, "dc-orphan-legacy");
    rig.accept_offer(&pane, "dc-orphan-legacy");
    assert_eq!(rig.asks().len(), 1, "the fill sent nothing");
}

/// Invariants 1, 6 (durability + Sent clears only its own): A's
/// cap-refused line is kept in A's record; B then closes one ask and
/// sends its own line successfully — and A's kept line still restores
/// for A afterwards, byte for byte.
#[test]
fn one_windows_sent_ask_never_clears_another_live_windows_kept_line() {
    let rig = Rig::new("dcsent");
    let a = rig.outside("win-a");
    rig.enter(&a);
    for at in 1..=5 {
        let body = format!("dc-open-{at}");
        rig.typed(&a, &body);
        rig.submit(&a, &body, at);
    }
    rig.typed(&a, "dc-cap-kept");
    rig.key(&a, "Enter");
    rig.wait(&a, "GUARD shared admission enforces five open", |s| {
        s.contains("5 requests open")
    });
    assert_kept_drafts(&rig.tool.dir, &[b"dc-cap-kept"]);
    rig.clear(&a, "dc-cap-kept");
    rig.escape(&a);
    let b = rig.outside("win-b");
    rig.enter(&b);
    let screen = rig.screen(&b);
    assert!(
        !offer_shown(&screen) && !rig.draft_row(&screen).contains("dc-cap-kept"),
        "S1 A's live kept line is invisible to B: {screen}"
    );
    rig.typed(&b, "/close");
    rig.key(&b, "Enter");
    rig.wait(&b, "GUARD B closes one open ask", |s| s.contains("closed "));
    assert_kept_drafts(&rig.tool.dir, &[b"dc-cap-kept"]);
    rig.typed(&b, "dc-b-sent");
    rig.submit(&b, "dc-b-sent", 6);
    assert_kept_drafts(&rig.tool.dir, &[b"dc-cap-kept"]);
    rig.escape(&b);
    rig.raw(&a, "i");
    rig.wait(&a, "S1 A restores its own cap-kept line", |s| {
        rig.draft_row(s).contains("dc-cap-kept") && s.contains(KEPT_WARN)
    });
    assert_eq!(rig.asks().len(), 6, "restoring sent nothing new");
}

/// Invariant 7 (owner chat path): a dead app window's kept line is
/// OFFERED at the owner chat's promotion — the prompt is not filled —
/// Enter fills it without sending, and the next Enter sends it.
#[test]
fn owner_chat_offers_a_dead_windows_line_and_enter_fills_without_sending() {
    let rig = Rig::new("dcchatfill");
    let a = rig.outside("win-a");
    rig.enter(&a);
    rig.failed_send(&a, "dc-chat-offered", false);
    rig.clear(&a, "dc-chat-offered");
    rig.escape(&a);
    rig.kill(&a);
    rig.fix_target();
    let chat = rig.console(true);
    let screen = rig.wait(&chat, "S1 owner chat offers the orphan", offer_shown);
    assert!(
        !screen.contains("to lead> dc-chat-offered"),
        "S1 the offer never fills the chat prompt: {screen}"
    );
    assert!(rig.asks().is_empty());
    rig.key(&chat, "Enter");
    rig.wait(&chat, "S1 chat Enter fills only", |s| {
        s.contains("to lead> dc-chat-offered")
    });
    assert!(
        rig.asks().is_empty() && rig.tool.submitted().is_empty(),
        "the filling Enter submitted nothing"
    );
    rig.key(&chat, "Enter");
    rig.wait(&chat, "GUARD chat send confirmed", |s| {
        s.contains("sent") && rig.asks().len() == 1
    });
    assert_eq!(
        rig.asks()[0].summary.as_deref(),
        Some("dc-chat-offered"),
        "the second Enter sent exactly the offered line"
    );
    rig.wait_bodies(&[]);
}

/// Invariant 7 (owner chat discard): Esc at the chat's offer discards
/// the legacy line durably and sends nothing; the chat then composes
/// and sends its own line normally.
#[test]
fn owner_chat_escape_on_an_offer_discards_without_sending() {
    let rig = Rig::new("dcchatesc");
    rig.kept("dc-chat-discarded");
    let chat = rig.console(true);
    rig.wait(&chat, "S1 owner chat offers the legacy line", |s| {
        offer_shown(s)
    });
    rig.key(&chat, "Escape");
    rig.wait_bodies(&[]);
    assert!(rig.asks().is_empty() && rig.tool.submitted().is_empty());
    rig.raw(&chat, "dc-chat-own-line");
    rig.key(&chat, "Enter");
    rig.wait(
        &chat,
        "GUARD chat sends its own line after a discard",
        |s| s.contains("sent") && rig.asks().len() == 1,
    );
    assert_eq!(
        rig.asks()[0].summary.as_deref(),
        Some("dc-chat-own-line"),
        "only the chat's own line was ever sent"
    );
}

/// Invariants 2, 7 (owner chat, live owner): while the app that kept a
/// line still lives, the owner chat is offered nothing, fills nothing,
/// and its own successful send leaves the live record untouched.
#[test]
fn owner_chat_never_offers_a_live_apps_kept_line() {
    let rig = Rig::new("dcchatlive");
    let a = rig.outside("win-a");
    rig.enter(&a);
    rig.failed_send(&a, "dc-chat-live-kept", false);
    rig.clear(&a, "dc-chat-live-kept");
    rig.escape(&a);
    rig.fix_target();
    let chat = rig.console(true);
    std::thread::sleep(Duration::from_millis(500));
    let screen = rig.screen(&chat);
    assert!(
        !offer_shown(&screen),
        "S1 a live app's line is never offered to the chat: {screen}"
    );
    assert!(
        !screen.contains("to lead> dc-chat-live-kept"),
        "S1 a live app's line never fills the chat prompt"
    );
    rig.raw(&chat, "dc-chat-direct");
    rig.key(&chat, "Enter");
    rig.wait(&chat, "GUARD owner chat still submits", |s| {
        s.contains("sent") && rig.asks().len() == 1
    });
    assert_kept_drafts(&rig.tool.dir, &[b"dc-chat-live-kept"]);
}

/// Hostile session directory: names that merely look draft-like — an
/// empty instance, a non-UUID instance, a record with extra segments —
/// and a grammar-valid name that is a SYMLINK are never offered, never
/// read into the composer and never modified or removed. The one true
/// legacy orphan beside them still is.
#[test]
fn draft_like_names_that_are_not_records_are_never_offered_or_touched() {
    let rig = Rig::new("dchostile");
    rig.kept("dc-real-legacy");
    let dir = rig.tool.dir.clone();
    fs::write(dir.join("console.draft."), "dc-hostile-empty").expect("empty instance");
    fs::write(dir.join("console.draft.not-an-instance"), "dc-hostile-word").expect("word instance");
    fs::write(
        dir.join("console.draft.0199c0de-cccc-4890-abcd-ef0123456789.extra"),
        "dc-hostile-extra",
    )
    .expect("extra segment");
    let secret = rig.root.join("secret");
    fs::write(&secret, "dc-hostile-secret").expect("link target outside the session");
    let link = dir.join("console.draft.0199c0de-dddd-4890-abcd-ef0123456789");
    std::os::unix::fs::symlink(&secret, &link).expect("valid-name symlink record");
    let pane = rig.outside("win-a");
    rig.enter_offering(&pane, "dc-real-legacy");
    let screen = rig.screen(&pane);
    for hostile in [
        "dc-hostile-empty",
        "dc-hostile-word",
        "dc-hostile-extra",
        "dc-hostile-secret",
    ] {
        assert!(
            !screen.contains(hostile),
            "S1 hostile bytes never reach the screen: {hostile}"
        );
    }
    rig.accept_offer(&pane, "dc-real-legacy");
    assert!(rig.asks().is_empty());
    assert_eq!(
        fs::read(dir.join("console.draft.")).expect("empty instance untouched"),
        b"dc-hostile-empty"
    );
    assert_eq!(
        fs::read(dir.join("console.draft.not-an-instance")).expect("word instance untouched"),
        b"dc-hostile-word"
    );
    assert_eq!(
        fs::read(dir.join("console.draft.0199c0de-cccc-4890-abcd-ef0123456789.extra"))
            .expect("extra segment untouched"),
        b"dc-hostile-extra"
    );
    assert!(
        fs::symlink_metadata(&link).is_ok_and(|meta| meta.file_type().is_symlink()),
        "the symlink record was never followed or replaced"
    );
    assert_eq!(
        fs::read(&secret).expect("link target untouched"),
        b"dc-hostile-secret"
    );
}

/// Invariant 2 (unknown liveness): an orphan whose lifetime lock cannot
/// be taken OR proven held — here the lock node is a directory, so the
/// zero-wait take fails for a reason other than a live holder — is not
/// offered, not filled and not touched. Once the lock node is a regular
/// free file again, the same orphan is offered.
#[test]
fn an_orphan_whose_liveness_cannot_be_proven_is_not_offered_and_not_touched() {
    let rig = Rig::new("dcunknown");
    let baseline = rig.entries();
    let a = rig.outside("win-a");
    rig.enter(&a);
    rig.failed_send(&a, "dc-unknown-line", false);
    rig.clear(&a, "dc-unknown-line");
    rig.escape(&a);
    rig.kill(&a);
    // Discover this instance's own files without assuming their names:
    // everything new beside the known admission lock and body store,
    // split into the record (it holds the line) and the rest (the
    // lifetime lock a claimant must take to prove A gone).
    let mut records = Vec::new();
    let mut locks = Vec::new();
    for name in rig.entries().difference(&baseline) {
        if name == ".console-admission.lock" || name == "messages" {
            continue;
        }
        let path = rig.tool.dir.join(name);
        if fs::read(&path).is_ok_and(|body| body == b"dc-unknown-line") {
            records.push(name.clone());
        } else {
            locks.push(name.clone());
        }
    }
    assert_eq!(
        records.len(),
        1,
        "exactly one record holds the line: {records:?}"
    );
    assert!(
        !locks.is_empty(),
        "S1 the instance holds a separate lifetime lock file beside its record"
    );
    for lock in &locks {
        let path = rig.tool.dir.join(lock);
        fs::remove_file(&path).expect("lock node replaced");
        fs::create_dir(&path).expect("lock node is now a directory");
    }
    let b = rig.outside("win-b");
    rig.enter(&b);
    let screen = rig.screen(&b);
    assert!(
        !offer_shown(&screen),
        "S1 unknown liveness offers nothing: {screen}"
    );
    assert!(
        !rig.draft_row(&screen).contains("dc-unknown-line"),
        "S1 unknown liveness fills nothing"
    );
    assert_kept_drafts(&rig.tool.dir, &[b"dc-unknown-line"]);
    rig.escape(&b);
    for lock in &locks {
        let path = rig.tool.dir.join(lock);
        fs::remove_dir(&path).expect("lock node restored");
        fs::write(&path, "").expect("lock node is a free regular file again");
    }
    rig.enter_offering(&b, "dc-unknown-line");
    rig.accept_offer(&b, "dc-unknown-line");
    assert!(rig.asks().is_empty(), "the recovered line was never resent");
}
