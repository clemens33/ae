//! Counts over the Accounts tab's existing snapshot; no additional observations.

use std::collections::BTreeMap;

pub(super) fn lines(seats: &crate::quota::LiveSeats) -> Vec<String> {
    let mut activity = BTreeMap::new();
    let mut clients = BTreeMap::new();
    let mut models = BTreeMap::new();
    let mut efforts = BTreeMap::new();
    let mut live = 0;
    let mut unknown_liveness = 0;
    for group in &seats.groups {
        unknown_liveness += group.unknown_liveness;
        let client = group
            .key
            .as_ref()
            .map_or("unknown", |key| key.tool.as_str());
        for row in &group.rows {
            live += 1;
            let state = match row.activity.as_str() {
                "busy" => "busy",
                "idle" => "idle",
                _ => "unknown",
            };
            *activity.entry(state.to_owned()).or_default() += 1;
            *clients.entry(client.to_owned()).or_default() += 1;
            let model = known(&row.model);
            *models.entry(model.clone()).or_default() += 1;
            *efforts
                .entry(format!("{model} {}", known(&row.effort)))
                .or_default() += 1;
        }
    }
    let partial = if seats.incomplete { " (partial)" } else { "" };
    let mut out = vec![format!("Live seats: {live}{partial}")];
    append(&mut out, "Activity", activity);
    if unknown_liveness > 0 {
        out.push(format!(
            "Liveness unknown: {unknown_liveness} (excluded from live counts)"
        ));
    }
    append(&mut out, "Clients", clients);
    append(&mut out, "Models", models);
    append(&mut out, "Model + effort", efforts);
    out
}

fn known(value: &str) -> String {
    let clean = crate::board::terminal_text(value).replace(['\n', '\r', '\t'], " ");
    let value = clean.trim();
    if value.is_empty() || value == "?" || value.eq_ignore_ascii_case("unknown") {
        "unknown".to_owned()
    } else {
        value.to_owned()
    }
}

fn append(out: &mut Vec<String>, title: &str, counts: BTreeMap<String, usize>) {
    if counts.is_empty() {
        return;
    }
    let mut counts: Vec<_> = counts.into_iter().collect();
    counts.sort_by(|(a, ac), (b, bc)| bc.cmp(ac).then_with(|| a.cmp(b)));
    let values: Vec<_> = counts
        .into_iter()
        .map(|(name, count)| format!("{name} {count}"))
        .collect();
    out.push(format!("{title}: {}", values.join(", ")));
}

#[cfg(test)]
mod tests {
    use super::lines;
    use crate::app::draw::{LEFT, Layout, Paint, paint_settings, settings_body};
    use crate::app::model::SettingsTab;
    use crate::app::settings::SettingsBodies;
    use crate::quota::{LiveGroup, LiveRow, LiveSeats, RecordedIdentity};
    use crate::theme::Look;
    use crate::tool::ToolKind;
    use ratatui_core::buffer::Buffer;
    use ratatui_core::layout::Rect;

    fn row(model: &str, effort: &str, activity: &str) -> LiveRow {
        LiveRow {
            model: model.into(),
            effort: effort.into(),
            activity: activity.into(),
            ..LiveRow::default()
        }
    }

    fn group(tool: Option<ToolKind>, rows: Vec<LiveRow>, unknown: usize) -> LiveGroup {
        LiveGroup {
            key: tool.map(|tool| RecordedIdentity {
                tool,
                source: "/account".into(),
            }),
            // A client-looking heading without a key must not prove a client.
            heading: "claude · /account".into(),
            rows,
            unknown_liveness: unknown,
            observed_at: Some(1),
            ..LiveGroup::default()
        }
    }

    fn mixed() -> LiveSeats {
        LiveSeats {
            incomplete: false,
            groups: vec![
                group(
                    Some(ToolKind::Claude),
                    vec![
                        row("Opus 5.5", "high", "busy"),
                        row("Opus 5.5", "high", "idle"),
                    ],
                    2,
                ),
                group(
                    Some(ToolKind::Codex),
                    vec![
                        row("GPT 6.1", "xhigh", "busy"),
                        row("GPT 6.1", "high", "unknown"),
                    ],
                    0,
                ),
                group(None, vec![row("?", "?", "idle")], 1),
                group(Some(ToolKind::Muse), vec![row("", "", "unrecognized")], 0),
            ],
        }
    }

