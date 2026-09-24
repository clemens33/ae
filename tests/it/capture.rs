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
        let scratch = super::cli::OwnedScratch::root("cap", tag).keep();
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
                 launch_time.main={launch_time}\ncapture_floor.main={launch_time}\nlaunch_id.main=tok-1\n",
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
        self.capture_slot("main")
    }

    /// Run the real `_capture-sid` child for one slot.
    fn capture_slot(&self, slot: &str) -> (Option<i32>, String) {
        let path = std::env::var("PATH").unwrap_or_default();
        let out = ae()
            .env("HOME", &self.home)
            .env("PATH", format!("{}:{path}", self.bin.display()))
            .arg(ae::cli::CAPTURE_SID)
            .args([&self.session.display().to_string(), slot, "%0"])
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
    // The conversation this launch started.
    rig.write_bytes(
        &store.join("643393ad-eb92-4b9e-ab7a-0fe7b1221fa1.db"),
        b"SQLite format 3\x00\xff\xfe\x00AE_AGY_LAUNCH_ID=tok-1\x00\xc3\x28",
    );
    // Another conversation, newer, holding a DIFFERENT launch's token.
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
             capture_floor.main=1\n",
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
    // `created` is milliseconds, so both local sessions clear the seat's
    // `capture_floor.main=1`. The older one was touched later, proving that
    // newest birth, not last touch, selects the captured conversation.
    rig.fake_opencode(&format!(
        r#"[{{"id":"ses_old","directory":"{project}","created":1000,"updated":9000}},
  {{"id":"ses_new","directory":"{project}","created":5000,"updated":2000}},
  {{"id":"ses_elsewhere","directory":"/nowhere","created":9000,"updated":9999}}]"#,
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

/// #56 R1: a pending seat never captures an id another seat already records.
/// The recorded newest session is invisible to the pending seat; its own older
/// session is the only attributable candidate.
#[test]
fn a_pending_opencode_seat_never_captures_another_seats_recorded_session() {
    let rig = Rig::new("oc-r1", "opencode", 1);
    rig.write(
        &rig.session.join("meta"),
        &format!(
            "session=cap\nwork_dir={project}\nmode=local\nschema=2\nseat.main=lead\n\
             profile.main=tool\nagent_bin.main=opencode\nharness_session.main=ses_new\n\
             launch_time.main=1\ncapture_floor.main=1\nlaunch_id.main=tok-1\n\
             seat.worker.1=w1\nprofile.worker.1=tool\nagent_bin.worker.1=opencode\n\
             harness_session.worker.1=pending\nlaunch_time.worker.1=1\n\
             capture_floor.worker.1=1\nlaunch_id.worker.1=tok-2\n",
            project = rig.project.display()
        ),
    );
    rig.fake_opencode(&format!(
        r#"[{{"id":"ses_new","directory":"{project}","created":9000,"updated":9000}},
  {{"id":"ses_old","directory":"{project}","created":2000,"updated":3000}}]"#,
        project = rig.project.display()
    ));

    let (code, stderr) = rig.capture_slot("worker.1");
    assert_eq!((code, stderr.as_str()), (Some(0), ""));
    let meta = rig.meta();
    assert!(
        meta.contains("harness_session.worker.1=ses_old\n"),
        "the pending seat captured another seat's recorded session:\n{meta}"
    );
}

/// #56 R2': a long-pending seat re-scanned after a pending sibling's session
/// exists takes its own older session, never the sibling's newer one. The
/// sibling's session covers two pending windows, so it is attributable to
/// neither; the older session covers only this seat.
#[test]
fn a_rescanned_pending_seat_takes_its_own_session_not_its_siblings() {
    let rig = Rig::new("oc-r2", "opencode", 1);
    rig.write(
        &rig.session.join("meta"),
        &format!(
            "session=cap\nwork_dir={project}\nmode=local\nschema=2\nseat.main=lead\n\
             profile.main=tool\nagent_bin.main=opencode\nharness_session.main=pending\n\
             launch_time.main=1\ncapture_floor.main=1\nlaunch_id.main=tok-1\n\
             seat.worker.1=w1\nprofile.worker.1=tool\nagent_bin.worker.1=opencode\n\
             harness_session.worker.1=pending\nlaunch_time.worker.1=4\n\
             capture_floor.worker.1=4\nlaunch_id.worker.1=tok-2\n",
            project = rig.project.display()
        ),
    );
    rig.fake_opencode(&format!(
        r#"[{{"id":"ses_sibling","directory":"{project}","created":5000,"updated":5000}},
  {{"id":"ses_own","directory":"{project}","created":2000,"updated":3000}}]"#,
        project = rig.project.display()
    ));

    let (code, stderr) = rig.capture_slot("main");
    assert_eq!((code, stderr.as_str()), (Some(0), ""));
    let meta = rig.meta();
    assert!(
        meta.contains("harness_session.main=ses_own\n"),
        "the rescanned seat captured its sibling's session:\n{meta}"
    );
}

