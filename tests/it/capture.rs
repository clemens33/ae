//! `_capture-sid` end to end: the detached child that turns a tool's own
//! post-launch session id into the roster's `harness_session.<slot>`.
//!
//! Run as a BLACK-BOX process, because that is the only shape in which the two
//! facts this entry depends on are real: the caller's `HOME`, where every tool
//! keeps its conversation history, and `PATH`, where `opencode` is found. A
//! library test would have to mutate the test runner's own environment to fake
//! either, and both are process-wide.
//!
//! Codex's arm is covered twice. The launch suite
//! (`session_launch::a_codex_launch_captures_the_session_id_it_registers`)
//! drives the `codex.<slot>.sid` file through a real launch; the last test here
//! drives the HANDSHAKE that is supposed to write it, through the real shim.
//! The two tools above it have no handshake — ae has to go and look — so they
//! are driven against a prepared history directory and a prepared `opencode`.

#![allow(
    clippy::disallowed_methods,
    reason = "fixtures build and inspect real directories; the boundary is about what \
              PRODUCT code may reach"
)]

use std::path::{Path, PathBuf};

use super::cli::ae;

/// One isolated world: a session directory, a fake `HOME`, a fake `PATH` entry.
struct Rig {
    scratch: PathBuf,
    session: PathBuf,
    home: PathBuf,
    project: PathBuf,
    bin: PathBuf,
}

