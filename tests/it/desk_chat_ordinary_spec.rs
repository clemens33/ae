//! A regular session can ask for its own record through the console.

use std::fs;

use super::Rig;

#[test]
fn board_command_is_available_in_an_ordinary_session() {
    let rig = Rig::new("deskordinary", 120, 32);
    let journal = rig.tool.events();
    fs::write(
        rig.tool.dir.join("desk-record"),
        "ae-desk 1\nupdated 2026-11-20T08:40:12Z\nD1\tAn ordinary session board\n",
    )
    .expect("record bound to the ordinary session");
    rig.paste("/board");
    rig.key("Enter");
    rig.wait("ordinary session shows its record", |screen| {
        screen.contains("An ordinary session board")
    });
    assert_eq!(rig.tool.events(), journal);
}
