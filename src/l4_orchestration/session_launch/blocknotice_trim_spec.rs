//! Resume retention keeps a vendor block notice receipt while its block
//! stands (quarantine2, lead ruling 3): the cut that a stop and a resume run
//! must not make the next watchdog tell the lead again.

use std::fmt::Write as _;

const RECORD: &str = "version=1\ntool=muse\nclient=muse\nsince=1788858600\nkind=billing\nseat=ember\nreason=Billing verification failed\n";

fn receipt(identity: &str) -> String {
    format!(
        r#"{{"ts":"2026-09-08T09:10:00Z","actor":"watchdog","action":"{}","ref":"{identity}#main"}}"#,
        crate::vendorblock::NOTICE_ACTION
    )
}

/// A fleet root with one session and one standing client-muse block.
fn fleet(tag: &str) -> (std::path::PathBuf, std::path::PathBuf, String) {
    let root = std::path::PathBuf::from(format!(
        "/tmp/ae-blocknotice-trim-{}-{tag}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let dir = root.join("sessions").join("s");
    std::fs::create_dir_all(&dir).expect("session dir");
    std::fs::create_dir_all(root.join("vendorblock")).expect("block dir");
    std::fs::write(root.join("vendorblock/client-muse.rec"), RECORD).expect("record");
    let record = crate::vendorblock::parse(RECORD.as_bytes()).expect("parses");
    (root, dir, record.identity())
}

fn seed(dir: &std::path::Path, first: &str) {
    let mut text = format!("{first}\n");
    for index in 1..=super::EVENTS_KEEP + 5 {
        let _ = writeln!(text, "{{\"seq\":{index}}}");
    }
    std::fs::write(dir.join("events.jsonl"), text).expect("journal");
}

fn journal(dir: &std::path::Path) -> String {
    #[allow(clippy::disallowed_methods, reason = "test reads the journal it trimmed")]
    let text = std::fs::read_to_string(dir.join("events.jsonl")).expect("journal");
    text
}

#[test]
fn a_resume_trim_keeps_the_receipt_of_a_standing_block() {
    let (root, dir, identity) = fleet("stands");
    let line = receipt(&identity);
    seed(&dir, &line);

    super::trim_events(&dir);

    let kept = journal(&dir);
    assert!(kept.starts_with(&format!("{line}\n")), "{}", &kept[..200]);
    assert_eq!(kept.lines().count(), super::EVENTS_KEEP + 1);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_resume_trim_drops_the_receipt_of_a_cleared_or_replaced_block() {
    for (tag, stale) in [
        ("cleared", None),
        ("replaced", Some(RECORD.replace("since=1788858600", "since=1788858700"))),
    ] {
        let (root, dir, identity) = fleet(tag);
        match stale {
            None => std::fs::remove_file(root.join("vendorblock/client-muse.rec")).expect("clear"),
            Some(body) => {
                std::fs::write(root.join("vendorblock/client-muse.rec"), body).expect("reblock");
            }
        }
        seed(&dir, &receipt(&identity));

        super::trim_events(&dir);

        let kept = journal(&dir);
        assert!(!kept.contains(crate::vendorblock::NOTICE_ACTION), "{tag}");
        assert_eq!(kept.lines().count(), super::EVENTS_KEEP, "{tag}");
        let _ = std::fs::remove_dir_all(root);
    }
}
