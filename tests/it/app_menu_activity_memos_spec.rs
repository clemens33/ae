//! S6a frozen acceptance specification: `Activity…` and `Memos…` in the `ae app`
//! session card menu (`#230` checklist; `plan-r2.md` §5 S6a).
//!
//! # Minimal `src/app/menu.rs` / `src/session_menu.rs` contract named by this spec
//!
//! * `src/app/menu.rs`:
//!   - `Item::Activity` (`"Activity…"` with `icons = true`, `"Activity..."` with
//!     `icons = false`; word `"Activity"`) and `Item::Memos` (`"Memos…"` with
//!     `icons = true`, `"Memos..."` with `icons = false`; word `"Memos"`).
//!   - `Card::items(&self) -> &'static [Item]` offers `Activity` and `Memos`
//!     for every non-orchestrator session card whether `self.stopped` is `true`
//!     (`&[Item::Start, Item::Activity, Item::Memos]`) or `false`
//!     (`&[Item::Stop, Item::Activity, Item::Memos]`), both inside and outside
//!     tmux (`TMUX` / `TMUX_PANE` unset).
//!   - Selecting `Item::Activity` or `Item::Memos` (by its shortcut key,
//!     `j`/`k`/`Up`/`Down` + `Enter`, or left-clicking `Hit::MenuItem(item)`)
//!     opens a read-only in-app dialog/overlay (`Modal` variant or equivalent
//!     in-app overlay); `Esc` and `q` dismiss it and return to the app
//!     unchanged.
//! * Content parity with `src/session_menu.rs` (`activity_cells` / `memo_cells`):
//!   - **Activity**: newest 30 records (`ACTIVITY_ROWS_MAX = 30`) of the 15
//!     curated actions (`state`, `done`, `goal`, `spawn`, `retire`, `relaunch`,
//!     `reseat`, `ask`, `review`, `reply`, `watchdog-start`, `watchdog-stop`,
//!     `effort-applied`, `effort-restored`, `effort-failed`), excluding
//!     housekeeping `watchdog-start` / `watchdog-stop` rows whose actor starts
//!     with `ae:`, and excluding non-curated actions (`say`, `send`, `memo`,
//!     `conversation-new`).
//!   - **Memos**: latest memo per topic (`crate::brief::topic_lines`), newest
//!     topic first, at most 30 rows (`MEMO_ROWS_MAX = 30`).
//!   - Every field is neutralised (`event_text::sanitize_menu_text` /
//!     `board::terminal_text`) so no raw escape byte reaches the terminal.
//!   - Records of another incarnation (`meta` `session_id` mismatch, missing,
//!     duplicate, or unreadable) never render; a gap is named (`unavailable` /
//!     `unreadable` / `none` / `replaced`), never filled with foreign records.
//!   - Purely read-only: writes nothing, journals nothing, types into no pane.

#![allow(
    clippy::disallowed_methods,
    reason = "integration fixtures plant and inspect temporary session directories; \
              the capability boundary governs product code"
)]

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use super::cli::OwnedScratch;
use super::parity::Invocation;
use super::parity::capture::ExitOutcome;
use super::parity::capture::raw::{kill_servers_under, run as run_raw};

static TMUX_CALL_SEQ: AtomicU64 = AtomicU64::new(0);

const ALPHA_UUID: &str = "0199c0de-aaaa-4890-abcd-ef0123456789";
const BETA_UUID: &str = "0199c0de-bbbb-4890-abcd-ef0123456789";
const OTHER_UUID: &str = "0199c0de-ffff-4890-abcd-ef0123456789";

struct Rig {
    root: PathBuf,
    ae_home: PathBuf,
    config: PathBuf,
    sock: PathBuf,
}

impl Rig {
    fn new(tag: &str) -> Self {
        let root = OwnedScratch::root("s6a", tag).keep();
        let ae_home = root.join("ae");
        let config = root.join("config");
        let sock = root.join("ae.sock");
        fs::create_dir_all(ae_home.join("sessions"))
            .unwrap_or_else(|err| panic!("create sessions dir: {err}"));
        fs::create_dir_all(root.join("home")).unwrap_or_else(|err| panic!("create home: {err}"));
        fs::write(&config, "[workspace]\nicons = on\npalette = darcula\n")
            .unwrap_or_else(|err| panic!("write config: {err}"));
        Self {
            root,
            ae_home,
            config,
            sock,
        }
    }

    fn session_dir(&self, name: &str) -> PathBuf {
        self.ae_home.join("sessions").join(name)
    }

    fn tmux(&self, args: &[&str]) -> (u8, String, String) {
        let mut command = vec!["-S".to_owned(), self.sock.display().to_string()];
        command.extend(args.iter().map(|item| (*item).to_owned()));
        if std::env::var_os("TMUX_TMPDIR").is_some() {
            let (ok, out) = super::phase2::run_tmux(&command, &self.root);
            return (u8::from(!ok), out.clone(), out);
        }
        let seq = TMUX_CALL_SEQ.fetch_add(1, Ordering::Relaxed);
        let out_file = self.root.join(format!("tmux-{seq}.out"));
        let err_file = self.root.join(format!("tmux-{seq}.err"));
        let mut inv = Invocation::new("env")
            .arg(format!("TMUX_TMPDIR={}", self.root.display()))
            .arg("tmux")
            .arg("-u")
            .env("TMUX_TMPDIR", self.root.display().to_string());
        for item in &command {
            inv = inv.arg(item);
        }
        let status = run_raw(&inv, &self.root, &out_file, &err_file)
            .unwrap_or_else(|err| panic!("run tmux {args:?}: {err}"));
        let code = match status.outcome() {
            ExitOutcome::Code(0) => 0,
            ExitOutcome::Code(_) | ExitOutcome::Signalled => 1,
        };
        let out = fs::read_to_string(&out_file).unwrap_or_default();
        let err = fs::read_to_string(&err_file).unwrap_or_default();
        let _ = fs::remove_file(&out_file);
        let _ = fs::remove_file(&err_file);
        (code, out, err)
    }

