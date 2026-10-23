//! Frozen statrow acceptance (#259), verdict half: an unreadable `ps` row
//! (`?` or another unknown state, an unknown flag, an empty name) poisons the
//! background-task verdict only when it is the tool's own row or its lineage
//! from the tool is not `Clear`; anything else is ignored and the verdict comes
//! from the remaining rows. Structural damage still yields no table.
//!
//! Oracle: the statrow brief (I1-I3) and issue #259, never the code. Every
//! case runs through the two existing functions, so this file compiles on the
//! base and its RED there is behavioural.

/// The measured idle tree (`.local/effort/idle-trees.md`): pane shell 90,
/// tool 100, an MCP wrapper `/bin/sh` (`S+`) running docker beneath it.
const IDLE: &str = "  1     0 Ss   /sbin/launchd\n 90     1 Ss   -zsh\n100    90 S+   claude\n\
                    101   100 S+   /bin/sh\n102   101 S+   docker\n";
/// A command the tool runs: a session-leading shell under it.
const BUSY_TAIL: &str = "103   100 Ss   /bin/zsh\n104   103 S    sleep\n";
/// The unrelated parent the measured row hangs from, placed under launchd.
const PARENT: &str = "48297     1 Ss   /bin/zsh\n";
/// The row measured during #258: a short-lived `ae` caught mid-exec/exit.
const MEASURED: &str = "48654 48297 ?NE  (ae)\n";

/// The verdict the auto-effort safe point reads for tool `tool`.
fn verdict(raw: &str, tool: u32) -> Option<bool> {
    super::background_task(super::parse_stat_table(raw).as_deref(), tool)
}

#[test]
fn the_measured_row_unrelated_to_the_seat_leaves_the_verdict_alone() {
    assert_eq!(
        verdict(IDLE, 100),
        Some(false),
        "control: the idle tree is clear"
    );
    let idle = format!("{IDLE}{PARENT}{MEASURED}");
    assert_eq!(
        verdict(&idle, 100),
        Some(false),
        "#259: an unrelated `?NE (ae)` row must not blank an idle tree"
    );
    let busy = format!("{IDLE}{BUSY_TAIL}{PARENT}{MEASURED}");
    assert_eq!(
        verdict(&busy, 100),
        Some(true),
        "#259: an unrelated `?NE (ae)` row must not hide a background task either"
    );
    let first = format!("{PARENT}{MEASURED}{IDLE}");
    assert_eq!(
        verdict(&first, 100),
        Some(false),
        "row order is not a placement: the unrelated row may come first"
    );
}

#[test]
fn the_measured_row_under_the_tool_is_unknown_never_clear() {
    let child = IDLE.to_owned() + "48654   100 ?NE  (ae)\n";
    assert_eq!(verdict(&child, 100), None, "I1: a `?` child of the tool");
    let deep = IDLE.to_owned() + "48654   102 ?NE  (ae)\n";
    assert_eq!(
        verdict(&deep, 100),
        None,
        "I1: a `?` row three levels under the tool"
    );
    let placed = format!("{IDLE}48297   100 S    /usr/bin/env\n{MEASURED}");
    assert_eq!(
        verdict(&placed, 100),
        None,
        "I1: the measured row itself, its parent placed under the tool"
    );
    let busy = format!("{IDLE}{BUSY_TAIL}48654   104 ?NE  (ae)\n");
    assert_eq!(
        verdict(&busy, 100),
        None,
        "I1: unknown under the tool answers unknown exactly as today, busy tree or not"
    );
}

#[test]
fn the_tool_row_itself_unreadable_is_unknown_never_clear() {
    for (tool_row, why) in [
        ("100    90 ?NE  claude\n", "a `?` state on the tool"),
        ("100    90 ?    claude\n", "a bare `?` state on the tool"),
        ("100    90 Sq   claude\n", "an unknown flag on the tool"),
        (
            "100    90 Q+   claude\n",
            "an unknown state letter on the tool",
        ),
        ("100    90 S+\n", "an empty name on the tool"),
        (
            "100    90 S+   \n",
            "an empty name with trailing blanks on the tool",
        ),
    ] {
        let raw = IDLE.replace("100    90 S+   claude\n", tool_row);
        assert_ne!(raw, IDLE, "fixture: the tool row was replaced for {why}");
        assert_eq!(verdict(&raw, 100), None, "I1: {why}");
    }
}

#[test]
fn every_kind_of_unreadable_row_unrelated_to_the_seat_is_ignored() {
    for (row, why) in [
        ("48654 48297 ?NE  (ae)\n", "measured `?NE`"),
        ("48654 48297 ?    (ae)\n", "bare `?`"),
        ("48654 48297 ??   (ae)\n", "doubled `?`"),
        ("48654 48297 Sq   helper\n", "unknown flag"),
        ("48654 48297 Q    helper\n", "unknown state letter"),
        ("48654 48297 S+\n", "empty name"),
        ("48654 48297 Ss   \n", "empty name, trailing blanks"),
        ("48654 48297 ?NE\n", "unknown state and empty name together"),
        ("48654     0 ?NE  (ae)\n", "ppid 0"),
        ("48654     1 ?NE  (ae)\n", "ppid 1"),
    ] {
        let idle = format!("{IDLE}{PARENT}{row}");
        assert_eq!(verdict(&idle, 100), Some(false), "I2 idle: {why}");
        let busy = format!("{IDLE}{BUSY_TAIL}{PARENT}{row}");
        assert_eq!(verdict(&busy, 100), Some(true), "I2 busy: {why}");
    }
}