/// Muse stores the session id in its fourth-level directory, while the launch
/// marker stays inside an encoded retained frame. Capture proves the marker as
/// raw bytes and takes only that directory basename; it does not parse JSON.
#[test]
fn a_muse_seat_captures_its_directory_named_session_from_raw_launch_token() {
    let rig = Rig::new("muse", "muse", 1);
    let day = ae::time::Timestamp::now().to_string()[..10].replace('-', "/");
    let mine = "01a09b51-c88a-7fc0-8f71-200ea396c8a7";
    let other = "02b09b51-c88a-7fc0-8f71-200ea396c8a7";
    let sessions = rig.home.join(".local/share/muse/sessions").join(day);
    rig.write_bytes(
        &sessions.join(mine).join("session.jsonl"),
        b"{\"type\":\"retained_frame\",\"children\":[{\"record_json\":\"{\\\"text\\\":\\\"AE_MUSE_LAUNCH_ID=tok-1\\\"}\"}]}\n",
    );
    rig.write_bytes(
        &sessions.join(other).join("session.jsonl"),
        b"{\"type\":\"retained_frame\",\"children\":[{\"record_json\":\"{\\\"text\\\":\\\"AE_MUSE_LAUNCH_ID=tok-other\\\"}\"}]}\n",
    );

    let (code, stderr) = rig.capture();
    assert_eq!((code, stderr.as_str()), (Some(0), ""));
    let meta = rig.meta();
    assert!(
        meta.contains(&format!("harness_session.main={mine}")),
        "{meta}"
    );
    assert!(
        !meta.contains(other),
        "another Muse launch's directory was captured: {meta}"
    );
}

#[test]
fn a_seat_holding_a_tool_that_needs_no_capture_is_left_alone() {
    // claude takes an ae-generated id at LAUNCH, so there is nothing to capture
    // and nothing to wait for: the child must answer immediately and touch
    // nothing.
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
    let wrong = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    rig.write(
        &rig.session.join("meta"),
        &rig.meta().replace(
            "harness_session.main=pending",
            &format!("harness_session.main={wrong}"),
        ),
    );
    plant_rollout(&rig, id);
    let registered = helper(&shim)
        .args(["main", id])
        .env("HOME", &rig.home)
        .output()
        .unwrap_or_else(|why| panic!("the shim should run: {why}"));
    assert!(
        registered.status.success(),
        "the rollout-proven handshake: {}",
        String::from_utf8_lossy(&registered.stderr)
    );

    let meta = rig.meta();
    assert!(
        meta.contains(&format!("harness_session.main={id}")),
        "the capture must report the handshake's id:\n{meta}"
    );
    // The replaced id is no longer the current one — and it is not lost either:
    // the authoritative arm moves it into the seat's predecessor row, TAGGED
    // with the tool whose store it lives in.
    assert!(
        !meta.contains(&format!("harness_session.main={wrong}"))
            && meta.contains(&format!("harness_session_prior.main=codex:{wrong}\n")),
        "the replaced id is a predecessor, never again the current id:\n{meta}"
    );
    assert!(
        !rig.session.join("codex.main.sid").exists(),
        "a consumed handshake file is removed:\n{meta}"
    );
}

/// A codex rollout proving `id` to the launch token the rig records.
fn plant_rollout(rig: &Rig, id: &str) {
    let day = ae::time::Timestamp::now().to_string()[..10].replace('-', "/");
    let started = ae::time::Timestamp::now();
    rig.write(
        &rig.home
            .join(".codex")
            .join("sessions")
            .join(day)
            .join(format!("rollout-{id}.jsonl")),
        &format!(
            "{{\"timestamp\":\"{started}\",\"type\":\"session_meta\",\"payload\":{{\"id\":\"{id}\",\"cwd\":\"{}\"}}}}\n\
             {{\"text\":\"AE_CODEX_LAUNCH_ID=tok-1\"}}\n",
            rig.project.display()
        ),
    );
}

/// Run the `_register-sid` handshake for `main` with `id`.
fn register_sid(rig: &Rig, id: &str) -> std::process::Output {
    use super::cli::helper;

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
    helper(&rig.session.join("_register-sid"))
        .args(["main", id])
        .env("HOME", &rig.home)
        .output()
        .unwrap_or_else(|why| panic!("the shim should run: {why}"))
}

