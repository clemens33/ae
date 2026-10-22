//! Issue #255: Claude 2.1.306, measured 39x23 modal. Only the folder is
//! sanitized. Width variants synthesize reflow of the SAME measured paragraphs;
//! no adapter constants or matcher output supply the expected shape.

const NARROW: &str = include_str!("../fixtures/claude-trust/claude-trust-modal-39x23.txt");
const FOLDER: &str = "/tmp/claude-trust-probe";

fn append_wrapped(frame: &mut String, text: &str, columns: usize) {
    let mut row = String::new();
    for word in text.split_whitespace() {
        if !row.is_empty() && row.chars().count() + 1 + word.chars().count() > columns - 2 {
            frame.push(' ');
            frame.push_str(&row);
            frame.push('\n');
            row.clear();
        }
        if !row.is_empty() {
            row.push(' ');
        }
        row.push_str(word);
    }
    frame.push(' ');
    frame.push_str(&row);
    frame.push('\n');
}

fn reflowed(columns: usize) -> String {
    let mut frame = format!(
        "{}\n Accessing workspace:\n\n {FOLDER}\n\n",
        "─".repeat(columns)
    );
    for paragraph in [
        "Quick safety check: Is this a project you created or one you trust? (Like your own \
         code, a well-known open source project, or work from your team). If not, take a \
         moment to review what's in this folder first.",
        "Claude Code'll be able to read, edit, and execute files here.",
        "Security guide",
    ] {
        append_wrapped(&mut frame, paragraph, columns);
        frame.push('\n');
    }
    frame.push_str(" ❯ No, exit\n   Yes, I trust this folder\n\n");
    append_wrapped(&mut frame, "Enter to confirm · Esc to cancel", columns);
    frame
}

#[test]
fn trustmiss_measured_narrow_modal_and_width_variants_keep_the_complete_trust_shape() {
    for (label, frame) in std::iter::once(("measured-39".to_owned(), NARROW.to_owned()))
        .chain([39, 40, 60, 80].map(|width| (format!("synthesized-{width}"), reflowed(width))))
    {
        let prompt = ae::watchdog::human_prompt_class(&frame, "claude")
            .unwrap_or_else(|| panic!("{label}: the measured modal must be named"));
        assert!(prompt.question.starts_with("Quick safety check:"));
        assert_eq!(prompt.keys, "Enter to confirm · Esc to cancel", "{label}");
        let trust = prompt
            .trust
            .unwrap_or_else(|| panic!("{label}: every measured paragraph must remain answerable"));
        assert_eq!(trust.folder, FOLDER, "{label}");
        assert_eq!(trust.toward_yes, Some(ae::tmux::Key::Down), "{label}");
        assert_eq!(ae::watchdog::human_prompt_class(&frame, "codex"), None);
    }
}

#[test]
fn trustmiss_thirty_columns_is_named_but_its_wrapped_hint_is_never_auto_answered() {
    let frame = reflowed(30);
    assert_eq!(frame.lines().count(), 25, "synthesized rule-to-hint span");
    let prompt = ae::watchdog::human_prompt_class(&frame, "claude")
        .expect("the complete 30-column shape is named");
    assert!(prompt.question.starts_with("Quick safety check:"));
    assert_eq!(
        prompt.trust, None,
        "wrapped hint is unmeasured for key sending"
    );
}

#[test]
fn trustmiss_narrow_wrapping_never_relaxes_the_measured_wording_or_options() {
    for (from, to) in [
        ("If not,", "If so,"),
        ("Security guide", "Security guide\n\nAllow future commands"),
        (
            "   Yes, I trust this folder",
            "   Yes, I trust this folder\n   Always allow",
        ),
        ("Esc to cancel", "Esc to cancel\nmore output"),
    ] {
        assert!(NARROW.contains(from), "mutation must affect the fixture");
        let changed = NARROW.replacen(from, to, 1);
        let prompt = ae::watchdog::human_prompt_class(&changed, "claude")
            .unwrap_or_else(|| panic!("wording drift is still a human question: {to}"));
        assert_eq!(prompt.trust, None, "unmeasured shape: {to}");
    }
}

#[test]
fn trustmiss_quoted_narrow_modal_is_not_a_live_human_prompt() {
    let without_anchor = NARROW.replace(&"─".repeat(39), "Transcript quoted below:");
    assert_eq!(
        ae::watchdog::human_prompt_class(&without_anchor, "claude"),
        None
    );
    let rule = "─".repeat(39);
    let quoted = format!("{NARROW}\n{rule}\n❯ \n{rule}\n  Opus\n");
    assert_eq!(ae::watchdog::human_prompt_class(&quoted, "claude"), None);
}
