//! Frozen appuse B1 acceptance, from R-B1..R-B7 and the R-B2b cut.
//! Real apps/chat, kernel locks, admission and delivery on private tmux.
//! Read gates attest real READ stamps before holding dispatch; no synthetic
//! acquisition time, ownership mock, or replacement of the submit path.

#![allow(
    clippy::disallowed_methods,
    clippy::expect_used,
    reason = "acceptance owns private terminal fixtures and reads their effects"
)]

use std::fs::{self, File, OpenOptions, TryLockError};
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::{Duration, Instant};

use ae::events::Event;

const UUID: &str = "0199c0de-cccc-4890-abcd-ef0123456789";
const OTHER_UUID: &str = "0199c0de-bbbb-4890-abcd-ef0123456789";
const WAIT: Duration = Duration::from_secs(20);
const WRITER: &str = ".console-writer.lock";

/// A stalled fixture releases on unwind and independently after 45 seconds.
struct Hold {
    path: PathBuf,
    trace: PathBuf,
    before: usize,
    release: Option<mpsc::Sender<()>>,
    watcher: Option<std::thread::JoinHandle<()>>,
}

impl Hold {
    fn new(dir: &std::path::Path, key: &str) -> Self {
        let path = dir.join(key);
        let trace = dir.join("trace");
        let before = fs::read_to_string(&trace)
            .unwrap_or_default()
            .lines()
            .count();
        fs::write(&path, "hold").expect("fixture read gate");
        let (release, wait) = mpsc::channel();
        let release_path = path.clone();
        let watcher = std::thread::spawn(move || {
            let _ = wait.recv_timeout(Duration::from_secs(45));
            let _ = fs::remove_file(release_path);
        });
        Self {
            path,
            trace,
            before,
            release: Some(release),
            watcher: Some(watcher),
        }
    }

    fn held(&self) {
        let until = Instant::now() + WAIT;
        let key = self.path.file_name().expect("gate key").to_string_lossy();
        loop {
            let trace = fs::read_to_string(&self.trace).unwrap_or_default();
            if trace
                .lines()
                .skip(self.before)
                .any(|row| row == format!("held {key}"))
            {
                assert!(self.path.exists(), "GUARD attested read is still held");
                return;
            }
            assert!(
                Instant::now() < until,
                "GUARD real {key} read never attested: {trace}"
            );
            std::thread::sleep(Duration::from_millis(25));
        }
    }
}

