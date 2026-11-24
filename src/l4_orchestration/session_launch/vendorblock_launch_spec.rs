//! `--lead`, `--colead` and `--seat` refuse a blocked client scope before any
//! server effect, by the same rule `spawn --using` applies: an `OpenCode` block
//! of one upstream provider refuses only the profiles whose model id names it.

#![allow(
    clippy::disallowed_methods,
    clippy::unwrap_used,
    reason = "the fixture writes a scratch state root; product code stays on the store doors"
)]

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use super::{OverrideCtx, SeatOverrideRefusal, resolve_one_override};

const SINCE: i64 = 1_788_858_600;

const CONFIG: &str = "\
[clients]
opencode = opencode
muse = muse

[profiles]
cpsol61h = opencode --model github-copilot/gpt-6.1-sol
ds41flashgom = opencode --model opencode-go/deepseek-v4.1-flash
emberm = muse --model muse-ember-1.0

[roster]
lead = ds41flashgom

[workspace]
main = lead
";

fn scratch(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "ae-vendorblock-launch-{tag}-{}-{nanos}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn block(root: &Path, tool: &str, client: &str, provider: Option<&str>) {
    let record = crate::vendorblock::Record {
        tool: tool.to_owned(),
        client: client.to_owned(),
        since: SINCE,
        kind: crate::vendorblock::Kind::Account,
        seat: "trustfix-nav".to_owned(),
        reason: "You have exceeded your monthly quota".to_owned(),
        provider: provider.map(str::to_owned),
    };
    assert_eq!(
        crate::vendorblock::record_first(root, &record).ok(),
        Some(true)
    );
}

fn resolve(root: &Path, agent: &str, value: &str) -> Result<(), SeatOverrideRefusal> {
    let cfg = crate::config::parse_identity(CONFIG).unwrap();
    let agents = vec!["lead".to_owned(), "colead".to_owned()];
    let ctx = OverrideCtx {
        cfg: &cfg,
        agents: &agents,
        known_agents: "lead, colead",
        known_profiles: "cpsol61h, ds41flashgom, emberm",
        known_clients: "opencode, muse",
        recorded: None,
        session: "s",
        home: Some(Path::new("/Users/a")),
        state_root: root,
    };
    resolve_one_override(&ctx, agent, value).map(|_| ())
}

fn refused(result: Result<(), SeatOverrideRefusal>) -> String {
    match result {
        Err(SeatOverrideRefusal::Failed(line)) => line,
        other => panic!("a blocked scope must refuse as Failed, got {other:?}"),
    }
}

#[test]
fn a_launch_override_on_a_blocked_client_refuses_with_the_clear_command() {
    let root = scratch("whole");
    block(&root, "muse", "muse", None);
    let line = refused(resolve(&root, "lead", "emberm"));
    assert!(
        line.starts_with("Error: scope muse blocked (account:"),
        "{line}"
    );
    assert!(line.contains("ae quota clear muse"), "{line}");
    // `--colead` and `--seat` ride the same path, and a free client spawns.
    assert!(resolve(&root, "colead", "emberm").is_err());
    assert!(resolve(&root, "lead", "ds41flashgom").is_ok());
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_provider_block_refuses_a_launch_override_of_that_provider_only() {
    let root = scratch("provider");
    block(&root, "opencode", "opencode", Some("github-copilot"));
    let line = refused(resolve(&root, "lead", "cpsol61h"));
    assert!(
        line.starts_with("Error: scope github-copilot blocked (account:"),
        "{line}"
    );
    assert!(line.contains("ae quota clear github-copilot"), "{line}");
    assert!(
        resolve(&root, "lead", "ds41flashgom").is_ok(),
        "another provider launches"
    );
    assert!(
        resolve(&root, "lead", "ds41flashgom@opencode").is_ok(),
        "the suffix keeps the profile's own provider"
    );
    assert!(resolve(&root, "lead", "cpsol61h@opencode").is_err());
    let _ = std::fs::remove_dir_all(&root);
}
