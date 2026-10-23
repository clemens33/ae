//! Frozen statrow acceptance (#259), selection half: `tool_in` is the pure
//! table-only selection the auto-effort safe point makes (`effort/leg.rs`
//! `tool_of` over one `stat_snapshot`). It names the seat's ONE recorded tool
//! under the pane, and an unreadable row (unknown state or flag, or an empty
//! name) that is the pane root or has non-`Clear` lineage from it is never
//! counted as, and never hides, the tool: it holds (`None`). Unreadable rows
//! outside the pane are ignored.
//!
//! Oracle: the statrow brief (I4) and issue #259, plan r1 IMPORTANT-1 (lead
//! ACK). Compile-RED on the base by the lead's R2 waiver: `tool_in` is new.

/// The measured idle tree: pane shell 90, tool 100 under it.
const IDLE: &str = "  1     0 Ss   /sbin/launchd\n 90     1 Ss   -zsh\n100    90 S+   claude\n\
                    101   100 S+   /bin/sh\n102   101 S+   docker\n";
/// The row measured during #258, with its unrelated parent under launchd.
const UNRELATED: &str = "48297     1 Ss   /bin/zsh\n48654 48297 ?NE  (ae)\n";

/// The seat's one tool under pane `pane`, recorded binary `bin`.
fn tool(raw: &str, pane: u32, bin: &str) -> Option<u32> {
    let table = super::parse_stat_table(raw).expect("fixture rows are structurally whole");
    super::tool_in(&table, pane, bin)
}

/// The safe point's whole answer: the selection, then the background verdict.
fn safe(raw: &str, pane: u32, bin: &str) -> Option<bool> {
    let table = super::parse_stat_table(raw).expect("fixture rows are structurally whole");
    super::tool_in(&table, pane, bin).and_then(|pid| super::background_task(Some(&table), pid))
}

#[test]
fn exactly_one_recorded_binary_under_the_pane_is_the_tool() {
    assert_eq!(
        tool(IDLE, 90, "claude"),
        Some(100),
        "one match under the pane"
    );
    assert_eq!(
        safe(IDLE, 90, "claude"),
        Some(false),
        "and its idle tree is clear"
    );
    let path = IDLE.replace("   claude\n", "   /opt/homebrew/bin/claude\n");
    assert_eq!(
        tool(&path, 90, "claude"),
        Some(100),
        "a full path names the tool"
    );
    let root = "  1     0 Ss   /sbin/launchd\n 90     1 S+   claude\n";
    assert_eq!(
        tool(root, 90, "claude"),
        Some(90),
        "the pane root itself may be the tool"
    );
    assert_eq!(tool(IDLE, 90, "codex"), None, "no match is no tool");
    let two = IDLE.to_owned() + "110    90 S+   claude\n";
    assert_eq!(
        tool(&two, 90, "claude"),
        None,
        "two matches is no single tool"
    );
    let elsewhere = IDLE.to_owned() + "300     1 S+   claude\n";
    assert_eq!(
        tool(&elsewhere, 90, "claude"),
        Some(100),
        "a claude outside the pane is not this seat's"
    );
}

#[test]
fn the_measured_row_outside_the_pane_does_not_hold_the_selection() {
    let raw = format!("{IDLE}{UNRELATED}");
    assert_eq!(
        tool(&raw, 90, "claude"),
        Some(100),
        "#259: unrelated `?NE (ae)` row"
    );
    assert_eq!(
        safe(&raw, 90, "claude"),
        Some(false),
        "#259: the safe point is clear"
    );
    let more = format!("{IDLE}{UNRELATED}48655 48297 S+\n48656 48297 Sq   helper\n");
    assert_eq!(
        tool(&more, 90, "claude"),
        Some(100),
        "empty names and bad flags outside the pane"
    );
}

#[test]
fn an_unreadable_name_under_the_pane_never_counts_as_or_hides_the_tool() {
    for (row, why) in [
        ("200    90 S+\n", "an unnamed sibling of the tool"),
        ("200    90 Ss   \n", "an unnamed sibling, trailing blanks"),
        ("103   100 S\n", "an unnamed child of the tool"),
        ("103   102 S+\n", "an unnamed row deep under the tool"),
    ] {
        let raw = IDLE.to_owned() + row;
        assert_eq!(tool(&raw, 90, "claude"), None, "I4: {why} holds");
        assert_eq!(safe(&raw, 90, "claude"), None, "I4 safe point: {why}");
    }
}

#[test]
fn an_unreadable_state_under_the_pane_holds_even_with_a_readable_name() {
    for (row, why) in [
        (
            "200    90 ?NE  (claude)\n",
            "a second claude caught mid-exec beside the tool",
        ),
        ("200    90 ?NE  (zsh)\n", "a `?` sibling of the tool"),
        ("200    90 Sq   helper\n", "an unknown flag beside the tool"),
        ("103   100 ?NE  (ae)\n", "a `?` child of the tool"),
    ] {
        let raw = IDLE.to_owned() + row;
        assert_eq!(tool(&raw, 90, "claude"), None, "IMPORTANT-1: {why} holds");
        assert_eq!(
            safe(&raw, 90, "claude"),
            None,
            "IMPORTANT-1 safe point: {why}"
        );
    }
    let own = IDLE.replace("100    90 S+   claude\n", "100    90 ?NE  claude\n");
    assert_ne!(own, IDLE, "fixture: the tool row was replaced");
    assert_eq!(
        safe(&own, 90, "claude"),
        None,
        "I1+I4: the tool row's own `?` state holds"
    );
}

#[test]
fn an_unreadable_pane_root_holds() {
    for (root, why) in [
        (" 90     1 ?NE  (zsh)\n", "a `?` pane root"),
        (" 90     1 Ss\n", "an unnamed pane root"),
        (" 90     1 Sq   -zsh\n", "an unknown flag on the pane root"),
    ] {
        let raw = IDLE.replace(" 90     1 Ss   -zsh\n", root);
        assert_ne!(raw, IDLE, "fixture: the pane root was replaced for {why}");
        assert_eq!(tool(&raw, 90, "claude"), None, "I4: {why}");
        assert_eq!(safe(&raw, 90, "claude"), None, "I4 safe point: {why}");
    }
}

#[test]
fn an_unreadable_row_that_cannot_be_placed_holds_the_selection() {
    let orphan = IDLE.to_owned() + "48654   777 ?NE  (ae)\n";
    assert_eq!(
        tool(&orphan, 90, "claude"),
        None,
        "I4: parent 777 missing is Broken, not Clear"
    );
    let cycle = IDLE.to_owned() + "500   501 ?    x\n501   500 S    y\n";
    assert_eq!(
        tool(&cycle, 90, "claude"),
        None,
        "I4: a ppid cycle is Broken, not Clear"
    );
}
