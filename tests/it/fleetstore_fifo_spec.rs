//! The fleet journal's bounded append observes the events node before it opens
//! it: a FIFO there is refused, never waited on.
use ae::store;
use std::sync::mpsc;
use std::time::Duration;

#[test]
fn a_bounded_append_refuses_a_fifo_events_node_without_blocking() {
    let dir = super::cli::OwnedScratch::root("fleetstore", "fifo").keep();
    crate::cli::mkfifo(&dir.join(store::EVENTS));
    let (tell, hear) = mpsc::channel();
    let at = dir.clone();
    std::thread::spawn(move || {
        let refused = store::open(&at)
            .append_event_bounded("{\"ref\":\"x\"}\n", 5)
            .is_err();
        let _ = tell.send(refused);
    });
    assert_eq!(
        hear.recv_timeout(Duration::from_secs(20)),
        Ok(true),
        "a FIFO events node must be refused, not opened"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_reader_refuses_a_fifo_or_a_linked_fleet_journal_without_blocking() {
    let root = super::cli::OwnedScratch::root("fleetstore", "reader").keep();
    let dir = root.join(ae::fleetfeed::DIR);
    std::fs::create_dir(&dir).unwrap();
    crate::cli::mkfifo(&dir.join(store::EVENTS));
    let (tell, hear) = mpsc::channel();
    let at = root.clone();
    std::thread::spawn(move || {
        let _ = tell.send(ae::fleetfeed::read(&at));
    });
    let read = hear
        .recv_timeout(Duration::from_secs(20))
        .expect("the reader must not open a FIFO");
    assert!(read.unreadable && read.rows.is_empty());
    // A fleet directory that is a link is no directory: nothing is read through it.
    std::fs::remove_dir_all(&dir).unwrap();
    let elsewhere = root.join("elsewhere");
    std::fs::create_dir(&elsewhere).unwrap();
    std::fs::write(
        elsewhere.join(store::EVENTS),
        "{\"ts\":\"2026-11-21T07:00:00Z\",\"actor\":\"human\",\"action\":\"fleet-ended\",\"target\":\"x\",\"ref\":\"11111111-1111-4111-8111-111111111111\"}\n",
    )
    .unwrap();
    std::os::unix::fs::symlink(&elsewhere, &dir).unwrap();
    let read = ae::fleetfeed::read(&root);
    assert!(read.unreadable && read.rows.is_empty(), "{read:?}");
    let _ = std::fs::remove_dir_all(&root);
}