#[test]
fn an_ignored_row_still_places_its_children_and_its_ancestors_stay_clear() {
    let leader = format!("{IDLE}600     1 ?NE  (ae)\n601   600 Ss   /bin/zsh\n");
    assert_eq!(
        verdict(&leader, 100),
        Some(false),
        "I2: a session-leading shell under an unrelated `?` row is placed through it, \
         Clear, not dropped into a broken lineage"
    );
    let ancestor = "  1     0 Ss   /sbin/launchd\n 90     1 ?NE  (zsh)\n100    90 S+   claude\n\
                    101   100 S+   /bin/sh\n";
    assert_eq!(
        verdict(ancestor, 100),
        Some(false),
        "I2: an unreadable ANCESTOR of the tool has Clear lineage from it and is ignored here"
    );
}

#[test]
fn an_unreadable_row_ae_cannot_place_is_unknown() {
    let orphan = IDLE.to_owned() + "48654   777 ?NE  (ae)\n";
    assert_eq!(
        verdict(&orphan, 100),
        None,
        "I1: parent 777 is missing: Broken, not Clear"
    );
    let cycle = IDLE.to_owned() + "500   501 ?    x\n501   500 S    y\n";
    assert_eq!(
        verdict(&cycle, 100),
        None,
        "I1: a ppid cycle is Broken, not Clear"
    );
    let named = IDLE.to_owned() + "48654   777 S+\n";
    assert_eq!(
        verdict(&named, 100),
        None,
        "I1: an empty name ae cannot place"
    );
}

#[test]
fn many_unrelated_unreadable_rows_on_a_large_host_never_blank_the_verdict() {
    let mut raw = format!("{IDLE}{PARENT}");
    for pid in 50_000_u32..51_000 {
        let row = match pid % 4 {
            0 => format!("{pid} 48297 ?NE  (ae)\n"),
            1 => format!("{pid} 48297 S+\n"),
            2 => format!("{pid} 48297 Sq   helper\n"),
            _ => format!("{pid} 48297 S    helper\n"),
        };
        raw.push_str(&row);
    }
    assert_eq!(verdict(&raw, 100), Some(false), "I2 at host scale");
}

#[test]
fn structural_damage_is_still_no_table() {
    for (row, why) in [
        ("x     1 S    helper\n", "non-numeric pid"),
        ("700   y S    helper\n", "non-numeric ppid"),
        ("-7    1 S    helper\n", "negative pid"),
        ("700  -1 S    helper\n", "negative ppid"),
        ("4294967296 1 S helper\n", "pid past u32"),
        ("700\n", "a row with only a pid"),
        ("700   1\n", "a row with no state field"),
        ("?NE  (ae)\n", "a row with no pid at all"),
    ] {
        let raw = format!("{IDLE}{row}");
        assert!(
            super::parse_stat_table(&raw).is_none(),
            "I3: {why} cannot be placed in the tree, so it could be under the tool"
        );
        assert_eq!(verdict(&raw, 100), None, "I3 verdict: {why}");
    }
    assert_eq!(
        super::parse_stat_table(""),
        Some(Vec::new()),
        "an empty read is an empty table"
    );
}

#[test]
fn the_readable_state_grammar_is_unchanged() {
    for stat in [
        "Ss", "S+", "Ss+", "R", "R+", "Ssl", "S<l", "SN", "I<", "I", "D", "D+", "U", "P", "W", "X",
        "Z", "T", "t", "SWE", "S<s", "SLl", "R<NA", "SH", "SV", "S>", "SX",
    ] {
        let child = format!("{IDLE}105   102 {stat:<4} helper\n");
        assert_eq!(
            verdict(&child, 100),
            Some(false),
            "a readable `{stat}` under the tool, no session-leading shell, stays clear"
        );
    }
    let leader = format!("{IDLE}105   102 Ss   /bin/bash\n");
    assert_eq!(
        verdict(&leader, 100),
        Some(true),
        "a session leader under the tool stays busy"
    );
    let unnamed = format!("{IDLE}105   102 Ss\n");
    assert_eq!(
        verdict(&unnamed, 100),
        None,
        "I1: a session-leading row under the tool with no name is unknown, never clear"
    );
}

#[test]
fn a_table_without_the_tool_is_still_unknown() {
    let raw = format!("{IDLE}{PARENT}{MEASURED}");
    assert_eq!(
        verdict(&raw, 7),
        None,
        "no tool row is unknown, with or without `?` rows"
    );
    assert_eq!(verdict("", 100), None, "an empty table has no tool");
}