    #[test]
    fn mixed_snapshot_counts_only_live_rows_and_sorts_ties_by_name() {
        assert_eq!(
            lines(&mixed()),
            [
                "Live seats: 6",
                "Activity: busy 2, idle 2, unknown 2",
                "Liveness unknown: 3 (excluded from live counts)",
                "Clients: claude 2, codex 2, muse 1, unknown 1",
                "Models: GPT 6.1 2, Opus 5.5 2, unknown 2",
                "Model + effort: Opus 5.5 high 2, unknown unknown 2, GPT 6.1 high 1, GPT 6.1 xhigh 1",
            ]
        );
    }

    #[test]
    fn empty_and_unknown_liveness_only_have_no_zero_categories() {
        assert_eq!(lines(&LiveSeats::default()), ["Live seats: 0"]);
        let seats = LiveSeats {
            incomplete: true,
            groups: vec![group(Some(ToolKind::Codex), vec![], 4)],
        };
        assert_eq!(
            lines(&seats),
            [
                "Live seats: 0 (partial)",
                "Liveness unknown: 4 (excluded from live counts)",
            ]
        );
    }

    #[test]
    fn incomplete_snapshot_qualifies_counts_without_discarding_rows() {
        let mut seats = mixed();
        seats.incomplete = true;
        assert_eq!(lines(&seats)[0], "Live seats: 6 (partial)");
        assert_eq!(
            lines(&seats)[3],
            "Clients: claude 2, codex 2, muse 1, unknown 1"
        );
    }

    #[test]
    fn unknown_client_and_model_never_come_from_profile_or_heading() {
        let seats = LiveSeats {
            groups: vec![group(
                Some(ToolKind::Unknown),
                vec![row(" UNKNOWN ", "?", "busy"), row("  ", "high", "busy")],
                0,
            )],
            ..LiveSeats::default()
        };
        assert_eq!(
            lines(&seats),
            [
                "Live seats: 2",
                "Activity: busy 2",
                "Clients: unknown 2",
                "Models: unknown 2",
                "Model + effort: unknown high 1, unknown unknown 1",
            ]
        );
    }

    #[test]
    fn summary_draws_above_accounts_at_narrow_width_with_theme_off() {
        let mut seats = mixed();
        seats.incomplete = true;
        let feed = SettingsBodies {
            live_seats: Some(seats),
            ..SettingsBodies::default()
        };
        for width in [40, 160] {
            for look in [None, Some(Look::read("on", "darcula", "on", "on"))] {
                let body = settings_body(
                    SettingsTab::Accounts,
                    &feed,
                    Paint::of(look.as_ref()),
                    width,
                    true,
                );
                let at = |needle: &str| {
                    body.iter()
                        .position(|(line, _)| line.contains(needle))
                        .expect(needle)
                };
                assert!(at("snapshot") < at("Live-seat summary"));
                assert!(at("incomplete fleet snapshot") < at("Live-seat summary"));
                assert!(at("Live seats: 6 (partial)") < at("Live seats by account"));
                let summary = body[..at("Live seats by account")]
                    .iter()
                    .map(|(line, _)| line.as_str())
                    .collect::<Vec<_>>()
                    .join(" ");
                for needle in [
                    "claude 2",
                    "codex 2",
                    "Opus 5.5 high 2",
                    "unknown unknown 2",
                ] {
                    assert!(summary.contains(needle), "{summary}");
                }
                assert!(
                    body.iter()
                        .all(|(line, _)| super::super::drawn_cols(line) <= width - LEFT)
                );
                let mut buf = Buffer::empty(Rect::new(0, 0, width, 60));
                paint_settings(
                    SettingsTab::Accounts,
                    0,
                    &feed,
                    look.as_ref(),
                    &mut buf,
                    &mut Layout::default(),
                );
                let rendered: Vec<String> = (0..60)
                    .map(|y| (0..width).map(|x| buf[(x, y)].symbol()).collect())
                    .collect();
                let at = |needle: &str| {
                    rendered
                        .iter()
                        .position(|line| line.contains(needle))
                        .expect(needle)
                };
                assert!(at("Live-seat summary") < at("Live seats by account"));
                if look.is_none() {
                    assert!(
                        buf.content
                            .iter()
                            .all(|cell| cell.fg == ratatui_core::style::Color::Reset)
                    );
                }
            }
        }
    }
}