/// The authoritative codex handshake may REPLACE a live id: the id it replaces
/// becomes the seat's newest predecessor, when it is a usable conversation name.
#[test]
fn an_authoritative_capture_keeps_only_a_usable_replaced_id_as_a_predecessor() {
    let old = "0199c0de-1234-4890-abcd-ef0123456789";
    let new = "0199c0de-9999-4890-abcd-ef0123456789";
    for (tag, recorded, replaced) in [
        ("replaced", old, true),
        ("pending", "pending", false),
        ("same", new, false),
        ("not-uuid", "NOT-A-UUID", false),
    ] {
        let rig = Rig::new(tag, "codex", 1);
        if recorded != "pending" {
            rig.write(
                &rig.session.join("meta"),
                &rig.meta().replace(
                    "harness_session.main=pending",
                    &format!("harness_session.main={recorded}"),
                ),
            );
        }
        plant_rollout(&rig, new);
        let registered = register_sid(&rig, new);
        assert!(
            registered.status.success(),
            "{tag}: {}",
            String::from_utf8_lossy(&registered.stderr)
        );
        let meta = rig.meta();
        assert!(
            meta.contains(&format!("harness_session.main={new}\n")),
            "{tag}: {meta}"
        );
        let prior = meta
            .lines()
            .find_map(|line| line.strip_prefix("harness_session_prior.main="));
        // TAGGED with the tool that owns it. A capture never crosses tools, so
        // the tag is the slot's own recorded binary — and it is written even
        // here, because a reader must never have to guess whether an element
        // predates a reseat.
        let tagged = format!("codex:{old}");
        assert_eq!(prior, replaced.then_some(tagged.as_str()), "{tag}: {meta}");
    }
}

/// A legacy tokenless seat may discover by cwd, but cwd is not proof strong
/// enough to redirect an already recorded conversation.
#[test]
fn a_tokenless_register_sid_never_replaces_a_recorded_id_by_cwd() {
    use super::cli::helper;

    let rig = Rig::new("regsid-tokenless", "codex", 1);
    let rendered = ae()
        .arg(ae::cli::SHIMS_RENDER)
        .arg(&rig.session)
        .output()
        .unwrap_or_else(|why| panic!("the ae binary should run: {why}"));
    assert!(rendered.status.success(), "the shims render: {rendered:?}");

    let recorded = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
    rig.write(
        &rig.session.join("meta"),
        &rig.meta()
            .replace(
                "harness_session.main=pending",
                &format!("harness_session.main={recorded}"),
            )
            .replace("launch_id.main=tok-1\n", ""),
    );
    let found = "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb";
    let day = ae::time::Timestamp::now().to_string()[..10].replace('-', "/");
    rig.write(
        &rig
            .home
            .join(".codex")
            .join("sessions")
            .join(day)
            .join(format!("rollout-{found}.jsonl")),
        &format!(
            "{{\"timestamp\":\"{}\",\"type\":\"session_meta\",\"payload\":{{\"id\":\"{found}\",\"cwd\":\"{}\"}}}}\n",
            ae::time::Timestamp::now(),
            rig.project.display()
        ),
    );

    let attempted = helper(&rig.session.join("_register-sid"))
        .args(["main", found])
        .env("HOME", &rig.home)
        .output()
        .unwrap_or_else(|why| panic!("the shim should run: {why}"));
    assert_eq!(attempted.status.code(), Some(1), "{attempted:?}");
    let meta = rig.meta();
    assert!(
        meta.contains(&format!("harness_session.main={recorded}")),
        "cwd-only discovery redirected the recorded conversation: {meta}"
    );
    assert!(
        !meta.contains(found),
        "the unproved id reached meta: {meta}"
    );
}

#[test]
fn an_explicit_opencode_seat_captures_the_session_in_its_own_seat_directory() {
    let rig = Rig::new("oc-exp", "opencode", 1);
    let seat = rig.scratch.join("seat");
    assert!(std::fs::create_dir_all(&seat).is_ok(), "a seat dir");
    rig.write(
        &rig.session.join("meta"),
        &format!(
            "session=cap\nwork_dir={}\nmode=local\nschema=2\nseat.main=lead\n\
             profile.main=tool\nagent_bin.main=opencode\nharness_session.main=pending\n\
             launch_time.main=1\ncapture_floor.main=1\nlaunch_id.main=tok-1\n\
             work_dir.main={}\n",
            rig.project.display(),
            seat.display(),
        ),
    );
    rig.fake_opencode(&format!(
        r#"[{{"id":"ses_session","directory":"{project}","created":5000,"updated":6000}},
  {{"id":"ses_seat","directory":"{seat}","created":2000,"updated":3000}}]"#,
        project = rig.project.display(),
        seat = seat.display(),
    ));

    let (code, stderr) = rig.capture();
    assert_eq!((code, stderr.as_str()), (Some(0), ""));
    let meta = rig.meta();
    assert!(meta.contains("harness_session.main=ses_seat\n"), "{meta}");
    assert!(
        !meta.contains("ses_session"),
        "the session-dir decoy was captured:\n{meta}"
    );
}
