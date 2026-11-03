//! Frozen recovery pins at b9cd091b. Oracle: reference/commands.md stopped
//! rename ordering; exact output and phase corrections are base behaviour pins.
//! Uses real launch/stop, real private tmux and attestation-driven process death.
#![allow(clippy::expect_used, reason = "isolated characterization fixtures")]

use super::*;

fn carrier(rig: &Rig, old: &str, new: &str) -> PathBuf {
    rig.home
        .join("sessions")
        .join(format!(".rename.{old}.{new}.intent"))
}
fn row(text: &str, key: &str) -> String {
    text.lines()
        .find_map(|line| line.strip_prefix(&format!("{key}=")))
        .unwrap_or_default()
        .into()
}
fn claim(rig: &Rig, old: &str, new: &str, phase: &str) {
    let path = carrier(rig, old, new);
    let text = std::fs::read_to_string(&path).expect("carrier");
    let prior = row(&text, "phase");
    std::fs::write(
        path,
        text.replace(&format!("phase={prior}\n"), &format!("phase={phase}\n")),
    )
    .expect("claim");
}
fn stopped(tag: &str, mode: &str) -> Rig {
    let rig = Rig::idle(tag);
    if mode == "git" {
        git_in(&rig.project, &["init", "-q"]);
        git_in(&rig.project, &["config", "user.email", "t@t"]);
        git_in(&rig.project, &["config", "user.name", "t"]);
        std::fs::write(rig.project.join("tracked"), b"repository contents\n")
            .expect("tracked file");
        git_in(&rig.project, &["add", "-A"]);
        git_in(&rig.project, &["commit", "-qm", "base"]);
    }
    let args = match mode {
        "local" => vec!["--local", "sold"],
        "full" => vec!["--copy", "sold"],
        _ => vec!["--worktree", "sold"],
    };
    let run = rig.launch(&args);
    assert_eq!(run.0, Some(0), "launch {mode}: {run:?}");
    assert_eq!(
        row(&rig.meta("sold"), "mode"),
        if mode == "full" { "full" } else { mode },
        "fixture really reaches claimed mode"
    );
    let run = public(&rig, &["stop", "sold"]);
    assert_eq!(run.0, Some(0), "stop: {run:?}");
    rig
}
fn finish(rig: &Rig, mode: &str) {
    let before = std::fs::read_to_string(carrier(rig, "sold", "snew")).expect("carrier");
    let work = row(&before, "new_work");
    let run = public(rig, &["rename", "sold", "snew"]);
    let detail = if mode == "local" {
        format!("work preserved at '{work}'")
    } else {
        format!("managed work moved to '{work}'")
    };
    assert_eq!(
        run,
        (
            Some(0),
            format!("Renamed 'sold' → 'snew' (stopped; {detail}).\n"),
            String::new()
        ),
        "recovery success bytes"
    );
    assert!(!rig.dir("sold").exists());
    let meta = rig.meta("snew");
    assert_eq!(row(&meta, "session"), "snew");
    assert_eq!(row(&meta, "session_id"), row(&before, "session_id"));
    assert_eq!(row(&meta, "work_dir"), work);
    assert_eq!(
        row(
            &std::fs::read_to_string(carrier(rig, "sold", "snew")).expect("complete carrier"),
            "phase"
        ),
        "complete"
    );
    assert!(
        !rig.sessions().iter().any(|n| n == "sold" || n == "snew"),
        "stays stopped"
    );
}

#[test]
fn recovery_retry_crash_table_attests_each_committed_phase_and_then_converges() {
    if skip() {
        return;
    }
    for mode in ["local", "full", "git"] {
        for (index, boundary, phase) in [
            (0, "after-work-move", "work-moved"),
            (1, "after-state-move", "state-moved"),
            (2, "after-meta", "meta-published"),
            (3, "after-assets", "assets-published"),
            (4, "after-result", "complete"),
        ] {
            if mode == "local" && index == 0 {
                continue;
            }
            let rig = stopped(&format!("rr-{mode}-{index}"), mode);
            kill_at_boundary(&rig, "sold", "snew", "after-intent");
            let prepared =
                std::fs::read_to_string(carrier(&rig, "sold", "snew")).expect("prepared");
            assert_eq!(row(&prepared, "phase"), "prepared");
            let said = kill_at_boundary(&rig, "sold", "snew", boundary);
            assert_eq!(
                said,
                format!("rename-crash-boundary: {boundary}\n"),
                "retry attestation bytes"
            );
            let cut = std::fs::read_to_string(carrier(&rig, "sold", "snew")).expect("cut carrier");
            assert_eq!(row(&cut, "phase"), phase, "{mode}/{boundary}");
            assert_eq!(rig.dir("sold").exists(), index == 0, "state address at cut");
            assert_eq!(rig.dir("snew").exists(), index != 0, "state address at cut");
            finish(&rig, mode);
        }
    }
}

#[test]
fn recovery_phase_correction_table_uses_proven_facts_before_retrying() {
    if skip() {
        return;
    }
    for (index, initial, claimed, retry, phase) in [
        (
            0,
            "after-intent",
            "assets-published",
            "after-state-move",
            "state-moved",
        ),
        (
            1,
            "after-state-move",
            "prepared",
            "after-meta",
            "meta-published",
        ),
        (
            2,
            "after-meta",
            "prepared",
            "after-assets",
            "assets-published",
        ),
        (3, "after-assets", "prepared", "after-result", "complete"),
    ] {
        let rig = stopped(&format!("rr-normalize-{index}"), "local");
        kill_at_boundary(&rig, "sold", "snew", initial);
        claim(&rig, "sold", "snew", claimed);
        let said = kill_at_boundary(&rig, "sold", "snew", retry);
        assert_eq!(said, format!("rename-crash-boundary: {retry}\n"));
        let text =
            std::fs::read_to_string(carrier(&rig, "sold", "snew")).expect("corrected carrier");
        assert_eq!(
            row(&text, "phase"),
            phase,
            "correction drives the proved next step"
        );
        finish(&rig, "local");
    }
}
