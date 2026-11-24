//! Measured startup dialogs read with their warnings: logins and panels are
//! report-only; a folder-trust block with settings warnings is a trust
//! sighting when its source row names `.claude/settings.json` alone (owner
//! ruling 2026-11-23), and named with the reason otherwise.

use crate::tool::{StartupEvidence, ToolAdapter, TrustWarnings};

use super::{
    HumanPrompt, NOTICE_CELLS, TrustSighting, TrustVariant, bottom_window, has_composer,
    rule_covers,
};

pub(super) fn read(buf: &str, adapter: &ToolAdapter) -> Option<HumanPrompt> {
    let spec = adapter.startup?;
    let window = bottom_window(buf, spec.window)?;
    let (footer, above) = window.split_last()?;
    if !same_footer(footer, spec.footer) {
        return None;
    }
    let (warnings, trust, differs) = match spec.evidence {
        StartupEvidence::TrustWarnings(warnings) => {
            let read = trust_warnings(&window, adapter, spec.question, warnings)?;
            (read.report, read.sighting, read.differs)
        }
        StartupEvidence::Rows { anchors, choices } => (
            dialog_rows(above, anchors, choices).then(Vec::new)?,
            None,
            None,
        ),
    };
    Some(HumanPrompt {
        kind: spec.kind,
        warnings,
        question: spec.question.to_owned(),
        keys: spec.footer.to_owned(),
        trust,
        dismiss: false,
        differs,
    })
}

