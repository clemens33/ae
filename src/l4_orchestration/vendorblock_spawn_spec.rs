//! FROZEN SPEC (vendorblock S2 spawn refusal).
//!
//! A blocked client refuses `_spawn --using` before the session meta is read,
//! so a missing meta never reaches tmux. The record is
//! `<state-root>/vendorblock/<client>.rec`. `<client>` is the `[clients]`
//! label, not the binary: `copilot` and `free` are both `opencode`.
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
copilot = opencode
free = opencode
codex = codex

[profiles]
emberm = muse --model muse-ember-1.0
cpsol61h = copilot --model github-copilot/gpt-6.1-sol
flash = free --model deepseek
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
client=copilot
since={SINCE}
kind=account
seat=trustfix-nav
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
    scratch.record("copilot", &copilot_record());
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
    assert!(err.contains("ae quota clear copilot"), "{err}");
    // The `@client` suffix is the scope, not the profile's own client.
    let (rc, err) = spawn(&scratch.root, "flash@copilot");
    assert_ne!(rc, 0, "{err}");
    assert!(err.contains("ae quota clear copilot"), "{err}");
}

#[test]
fn an_unblocked_client_of_the_same_binary_is_not_refused_as_blocked() {
    let scratch = Scratch::new("open");
    scratch.record("copilot", &copilot_record());
    for using in ["flash", "cpsol61h@free", "solx"] {
        let (rc, err) = spawn(&scratch.root, using);
        assert_ne!(rc, 0, "{using}: a missing session still refuses");
        assert!(
            !err.contains("blocked ("),
            "{using} is not the blocked client: {err}"
        );
    }
}