impl Drop for Hold {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
        if let Some(release) = self.release.take() {
            let _ = release.send(());
        }
        if let Some(watcher) = self.watcher.take() {
            let _ = watcher.join();
        }
    }
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
    fn outside(&self, name: &str) -> (String, PathBuf) {
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
            s.contains("Sessions 1") && s.contains(&self.home)
        });
        (pane, gates)
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

    fn raw(&self, pane: &str, text: &str) {
        self.tmux(&["send-keys", "-t", pane, "-l", "--", text]);
    }

    fn old_chunk(&self, pane: &str, text: &str) {
        // Keep dispatch before acquisition while the reader stamps the
        // literal paste. The separate boundary proof also holds @app-read.
        let keys = Hold::new(&self.root.join("gates-owner"), "@app-keys");
        self.raw(pane, text);
        keys.held();
        drop(keys);
    }

    fn key(&self, pane: &str, key: &str) {
        self.tmux(&["send-keys", "-t", pane, key]);
    }

    fn enter(&self, pane: &str) {
        self.raw(pane, "i");
        self.wait(pane, "R-B1 home app can acquire write mode", |s| {
            s.contains("Enter sends")
        });
        self.held();
    }

    fn escape(&self, pane: &str) {
        self.key(pane, "Escape");
        self.wait(pane, "Esc returns to browsing", |s| {
            s.contains("Enter writes")
        });
        self.free();
    }

    fn draft_row<'a>(&self, screen: &'a str) -> &'a str {
        screen
            .lines()
            .rev()
            .find(|row| row.contains("› lead") && row.contains(&self.home))
            .expect("home composer row")
    }

    fn typed(&self, pane: &str, text: &str) {
        self.raw(pane, text);
        self.wait(pane, "GUARD fresh draft bytes drawn", |s| {
            self.draft_row(s).contains(text)
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

    fn submit(&self, pane: &str, text: &str, count: usize) {
        self.key(pane, "Enter");
        self.wait(pane, "one literal ask confirmed", |s| {
            s.contains("sent") && self.asks().len() == count
        });
        let asks = self.asks();
        let ask = asks.last().expect("new ask");
        assert_eq!(ask.summary.as_deref(), Some(text), "literal journal body");
        assert_eq!(ask.target.as_deref(), Some("lead"));
        let body = fs::read_to_string(ask.body_file.as_deref().expect("delivered body file"))
            .expect("stored body");
        assert!(
            body.contains(&format!("from human:chat: {text}\n\nREQUIRED:")),
            "literal tracked delivery: {body}"
        );
        assert!(
            self.tool.submitted().contains(text),
            "real harness submitted the body"
        );
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

    fn regular(&self) {
        assert!(
            fs::symlink_metadata(self.tool.dir.join(WRITER))
                .is_ok_and(|meta| meta.file_type().is_file()),
            "R-B1 successful acquisition creates a regular lease node"
        );
    }

    fn held(&self) {
        let file = OpenOptions::new()
            .append(true)
            .open(self.tool.dir.join(WRITER))
            .expect("lease exists");
        assert!(
            matches!(file.try_lock(), Err(TryLockError::WouldBlock)),
            "R-B1 write mode holds an actual kernel lease"
        );
    }

    fn free(&self) {
        let until = Instant::now() + WAIT;
        loop {
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
                "R-B5 exit failed to release kernel lease"
            );
            std::thread::sleep(Duration::from_millis(25));
        }
    }

    fn click(&self, pane: &str, needle: &str) {
        let screen = self.screen(pane);
        let (y, x) = screen
            .lines()
            .enumerate()
            .find_map(|(y, row)| row.find(needle).map(|at| (y, row[..at].chars().count())))
            .expect("drawn click target");
        self.raw(pane, &format!("\x1b[<0;{};{}M", x + 1, y + 1));
    }

    fn kept(&self, bytes: &str) {
        fs::write(self.tool.dir.join("console.draft"), bytes).expect("shared kept file");
    }

    fn swallow_submit(&self) {
        // The existing delivery fixture owns this private pane and script.
        // Swallowing a modelled Enter exercises a real Uncertain outcome.
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
}

#[test]
fn outside_tmux_app_writes_home_as_console_local_and_keeps_the_five_open_cap() {
    let rig = Rig::new("wboutside");
    let (pane, _) = rig.outside("plain");
    fs::remove_file(rig.tool.dir.join(WRITER)).expect("fresh app has no lease node");
    rig.enter(&pane);
    rig.regular();
    for at in 1..=5 {
        let body = format!("home-lease-{at}");
        rig.typed(&pane, &body);
        rig.submit(&pane, &body, at);
    }
    rig.typed(&pane, "sixth");
    rig.key(&pane, "Enter");
    rig.wait(&pane, "shared admission enforces five open", |s| {
        s.contains("5 requests open")
    });
    assert_eq!(rig.asks().len(), 5);
    rig.held();
    rig.key(&pane, "C-u");
    rig.typed(&pane, "/close");
    rig.key(&pane, "Enter");
    rig.wait(&pane, "app close through console admission", |s| {
        s.contains("closed an ask")
    });
    assert!(
        rig.tool
            .events()
            .lines()
            .filter_map(|r| Event::parse_line(r).ok())
            .any(|e| e.actor == "console:local" && e.action == "cancel")
    );
    rig.escape(&pane);
}

#[test]
fn successful_entry_drops_its_old_chunk_tail_and_names_that_drop() {
    let rig = Rig::new("wbtail");
    let pane = rig.console(false);
    rig.old_chunk(&pane, "iTAIL\r");
    let screen = rig.wait(&pane, "R-B3 entry tail is visibly dropped", |s| {
        s.contains("keys typed before writing started")
    });
    assert!(screen.contains("dropped"));
    assert!(!rig.draft_row(&screen).contains("TAIL"));
    assert!(rig.asks().is_empty(), "old Enter submits nothing");
    rig.held();
    rig.typed(&pane, "fresh-after-entry");
    rig.submit(&pane, "fresh-after-entry", 1);
}

#[test]
fn old_entry_enter_cannot_submit_a_line_restored_by_that_acquisition() {
    let rig = Rig::new("wbrestoretail");
    rig.kept("kept-before-entry");
    let pane = rig.console(false);
    rig.old_chunk(&pane, "i\r");
    let screen = rig.wait(
        &pane,
        "R-B2/R-B6 restored line waits for a fresh Enter",
        |s| s.contains("keys typed before writing started") && s.contains("Enter sends"),
    );
    assert!(rig.draft_row(&screen).contains("kept-before-entry"));
    assert!(
        rig.asks().is_empty(),
        "old Enter cannot send the restored line"
    );
    rig.held();
    rig.submit(&pane, "kept-before-entry", 1);
}

#[test]
fn old_escape_interrupt_and_browse_tail_are_never_held_by_the_write_gate() {
    for (tag, bytes, quit) in [
        ("wbstaleesc", "i\x1b\x1bZs", false),
        ("wbstalectrlc", "i\x03", true),
        // Two ESCs complete the first as Escape. The unsupported ESC-Z is
        // consumed; the old Tab that follows is now a browse key.
        ("wbstalebrowse", "i\x1b\x1bZ\t", false),
    ] {
        let rig = Rig::new(tag);
        let pane = rig.console(false);
        rig.old_chunk(&pane, bytes);
        if quit {
            let until = Instant::now() + WAIT;
            while rig
                .tmux(&["display-message", "-p", "-t", &pane, "#{pane_dead}"])
                .trim()
                != "1"
            {
                assert!(
                    Instant::now() < until,
                    "R-B2 stale Interrupt must still quit"
                );
                std::thread::sleep(Duration::from_millis(25));
            }
        } else {
            rig.wait(
                &pane,
                "R-B2 stale Escape releases and its tail can browse",
                |s| {
                    if tag == "wbstaleesc" {
                        // Settings proves dispatch happened; the initial
                        // browse hint alone could precede this old chunk.
                        s.contains("Quota") && s.contains("About")
                    } else {
                        s.contains("Enter writes") && s.contains("No seat facts:")
                    }
                },
            );
        }
        rig.free();
        assert!(rig.asks().is_empty());
    }
}

#[test]
fn processing_lag_cannot_submit_a_copied_stream_at_any_byte_boundary() {
    // P receives i, X, Enter. Q receives the same bytes while P holds the
    // lease. Its first READ reaches @app-keys and waits before decoding;
    // any second READ reaches @app-read AFTER stamping and waits too.
    // P submits and releases FIRST; only then may Q process those reads.
    // Thus every Q draft byte has a witnessed READ before its acquisition.
    const STREAM: &str = "iX\r";
    for split in 1..=STREAM.len() {
        let rig = Rig::new(&format!("wblag{split}"));
        let p = rig.console(false);
        let (q, gates) = rig.outside("lagging");
        rig.enter(&p);
        rig.typed(&p, "X");
        let keys = Hold::new(&gates, "@app-keys");
        rig.raw(&q, &STREAM[..split]);
        keys.held();
        let read = (split < STREAM.len()).then(|| {
            let held = Hold::new(&gates, "@app-read");
            rig.raw(&q, &STREAM[split..]);
            held.held();
            held
        });
        rig.submit(&p, "X", 1);
        rig.escape(&p);
        drop(read);
        drop(keys);
        let screen = rig.wait(
            &q,
            "R-B2 delayed old entry acquires with a new real instant",
            |s| s.contains("Enter sends"),
        );
        rig.held();
        assert!(
            !rig.draft_row(&screen).contains('X'),
            "split={split}: old bytes never enter the draft"
        );
        rig.typed(&q, "Y");
        rig.submit(&q, "Y", 2);
        let summaries: Vec<_> = rig
            .asks()
            .into_iter()
            .map(|e| e.summary.expect("body"))
            .collect();
        assert_eq!(
            summaries,
            ["X", "Y"],
            "split={split}: old Enter and body submitted nothing"
        );
    }
}

#[test]
fn busy_entry_is_held_across_reads_and_retry_drops_its_own_old_tail() {
    let rig = Rig::new("wbheld");
    let pane = rig.console(false);
    let witness = rig.lease();
    rig.raw(&pane, "i");
    let busy = format!("not writing: an ae app is writing to {}", rig.home);
    rig.wait(&pane, "R-B4 busy entry names writer and stays held", |s| {
        s.contains(&busy) && s.contains("Esc browses")
    });
    for text in ["j", "\t", "q", "n", "TAIL"] {
        rig.raw(&pane, text);
        rig.wait(&pane, "held stream cannot become browse commands", |s| {
            s.contains(&busy) && s.contains("Overview")
        });
    }
    drop(witness);
    // Release alone does not promote a HELD attempt; retry is explicit.
    rig.raw(&pane, "q");
    rig.wait(&pane, "released lease does not end held mode", |s| {
        s.contains(&busy)
    });
    rig.old_chunk(&pane, "\rRETRY\r");
    let screen = rig.wait(
        &pane,
        "fresh Enter retry acquires and drops old retry tail",
        |s| s.contains("Enter sends") && s.contains("keys typed before writing started"),
    );
    assert!(screen.contains("Topics"), "held Tab never changed Overview");
    assert!(!rig.draft_row(&screen).contains("RETRY"));
    assert!(rig.asks().is_empty());
    rig.held();
    rig.typed(&pane, "retry-fresh");
    rig.submit(&pane, "retry-fresh", 1);
}

#[test]
fn nonregular_lease_has_its_own_held_refusal_and_reads_no_kept_draft() {
    let rig = Rig::new("wbnode");
    rig.kept("disk-secret");
    fs::remove_file(rig.tool.dir.join(WRITER)).expect("replace fixture lease node");
    fs::create_dir(rig.tool.dir.join(WRITER)).expect("directory lease node");
    let pane = rig.console(false);
    rig.raw(&pane, "i");
    let screen = rig.wait(&pane, "R-B4 nonregular refusal distinct from busy", |s| {
        s.contains("not writing") && (s.contains("regular") || s.contains("directory"))
    });
    assert!(!screen.contains("an ae app is writing"));
    assert!(
        !screen.contains("disk-secret"),
        "R-B6 failed entry never restores"
    );
    rig.raw(&pane, "q\tTAIL");
    rig.wait(&pane, "nonregular HELD mode swallows keys", |s| {
        s.contains("not writing") && s.contains("Overview")
    });
    assert_eq!(
        fs::read_to_string(rig.tool.dir.join("console.draft")).expect("kept file"),
        "disk-secret"
    );
}

#[test]
fn held_entry_ends_only_on_escape_interrupt_or_a_click() {
    for (tag, action) in [
        ("wbheldesc", "Escape"),
        ("wbheldstop", "C-c"),
        ("wbheldclick", "click"),
    ] {
        let rig = Rig::new(tag);
        let pane = rig.console(false);
        let witness = rig.lease();
        rig.raw(&pane, "i");
        rig.wait(&pane, "R-B4 refused entry remains held", |s| {
            s.contains("not writing: an ae app is writing to")
        });
        if action == "click" {
            rig.click(&pane, "Agents");
        } else {
            rig.key(&pane, action);
        }
        if action == "C-c" {
            let until = Instant::now() + WAIT;
            while rig
                .tmux(&["display-message", "-p", "-t", &pane, "#{pane_dead}"])
                .trim()
                != "1"
            {
                assert!(
                    Instant::now() < until,
                    "R-B4 fresh Interrupt quits HELD mode"
                );
                std::thread::sleep(Duration::from_millis(25));
            }
        } else {
            rig.wait(
                &pane,
                "R-B4 escape/click returns HELD mode to browsing",
                |s| {
                    s.contains("Enter writes")
                        && !s.contains("not writing: an ae app is writing to")
                },
            );
        }
        assert!(rig.asks().is_empty());
        // Ending a failed attempt cannot unlock the other writer's handle.
        rig.held();
        drop(witness);
        rig.free();
    }
}

#[test]
fn esc_interrupt_quit_and_crash_release_the_kernel_lease() {
    for (tag, exit) in [
        ("wbesc", "Escape"),
        ("wbctrlc", "C-c"),
        ("wbcrash", "crash"),
        ("wbquit", "q"),
    ] {
        let rig = Rig::new(tag);
        let pane = rig.console(false);
        rig.enter(&pane);
        rig.typed(&pane, "exit-draft");
        match exit {
            "Escape" => rig.escape(&pane),
            "q" => {
                rig.escape(&pane);
                rig.key(&pane, "q");
            }
            "crash" => {
                rig.tmux(&["kill-window", "-t", &pane]);
            }
            key => rig.key(&pane, key),
        }
        rig.free();
    }
}

#[test]
fn session_card_and_tab_click_leave_write_mode_and_release() {
    for (tag, target) in [("wbcard", "card"), ("wbtab", "Agents")] {
        let rig = Rig::new(tag);
        let pane = rig.console(false);
        rig.enter(&pane);
        rig.typed(&pane, "click-draft");
        if target == "card" {
            // The list card is the first occurrence below its Sessions row;
            // the chat heading on row 1 is deliberately not a click target.
            let screen = rig.screen(&pane);
            let y = screen
                .lines()
                .enumerate()
                .find_map(|(y, row)| {
                    (y >= 5 && row.split_whitespace().any(|w| w == rig.home)).then_some(y)
                })
                .expect("drawn home card");
            rig.raw(&pane, &format!("\x1b[<0;3;{}M", y + 1));
        } else {
            rig.click(&pane, target);
        }
        rig.wait(&pane, "R-B5 clicked card/tab leaves writing", |s| {
            s.contains("draft kept") && s.contains("Enter writes")
        });
        rig.free();
    }
}

#[test]
fn settings_suspends_writing_with_the_lease_and_draft_then_resumes() {
    let rig = Rig::new("wbsettings");
    let pane = rig.console(false);
    rig.enter(&pane);
    rig.typed(&pane, "settings-draft");
    rig.click(&pane, "⚙");
    rig.wait(&pane, "GUARD Settings overlay", |s| {
        s.contains("Quota") && s.contains("About")
    });
    rig.held();
    rig.key(&pane, "Escape");
    rig.wait(
        &pane,
        "R-B5 closing Settings resumes the same writing draft",
        |s| s.contains("Enter sends") && s.contains("settings-draft"),
    );
    rig.held();
    rig.escape(&pane);
}

#[test]
fn replacement_pair_change_and_unusable_home_release_without_a_submit() {
    for (tag, uuid, peer) in [
        ("wbuuid", Some(OTHER_UUID), "colead"),
        ("wbpair", Some(UUID), "replacement"),
        ("wboff", None, "colead"),
    ] {
        let rig = Rig::new(tag);
        let pane = rig.console(false);
        rig.enter(&pane);
        rig.typed(&pane, "old-incarnation-draft");
        // The fleet read already took its OLD ownership reading, then
        // attested at @world. It cannot send a fresh revocation to the UI
        // until this guard drops, so Enter must decide from fresh meta.
        let background = Hold::new(&rig.root.join("gates-owner"), "@world");
        background.held();
        if let Some(uuid) = uuid {
            rig.meta(uuid, peer);
        } else {
            fs::remove_file(rig.tool.dir.join("meta"))
                .expect("fresh admission has an unusable home");
        }
        // No refresh wait: admission must use fresh facts, not its last
        // background ownership reading or the cached session stamp.
        rig.key(&pane, "Enter");
        rig.wait(
            &pane,
            "R-B5/R-B7 fresh invalid home refuses and leaves writing",
            |s| s.contains("refused"),
        );
        rig.free();
        assert!(
            rig.asks().is_empty(),
            "old identity/pair never records an ask"
        );
        assert!(
            rig.tool.submitted().is_empty(),
            "nothing reaches the real harness"
        );
        drop(background);
    }
}

#[test]
fn acquisition_restores_only_empty_drafts_and_reloads_the_shared_kept_file() {
    let rig = Rig::new("wbrestore");
    rig.kept("disk-first");
    let (pane, _) = rig.outside("restore");
    let before = rig.screen(&pane);
    assert!(
        !before.contains("disk-first"),
        "R-B6 browsing has no lease and must not restore"
    );
    rig.enter(&pane);
    rig.wait(
        &pane,
        "R-B6 acquisition restores kept bytes and banner",
        |s| s.contains("disk-first") && s.contains("Kept line, maybe already sent"),
    );
    rig.key(&pane, "C-u");
    rig.wait(&pane, "GUARD composer emptied without deleting disk", |s| {
        !rig.draft_row(s).contains("disk-first")
    });
    assert_eq!(
        fs::read_to_string(rig.tool.dir.join("console.draft")).expect("shared file"),
        "disk-first"
    );
    rig.escape(&pane);
    rig.enter(&pane);
    rig.wait(
        &pane,
        "R-B6 empty reacquisition restores after clear",
        |s| rig.draft_row(s).contains("disk-first"),
    );
    rig.key(&pane, "C-u");
    rig.escape(&pane);
    {
        let _other = rig.lease();
        rig.kept("disk-latest");
    }
    rig.enter(&pane);
    rig.wait(&pane, "R-B6 next writer reads disk now", |s| {
        rig.draft_row(s).contains("disk-latest")
    });
    rig.escape(&pane);
    {
        let _other = rig.lease();
        rig.kept("disk-newer");
    }
    rig.enter(&pane);
    let screen = rig.screen(&pane);
    assert!(
        rig.draft_row(&screen).contains("disk-latest"),
        "nonempty memory wins"
    );
    assert!(
        !rig.draft_row(&screen).contains("disk-newer"),
        "shared file is not merged"
    );
}

#[test]
fn failed_send_keeps_disk_and_empty_reacquisition_restores_with_the_banner() {
    for (tag, uncertain) in [("wbunknown", false), ("wbuncertain", true)] {
        let rig = Rig::new(tag);
        let pane = rig.console(false);
        rig.enter(&pane);
        failed_send(&rig, &pane, uncertain);
        rig.key(&pane, "C-u");
        rig.escape(&pane);
        rig.enter(&pane);
        rig.wait(
            &pane,
            "R-B6 failed-send line restored on later empty acquisition",
            |s| {
                rig.draft_row(s).contains("failed-kept")
                    && s.contains("Kept line, maybe already sent")
            },
        );
    }
}

#[test]
fn owner_chat_submit_and_close_refuse_busy_without_waiting_or_writing_disk() {
    let rig = Rig::new("wbchat");
    let chat = rig.console(true);
    fs::remove_file(rig.tool.dir.join(WRITER)).expect("fresh chat has no lease node");
    rig.raw(&chat, "chat-first");
    rig.key(&chat, "Enter");
    rig.wait(
        &chat,
        "GUARD owner chat established one real open ask",
        |s| s.contains("sent") && rig.asks().len() == 1,
    );
    rig.regular();
    let (app, _) = rig.outside("app");
    rig.enter(&app);
    rig.raw(&chat, "chat-while-app-writes");
    rig.wait(&chat, "GUARD chat literal draft", |s| {
        s.contains("chat-while-app-writes")
    });
    let began = Instant::now();
    rig.key(&chat, "Enter");
    rig.wait(&chat, "R-B1 owner chat nonblocking refusal", |s| {
        s.contains("an ae app is writing to")
    });
    assert!(
        began.elapsed() < Duration::from_secs(3),
        "chat never waits for writer lease"
    );
    assert_eq!(rig.asks().len(), 1, "busy chat records no second ask");
    assert!(
        !rig.tool.dir.join("console.draft").exists(),
        "busy chat must not replace kept draft"
    );
    rig.key(&chat, "C-u");
    rig.raw(&chat, "/close");
    let previous = rig.screen(&chat).matches("an ae app is writing to").count();
    rig.key(&chat, "Enter");
    rig.wait(&chat, "R-B1 close takes the same nonblocking lease", |s| {
        s.matches("an ae app is writing to").count() > previous
    });
    assert!(
        !rig.tool
            .events()
            .lines()
            .filter_map(|r| Event::parse_line(r).ok())
            .any(|e| e.action == "cancel"),
        "busy close changed no open request"
    );
    rig.held();
    rig.escape(&app);
    rig.key(&chat, "C-u");
    rig.raw(&chat, "/close");
    rig.key(&chat, "Enter");
    rig.wait(&chat, "owner chat closes after app releases", |s| {
        s.contains("closed an ask")
    });
    assert!(
        rig.tool
            .events()
            .lines()
            .filter_map(|r| Event::parse_line(r).ok())
            .any(|e| e.actor == "console:local" && e.action == "cancel")
    );
    rig.free();
}

#[test]
fn owner_chat_names_a_nonregular_lease_without_publishing_or_delivering() {
    let rig = Rig::new("wbchatnode");
    let chat = rig.console(true);
    fs::remove_file(rig.tool.dir.join(WRITER)).expect("fixture lease replaced");
    fs::create_dir(rig.tool.dir.join(WRITER)).expect("nonregular lease node");
    rig.raw(&chat, "blocked-chat-node");
    rig.key(&chat, "Enter");
    rig.wait(&chat, "R-B1 chat names its nonregular lease refusal", |s| {
        s.contains("refused") && (s.contains("regular") || s.contains("directory"))
    });
    assert!(rig.asks().is_empty());
    assert!(!rig.tool.dir.join("console.draft").exists());
    assert!(rig.tool.submitted().is_empty());
}

fn failed_send(rig: &Rig, pane: &str, uncertain: bool) {
    rig.typed(pane, "failed-kept");
    if uncertain {
        rig.swallow_submit();
    } else {
        // A real tracked-path refusal with no ask record gives Unknown.
        rig.tmux(&[
            "set-option",
            "-p",
            "-t",
            &rig.tool.pane,
            "@ae_agent",
            "missing-target",
        ]);
    }
    rig.key(pane, "Enter");
    rig.wait(pane, "GUARD real failed outcome", |s| {
        s.contains(if uncertain {
            "uncertain: check lead pane"
        } else {
            "no record of it"
        })
    });
    assert_eq!(
        fs::read_to_string(rig.tool.dir.join("console.draft")).expect("kept failed draft"),
        "failed-kept"
    );
    assert_eq!(
        rig.asks().len(),
        usize::from(uncertain),
        "Unknown has no ask; Uncertain has a durable unconfirmed ask"
    );
    if uncertain {
        assert!(
            rig.asks()[0]
                .summary
                .as_deref()
                .expect("summary")
                .starts_with("[unconfirmed]")
        );
    }
    assert!(
        rig.tool.submitted().is_empty(),
        "fake harness confirms no submit"
    );
}

#[test]
fn controls_real_submit_unknown_and_uncertain_paths_are_satisfiable() {
    for (tag, uncertain) in [("wbcontrolunknown", false), ("wbcontroluncertain", true)] {
        let rig = Rig::new(tag);
        let pane = rig.console(false);
        // The base can compose in its OWN console. This control deliberately
        // requires no B1 lease, so delivery/setup errors cannot count as RED.
        rig.raw(&pane, "i");
        rig.wait(&pane, "GUARD base owner may compose", |s| {
            s.contains("Enter sends")
        });
        failed_send(&rig, &pane, uncertain);
    }
}

#[test]
fn controls_literal_delivery_and_each_old_read_boundary_are_attested() {
    const STREAM: &str = "iX\r";
    for split in 1..=STREAM.len() {
        let rig = Rig::new(&format!("wbgatecontrol{split}"));
        let p = rig.console(false);
        let (q, gates) = rig.outside("reader");
        rig.raw(&p, "i");
        rig.wait(&p, "GUARD base owner composing", |s| {
            s.contains("Enter sends")
        });
        rig.typed(&p, "X");
        let keys = Hold::new(&gates, "@app-keys");
        rig.raw(&q, &STREAM[..split]);
        keys.held();
        let read = (split < STREAM.len()).then(|| {
            let hold = Hold::new(&gates, "@app-read");
            rig.raw(&q, &STREAM[split..]);
            hold.held();
            hold
        });
        // Same real tracked delivery as the causal test, with its two gates
        // reached before the submit. This base control does not require Q
        // to write: its frozen RED counterpart judges Q's later acquisition.
        rig.submit(&p, "X", 1);
        rig.escape(&p);
        drop(read);
        drop(keys);
    }
}

#[test]
fn refresh_releases_an_identity_pair_or_home_that_is_no_longer_proven() {
    for case in 0..3 {
        let rig = Rig::new(&format!("wbrefresh{case}"));
        let pane = rig.console(false);
        rig.enter(&pane);
        rig.typed(&pane, "refresh-draft");
        match case {
            0 => rig.meta(OTHER_UUID, "colead"),
            1 => rig.meta(UUID, "replacement"),
            _ => fs::remove_file(rig.tool.dir.join("meta"))
                .expect("fixture home is no longer usable"),
        }
        // No key forces admission. The next ownership reading must revoke
        // permission and release, rather than leaving a stale writer alive.
        rig.free();
        assert!(rig.asks().is_empty());
        rig.wait(&pane, "R-B5 refresh left write mode", |s| {
            !s.contains("Enter sends")
        });
    }
}
