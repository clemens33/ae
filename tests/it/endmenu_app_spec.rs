//! #248: the app groups the retirement actions and preserves history.

use super::*;

#[test]
fn endmenu_app_groups_stop_and_end_below_the_safe_rows() {
    let rig = Rig::new("endmenu-group");
    rig.plant("alpha", UUID, true);
    let pane = rig.app("alpha", false);
    rig.key(&pane, "m");
    let menu = rig.wait(&pane, "the session menu", |s| s.contains("End…"));
    let row = |label| {
        menu.lines()
            .position(|line| line.contains(label))
            .unwrap_or_else(|| panic!("{label:?} is missing from the menu:\n{menu}"))
    };
    let (activity, memos, stop, end) = (row("Activity"), row("Memos"), row("Stop…"), row("End…"));
    assert!(
        activity < memos && memos + 1 < stop && stop + 1 == end,
        "safe rows and the adjacent Stop and End rows need a visible separator:\n{menu}"
    );
    assert!(!menu.contains("Purge"), "{menu}");
}

#[test]
fn endmenu_app_describes_retirement_and_keeps_history_with_legacy_config() {
    let rig = Rig::new("endmenu-keep");
    rig.plant("alpha", UUID, true);
    let policy = rig.root.join("legacy-config");
    fs::write(&policy, "[workspace]\npurge_agent_history = true\n").unwrap();
    let claude_home = rig.root.join("home/.claude");
    let conversations = claude_home.join("projects/work");
    fs::create_dir_all(&conversations).unwrap();
    let transcript = conversations.join(format!("{OTHER}.jsonl"));
    fs::write(&transcript, "the existing Claude conversation\n").unwrap();
    for (key, value) in [
        ("config", policy.to_str().unwrap()),
        ("agent_bin.main", "claude"),
        ("config_home.main", claude_home.to_str().unwrap()),
        ("harness_session.main", OTHER),
    ] {
        ae::meta::rewrite(&rig.dir("alpha"), key, Some(value)).unwrap();
    }
    let pane = rig.app("alpha", false);
    let question = rig.question(&pane, "alpha");
    let question_words = question.split_whitespace().collect::<Vec<_>>().join(" ");
    assert!(
        question_words.contains("removed from the session list")
            && question_words.contains("history")
            && question_words.contains("archive")
            && !question_words.contains("DELETED"),
        "the question must name the session's retirement and preservation:\n{question}"
    );
    let before = snapshot(&rig.dir("alpha"));
    rig.key(&pane, "Enter");
    rig.wait(&pane, "the default Cancel closes the question", |s| {
        !s.contains("[ End ]")
    });
    assert_eq!(snapshot(&rig.dir("alpha")), before);
    assert!(rig.live("alpha"));
    assert!(!rig.archive().exists());

    rig.question(&pane, "alpha");
    rig.key(&pane, "y");
    rig.wait(&pane, "End finishes and retires the session", |s| {
        s.contains("Ended alpha") && !rig.dir("alpha").exists()
    });
    assert!(!rig.live("alpha"));
    assert_archive(&rig);
    assert_eq!(
        fs::read_to_string(transcript).unwrap(),
        "the existing Claude conversation\n"
    );
}

#[test]
fn endmenu_supervisor_does_not_expand_a_literal_all_to_new_sessions() {
    let rig = Rig::new("endmenu-all");
    rig.plant("all", UUID, true);
    // The old-style handoff produces a supervisor per carried target. Its
    // argv for a literal `all` session carries the ordinary confirmed plan,
    // without the optional app expectation. Take it before the fleet changes.
    let mut confirmed = rig.frozen("all", true);
    confirmed.retain(|word| !word.starts_with("--expect-"));
    rig.plant("late", OTHER, true);
    let late = snapshot(&rig.dir("late"));

    let (code, out, err) = rig.run_end(&confirmed, &[]);
    assert!(
        rig.live("late"),
        "an unconfirmed session stopped: {out}\n{err}"
    );
    assert_eq!(snapshot(&rig.dir("late")), late);
    assert!(!rig.state.join("archive").join(OTHER).exists());
    assert_eq!(code, 0, "{out}\n{err}");
    assert!(!rig.live("all"));
    assert!(!rig.dir("all").exists());
    assert_archive(&rig);
}
