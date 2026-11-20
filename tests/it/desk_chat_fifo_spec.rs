//! A named pipe at the board record path cannot stall the console.

use std::fs;

use super::super::cli::mkfifo;
use super::Rig;

#[test]
fn board_command_refuses_a_fifo_without_waiting_for_a_writer_and_recovers() {
    let rig = Rig::new("deskfifo", 120, 32);
    let path = rig.tool.dir.join("desk-record");
    mkfifo(&path);

    rig.paste("/board");
    rig.key("Enter");
    rig.wait("a named pipe is refused without a writer", |screen| {
        screen.contains("board unreadable: not a regular file")
    });
    fs::remove_file(&path).expect("remove named pipe");
    fs::write(
        &path,
        "ae-desk 1\nupdated 2026-11-20T08:40:12Z\nD1\tA readable board after the FIFO\n",
    )
    .expect("replace the pipe with a valid record");

    rig.paste("/board");
    rig.key("Enter");
    rig.wait("console reads the replacement", |screen| {
        screen.contains("A readable board after the FIFO")
    });
}
