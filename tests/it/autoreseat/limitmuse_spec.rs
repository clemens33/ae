//! Uses the existing real stop-leg vehicle, replacing only the external harness.
use super::*;

const PLAIN: &str = include_str!("../../fixtures/harness-state/muse-limit-plain.txt");
const WATCHING: &str = include_str!("../../fixtures/harness-state/muse-limit-watching.txt");

fn attempt(tag: &str, frame: &str) -> (Rig, String) {
    let rig = Rig::new(tag);
    // The existing fake tool executable identifies its binary by basename.
    // Its implementation remains the rig's external-harness replacement.
    assert!(
        std::fs::copy(
            rig.scratch.join("tools/claude"),
            rig.scratch.join("tools/muse")
        )
        .is_ok()
    );
    let config = rig.scratch.join("config");
    let text = std::fs::read_to_string(&config).expect("fixture config");
    let profile = format!(
        "fake-muse = \"{} {}\"\n[workspace]",
        rig.scratch.join("tools/muse").display(),
        rig.scratch.join("muse.pl").display()
    );
    assert!(std::fs::write(&config, text.replace("[workspace]", &profile)).is_ok());
    configure_from(&rig, ON_NOW, "fake-muse", "fake-opencode");
    let pane = limitstop_playback(&rig, "muse", frame);
    journal(&rig, "limit", None);
    journal(&rig, ATTEMPT_ACTION, Some(KEY));
    (rig, pane)
}

#[test]
fn limitmuse_real_stop_leg_moves_the_measured_plain_frame() {
    let (rig, pane) = attempt("museplain", PLAIN);
    let pid = rig.tool_pid(&pane, "muse").expect("the limited tool");
    let (code, out, err) = leg(&rig, "spawned.0", KEY);
    assert_eq!(code, Some(0), "out={out} err={err}");
    assert!(rig.tool_pid(&pane, "muse").is_none(), "predecessor stopped");
    assert!(
        rig.tool_pid(&pane, "opencode").is_some(),
        "replacement running"
    );
    assert_ne!(rig.tool_pid(&pane, "opencode"), Some(pid));
    assert_eq!(rig.meta_row("profile.spawned.0"), "fake-opencode");
    assert_eq!(
        one_outcome(&rig, DONE_ACTION).reference.as_deref(),
        Some("fake-muse")
    );
}

fn refuses(tag: &str, frame: &str, reason: &str) {
    let (rig, pane) = attempt(tag, frame);
    let before = rig.tool_pid(&pane, "muse").expect("predecessor running");
    let (code, out, err) = leg(&rig, "spawned.0", KEY);
    assert_eq!(code, Some(1), "out={out} err={err}");
    assert_eq!(
        rig.tool_pid(&pane, "muse"),
        Some(before),
        "same process survives"
    );
    assert_eq!(rig.meta_row("profile.spawned.0"), "fake-muse");
    assert!(
        with_action(&records(&rig), "reseat").is_empty(),
        "no reseat"
    );
    assert!(
        !rig.dir.join("seed.scout.md").exists(),
        "no replacement seed"
    );
    let outcome = one_outcome(&rig, REFUSED_ACTION);
    let summary = outcome.summary.as_deref().unwrap_or_default();
    assert!(
        summary.contains(reason),
        "journal must name {reason:?}: {summary}"
    );
    assert_eq!(booked(&rig, DONE_ACTION), 0);
}

#[test]
fn limitmuse_real_stop_leg_preserves_a_watching_background_task_and_names_it() {
    refuses("musewatch", WATCHING, "background evidence watch");
}

#[test]
fn limitmuse_real_stop_leg_preserves_a_running_background_task_and_names_it() {
    refuses(
        "muserun",
        &WATCHING.replace("watching", "running"),
        "background evidence watch",
    );
}

#[test]
fn limitmuse_real_stop_leg_preserves_a_draft() {
    refuses(
        "musedraft",
        &PLAIN.replace("\n❯\n", "\n❯ unfinished human draft\n"),
        "frame",
    );
}

#[test]
fn limitmuse_real_stop_leg_names_a_later_transcript_row() {
    let frame = PLAIN.replace(
        "── Voice input",
        "◆ Running after the limit\n── Voice input",
    );
    refuses("muselater", &frame, "Running after the limit");
}

#[test]
fn limitmuse_real_stop_leg_names_a_missing_limit_block() {
    // Preserve the playback readiness marker, but quote the head: it is no
    // longer a column-zero vendor block, so the product must find no cell.
    let frame = PLAIN.replacen("◆ Usage limit reached", "  ◆ Usage limit reached", 1);
    refuses("musenocell", &frame, "limit");
}