    fn start_host(&self) {
        let (code, _, err) = self.tmux(&[
            "-f",
            "/dev/null",
            "new-session",
            "-d",
            "-s",
            "host",
            "-x",
            "160",
            "-y",
            "50",
            "sh",
            "-c",
            "while :; do sleep 1; done",
        ]);
        assert_eq!(code, 0, "start host session: {err}");
    }

    fn plant_running_session(&self, name: &str, uuid: &str) {
        let (_, pane, _) = self.tmux(&[
            "new-session",
            "-d",
            "-P",
            "-F",
            "#{pane_id}",
            "-s",
            name,
            "-n",
            "main",
            "-x",
            "160",
            "-y",
            "50",
            "sh",
            "-c",
            "while :; do sleep 1; done",
        ]);
        let pane = pane.trim();
        let (_, pid, _) = self.tmux(&["display-message", "-p", "-t", pane, "#{pane_pid}"]);
        let pid = pid.trim();
        self.tmux(&["set-option", "-t", name, "@ae_session", "1"]);
        self.tmux(&["set-option", "-t", name, "@ae_session_uuid", uuid]);
        self.tmux(&["set-option", "-p", "-t", pane, "@ae_agent", "lead"]);
        self.tmux(&["set-option", "-p", "-t", pane, "@ae_slot", "main"]);
        self.tmux(&[
            "set-option",
            "-t",
            name,
            "@ae_picker_agents",
            &format!("v2;4102444800;60;lead:cl:working:{pane}:claude:Opus 4.7:high:"),
        ]);
        let dir = self.session_dir(name);
        fs::create_dir_all(&dir).unwrap_or_else(|err| panic!("create {}: {err}", dir.display()));
        let meta = format!(
            "session_id={uuid}\n\
             mode=single\n\
             work_dir={}\n\
             seat.main=lead\n\
             profile.main=cl\n\
             agent_bin.main=claude\n\
             pane.main={pane}\n\
             pid.main={pid}\n\
             tmux_server_kind=socket\n\
             tmux_server={}\n",
            self.root.display(),
            self.sock.display(),
        );
        fs::write(dir.join("meta"), meta).unwrap_or_else(|err| panic!("write meta: {err}"));
    }

    fn plant_stopped_session(&self, name: &str, uuid: &str) {
        let dir = self.session_dir(name);
        fs::create_dir_all(&dir).unwrap_or_else(|err| panic!("create {}: {err}", dir.display()));
        let meta = format!(
            "session_id={uuid}\n\
             mode=single\n\
             work_dir={}\n\
             seat.main=lead\n\
             profile.main=cl\n\
             agent_bin.main=claude\n\
             tmux_server_kind=socket\n\
             tmux_server={}\n",
            self.root.display(),
            self.sock.display(),
        );
        fs::write(dir.join("meta"), meta).unwrap_or_else(|err| panic!("write meta: {err}"));
    }

    /// Launch `ae app <session>` inside a fresh window on the private host
    /// server. When `inside_tmux` is `false`, `TMUX` and `TMUX_PANE` are
    /// stripped from the child process environment so `ae app` runs outside
    /// tmux (`doors::caller_server() == None`, `app.attended() == false`)
    /// while still attached to a real PTY we can inspect with `capture-pane`.
    fn open_app(&self, session: &str, inside_tmux: bool) -> String {
        let bin = PathBuf::from(env!("CARGO_BIN_EXE_ae"));
        let tmux_prelude = if inside_tmux {
            ""
        } else {
            "unset TMUX TMUX_PANE; "
        };
        let cmd = format!(
            "{tmux_prelude}HOME='{}' AE_HOME='{}' CONFIG_FILE='{}' TMUX_TMPDIR='{}' \
             AE_TMUX_SERVER_KIND=socket AE_TMUX_SERVER='{}' AE_NO_AUTOSTART=1 \
             exec '{}' app '{}'",
            self.root.join("home").display(),
            self.ae_home.display(),
            self.config.display(),
            self.root.display(),
            self.sock.display(),
            bin.display(),
            session,
        );
        let (code, pane, err) = self.tmux(&[
            "new-window",
            "-d",
            "-P",
            "-F",
            "#{pane_id}",
            "-t",
            "host",
            "-n",
            "app",
            "sh",
            "-c",
            &cmd,
        ]);
        assert_eq!(code, 0, "spawn ae app window: {err}");
        let pane = pane.trim().to_owned();
        let _ = self.wait_until(&pane, Duration::from_secs(6), |screen| {
            screen.contains(session) && screen.contains("Enter writes")
        });
        pane
    }

    fn capture(&self, pane: &str) -> String {
        self.tmux(&["capture-pane", "-p", "-t", pane]).1
    }

    /// Observe pane OUTPUT bytes before tmux consumes CSI/OSC. `capture-pane -e`
    /// preserves SGR only: it still loses OSC 52, so it is not a byte oracle.
    fn record_output(&self, pane: &str) -> PathBuf {
        let path = self
            .root
            .join(format!("output-{}.raw", pane.trim_start_matches('%')));
        let command = format!("cat > '{}'", path.display());
        let (code, _, err) = self.tmux(&["pipe-pane", "-O", "-t", pane, &command]);
        assert_eq!(code, 0, "record output bytes: {err}");
        path
    }

    fn short_viewport(&self, pane: &str) {
        let (code, _, err) = self.tmux(&["resize-window", "-t", pane, "-y", "20"]);
        assert_eq!(code, 0, "resize private probe: {err}");
        thread::sleep(Duration::from_millis(150));
    }

    fn send_keys(&self, pane: &str, keys: &[&str]) {
        let mut args = vec!["send-keys", "-t", pane];
        args.extend_from_slice(keys);
        self.tmux(&args);
    }

    fn send_literal(&self, pane: &str, text: &str) {
        self.tmux(&["send-keys", "-l", "-t", pane, text]);
    }