/// What a measured settings-warning trust block says: the warnings to report
/// and, when `auto_trust` may answer it, the sighting; else why not.
struct SettingsRead {
    report: Vec<String>,
    sighting: Option<TrustSighting>,
    differs: Option<String>,
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

#[allow(
    clippy::too_many_lines,
    reason = "one top-to-bottom match of the measured block, rule to hint; a split hides the row order"
)]
fn trust_warnings(
    window: &[&str],
    adapter: &ToolAdapter,
    question: &str,
    warnings: TrustWarnings,
) -> Option<SettingsRead> {
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
    let folder_run = folder.next()?;
    if !folder_run.first()?.trim_start().starts_with('/') || folder.next().is_some() {
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
    let access = joined(access);
    let home = trust
        .home
        .is_some_and(|home| home.at == 1 && access == home.access);
    if joined(intro) != *trust.paragraphs.first()? || (access != *trust.paragraphs.get(1)? && !home)
    {
        return None;
    }
    let selected = trust_choices(options, trust.options, prompt.selected)?;

    let (guide, settings_rows) = settings.split_last()?;
    if guide.trim() != *trust.paragraphs.last()? {
        return None;
    }
    let settings = joined(settings_rows);
    if !warnings.loads.contains(&settings.as_str()) {
        return None;
    }

    let mut report = Vec::new();
    let warned = match between {
        [] => Warned {
            plain: true,
            permissions: false,
            hooks: false,
        },
        [group] => permission_warnings(&joined(group), warnings, &mut report)?,
        _ => return None,
    };
    // The answer is matched on the RAW joined rows, never on the report text
    // above: every paragraph but the permission list is a measured constant
    // already, and the list must be plain drawn text with one notice after it.
    let yes = trust
        .options
        .iter()
        .position(|option| *option == trust.yes)?;
    let (sighting, differs) = match (
        drawn_folder(
            folder_run,
            window[start].trim_end().chars().count(),
            warnings.wraps,
        ),
        refusal(warned, home, warnings, &settings),
    ) {
        (Some(folder), None) => (
            Some(TrustSighting {
                folder,
                toward_yes: toward(yes, selected),
                variant: if home {
                    TrustVariant::HomeSettings
                } else {
                    TrustVariant::Settings
                },
                block: between
                    .iter()
                    .chain([&settings_rows])
                    .map(|rows| joined(rows))
                    .collect::<Vec<_>>()
                    .join("\n"),
            }),
            None,
        ),
        (folder, why) => {
            let why = why.filter(|_| folder.is_some());
            let why = why
                .as_deref()
                .unwrap_or("the folder rows are not one wrapped path");
            (None, Some(differs(trust.measured, why)))
        }
    };
    report.push(settings);
    Some(SettingsRead {
        report,
        sighting,
        differs,
    })
}

/// The loud reason a trust block that is not measured is named with: the
/// measured version, then `why` clipped to the notice. Both trust readers
/// build it here, beside the menu-text cutter's other named caller.
pub(super) fn differs(measured: &str, why: &str) -> String {
    let why = crate::event_text::clip_head(why, NOTICE_CELLS);
    format!("dialog differs from the measured one ({measured}): {why}")
}

/// The first paragraph where `drawn` and `want` part, in words.
pub(super) fn first_difference(drawn: &[String], want: &[String]) -> String {
    let quote = |text: &str| crate::event_text::clip_head(text, 100);
    let at = drawn
        .iter()
        .zip(want)
        .position(|(drawn, want)| drawn != want);
    match (at, drawn.len().cmp(&want.len())) {
        (Some(at), _) => format!(
            "paragraph {} reads \"{}\", not \"{}\"",
            at + 1,
            quote(&drawn[at]),
            quote(&want[at])
        ),
        (None, std::cmp::Ordering::Greater) => {
            format!("unmeasured paragraph \"{}\"", quote(&drawn[want.len()]))
        }
        (None, _) => format!(
            "paragraph {} \"{}\" is missing",
            drawn.len() + 1,
            quote(&want[drawn.len()])
        ),
    }
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

/// The arrow from the `selected` option onto option `yes`.
fn toward(yes: usize, selected: usize) -> Option<crate::tmux::Key> {
    match yes.cmp(&selected) {
        std::cmp::Ordering::Equal => None,
        std::cmp::Ordering::Greater => Some(crate::tmux::Key::Down),
        std::cmp::Ordering::Less => Some(crate::tmux::Key::Up),
    }
}

/// The index of the ONE selected row when `rows` are exactly `choices`.
fn trust_choices(rows: &[&str], choices: &[&str], marker: char) -> Option<usize> {
    let mut selected = Vec::new();
    let same = rows
        .iter()
        .enumerate()
        .map(|(at, row)| {
            let row = row.trim();
            let choice = row.strip_prefix(marker);
            if choice.is_some() {
                selected.push(at);
            }
            choice.unwrap_or(row).trim_start()
        })
        .eq(choices.iter().copied());
    match selected.as_slice() {
        [at] if same => Some(*at),
        _ => None,
    }
}

/// Report the permission and hooks warnings of `group`; `None` when it is
/// not the measured grammar. The flag says whether the repository-echoed
/// permission list is plain drawn text, the one condition under which the
/// block may be answered: no control, invisible or bidi-format character,
/// and the vendor notice exactly once after it.
fn permission_warnings(
    group: &str,
    spec: TrustWarnings,
    report: &mut Vec<String>,
) -> Option<Warned> {
    let mut warned = Warned {
        plain: true,
        permissions: false,
        hooks: false,
    };
    let remaining = if let Some(permissions) = group.strip_prefix(spec.permissions) {
        let (count, permissions) = permissions.split_once(" tool permission")?;
        let count = count.parse::<u32>().ok()?;
        let suffix = if count == 1 { "" } else { "s" };
        let permissions = permissions.strip_prefix(suffix)?.strip_prefix(' ')?;
        let permissions = permissions.strip_prefix(spec.permission_file)?;
        // The list before this sentence can echo repository commands. Report
        // the vendor's warning and count without copying any list entry.
        let (list, remaining) = permissions.rsplit_once(spec.permission_notice)?;
        warned.permissions = true;
        warned.plain = drawn_text(list)
            && !list.contains(spec.permission_notice)
            && measured_list(list.trim(), count);
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
        warned.hooks = true;
    }
    Some(warned)
}

/// Which warnings a settings block draws, and whether its permission list is
/// the measured one (`plain`; true when there is none).
#[derive(Clone, Copy)]
struct Warned {
    plain: bool,
    permissions: bool,
    hooks: bool,
}

/// Why `auto_trust` does not answer a measured settings block, if it does
/// not: a permission list outside the measured grammar, a source row that
/// loads more than `.claude/settings.json`, or rows never captured together.
fn refusal(warned: Warned, home: bool, warnings: TrustWarnings, settings: &str) -> Option<String> {
    if !warnings.answers.iter().any(|known| known.loads == settings) {
        Some(format!(
            "\"{}\" loads more than .claude/settings.json",
            crate::event_text::clip_head(settings, 100)
        ))
    } else if !warned.plain {
        Some("the permission list is not the measured list".to_owned())
    } else if !warnings.answers.iter().any(|known| {
        known.home == home
            && known.permissions == warned.permissions
            && known.hooks == warned.hooks
            && known.loads == settings
    }) {
        Some("these warning and source rows were never measured together".to_owned())
    } else {
        None
    }
}

/// The folder a settings block names: one row, or a path claude wrapped over
/// exactly two rows at a wrap column measured for this frame `width`
/// (`TrustWarnings::wraps`), both printable-ASCII fragments (lead ruling
/// 2026-11-24). The result must still equal the seat's folder canonically
/// before any key; any other join is named.
fn drawn_folder<'a>(rows: &[&'a str], width: usize, wraps: &[(usize, usize)]) -> Option<String> {
    match rows {
        [row] => Some(row.trim().to_owned()),
        [head, tail] => {
            // The captured one-space indent, then printable ASCII only:
            // terminal-safe, and one byte is one cell, so the measured column
            // is the drawn column. The second row fits within the same column,
            // or claude would wrap it again.
            let indented = |row: &'a str| row.trim_end().strip_prefix(' ');
            let (Some(head), Some(tail)) = (indented(head), indented(tail)) else {
                return None;
            };
            let fragment =
                |row: &str| !row.is_empty() && row.bytes().all(|byte| byte.is_ascii_graphic());
            (fragment(head)
                && fragment(tail)
                && wraps.contains(&(width, head.len()))
                && tail.len() <= head.len())
            .then(|| format!("{head}{tail}"))
        }
        _ => None,
    }
}

