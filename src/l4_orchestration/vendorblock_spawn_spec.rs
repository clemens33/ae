//! FROZEN SPEC (vendorblock S2 spawn refusal).
//!
//! A blocked scope refuses `_spawn --using` before the session meta is read,
//! so a missing meta never reaches tmux. The record is
//! `<state-root>/vendorblock/<scope>.rec`. An opencode scope is the model
//! id's provider, the text before the first `/` in `--model` or `-m`. A model
//! id with no `/` uses the whole client, and the refusal names that client.
//! No config grammar was added: the model id is the existing flag value.
//!
//! `--lead`, `--colead` and `--seat` must call this same refusal before any
//! server effect. Launch has no pure seam that fails before tmux, so this
//! file does not drive it.

#![allow(
    clippy::disallowed_methods,
    clippy::unwrap_used,
    reason = "the fixture writes a scratch state root; product code stays on the store doors"
)]

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::time::Timestamp;

const SINCE: i64 = 1_788_858_600;
const SINCE_ISO: &str = "2026-09-08T09:10:00Z";

struct Scratch {
    root: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "ae-vendorblock-{tag}-{}-{nanos}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("vendorblock")).unwrap();
        std::fs::write(root.join("config"), CONFIG).unwrap();
        Self { root }
    }

    fn record(&self, client: &str, body: &str) {
        std::fs::write(
            self.root.join("vendorblock").join(format!("{client}.rec")),
            body,
        )
        .unwrap();
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

const CONFIG: &str = "\
[clients]
muse = muse
opencode = opencode
codex = codex

[profiles]
emberm = muse --model muse-ember-1.0
cpsol61h = opencode --model github-copilot/gpt-6.1-sol
short = opencode -m github-copilot/claude-x
flashgo = opencode --model opencode-go/deepseek-v4-flash
zen = opencode --model opencode/big-pickle
bare = opencode --model glm-5
solx = codex --model sol
";

fn muse_record() -> String {
    format!(
        "\
version=1
tool=muse
client=muse
since={SINCE}
kind=billing
seat=testpins-rev
reason=Billing verification failed
"
    )
}

fn copilot_record() -> String {
    format!(
        "\
version=1
tool=opencode
client=opencode
provider=github-copilot
since={SINCE}
kind=account
seat=trustfix-nav
reason=You have exceeded your monthly quota
"
    )
}

fn client_record() -> String {
    format!(
        "\
version=1
tool=opencode
client=opencode
since={SINCE}
kind=account
seat=bare-seat
reason=You have exceeded your monthly quota
"
    )
}

fn spawn(root: &Path, using: &str) -> (u8, String) {
    let session = root.join("session");
    std::fs::create_dir_all(&session).unwrap();
    let tail = ["scout", "--using", using, "look"]
        .map(str::to_owned)
        .to_vec();
    let mut out = Vec::new();
    let mut err = Vec::new();
    let rc = super::run_spawn_inner(
        &session,
        &tail,
        "",
        Timestamp::from_epoch(SINCE),
        &mut out,
        &mut err,
        None,
        Some(root),
        root,
    )
    .unwrap();
    assert!(
        !session.join("meta").exists(),
        "{using} wrote a seat before the refusal: rc {rc}"
    );
    (rc, String::from_utf8(err).unwrap())
}

#[test]
fn a_blocked_client_refuses_spawn_before_the_session_exists() {
    let scratch = Scratch::new("block");
    scratch.record("muse", &muse_record());
    scratch.record("github-copilot", &copilot_record());
    let (rc, err) = spawn(&scratch.root, "emberm");
    assert_ne!(rc, 0, "{err}");
    assert!(
        err.contains(&format!(
            "blocked (billing: Billing verification failed) since {SINCE_ISO}"
        )),
        "{err}"
    );
    assert!(err.contains("ae quota clear muse"), "{err}");
    let (rc, err) = spawn(&scratch.root, "cpsol61h");
    assert_ne!(rc, 0, "{err}");
    assert!(
        err.contains(&format!(
            "blocked (account: You have exceeded your monthly quota) since {SINCE_ISO}"
        )),
        "{err}"
    );
    assert!(err.contains("ae quota clear github-copilot"), "{err}");
    let (rc, err) = spawn(&scratch.root, "short");
    assert_ne!(rc, 0, "{err}");
    assert!(err.contains("ae quota clear github-copilot"), "{err}");
}

/// One opencode client carries every provider. A github-copilot block admits
/// the others. The admitted spawn still stops on the missing session, before
/// a pane exists, so the test does not exec opencode.
#[test]
fn a_copilot_block_does_not_stop_another_opencode_provider() {
    let scratch = Scratch::new("providers");
    scratch.record("github-copilot", &copilot_record());
    for using in ["flashgo", "zen", "bare", "solx"] {
        let (rc, err) = spawn(&scratch.root, using);
        assert_ne!(rc, 0, "{using}: a missing session still refuses");
        assert!(
            !err.contains("blocked ("),
            "{using} is not the github-copilot provider: {err}"
        );
        assert!(
            err.contains("cannot read the meta") || err.contains("session not found"),
            "{using} was not admitted to the normal spawn path: {err}"
        );
    }
    let fallback = Scratch::new("bare");
    fallback.record("opencode", &client_record());
    let (rc, err) = spawn(&fallback.root, "bare");
    assert_ne!(rc, 0, "{err}");
    assert!(
        err.contains(&format!(
            "blocked (account: You have exceeded your monthly quota) since {SINCE_ISO}"
        )),
        "{err}"
    );
    assert!(err.contains("ae quota clear opencode"), "{err}");
    let (rc, err) = spawn(&fallback.root, "cpsol61h");
    assert!(
        !err.contains("blocked ("),
        "a provider profile is not the client fallback: {err}"
    );
    let _ = rc;
}

const MUSE_402: &str = include_str!("../../tests/fixtures/vendorblock/muse-402.reconstructed.txt");
const MUSE_HEALTHY: &str = include_str!("../../tests/fixtures/vendorblock/muse-healthy-hi.raw.txt");

/// D5: booking the block writes the record. A later frame with the row gone
/// releases the usage-limit latch and does not delete the record. Spawn
/// still refuses.
#[test]
fn a_released_latch_does_not_clear_the_block_record() {
    let scratch = Scratch::new("latch");
    let meta_dir = scratch.root.join("sessions/latch");
    let record = scratch.root.join("vendorblock/muse.rec");
    let (seen, first) = sight(&crate::watchdog_daemon::PaneState::default(), MUSE_402);
    assert!(
        format!("{:?}", first.effects).contains("BlockScope"),
        "first sight books the block: {:?}",
        first.effects
    );
    assert_eq!(first.next.limit_since, Some(10_000), "the latch is held");
    let mut state = first.next.clone();
    crate::watchdog_daemon::apply_booked(&meta_dir, &first.effects, &seen, &mut state);
    let kept = std::fs::read(&record).expect("first sight writes vendorblock/muse.rec");
    let (healthy, released) = sight(&first.next, MUSE_HEALTHY);
    assert_eq!(released.next.limit_since, None, "the latch released");
    let mut state = released.next.clone();
    crate::watchdog_daemon::apply_booked(&meta_dir, &released.effects, &healthy, &mut state);
    assert_eq!(
        std::fs::read(&record).expect("the record survives the latch release"),
        kept
    );
    let (rc, err) = spawn(&scratch.root, "emberm");
    assert_ne!(rc, 0, "{err}");
    assert!(err.contains("ae quota clear muse"), "{err}");
    assert_eq!(
        std::fs::read(&record).expect("spawn leaves the record"),
        kept
    );
}

fn sight(
    prior: &crate::watchdog_daemon::PaneState,
    frame: &str,
) -> (
    crate::watchdog_daemon::Observation,
    crate::watchdog_daemon::Accounting,
) {
    let mut seen = latch_obs();
    seen.throttle = crate::watchdog::throttle_class(frame, "muse");
    seen.limit_notice = crate::watchdog::limit_notice(frame, "muse");
    let booked =
        crate::watchdog_daemon::account(prior, &seen, &crate::watchdog_daemon::Knobs::default());
    (seen, booked)
}

fn latch_obs() -> crate::watchdog_daemon::Observation {
    crate::watchdog_daemon::Observation {
        now_epoch: 10_000,
        hash: 7,
        harness: crate::watchdog_daemon::HarnessObservation {
            frame: crate::harness_state::HarnessState::Unknown,
            human_draft: false,
            durable_stale: false,
            declaration: None,
        },
        identity: 1,
        is_dead: false,
        throttle: None,
        limit_notice: None,
        limit_reset: None,
        limit_woke_journal: None,
        auto_continue: false,
        capture_ok: true,
        human_prompt: None,
        throttle_quota: None,
        quiet: None,
        ended_wait: None,
        done_progress: crate::watchdog::DoneProgress::None,
        wait_progress: crate::watchdog::WaitProgress::None,
        wait_rung: 0,
        wait_ends_elsewhere: false,
        target_moved: None,
        descendancy: crate::procs::Descendancy::Present,
        launch_attempt: crate::tmux::Evidence::Silent,
        outstanding_alert: None,
        unreachable: false,
        failed_elsewhere: false,
        prompt_named: false,
        start: None,
        last_actor_event_age_secs: 0,
        declared_age_secs: 0,
        suspended: crate::watchdog::Suspensions::default(),
        sweep: None,
        own_work: crate::session::OwnWork::default(),
        auto_in_flight: None,
        auto_deadline: None,
    }
}