    fn wait_until(
        &self,
        pane: &str,
        timeout: Duration,
        mut predicate: impl FnMut(&str) -> bool,
    ) -> String {
        let deadline = Instant::now() + timeout;
        let mut last = String::new();
        while Instant::now() < deadline {
            last = self.capture(pane);
            if predicate(&last) {
                return last;
            }
            thread::sleep(Duration::from_millis(25));
        }
        last
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = kill_servers_under(&self.root, std::slice::from_ref(&self.sock));
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn menu_has_activity_and_memos(screen: &str) -> bool {
    // Kills ASCII labels with icons explicitly on, rather than accepting either.
    screen.contains("Activity…") && screen.contains("Memos…")
}

fn right_click_card(rig: &Rig, pane: &str, session: &str) {
    let screen = rig.capture(pane);
    let row = screen
        .lines()
        .enumerate()
        .find_map(|(idx, line)| {
            line.chars()
                .take(36)
                .collect::<String>()
                .split_whitespace()
                .any(|word| word == session)
                .then_some(idx)
        })
        .unwrap_or_else(|| panic!("session card '{session}' not found in sidebar:\n{screen}"));
    thread::sleep(Duration::from_millis(20));
    rig.send_literal(pane, &format!("\x1b[<2;9;{}M", row + 1));
}

/// Activate `target_label` (`"Activity"` or `"Memos"`) from an already-open
/// session card menu. Clicks the painted `Hit::MenuItem` row first (which is
/// invariant across shortcut-key choices), and if the card menu remains open
/// falls back to the item's shortcut key (`a` for Activity, `m`/`e` or
/// `Down`+`Enter` for Memos).
fn activate_menu_dialog(rig: &Rig, pane: &str, target_label: &str) -> String {
    let menu_screen = rig.wait_until(pane, Duration::from_secs(2), menu_has_activity_and_memos);
    assert!(
        menu_has_activity_and_memos(&menu_screen),
        "card menu did not offer Activity… and Memos… before selecting {target_label}:\n{menu_screen}"
    );
    // Ensure the modal's `shown_at` frame timestamp strictly precedes the input.
    thread::sleep(Duration::from_millis(25));

    if let Some((row, col)) = menu_screen.lines().enumerate().find_map(|(r, line)| {
        line.find(target_label)
            .map(|byte| (r, line[..byte].chars().count()))
    }) {
        rig.send_literal(pane, &format!("\x1b[<0;{};{}M", col + 1, row + 1));
    }

    let after_click = rig.wait_until(pane, Duration::from_millis(500), |s| {
        !s.contains("s  Stop") && !s.contains("s  Start")
    });
    if !after_click.contains("s  Stop") && !after_click.contains("s  Start") {
        return after_click;
    }

    thread::sleep(Duration::from_millis(25));
    if target_label == "Activity" {
        rig.send_literal(pane, "a");
    } else {
        rig.send_keys(pane, &["Down", "Down", "Enter"]);
    }
    rig.wait_until(pane, Duration::from_secs(2), |s| {
        !s.contains("s  Stop") && !s.contains("s  Start")
    })
}

/// Exercise ordinary Down navigation through the bounded 30-row dialog.
/// Cap cases also REQUIRE the oldest kept row to appear and the newest to
/// leave the short viewport: an unbound pager cannot prove absence offscreen.
fn collect_dialog_pages(rig: &Rig, pane: &str) -> (String, String) {
    let first_page = rig.capture(pane);
    let mut combined = first_page.clone();
    for _ in 0..40 {
        thread::sleep(Duration::from_millis(20));
        rig.send_keys(pane, &["Down"]);
        thread::sleep(Duration::from_millis(40));
        let screen = rig.capture(pane);
        if screen == combined || !combined.contains(&screen) {
            combined.push('\n');
            combined.push_str(&screen);
        }
    }
    (first_page, combined)
}

fn assert_scrolled_to_end(rig: &Rig, pane: &str, newest: &str, oldest_kept: &str) {
    let end = rig.capture(pane);
    assert!(
        end.contains(oldest_kept) && !end.contains(newest),
        "Down must demonstrably reach the last kept row in the short viewport; \
         a frozen viewport cannot prove the 30-row cap:\n{end}"
    );
}

/// Column padding is presentation, but age/actor/kind/detail and the detail's
/// em dash are content. JSON dumps and body-only renderers fail these pins.
fn compact(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn assert_cells_on_screen(screen: &str, cells: &[String]) {
    assert!(
        screen.lines().any(|line| {
            let line = compact(line);
            let mut tail = line.as_str();
            for cell in cells {
                let cell = compact(cell);
                let Some(at) = tail.find(&cell) else {
                    return false;
                };
                tail = &tail[at + cell.len()..];
            }
            true
        }),
        "dialog must paint these ordered cells on ONE row: {cells:?}\n{screen}"
    );
}

/// These are attacker fixture bytes, NOT the app's legitimate terminal styles.
/// Both consumers use this same predicate; the negative control below proves
/// raw CSI and OSC emitters fail it even though tmux consumes OSC from its grid.
fn has_hostile_escape(bytes: &[u8]) -> bool {
    [
        b"\x1b[31mRED_TEXT".as_slice(),
        b"\x1b[32mGREEN",
        b"\x1b]52;c;dGVzdA==",
    ]
    .iter()
    .any(|needle| bytes.windows(needle.len()).any(|part| part == *needle))
}

fn assert_safe_output(path: &Path) {
    let bytes = fs::read(path).unwrap_or_else(|err| panic!("read raw pane output: {err}"));
    assert_safe_bytes(&bytes);
}

fn assert_safe_bytes(bytes: &[u8]) {
    assert!(
        !has_hostile_escape(bytes),
        "raw attacker CSI/OSC reached terminal output"
    );
}

#[test]
fn escape_oracle_rejects_raw_csi_and_osc_before_tmux_consumes_them() {
    let rig = Rig::new("escape-oracle-control");
    rig.start_host();
    // The host shell is deliberately idle; display-message -p goes to the
    // client, so use a gated fixture child that emits the wrong renderer bytes.
    let gate = rig.root.join("emit");
    let cmd = format!(
        "while [ ! -f '{}' ]; do sleep 0.02; done; \
         printf 'CSI_START \\033[31mRED_TEXT\\033[0m CSI_END\\n'; \
         printf 'OSC_START \\033]52;c;dGVzdA==\\007 OSC_END\\n'; \
         while :; do sleep 1; done",
        gate.display()
    );
    let (_, pane, _) = rig.tmux(&[
        "new-window",
        "-d",
        "-P",
        "-F",
        "#{pane_id}",
        "-t",
        "host",
        "sh",
        "-c",
        &cmd,
    ]);
    let pane = pane.trim();
    let output = rig.record_output(pane);
    fs::write(gate, b"go").expect("release oracle control");
    let screen = rig.wait_until(pane, Duration::from_secs(2), |s| s.contains("OSC_END"));
    let deadline = Instant::now() + Duration::from_secs(2);
    let bytes = loop {
        let bytes = fs::read(&output).unwrap_or_default();
        if bytes.windows(7).any(|part| part == b"OSC_END") || Instant::now() >= deadline {
            break bytes;
        }
        thread::sleep(Duration::from_millis(20));
    };
    // Independently reject each sequence, not just a combined contaminated blob.
    for (start, end) in [("CSI_START", "CSI_END"), ("OSC_START", "OSC_END")] {
        let raw = String::from_utf8_lossy(&bytes);
        let part = raw
            .split_once(start)
            .expect("control start")
            .1
            .split_once(end)
            .expect("control end")
            .0;
        assert!(
            std::panic::catch_unwind(|| assert_safe_bytes(part.as_bytes())).is_err(),
            "raw {start} emitter must fail the SAME assertion used by app dialogs"
        );
    }
    assert!(
        !has_hostile_escape(screen.as_bytes()),
        "plain capture strips the evidence"
    );
    assert_safe_bytes(b"ESC_SAFE ?RED_TEXT? ? END_SAFE");
}

fn snapshot_dir_bytes(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    let entries =
        fs::read_dir(dir).unwrap_or_else(|err| panic!("read_dir {}: {err}", dir.display()));
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let path = entry.path();
        if path.is_file() {
            let bytes =
                fs::read(&path).unwrap_or_else(|err| panic!("read {}: {err}", path.display()));
            out.insert(name, bytes);
        } else {
            out.insert(format!("{name}/"), Vec::new());
        }
    }
    out
}

#[test]
fn card_menu_offers_activity_and_memos_for_running_and_stopped_inside_and_outside_tmux() {
    for inside_tmux in [true, false] {
        let mode_tag = if inside_tmux { "in-tmux" } else { "out-tmux" };
        let rig = Rig::new(&format!("offers-{mode_tag}"));
        rig.start_host();
        rig.plant_running_session("alpha", ALPHA_UUID);
        rig.plant_stopped_session("beta", BETA_UUID);

        let pane = rig.open_app("alpha", inside_tmux);

        // 1. Running session ("alpha") via keyboard `m`.
        rig.send_literal(&pane, "m");
        let running_menu = rig.wait_until(&pane, Duration::from_secs(2), |s| {
            s.contains("s  Stop") || menu_has_activity_and_memos(s)
        });
        assert!(
            running_menu.contains("s  Stop…") || running_menu.contains("s  Stop..."),
            "[{mode_tag}] running card menu must offer Stop…:\n{running_menu}"
        );
        assert!(
            menu_has_activity_and_memos(&running_menu),
            "[{mode_tag}] running card menu must offer Activity… and Memos…:\n{running_menu}"
        );

        // Dismiss with Esc and verify right-click on the running card also offers both.
        thread::sleep(Duration::from_millis(25));
        rig.send_keys(&pane, &["Escape"]);
        let _ = rig.wait_until(&pane, Duration::from_secs(2), |s| !s.contains("s  Stop"));
        right_click_card(&rig, &pane, "alpha");
        let right_menu = rig.wait_until(&pane, Duration::from_secs(2), |s| {
            s.contains("s  Stop") || menu_has_activity_and_memos(s)
        });
        assert!(
            menu_has_activity_and_memos(&right_menu),
            "[{mode_tag}] right-click card menu on running session must offer Activity… and Memos…:\n{right_menu}"
        );
        thread::sleep(Duration::from_millis(25));
        rig.send_keys(&pane, &["Escape"]);
        let _ = rig.wait_until(&pane, Duration::from_secs(2), |s| !s.contains("s  Stop"));

        // 2. Stopped session ("beta"): expand stopped group with `z`, select `beta` with `j`, open menu with `m`.
        rig.send_literal(&pane, "z");
        let _ = rig.wait_until(&pane, Duration::from_secs(2), |s| s.contains("beta"));
        rig.send_literal(&pane, "j");
        thread::sleep(Duration::from_millis(50));
        rig.send_literal(&pane, "m");
        let stopped_menu = rig.wait_until(&pane, Duration::from_secs(2), |s| {
            s.contains("s  Start") || menu_has_activity_and_memos(s)
        });
        assert!(
            stopped_menu.contains("s  Start"),
            "[{mode_tag}] stopped card menu must offer Start:\n{stopped_menu}"
        );
        assert!(
            menu_has_activity_and_memos(&stopped_menu),
            "[{mode_tag}] stopped card menu must offer Activity… and Memos…:\n{stopped_menu}"
        );
        thread::sleep(Duration::from_millis(25));
        rig.send_keys(&pane, &["Escape"]);
        let _ = rig.wait_until(&pane, Duration::from_secs(2), |s| !s.contains("s  Start"));
        right_click_card(&rig, &pane, "beta");
        let stopped_right_menu =
            rig.wait_until(&pane, Duration::from_secs(2), menu_has_activity_and_memos);
        assert!(
            stopped_right_menu.contains("s  Start")
                && menu_has_activity_and_memos(&stopped_right_menu),
            "[{mode_tag}] right-click on stopped beta must offer its own menu:\n{stopped_right_menu}"
        );
    }
}

const CURATED_EVENTS: [(&str, &str, &str, &str, &str); 15] = [
    ("state", "lead", "", "working", "KIND_STATE_OK"),
    ("done", "lead", "", "", "KIND_DONE_OK"),
    ("goal", "human", "", "", "KIND_GOAL_OK"),
    ("spawn", "lead", "worker1", "sonnet55x", "KIND_SPAWN_OK"),
    ("retire", "lead", "worker1", "", "KIND_RETIRE_OK"),
    ("relaunch", "lead", "worker1", "", "KIND_RELAUNCH_OK"),
    ("reseat", "lead", "worker1", "sol61x", "KIND_RESEAT_OK"),
    ("ask", "lead", "colead", "req-01", "KIND_ASK_OK"),
    ("review", "lead", "colead", "req-02", "KIND_REVIEW_OK"),
    ("reply", "colead", "lead", "req-02", "KIND_REPLY_OK"),
    ("watchdog-start", "watchdog", "", "", "KIND_WD_START_OK"),
    ("watchdog-stop", "watchdog", "", "", "KIND_WD_STOP_OK"),
    ("effort-applied", "lead", "main", "xhigh", "KIND_EFF_APP_OK"),
    ("effort-restored", "lead", "main", "high", "KIND_EFF_RES_OK"),
    ("effort-failed", "lead", "main", "max", "KIND_EFF_FAIL_OK"),
];

fn write_curated_and_filtered_events(dir: &Path, base_ts: i64) {
    let mut lines = Vec::new();
    for (idx, (kind, actor, target, reference, summary)) in (0_i64..).zip(CURATED_EVENTS.iter()) {
        let ts = ae::time::Timestamp::from_epoch(base_ts + idx);
        lines.push(format!(
            r#"{{"ts":"{ts}","actor":"{actor}","action":"{kind}","target":"{target}","ref":"{reference}","summary":"{summary}"}}"#
        ));
    }
    lines.push(format!(
        r#"{{"ts":"{}","actor":"ae:watchdog","action":"watchdog-start","target":"","ref":"","summary":"HOUSEKEEPING_WD_START_DROP"}}"#,
        ae::time::Timestamp::from_epoch(base_ts + 20)
    ));
    lines.push(format!(
        r#"{{"ts":"{}","actor":"ae:daemon","action":"watchdog-stop","target":"","ref":"","summary":"HOUSEKEEPING_WD_STOP_DROP"}}"#,
        ae::time::Timestamp::from_epoch(base_ts + 21)
    ));
    for (offset, (bad_kind, token)) in (0_i64..).zip([
        ("say", "EXCLUDED_SAY_ROW"),
        ("send", "EXCLUDED_SEND_ROW"),
        ("memo", "EXCLUDED_MEMO_ROW"),
        ("conversation-new", "EXCLUDED_CONV_ROW"),
    ]) {
        let ts = ae::time::Timestamp::from_epoch(base_ts + 30 + offset);
        lines.push(format!(
            r#"{{"ts":"{ts}","actor":"lead","action":"{bad_kind}","target":"colead","ref":"","summary":"{token}"}}"#
        ));
    }
    lines.push(format!(
        "{{\"ts\":\"{}\",\"actor\":\"lead\",\"action\":\"goal\",\"target\":\"\",\"ref\":\"\",\"summary\":\"ESC_SAFE \u{1b}[31mRED_TEXT\u{1b}[0m \u{1b}]52;c;dGVzdA==\u{7} END_SAFE\"}}",
        ae::time::Timestamp::from_epoch(base_ts + 40)
    ));
    fs::write(dir.join("events.jsonl"), lines.join("\n") + "\n")
        .unwrap_or_else(|err| panic!("write events.jsonl: {err}"));
}

/// Existing tmux-menu fold is an independent reference, not the new app fold.
/// Compare its complete cells to the painted screen; kills raw journal dumps,
/// missing ages/actors/kinds, swapped cells and missing detail em dashes.
fn activity_reference(dir: &Path) -> Vec<Vec<String>> {
    let meta_text =
        fs::read_to_string(dir.join("meta")).unwrap_or_else(|err| panic!("reference meta: {err}"));
    let meta = ae::session_menu::MetaSource::Parsed {
        uuid: ALPHA_UUID.to_owned(),
        actors: vec!["lead".to_owned()],
        mode: String::new(),
        origin: String::new(),
        work_dir: String::new(),
        branch: String::new(),
        meta: Box::new(ae::meta::Meta::parse(&meta_text)),
    };
    ae::session_menu::activity_cells(
        &ae::tmux::OptionReading::Set(ALPHA_UUID.to_owned()),
        &meta,
        &ae::store::SourceRead::Ready(
            fs::read(dir.join("events.jsonl"))
                .unwrap_or_else(|err| panic!("reference events: {err}")),
        ),
        ae::time::Timestamp::now(),
    )
    .into_iter()
    .map(|row| match row {
        ae::session_menu::CellRow::Cells(cells) => cells,
        ae::session_menu::CellRow::Gap(gap) => panic!("fixture must yield rows: {gap}"),
    })
    .collect()
}

#[test]
fn activity_dialog_renders_newest_30_of_15_curated_kinds_filters_ae_watchdog_and_neutralises_escapes()
 {
    let rig = Rig::new("activity-parity");
    rig.start_host();
    rig.plant_running_session("alpha", ALPHA_UUID);
    let dir = rig.session_dir("alpha");
    let base_ts: i64 = 1_578_000_000;
    write_curated_and_filtered_events(&dir, base_ts);
    let expected_rows = activity_reference(&dir);
    assert_eq!(expected_rows.len(), 16, "15 curated kinds + escaped goal");
    assert!(expected_rows.iter().all(|cells| cells[0] == ">7d"));

    let pane = rig.open_app("alpha", false);
    let output = rig.record_output(&pane);
    rig.send_literal(&pane, "m");
    let _ = activate_menu_dialog(&rig, &pane, "Activity");
    let (_, all_pages) = collect_dialog_pages(&rig, &pane);

    for cells in &expected_rows {
        assert_cells_on_screen(&all_pages, cells);
    }

    for (_, _, _, _, expected_token) in &CURATED_EVENTS {
        assert!(
            all_pages.contains(expected_token),
            "Activity dialog missing curated kind token {expected_token}:\n{all_pages}"
        );
    }
    for forbidden in [
        "HOUSEKEEPING_WD_START_DROP",
        "HOUSEKEEPING_WD_STOP_DROP",
        "EXCLUDED_SAY_ROW",
        "EXCLUDED_SEND_ROW",
        "EXCLUDED_MEMO_ROW",
        "EXCLUDED_CONV_ROW",
    ] {
        assert!(
            !all_pages.contains(forbidden),
            "Activity dialog must exclude {forbidden}:\n{all_pages}"
        );
    }
    assert!(
        all_pages.contains("ESC_SAFE ?RED_TEXT? ? END_SAFE"),
        "Activity dialog must render sanitized event summary:\n{all_pages}"
    );
    assert_safe_output(&output);

    // `Esc` returns to the app unchanged.
    thread::sleep(Duration::from_millis(25));
    rig.send_keys(&pane, &["Escape"]);
    let after_esc = rig.wait_until(&pane, Duration::from_secs(2), |s| {
        s.contains("Enter writes") && !s.contains("KIND_GOAL_OK")
    });
    assert!(
        after_esc.contains("alpha") && after_esc.contains("Enter writes"),
        "Esc must close Activity dialog and return to the app:\n{after_esc}"
    );

    // Plant 35 chronological curated events (`ACT_ROW_01`..`ACT_ROW_35`) and
    // verify the newest 30 (`ACT_ROW_35`..`ACT_ROW_06`) render newest-first
    // while `ACT_ROW_01`..`ACT_ROW_05` are capped out (`ACTIVITY_ROWS_MAX = 30`).
    let mut cap_lines = Vec::new();
    for i in 1..=35 {
        let ts = ae::time::Timestamp::from_epoch(base_ts + 100 + i);
        cap_lines.push(format!(
            r#"{{"ts":"{ts}","actor":"lead","action":"goal","target":"","ref":"","summary":"ACT_ROW_{i:02}"}}"#
        ));
    }
    fs::write(dir.join("events.jsonl"), cap_lines.join("\n") + "\n")
        .unwrap_or_else(|err| panic!("write capped events.jsonl: {err}"));

    rig.short_viewport(&pane);
    rig.send_literal(&pane, "m");
    let _ = activate_menu_dialog(&rig, &pane, "Activity");
    let (top_page, full_cap_text) = collect_dialog_pages(&rig, &pane);

    let pos_35 = top_page
        .find("ACT_ROW_35")
        .unwrap_or_else(|| panic!("newest row ACT_ROW_35 must appear on top page:\n{top_page}"));
    let pos_34 = top_page
        .find("ACT_ROW_34")
        .unwrap_or_else(|| panic!("second newest row ACT_ROW_34 must appear:\n{top_page}"));
    assert!(
        pos_35 < pos_34,
        "Activity dialog must order newest records first (ACT_ROW_35 before ACT_ROW_34):\n{top_page}"
    );
    for kept in 6..=35 {
        let token = format!("ACT_ROW_{kept:02}");
        assert!(
            full_cap_text.contains(&token),
            "Activity dialog must keep newest 30 rows (missing {token}):\n{full_cap_text}"
        );
    }
    for dropped in 1..=5 {
        let token = format!("ACT_ROW_{dropped:02}");
        assert!(
            !full_cap_text.contains(&token),
            "Activity dialog must cap at 30 rows (found old {token}):\n{full_cap_text}"
        );
    }
    assert_scrolled_to_end(&rig, &pane, "ACT_ROW_35", "ACT_ROW_06");

    // `q` also returns to the app unchanged without quitting `ae app`.
    thread::sleep(Duration::from_millis(25));
    rig.send_literal(&pane, "q");
    let after_q = rig.wait_until(&pane, Duration::from_secs(2), |s| {
        s.contains("Enter writes") && !s.contains("ACT_ROW_35")
    });
    assert!(
        after_q.contains("alpha") && after_q.contains("Enter writes"),
        "q must close Activity dialog and return to the app without quitting:\n{after_q}"
    );
}

fn plant_stopped_beta_memos_and_verify_oracle(beta_dir: &Path) -> Vec<ae::brief::TopicLine> {
    let memo_tsv = "\
2020-10-30T10:00:00Z\tlead\tarch\tOLD_ARCH_MEMO_MUST_BE_SUPERSEDED\n\
2020-10-30T10:05:00Z\tlead\tplan\tOLD_PLAN_MEMO_MUST_BE_SUPERSEDED\n\
2020-10-30T10:10:00Z\tlead\tarch\tLATEST_ARCH_MEMO_WINS\n\
2020-10-30T10:15:00Z\tcolead\tplan\tLATEST_PLAN_MEMO_WINS\n\
2020-10-30T10:20:00Z\tlead\tsec\tMEMO_ESC_SAFE \x1b[32mGREEN\x1b[0m \x1b]52;c;dGVzdA==\x07 DONE_SAFE\n";
    fs::write(beta_dir.join("memo.tsv"), memo_tsv)
        .unwrap_or_else(|err| panic!("write beta memo.tsv: {err}"));
    fs::write(
        beta_dir.join("events.jsonl"),
        r#"{"ts":"2020-10-30T10:20:00Z","actor":"lead","action":"goal","target":"","ref":"","summary":"BETA_STOPPED_GOAL"}"#.to_owned()
            + "\n",
    )
    .unwrap_or_else(|err| panic!("write beta events.jsonl: {err}"));

    let oracle_topics =
        ae::brief::topic_lines(memo_tsv.as_bytes(), ae::time::Timestamp::now(), None);
    assert_eq!(
        oracle_topics
            .iter()
            .map(|line| line.topic.as_str())
            .collect::<Vec<_>>(),
        ["sec", "plan", "arch"],
        "independent brief reference orders latest-per-topic newest first"
    );
    oracle_topics
}

fn assert_memo_parity(
    oracle_topics: &[ae::brief::TopicLine],
    first_memo_page: &str,
    all_memo_pages: &str,
) {
    // Governing parity is the tmux Memos dialog's age/topic/text. Plan's
    // additional author field waived by lead ruling ae-20261030T115339Z-88166f8f.
    // Kills body-only rendering and every permutation except sec→plan→arch.
    let mut prior = None;
    for topic in oracle_topics {
        let text = if topic.topic == "sec" {
            "MEMO_ESC_SAFE ?GREEN? ? DONE_SAFE"
        } else {
            topic.text.as_str()
        };
        assert_cells_on_screen(
            all_memo_pages,
            &[
                ae::brief::age(topic.age_secs),
                topic.topic.clone(),
                text.to_owned(),
            ],
        );
        let at = first_memo_page
            .find(text)
            .unwrap_or_else(|| panic!("latest topic on first page: {text}"));
        assert!(
            prior.is_none_or(|before| before < at),
            "newest-first sec→plan→arch:\n{first_memo_page}"
        );
        prior = Some(at);
    }
    assert!(
        !all_memo_pages.contains("OLD_ARCH_MEMO_MUST_BE_SUPERSEDED")
            && !all_memo_pages.contains("OLD_PLAN_MEMO_MUST_BE_SUPERSEDED"),
        "Memos dialog must supersede older memos of the same topic:\n{all_memo_pages}"
    );
}

#[test]
fn memos_dialog_matches_ae_brief_latest_per_topic_caps_at_30_and_stays_read_only() {
    let rig = Rig::new("memos-parity");
    rig.start_host();
    rig.plant_running_session("alpha", ALPHA_UUID);
    rig.plant_stopped_session("beta", BETA_UUID);

    let beta_dir = rig.session_dir("beta");
    let oracle_topics = plant_stopped_beta_memos_and_verify_oracle(&beta_dir);

    let before_beta_files = snapshot_dir_bytes(&beta_dir);
    let (_, before_agent_pane, _) = rig.tmux(&["capture-pane", "-p", "-t", "alpha:main"]);

    // Run `ae app` outside tmux, expand stopped sessions (`z`), select `beta` (`j`),
    // and inspect `Memos…` and `Activity…` on the stopped session.
    let pane = rig.open_app("alpha", false);
    let output = rig.record_output(&pane);
    rig.send_literal(&pane, "z");
    let _ = rig.wait_until(&pane, Duration::from_secs(2), |s| s.contains("beta"));
    rig.send_literal(&pane, "j");
    thread::sleep(Duration::from_millis(50));

    rig.send_literal(&pane, "m");
    let _ = activate_menu_dialog(&rig, &pane, "Memos");
    let (first_memo_page, all_memo_pages) = collect_dialog_pages(&rig, &pane);
    assert_memo_parity(&oracle_topics, &first_memo_page, &all_memo_pages);
    assert_safe_output(&output);

    // Close with `Esc`, then open `Activity…` on the stopped session and close with `q`.
    thread::sleep(Duration::from_millis(25));
    rig.send_keys(&pane, &["Escape"]);
    let _ = rig.wait_until(&pane, Duration::from_secs(2), |s| {
        !s.contains("LATEST_ARCH_MEMO_WINS")
    });
    rig.send_literal(&pane, "m");
    let _ = activate_menu_dialog(&rig, &pane, "Activity");
    let (_, stopped_activity) = collect_dialog_pages(&rig, &pane);
    assert!(
        stopped_activity.contains("BETA_STOPPED_GOAL"),
        "Activity dialog on stopped session must render its durable events:\n{stopped_activity}"
    );
    thread::sleep(Duration::from_millis(25));
    rig.send_literal(&pane, "q");
    let _ = rig.wait_until(&pane, Duration::from_secs(2), |s| {
        !s.contains("BETA_STOPPED_GOAL")
    });

    // Read-only invariant: zero bytes written or journaled in `beta`, zero keys typed into `alpha:main`.
    assert_eq!(
        before_beta_files,
        snapshot_dir_bytes(&beta_dir),
        "opening and closing Activity… and Memos… must write nothing and journal nothing"
    );
    let (_, after_agent_pane, _) = rig.tmux(&["capture-pane", "-p", "-t", "alpha:main"]);
    assert_eq!(
        before_agent_pane, after_agent_pane,
        "opening and closing Activity… and Memos… must type into no pane"
    );

    // 30-row cap on Memos (`MEMO_ROWS_MAX = 30`): plant 34 distinct topics on `alpha`.
    let alpha_dir = rig.session_dir("alpha");
    let mut many_memos = String::new();
    for i in 1..=34 {
        let _ = writeln!(
            many_memos,
            "2026-10-30T11:{:02}:00Z\tlead\ttopic-{i:02}\tMEMO_CAP_BODY_{i:02}",
            i % 60
        );
    }
    fs::write(alpha_dir.join("memo.tsv"), &many_memos)
        .unwrap_or_else(|err| panic!("write alpha memo.tsv: {err}"));
    let before_alpha_files = snapshot_dir_bytes(&alpha_dir);

    rig.send_literal(&pane, "k");
    thread::sleep(Duration::from_millis(50));
    rig.short_viewport(&pane);
    rig.send_literal(&pane, "m");
    let _ = activate_menu_dialog(&rig, &pane, "Memos");
    let (_, capped_memos) = collect_dialog_pages(&rig, &pane);

    for kept in 5..=34 {
        let token = format!("MEMO_CAP_BODY_{kept:02}");
        assert!(
            capped_memos.contains(&token),
            "Memos dialog must keep newest 30 topics (missing {token}):\n{capped_memos}"
        );
    }
    for dropped in 1..=4 {
        let token = format!("MEMO_CAP_BODY_{dropped:02}");
        assert!(
            !capped_memos.contains(&token),
            "Memos dialog must cap at 30 topics (found old {token}):\n{capped_memos}"
        );
    }
    assert_scrolled_to_end(&rig, &pane, "MEMO_CAP_BODY_34", "MEMO_CAP_BODY_05");
    rig.send_literal(&pane, "q");
    let _ = rig.wait_until(&pane, Duration::from_secs(2), |s| {
        s.contains("Enter writes") && !s.contains("MEMO_CAP_BODY_05")
    });
    assert_eq!(
        before_alpha_files,
        snapshot_dir_bytes(&alpha_dir),
        "alpha cap dialog also writes and journals nothing"
    );
    let (_, after_cap_agent_pane, _) = rig.tmux(&["capture-pane", "-p", "-t", "alpha:main"]);
    assert_eq!(
        before_agent_pane, after_cap_agent_pane,
        "alpha cap dialog types into no agent pane"
    );
}

fn assert_dialog_gap(rig: &Rig, pane: &str, item: &str, expected_gap: &str) {
    rig.send_literal(pane, "m");
    let _ = activate_menu_dialog(rig, pane, item);
    let (screen, _) = collect_dialog_pages(rig, pane);
    assert!(
        screen.contains(expected_gap),
        "{item} dialog must render '{expected_gap}':\n{screen}"
    );
    thread::sleep(Duration::from_millis(25));
    rig.send_keys(pane, &["Escape"]);
    let _ = rig.wait_until(pane, Duration::from_secs(2), |s| !s.contains(expected_gap));
}

#[test]
fn incarnation_mismatch_empty_and_unreadable_sources_render_honest_gaps() {
    let rig = Rig::new("gaps-and-incarnation");
    rig.start_host();
    rig.plant_running_session("alpha", ALPHA_UUID);
    let dir = rig.session_dir("alpha");

    let pane = rig.open_app("alpha", true);

    // 1. Empty / absent `events.jsonl` and `memo.tsv` render `activity: none` and `memos: none`.
    assert_dialog_gap(&rig, &pane, "Activity", "activity: none");
    assert_dialog_gap(&rig, &pane, "Memos", "memos: none");

    // 2. Non-regular `events.jsonl` and `memo.tsv` (directories) render `unreadable` gaps.
    fs::create_dir_all(dir.join("events.jsonl"))
        .unwrap_or_else(|err| panic!("mkdir events.jsonl: {err}"));
    fs::create_dir_all(dir.join("memo.tsv")).unwrap_or_else(|err| panic!("mkdir memo.tsv: {err}"));

    assert_dialog_gap(&rig, &pane, "Activity", "activity: unreadable");
    assert_dialog_gap(&rig, &pane, "Memos", "memos: unreadable");

    fs::remove_dir_all(dir.join("events.jsonl"))
        .unwrap_or_else(|err| panic!("rmdir events.jsonl: {err}"));
    fs::remove_dir_all(dir.join("memo.tsv")).unwrap_or_else(|err| panic!("rmdir memo.tsv: {err}"));

    // 3. Incarnation mismatch (`meta` rewritten to `OTHER_UUID` with foreign
    // records while the drawn card or live `@ae_session_uuid` still names
    // `ALPHA_UUID`): foreign records must NEVER render.
    fs::write(
        dir.join("events.jsonl"),
        r#"{"ts":"2020-10-30T10:20:00Z","actor":"lead","action":"goal","target":"","ref":"","summary":"FOREIGN_INCARNATION_EVENT_LEAK"}"#.to_owned()
            + "\n",
    )
    .unwrap_or_else(|err| panic!("write foreign events.jsonl: {err}"));
    fs::write(
        dir.join("memo.tsv"),
        "2026-10-30T12:00:00Z\tlead\tleak\tFOREIGN_INCARNATION_MEMO_LEAK\n",
    )
    .unwrap_or_else(|err| panic!("write foreign memo.tsv: {err}"));
    let old_meta =
        fs::read_to_string(dir.join("meta")).unwrap_or_else(|err| panic!("read meta: {err}"));
    let foreign_meta = old_meta.replace(ALPHA_UUID, OTHER_UUID);
    fs::write(dir.join("meta"), &foreign_meta)
        .unwrap_or_else(|err| panic!("write foreign meta: {err}"));

    for (item, leak_token) in [
        ("Activity", "FOREIGN_INCARNATION_EVENT_LEAK"),
        ("Memos", "FOREIGN_INCARNATION_MEMO_LEAK"),
    ] {
        rig.send_literal(&pane, "m");
        let _ = activate_menu_dialog(&rig, &pane, item);
        let (_, screen) = collect_dialog_pages(&rig, &pane);
        assert!(
            !screen.contains(leak_token),
            "records of another incarnation must NEVER render in {item}:\n{screen}"
        );
        assert!(
            screen.contains(&format!(
                "{}: unavailable (meta: identity mismatch)",
                item.to_ascii_lowercase()
            )),
            "incarnation mismatch in {item} must name the gap or refusal:\n{screen}"
        );
        thread::sleep(Duration::from_millis(25));
        rig.send_keys(&pane, &["Escape"]);
        let gap = format!(
            "{}: unavailable (meta: identity mismatch)",
            item.to_ascii_lowercase()
        );
        let closed = rig.wait_until(&pane, Duration::from_secs(2), |s| !s.contains(&gap));
        assert!(
            !closed.contains(&gap),
            "{item} dialog did not close after Escape:\n{closed}"
        );
    }
}