/// Whether `list` is the permission list claude 2.1.330 draws in full for
/// `count` entries: one entry alone, or entries joined by ", " with "and "
/// before the last (captured with one and six entries; the count is a field).
/// An entry is a tool name (ASCII letter, then letters, digits, `_` or `-`)
/// with an optional parenthesised rule, parentheses balanced. The elided
/// "and <k> more" form was captured only beside an `.mcp.json` source, so it
/// is not answered (lead ruling 2026-11-24, fail closed); nor is anything
/// else, a row added between the entries or after them included.
fn measured_list(list: &str, count: u32) -> bool {
    let mut rest = list;
    let mut entries = 0_u32;
    loop {
        let final_entry = match rest.strip_prefix("and ") {
            Some(tail) if entries > 0 => {
                rest = tail;
                true
            }
            _ => false,
        };
        let Some(after) = permission_entry(rest) else {
            return false;
        };
        entries += 1;
        if after.is_empty() {
            return (final_entry || entries == 1) && entries == count;
        }
        match after.strip_prefix(", ") {
            Some(next) if !final_entry => rest = next,
            _ => return false,
        }
    }
}

/// The text after one permission entry at the start of `text`.
fn permission_entry(text: &str) -> Option<&str> {
    let name = text
        .find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '-'))
        .unwrap_or(text.len());
    if name == 0 || !text.starts_with(|ch: char| ch.is_ascii_alphabetic()) {
        return None;
    }
    let rest = &text[name..];
    let Some(rule) = rest.strip_prefix('(') else {
        return Some(rest);
    };
    let mut depth = 1_usize;
    for (at, ch) in rule.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&rule[at + 1..]);
                }
            }
            _ => {}
        }
    }
    None
}

/// Text a terminal draws as-is: no control character, and no zero-width,
/// bidi-format or replacement character that hides or reorders what it shows.
fn drawn_text(text: &str) -> bool {
    !text.chars().any(|ch| {
        ch.is_control()
            || matches!(
                ch,
                '\u{00ad}'
                    | '\u{200b}'..='\u{200f}'
                    | '\u{202a}'..='\u{202e}'
                    | '\u{2060}'..='\u{206f}'
                    | '\u{feff}'
                    | '\u{fffd}'
            )
    })
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
