//! The blocked scopes `ae quota` and `ae list` print (quarantine2, lead
//! ruling 4) include a block on an approved `profile@client` pair's scope,
//! whose client no profile names by default.

fn scratch() -> std::path::PathBuf {
    let root =
        std::env::temp_dir().join(format!("ae-quota-blocked-display-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("sessions")).expect("sessions");
    root
}

#[test]
fn a_block_on_an_approved_pair_scope_is_listed() {
    let root = scratch();
    let global = root.join("config");
    std::fs::write(
        &global,
        "[clients]\nopencode = opencode\nocwork = opencode\n\n\
         [profiles]\ncop = opencode --model github-copilot/gpt-6.1-sol\n\n\
         [client_pairs]\ncop = ocwork\n",
    )
    .expect("config");
    let record = crate::vendorblock::parse(
        b"version=1\ntool=opencode\nclient=ocwork\nsince=1788858600\nkind=account\n\
          seat=w\nreason=You have exceeded your monthly quota\nprovider=github-copilot\n",
    )
    .expect("record");
    assert_eq!(
        crate::vendorblock::record_first(&root, &record).ok(),
        Some(true)
    );
    let sessions = root.join("sessions");
    let inputs = super::Inputs {
        home: None,
        global: Some(&global),
        local: None,
        sessions: Some(&sessions),
        now: record.since,
    };
    let shown = super::blocked_scopes(&inputs);
    assert!(
        shown.contains("ae quota clear provider-github-copilot"),
        "{shown:?}"
    );
    let _ = std::fs::remove_dir_all(&root);
}
