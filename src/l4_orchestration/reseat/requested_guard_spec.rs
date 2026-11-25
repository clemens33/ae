//! A requested move's own checks under the lifecycle lock ([`requested_guard`]):
//! the binding of the incarnation that asked, then the adapter rule against
//! the records as they stand now. Any other move is not asked.

#![allow(
    clippy::disallowed_methods,
    reason = "the fixture owns and removes only its private files"
)]

use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

const UUID: &str = "d04bc042-f51e-4fea-a814-b8bcac07557e";
const ASKED: &str = "11111111-2222-4333-8444-555555555555";
const SINCE: &str = "66666666-7777-4888-9999-aaaaaaaaaaaa";

struct Root(PathBuf);
impl Drop for Root {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// One `requested_guard` call: the seat records `bin` and `launch` under the
/// lock, the move goes to `to` on `route`, bound to the launch that asked when
/// `bound`. The verdict and what it wrote to stderr.
fn guard(
    root: &Path,
    (bin, launch): (&str, &str),
    (route, bound, to): (Route, bool, &str),
) -> (Result<(), Halt>, String) {
    let dir = root.join("sessions/check");
    let file = root.join("config");
    let locked = format!(
        "schema=2\nsession=check\nsession_id={UUID}\nconfig={}\norigin={}\nmeta_agent=true\n\
         seat.main=orchestrator\nprofile.main=claude-a\nagent_bin.main={bin}\nlaunch_id.main={launch}\n",
        file.display(),
        root.display()
    );
    let moving = resolve_profile(&dir, locked.as_bytes(), to, "orchestrator")
        .expect("a real destination resolution");
    let entry = crate::digest::SessionEntry::new("check", crate::digest::Status::Running);
    let server = ServerId::from_typed_flags("socket", &root.join("unused.sock").to_string_lossy())
        .expect("a private selector");
    let seat = Admitted {
        entry: &entry,
        dir,
        caller: Caller::default(),
        was: "profile 'claude-a'".to_owned(),
        from: "claude-a".to_owned(),
        to: to.to_owned(),
        moving,
        from_command: None,
        target: Target {
            agent: "orchestrator".to_owned(),
            slot: "main".to_owned(),
            pane: "%fixture".to_owned(),
            server,
        },
    };
    let parsed = Parsed {
        session: "check".to_owned(),
        agent: "orchestrator".to_owned(),
        profile: to.to_owned(),
        client: None,
        stop: StopPolicy::Proven,
        route,
        bound: bound.then(|| Binding {
            uuid: UUID.to_owned(),
            slot: "main".to_owned(),
            agent: "orchestrator".to_owned(),
            launch: ASKED.to_owned(),
        }),
        long_limit: None,
        expect: None,
        ended: std::cell::RefCell::default(),
    };
    let ask = Ask {
        root,
        world: None,
        parsed: &parsed,
        now: Timestamp::from_epoch(1_800_000_000),
    };
    let mut err = Vec::new();
    let verdict = requested_guard(&ask, &seat, locked.as_bytes(), &mut err);
    (verdict, String::from_utf8_lossy(&err).into_owned())
}

fn refused(verdict: &Result<(), Halt>) -> bool {
    matches!(verdict, Err(Halt::End(Ended::Refused(CauseClass::Other))))
}

#[test]
fn a_requested_move_rechecks_its_binding_and_adapter_under_the_lock_and_no_other_move_is_asked() {
    let owned = Root(std::env::temp_dir().join(format!(
        "ae-requested-guard-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )));
    std::fs::create_dir_all(owned.0.join("sessions/check")).expect("a private session dir");
    let root = owned.0.canonicalize().expect("a canonical private root");
    std::fs::write(
        root.join("config"),
        "[profiles]\nclaude-a = \"claude --model opus\"\nclaude-b = \"claude --model sonnet\"\n\
         codex-b = \"codex --model gpt-b\"\n",
    )
    .expect("a private global config");

    let (verdict, err) = guard(
        &root,
        ("claude", ASKED),
        (Route::Requested, true, "claude-b"),
    );
    assert!(
        verdict.is_ok(),
        "the incarnation that asked, one portable adapter: {err}"
    );
    assert_eq!(err, "");

    let (verdict, err) = guard(
        &root,
        ("claude", SINCE),
        (Route::Requested, true, "claude-b"),
    );
    assert!(refused(&verdict), "a relaunch since the request: {err}");
    assert_eq!(
        err,
        format!(
            "Error: stale request: 'orchestrator' restarted since the request (launch {SINCE}) \
             — nothing was stopped or reseated.\n"
        )
    );

    let (verdict, err) = guard(&root, ("codex", ASKED), (Route::Requested, true, "codex-b"));
    assert!(refused(&verdict), "a keep on a nonportable adapter: {err}");
    assert_eq!(
        err,
        "Error: the adapter of 'codex' does not carry a conversation as portable files \
         — nothing was stopped or reseated.\n"
    );

    for route in [Route::Manual, Route::Automatic] {
        let (verdict, err) = guard(&root, ("codex", SINCE), (route, false, "codex-b"));
        assert!(verdict.is_ok(), "{route:?} is not a request: {err}");
        assert_eq!(err, "", "{route:?}");
    }
}
