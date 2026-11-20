//! Run /board through the real console input and read what the pane displays.

use std::fs;

use super::Rig;

#[test]
fn board_command_reads_the_bound_record_at_each_use_without_writing_the_journal() {
    let rig = Rig::new("deskspec", 130, 34);
    let meta = rig.tool.dir.join("meta");
    let overlay = rig.root.join("orchestrator.config");
    fs::write(
        &overlay,
        include_str!("../../contrib/aeorchestrator/orchestrator.config"),
    )
    .expect("real orchestrator overlay");
    let before_meta = fs::read_to_string(&meta).expect("console session meta");
    fs::write(
        &meta,
        format!(
            "{before_meta}meta_agent=true\nlocal_config={}\n",
            overlay.display()
        ),
    )
    .expect("orchestrator role in the bound session");
    let path = rig.tool.dir.join("desk-record");
    let journal = rig.tool.events();
    let command = |line| {
        rig.paste(line);
        rig.key("Enter");
    };

    command("/board");
    rig.wait("absence is explicit", |screen| {
        screen.contains("no board set")
    });

    let first = concat!(
        "ae-desk 1\nupdated 2026-11-20T08:40:12Z\n",
        "D7\tChoose the west region\thttps://example.test/west?q=1&n=2\t/tmp/board/west.md\n",
    );
    fs::write(&path, first).expect("first record");
    command("/board");
    let screen = rig.wait("first board appears", |screen| {
        screen.contains("Choose the west region") && screen.contains("/tmp/board/west.md")
    });
    assert!(screen.contains("08:40"), "{screen}");
    assert!(
        screen.contains("D7") && screen.contains("DECIDE"),
        "{screen}"
    );
    assert!(
        screen.contains("https://example.test/west?q=1&n=2"),
        "{screen}"
    );

    let second = "ae-desk 1\nupdated 2026-11-20T08:41:12Z\nC3\tSchedule the east release\n";
    fs::write(&path, second).expect("replace the whole record");
    command("/board more");
    let screen = rig.wait("extra operand is refused", |screen| {
        screen.contains("/board takes no argument")
    });
    assert!(!screen.contains("Schedule the east release"), "{screen}");
    command("/board");
    let screen = rig.wait("changed board appears", |screen| {
        screen.contains("Schedule the east release")
    });
    assert!(
        screen.contains("08:41") && screen.contains("C3"),
        "{screen}"
    );
    assert_eq!(fs::read_to_string(&path).expect("read-only board"), second);

    let damaged = "ae-desk 1\nupdated yesterday\nL1\tthis malformed item must not show\n";
    fs::write(&path, damaged).expect("damaged input fixture");
    command("/board");
    let screen = rig.wait("damage is explicit", |screen| {
        screen.contains("board unreadable")
    });
    assert!(
        !screen.contains("this malformed item must not show"),
        "{screen}"
    );
    assert_eq!(
        fs::read_to_string(&path).expect("damaged record kept"),
        damaged
    );
    assert_eq!(
        rig.tool.events(),
        journal,
        "board reads never become asks or events"
    );
}
