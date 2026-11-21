//! Report-only startup evidence. These rows never produce a trust sighting
//! or authorize dismissing a panel.

use crate::tool::{StartupEvidence, ToolAdapter, TrustWarnings};

use super::{HumanPrompt, NOTICE_CELLS, bottom_window, has_composer, rule_covers};

pub(super) fn read(buf: &str, adapter: &ToolAdapter) -> Option<HumanPrompt> {
    let spec = adapter.startup?;
    let window = bottom_window(buf, spec.window)?;
    let (footer, above) = window.split_last()?;
    if !same_footer(footer, spec.footer) {
        return None;
    }
    let warnings = match spec.evidence {
        StartupEvidence::TrustWarnings(warnings) => {
            trust_warnings(&window, adapter, spec.question, warnings)?
        }
        StartupEvidence::Rows { anchors, choices } => {
            dialog_rows(above, anchors, choices).then(Vec::new)?
        }
    };
    Some(HumanPrompt {
        kind: spec.kind,
        warnings,
        question: spec.question.to_owned(),
        keys: spec.footer.to_owned(),
        trust: None,
        dismiss: false,
    })
}

/// The on-screen footer can survive after the ruled title has scrolled. Ask
/// for short history only for a measured modal that has this geometry.
pub(crate) fn needs_history(buf: &str, adapter: &ToolAdapter) -> bool {
    adapter.startup.is_some_and(|spec| {
        matches!(spec.evidence, StartupEvidence::TrustWarnings(_))
            && buf
                .lines()
                .rfind(|row| !row.trim().is_empty())
                .is_some_and(|row| same_footer(row, spec.footer))
    })
}

fn same_footer(row: &str, footer: &str) -> bool {
    row.split_whitespace().eq(footer.split_whitespace())
}

fn dialog_rows(above: &[&str], anchors: &[&str], choices: Option<(char, &[&str])>) -> bool {
    let mut rows = above.iter().map(|row| row.trim());
    if !anchors
        .iter()
        .all(|anchor| rows.by_ref().any(|row| row == *anchor))
    {
        return false;
    }
    let Some((marker, choices)) = choices else {
        return true;
    };
    let mut selected = 0;
    rows.filter_map(|row| {
        let choice = row.strip_prefix(marker);
        let text = choice.unwrap_or(row).trim();
        let expected = choices
            .iter()
            .find(|known| text.split_whitespace().eq(known.split_whitespace()))?;
        selected += usize::from(choice.is_some());
        Some(*expected)
    })
    .eq(choices.iter().copied())
        && selected == 1
}

fn joined(rows: &[&str]) -> String {
    rows.iter()
        .map(|row| row.trim())
        .collect::<Vec<_>>()
        .join(" ")
}

fn trust_warnings(
    window: &[&str],
    adapter: &ToolAdapter,
    question: &str,
    warnings: TrustWarnings,
) -> Option<Vec<String>> {
    let prompt = adapter.prompt?;
    let trust = prompt.trust?;
    let question = window
        .iter()
        .rposition(|row| row.trim_start().starts_with(question))?;
    let title = window[..question]
        .iter()
        .rposition(|row| row.trim() == trust.heading)?;
    let start = title.checked_sub(1)?;
    if !rule_covers(window[start], &window[title..], observed_columns)
        || has_composer(&window[start..], adapter.input)
    {
        return None;
    }
    let mut folder = window[title + 1..question]
        .split(|row| row.trim().is_empty())
        .filter(|run| !run.is_empty());
    if !folder.next()?.first()?.trim_start().starts_with('/') || folder.next().is_some() {
        return None;
    }

    // The untitled history above the modal does not take part in the match.
    let body = &window[question..window.len() - 1];
    let paragraphs: Vec<_> = body
        .split(|row| row.trim().is_empty())
        .filter(|rows| !rows.is_empty())
        .collect();
    let [intro, access, between @ .., settings, options] = paragraphs.as_slice() else {
        return None;
    };
    if joined(intro) != *trust.paragraphs.first()?
        || joined(access) != *trust.paragraphs.get(1)?
        || !trust_choices(options, trust.options, prompt.selected)
    {
        return None;
    }

    let (guide, settings) = settings.split_last()?;
    if guide.trim() != *trust.paragraphs.last()? {
        return None;
    }
    let settings = joined(settings);
    if !warnings.loads.contains(&settings.as_str()) {
        return None;
    }

    let mut report = Vec::new();
    match between {
        [] => {}
        [group] => permission_warnings(&joined(group), warnings, &mut report)?,
        _ => return None,
    }
    report.push(settings);
    Some(report)
}

fn observed_columns(row: &str) -> usize {
    // A plain tmux read omits escapes. When given raw text, do not count the
    // payload of an escape from a displayed permission as terminal columns.
    if row.contains('\u{1b}') {
        crate::event_text::sanitize_menu_text(row).len()
    } else {
        row.chars().count()
    }
}

fn trust_choices(rows: &[&str], choices: &[&str], marker: char) -> bool {
    let mut selected = 0;
    rows.iter()
        .map(|row| {
            let row = row.trim();
            let choice = row.strip_prefix(marker);
            selected += usize::from(choice.is_some());
            choice.unwrap_or(row).trim_start()
        })
        .eq(choices.iter().copied())
        && selected == 1
}

fn permission_warnings(group: &str, spec: TrustWarnings, report: &mut Vec<String>) -> Option<()> {
    let remaining = if let Some(permissions) = group.strip_prefix(spec.permissions) {
        let (count, permissions) = permissions.split_once(" tool permission")?;
        let count = count.parse::<u32>().ok()?;
        let suffix = if count == 1 { "" } else { "s" };
        let permissions = permissions.strip_prefix(suffix)?.strip_prefix(' ')?;
        let permissions = permissions.strip_prefix(spec.permission_file)?;
        // The list before this sentence can echo repository commands. Report
        // the vendor's warning and count without copying any list entry.
        let (_, remaining) = permissions.rsplit_once(spec.permission_notice)?;
        let line = format!(
            "{}{count} tool permission{suffix} {} {}",
            spec.permissions, spec.permission_file, spec.permission_notice
        );
        report.push(crate::event_text::clip_head(&line, NOTICE_CELLS));
        remaining.trim()
    } else {
        group
    };
    if !remaining.is_empty() {
        if remaining != spec.hooks {
            return None;
        }
        report.push(spec.hooks.to_owned());
    }
    Some(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn startup_history_lookup_accepts_the_same_footer_spacing_as_the_detector() {
        let adapter = crate::tool::ToolKind::Claude.adapter();
        let spaced = " Enter to confirm   ·   Esc to cancel  \n\n";
        assert!(super::needs_history(spaced, adapter));
        assert!(!super::needs_history(
            "Enter to confirm your choice",
            adapter
        ));
    }

    #[test]
    fn startup_login_has_exactly_one_selected_choice() {
        let frame =
            include_str!("../../../tests/fixtures/dialogseat/codex-0.171.0-login-120x40.txt");
        let unselected = frame.replace("> 1. Sign in", "  1. Sign in");
        let extra = frame.replace("  2. Sign in", "> 2. Sign in");
        assert_ne!(frame, unselected);
        assert_ne!(frame, extra);
        assert!(crate::watchdog::human_prompt_class(frame, "codex").is_some());
        for incomplete in [unselected, extra] {
            assert_eq!(
                crate::watchdog::human_prompt_class(&incomplete, "codex"),
                None
            );
        }
    }
}