impl Rig {
    fn new(tag: &str, binary: &str, launch_time: i64) -> Self {
        let scratch = PathBuf::from(format!("/tmp/aecap.{}.{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scratch);
        let rig = Self {
            session: scratch.join("session"),
            home: scratch.join("home"),
            project: scratch.join("project"),
            bin: scratch.join("bin"),
            scratch,
        };
        for dir in [&rig.session, &rig.home, &rig.project, &rig.bin] {
            assert!(std::fs::create_dir_all(dir).is_ok(), "a fixture directory");
        }
        // A v2 meta with ONE seat: `set-harness-session` refuses a slot that is
        // not in the roster, so the seat is what makes the write possible, and
        // `agent_bin.main` is what tells the capture which tool to ask.
        rig.write(
            &rig.session.join("meta"),
            &format!(
                "session=cap\nwork_dir={}\nmode=local\nschema=2\nseat.main=lead\n\
                 profile.main=tool\nagent_bin.main={binary}\nharness_session.main=pending\n\
                 launch_time.main={launch_time}\nlaunch_id.main=tok-1\n",
                rig.project.display()
            ),
        );
        rig
    }

    fn write(&self, path: &Path, body: &str) {
        self.write_bytes(path, body.as_bytes());
    }

    /// A fixture file that is NOT text: agy's conversation store is `SQLite`, and
    /// a capture that could only read UTF-8 would never see one.
    fn write_bytes(&self, path: &Path, body: &[u8]) {
        assert!(
            std::fs::create_dir_all(path.parent().unwrap_or(&self.scratch)).is_ok(),
            "a fixture parent"
        );
        assert!(std::fs::write(path, body).is_ok(), "a fixture file");
    }

    /// Install an executable that answers as `opencode` would.
    fn fake_opencode(&self, stdout: &str) {
        use std::os::unix::fs::PermissionsExt;
        let path = self.bin.join("opencode");
        self.write(&path, &format!("#!/bin/sh\ncat <<'JSON'\n{stdout}\nJSON\n"));
        assert!(
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).is_ok(),
            "an executable fake opencode"
        );
    }

    /// Run the real `_capture-sid` child with this rig's `HOME` and `PATH`.
    fn capture(&self) -> (Option<i32>, String) {
        let path = std::env::var("PATH").unwrap_or_default();
        let out = ae()
            .env("HOME", &self.home)
            .env("PATH", format!("{}:{path}", self.bin.display()))
            .arg(ae::cli::CAPTURE_SID)
            .args([&self.session.display().to_string(), "main", "%0"])
            .output()
            .unwrap_or_else(|why| panic!("the ae binary should run: {why}"));
        (
            out.status.code(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    fn meta(&self) -> String {
        std::fs::read_to_string(self.session.join("meta")).unwrap_or_default()
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.scratch);
    }
}

#[test]
fn a_gemini_seat_captures_the_session_id_out_of_its_own_chat_history() {
    let rig = Rig::new("gemini", "gemini", 1);
    let project = rig.home.join(".gemini").join("tmp").join("digest");
    rig.write(
        &project.join(".project_root"),
        &rig.project.display().to_string(),
    );
    rig.write(
        &project.join("chats").join("session-1.json"),
        r#"{"sessionId":"gem-42","history":["AE_GEMINI_LAUNCH_ID=tok-1"]}"#,
    );
    // A chat rooted in ANOTHER directory is not this session's, whatever token
    // it carries — the negative half of the same fixture.
    let other = rig.home.join(".gemini").join("tmp").join("elsewhere");
    rig.write(&other.join(".project_root"), "/nowhere");
    rig.write(
        &other.join("chats").join("session-2.json"),
        r#"{"sessionId":"gem-99","history":["AE_GEMINI_LAUNCH_ID=tok-1"]}"#,
    );

    assert!(
        rig.meta().contains("harness_session.main=pending"),
        "the seat starts pending:\n{}",
        rig.meta()
    );
    let (code, stderr) = rig.capture();
    assert_eq!((code, stderr.as_str()), (Some(0), ""));
    let meta = rig.meta();
    assert!(meta.contains("harness_session.main=gem-42"), "{meta}");
    assert!(
        !meta.contains("gem-99"),
        "another project's chat was captured:\n{meta}"
    );
    assert_eq!(
        meta.matches("harness_session.main=").count(),
        1,
        "the row was replaced, not appended:\n{meta}"
    );
}

#[test]
fn an_agy_seat_captures_the_conversation_whose_database_carries_its_launch_token() {
    let rig = Rig::new("agy", "agy", 1);
    let store = rig
        .home
        .join(".gemini")
        .join("antigravity-cli")
        .join("conversations");
    // The conversation this launch started. Its NAME is the id — nothing is
    // parsed out of the file — and the token sits in the injected context agy
    // recorded. The bytes around it are not UTF-8, which is the point: a real
    // conversation store is SQLite, and the text reader every other scan uses
    // answers None for all of it.
    rig.write_bytes(
        &store.join("643393ad-eb92-4b9e-ab7a-0fe7b1221fa1.db"),
        b"SQLite format 3\x00\xff\xfe\x00AE_AGY_LAUNCH_ID=tok-1\x00\xc3\x28",
    );
    // Another conversation, newer, holding a DIFFERENT launch's token. Picking
    // it would mean the token filter never ran.
    rig.write_bytes(
        &store.join("11111111-2222-4333-8444-555555555555.db"),
        b"\x00\xffAE_AGY_LAUNCH_ID=tok-9\x00",
    );
    // The sidecars SQLite writes beside a live database are not conversations.
    rig.write_bytes(
        &store.join("643393ad-eb92-4b9e-ab7a-0fe7b1221fa1.db-wal"),
        b"AE_AGY_LAUNCH_ID=tok-1",
    );

    let (code, stderr) = rig.capture();
    assert_eq!((code, stderr.as_str()), (Some(0), ""));
    let meta = rig.meta();
    assert!(
        meta.contains("harness_session.main=643393ad-eb92-4b9e-ab7a-0fe7b1221fa1"),
        "{meta}"
    );
    assert!(
        !meta.contains("11111111") && !meta.contains("db-wal"),
        "another launch's conversation was captured:\n{meta}"
    );
    assert_eq!(
        meta.matches("harness_session.main=").count(),
        1,
        "the row was replaced, not appended:\n{meta}"
    );
}

/// A FIFO in the conversation store is SKIPPED, and the capture still answers.
///
/// `open(2)` on a named pipe blocks until a writer appears. A pipe called
/// `<uuid>.db` reports a length of 0, so a size-only gate waves it through and
/// the open hangs — in the launch's detached child, and worse, in the
/// watchdog's own cycle, where one bad node stops every nudge on the machine.
///
/// The pipe is named to sort FIRST, so it is visited before the real
/// conversation: if the guard were missing, this test would never reach the
/// assertion, and its timeout would be the failure. The real conversation is
/// there so a PASS means "skipped the pipe and carried on" rather than the
/// weaker "found nothing", which a blocked open could also look like.
#[test]
fn an_agy_fifo_in_the_store_is_skipped_and_does_not_block() {
    let rig = Rig::new("agyfifo", "agy", 1);
    let store = rig
        .home
        .join(".gemini")
        .join("antigravity-cli")
        .join("conversations");
    assert!(std::fs::create_dir_all(&store).is_ok(), "a fixture store");
    super::cli::mkfifo(&store.join("00000000-0000-4000-8000-000000000000.db"));
    let real = "643393ad-eb92-4b9e-ab7a-0fe7b1221fa1";
    rig.write_bytes(
        &store.join(format!("{real}.db")),
        b"\x00\xffAE_AGY_LAUNCH_ID=tok-1\x00",
    );

    let (code, stderr) = rig.capture();
    assert_eq!((code, stderr.as_str()), (Some(0), ""));
    let meta = rig.meta();
    assert!(
        meta.contains(&format!("harness_session.main={real}")),
        "the capture must step over the pipe and find the real conversation:\n{meta}"
    );
}

#[test]
fn an_agy_seat_with_no_token_falls_back_to_the_cli_log_for_its_own_workspace() {
    let rig = Rig::new("agylog", "agy", 1);
    // No `launch_id.<slot>`, so the token half cannot run: this is the arm a
    // seat launched before the marker existed, or one whose context never
    // reached the transcript, comes down to.
    rig.write(
        &rig.session.join("meta"),
        &format!(
            "session=cap\nwork_dir={}\nmode=local\nschema=2\nseat.main=lead\n\
             profile.main=tool\nagent_bin.main=agy\nharness_session.main=pending\n\
             launch_time.main=1\n",
            rig.project.display()
        ),
    );
    let logs = rig.home.join(".gemini").join("antigravity-cli").join("log");
    // agy's own log shape: the workspace once at start-up, then the id of each
    // conversation that run created.
    rig.write(
        &logs.join("cli-20260904_180410.log"),
        &format!(
            "I0904 server.go:285] Creating CLI server backend: product=antigravity \
             workspaceDirs=[{}] appDataDir=/x cascadeManager=true\n\
             I0904 server.go:1137] Created conversation 643393ad-eb92-4b9e-ab7a-0fe7b1221fa1\n\
             I0904 server.go:1137] Created conversation 99999999-9999-4999-8999-999999999999\n",
            rig.project.display()
        ),
    );
    // A run in ANOTHER directory is not this seat's, however recent.
    rig.write(
        &logs.join("cli-20260904_181500.log"),
        "I0904 server.go:285] Creating CLI server backend: workspaceDirs=[/nowhere] appDataDir=/x\n\
         I0904 server.go:1137] Created conversation deadbeef-0000-4000-8000-000000000000\n",
    );

    let (code, stderr) = rig.capture();
    assert_eq!((code, stderr.as_str()), (Some(0), ""));
    let meta = rig.meta();
    assert!(
        meta.contains("harness_session.main=643393ad-eb92-4b9e-ab7a-0fe7b1221fa1"),
        "{meta}"
    );
    assert!(
        !meta.contains("deadbeef"),
        "another workspace's conversation was captured:\n{meta}"
    );
    assert!(
        !meta.contains("99999999"),
        "a later hand-started conversation was captured instead of the launch's:\n{meta}"
    );
}

#[test]
fn an_opencode_seat_captures_the_newest_session_in_its_own_directory() {
    let rig = Rig::new("opencode", "opencode", 1);
    // `updated` is milliseconds, so everything here is at or after the seat's
    // `launch_time.main=1`. The elsewhere entry is the newest of the three:
    // picking it would mean the directory filter never ran.
    rig.fake_opencode(&format!(
        r#"[{{"id":"ses_old","directory":"{project}","time":{{"updated":2000}}}},
  {{"id":"ses_new","directory":"{project}","time":{{"updated":5000}}}},
  {{"id":"ses_elsewhere","directory":"/nowhere","time":{{"updated":9000}}}}]"#,
        project = rig.project.display()
    ));

    let (code, stderr) = rig.capture();
    assert_eq!((code, stderr.as_str()), (Some(0), ""));
    let meta = rig.meta();
    assert!(meta.contains("harness_session.main=ses_new"), "{meta}");
    assert!(
        !meta.contains("ses_elsewhere") && !meta.contains("ses_old"),
        "the wrong session was captured:\n{meta}"
    );
}

#[test]
fn a_seat_holding_a_tool_that_needs_no_capture_is_left_alone() {
    // claude takes an ae-generated id at LAUNCH, so there is nothing to capture
    // and nothing to wait for: the child must answer immediately and touch
    // nothing. A prepared gemini history is present to prove the arm is chosen
    // by the SEAT's tool and not by whatever happens to be lying around.
    let rig = Rig::new("claude", "claude", 1);
    let project = rig.home.join(".gemini").join("tmp").join("digest");
    rig.write(
        &project.join(".project_root"),
        &rig.project.display().to_string(),
    );
    rig.write(
        &project.join("chats").join("session-1.json"),
        r#"{"sessionId":"gem-42"}"#,
    );
    let before = rig.meta();
    let (code, stderr) = rig.capture();
    assert_eq!((code, stderr.as_str()), (Some(0), ""));
    assert_eq!(rig.meta(), before, "the meta was rewritten");
}

/// The `_register-sid` handshake, end to end through the shim it names.
///
/// THE LINK THAT WAS DEAD. Codex's `developer_instructions` have always told it
/// to run `<session-dir>/_register-sid <slot>` as its first action, and the
/// capture has always polled for `codex.<slot>.sid` — but the shim left the
/// helper set with the `declare -f` template library and no core entry replaced
/// it, so the instruction named a file that was not there and every codex seat
/// fell through to the history scans below it.
///
/// Nothing here writes that file but the shim, and codex's history directory is
/// empty, so the id the capture reports can have come from nowhere else.
#[test]
fn the_register_sid_handshake_is_the_id_the_capture_reports() {
    use super::cli::helper;

    let rig = Rig::new("regsid", "codex", 1);
    // The shim set as a session gets it — the same writer the launch runs.
    let rendered = ae()
        .arg(ae::cli::SHIMS_RENDER)
        .arg(&rig.session)
        .output()
        .unwrap_or_else(|why| panic!("the ae binary should run: {why}"));
    assert!(
        rendered.status.success(),
        "the shims render: {}",
        String::from_utf8_lossy(&rendered.stderr)
    );
    let shim = rig.session.join("_register-sid");
    assert!(
        shim.is_file(),
        "the handshake codex is TOLD to run must be a file in {}",
        rig.session.display()
    );

    // A malformed id is refused before anything is written: the value lands in
    // a file the capture reads back as a session id and writes to the roster,
    // and a validator that saw only "non-empty" would let it.
    let refused = helper(&shim)
        .args(["main", "NOT-A-UUID"])
        .output()
        .unwrap_or_else(|why| panic!("the shim should run: {why}"));
    assert_eq!(
        refused.status.code(),
        Some(2),
        "a malformed id is a refusal"
    );
    assert!(
        !rig.session.join("codex.main.sid").exists(),
        "a refused id writes nothing"
    );

    let id = "0199c0de-1234-4890-abcd-ef0123456789";
    let registered = helper(&shim)
        .args(["main", id])
        .output()
        .unwrap_or_else(|why| panic!("the shim should run: {why}"));
    assert!(
        registered.status.success(),
        "the handshake: {}",
        String::from_utf8_lossy(&registered.stderr)
    );

    let (code, stderr) = rig.capture();
    assert_eq!((code, stderr.as_str()), (Some(0), ""));
    let meta = rig.meta();
    assert!(
        meta.contains(&format!("harness_session.main={id}")),
        "the capture must report the handshake's id:\n{meta}"
    );
    assert!(
        !rig.session.join("codex.main.sid").exists(),
        "a consumed handshake file is removed:\n{meta}"
    );
}
