#![no_main]

use ae::effort::Level;
use ae::launch_cmd::{effort_flag, effort_skeleton, replace_effort_flag};
use ae::tool::ToolKind;
use libfuzzer_sys::fuzz_target;

// The reasoning-effort grammar over a profile command: the first line names
// the harness binary, the rest is the command. A command that proves a level
// must rewrite to `high`, read `high` back, and keep its skeleton: the rewrite
// changes the effort value and nothing else.
fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let (binary, command) = text.split_once('\n').unwrap_or(("", &text));
    let tool = ToolKind::from_binary_name(binary);
    let read = std::hint::black_box(effort_flag(command, tool));
    let skeleton = std::hint::black_box(effort_skeleton(command, tool));
    assert_eq!(
        read.is_ok(),
        skeleton.is_ok(),
        "the reader and the skeleton agree"
    );
    if let Ok((words, _)) = skeleton {
        let rewritten = replace_effort_flag(command, tool, Level::High);
        let Ok(rewritten) = rewritten else {
            panic!("a command that proves a level must rewrite: {rewritten:?}");
        };
        assert_eq!(effort_flag(&rewritten, tool), Ok(Level::High));
        assert_eq!(effort_skeleton(&rewritten, tool), Ok((words, Level::High)));
    }
});
