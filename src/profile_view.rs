//! Pure descriptive projection of configured profiles and local seat evidence.

use std::fmt::Write as _;
use std::path::Path;

use crate::tool::{ToolKind, UsageSource};
use crate::usage::{Coverage, SeatUsage, Tokens};

pub(crate) struct Seat {
    pub(crate) profile: String,
    pub(crate) session: String,
    pub(crate) name: String,
    pub(crate) slot: String,
    pub(crate) source: UsageSource,
    pub(crate) rows: Vec<SeatUsage>,
    pub(crate) drift: Option<String>,
    pub(crate) lifetime: Option<u64>,
    pub(crate) retired: bool,
}

#[derive(Default)]
pub(crate) struct Audit {
    pub(crate) seats: Vec<Seat>,
    pub(crate) gaps: Vec<String>,
    pub(crate) notes: Vec<String>,
}

fn cell(text: &str) -> String {
    crate::board::terminal_text(text).replace(['\n', '\t'], " ")
}

/// Closed observed-ID grammar; labels, client names and configured pins never enter it.
fn provider(id: &str) -> Option<&'static str> {
    const PREFIXES: &[(&str, &str)] = &[
        ("claude-fable-", "Anthropic"),
        ("claude-opus-", "Anthropic"),
        ("claude-sonnet-", "Anthropic"),
        ("claude-haiku-", "Anthropic"),
        ("muse-spark-", "Meta"),
        ("muse-wonder-", "Meta"),
    ];
    if let Some(rest) = id.strip_prefix("gpt-")
        && rest.starts_with(|ch: char| ch.is_ascii_digit())
    {
        return Some("OpenAI");
    }
    PREFIXES.iter().find_map(|(prefix, owner)| {
        id.strip_prefix(prefix)
            .filter(|rest| !rest.is_empty())
            .map(|_| *owner)
    })
}

struct Evidence<'a> {
    models: Vec<&'a str>,
    source: &'static str,
    provider: &'static str,
    partial: bool,
}

fn evidence<'a>(seats: &[&'a Seat]) -> Evidence<'a> {
    let mut models: Vec<_> = seats
        .iter()
        .flat_map(|seat| &seat.rows)
        .filter(|row| {
            matches!(row.coverage, Coverage::Read | Coverage::Truncated)
                && crate::usage::is_model_id(&row.model)
                && row.model != "?"
        })
        .map(|row| row.model.as_str())
        .collect();
    let source = if models.is_empty() {
        models.extend(seats.iter().filter_map(|seat| seat.drift.as_deref()));
        if models.is_empty() { "none" } else { "drift" }
    } else {
        "transcript"
    };
    models.sort_unstable();
    models.dedup();
    let mut owners: Vec<_> = models.iter().filter_map(|model| provider(model)).collect();
    owners.sort_unstable();
    owners.dedup();
    let partial = models.iter().any(|model| provider(model).is_none());
    Evidence {
        models,
        source,
        provider: match owners.as_slice() {
            [one] => one,
            [] => "unknown",
            _ => "mixed",
        },
        partial,
    }
}

pub(crate) fn render(
    cfg: &crate::config::IdentityConfig,
    home: Option<&Path>,
    audit: &Audit,
    quota: &[(String, String)],
) -> String {
    let mut out = String::from(
        "Profiles: policy eligibility not known to ae; resolves (syntax only).\nHost canonical sessions only; archives and nested legacy layouts excluded.\nDescriptive seat observations, ordered by last recorded sign of life; no liveness probe.\nAPI-equivalent list-price spend is separate from subscription cash and quota.\n",
    );
    for (name, _) in &cfg.profiles {
        let seats: Vec<_> = audit
            .seats
            .iter()
            .filter(|seat| &seat.profile == name)
            .collect();
        let command = cfg.command(name, home).ok().flatten();
        let raw = command.as_ref().map_or("", |command| command.as_str());
        let tool = crate::launch_cmd::split_binary(raw)
            .map(|split| ToolKind::from_binary_name(split.binary_name()));
        let resolves = tool.is_some_and(ToolKind::is_known)
            && crate::launch_cmd::lex_simple_command(raw).is_ok();
        let model = tool
            .and_then(|tool| crate::launch_cmd::sole_flag_value(raw, tool.adapter().model.flags()));
        let effort = tool.and_then(|tool| crate::launch_cmd::effort_flag(raw, tool).ok());
        let evidence = evidence(&seats);
        let seen = if evidence.models.is_empty() {
            "no".to_owned()
        } else {
            format!("yes/{}", evidence.source)
        };
        let _ = writeln!(
            out,
            "{} resolves={} (syntax only) client={} configured-model={} effort={} observed-provider={} ({})",
            cell(name),
            if resolves { "yes" } else { "no" },
            cell(
                cfg.profile_client_label(name)
                    .as_deref()
                    .unwrap_or("unknown")
            ),
            cell(model.as_deref().unwrap_or("unknown")),
            effort.map_or("unknown", crate::effort::Level::word),
            evidence.provider,
            evidence.source
        );
        let rows: Vec<_> = seats.iter().flat_map(|seat| &seat.rows).collect();
        let available: Vec<_> = rows
            .iter()
            .filter(|row| {
                matches!(row.coverage, Coverage::Read | Coverage::Truncated) && row.model != "?"
            })
            .copied()
            .collect();
        let supported =
            tool.is_some_and(|tool| tool.adapter().usage.source != UsageSource::Unsupported);
        let partial = !audit.gaps.is_empty()
            || evidence.partial
            || rows.iter().any(|row| {
                row.approximate || !matches!(row.coverage, Coverage::Read | Coverage::Unsupported)
            });
        let priced = !available.is_empty() && available.iter().all(|row| row.usd_micro.is_some());
        let _ = writeln!(
            out,
            "  coverage usage={} priced={} observed-model={}{}",
            if supported { "yes" } else { "no" },
            if priced { "yes" } else { "no" },
            seen,
            if partial { " partial" } else { "" }
        );
        let _ = writeln!(out, "{}", summary(&seats, &available, priced, partial));
        for model in evidence.models {
            let _ = writeln!(
                out,
                "  observed-model-id={} ({})",
                cell(model),
                evidence.source
            );
        }
        seat_details(&mut out, &seats);
        for (_, table) in quota.iter().filter(|(profile, _)| profile == name) {
            out.push_str(table);
        }
    }
    for gap in audit.gaps.iter().chain(&audit.notes) {
        let _ = writeln!(out, "{}", cell(gap));
    }
    out
}

fn summary(seats: &[&Seat], available: &[&SeatUsage], priced: bool, partial: bool) -> String {
    let current: Vec<_> = seats.iter().filter(|seat| !seat.retired).collect();
    let lifetime = (!current.is_empty())
        .then(|| {
            current.iter().try_fold(0_u64, |sum, seat| {
                seat.lifetime.map(|value| sum.saturating_add(value))
            })
        })
        .flatten();
    let mut summary = format!(
        "  descriptive seats={} current-incarnation-lifetime={} (seat-seconds)",
        seats.len(),
        lifetime.map_or_else(|| "unknown".to_owned(), |value| format!("{value}s"))
    );
    if available.is_empty() {
        summary.push_str(" input=unknown output=unknown API-equivalent-USD=unknown");
    } else {
        let total = available
            .iter()
            .fold(Tokens::default(), |sum, row| sum.saturating_add(row.tokens));
        let _ = write!(
            summary,
            " input={} cache-write={} cache-read={} output={}",
            total.input, total.cache_write, total.cache_read, total.output
        );
        if priced {
            let micro = available.iter().fold(0_u64, |sum, row| {
                sum.saturating_add(row.usd_micro.unwrap_or(0))
            });
            let _ = write!(
                summary,
                " API-equivalent-USD={}.{:06}",
                micro / 1_000_000,
                micro % 1_000_000
            );
        } else {
            summary.push_str(" API-equivalent-USD=unknown unpriced");
        }
    }
    if partial {
        summary.push_str(" partial");
    }
    summary
}

fn seat_details(out: &mut String, seats: &[&Seat]) {
    for seat in seats {
        let label = if seat.retired {
            "retired-seat-lifetime"
        } else {
            "current-incarnation-lifetime"
        };
        let _ = writeln!(
            out,
            "  seat={}/{} slot={} {label}={} reasoning {}",
            cell(&seat.session),
            cell(&seat.name),
            cell(&seat.slot),
            seat.lifetime
                .map_or_else(|| "unknown".to_owned(), |value| format!("{value}s")),
            match seat.source {
                UsageSource::ClaudeTranscripts => "not separately reported (claude)",
                UsageSource::CodexRollout => "included in output (codex)",
                UsageSource::Unsupported => "unsupported",
            }
        );
        for row in &seat.rows {
            let coverage = match &row.coverage {
                Coverage::Read => "read".to_owned(),
                Coverage::Unsupported => "unsupported".to_owned(),
                Coverage::Unreadable(reason) => format!("unreadable: {reason}"),
                Coverage::Unlocated => "unlocated".to_owned(),
                Coverage::Truncated => "truncated".to_owned(),
            };
            let _ = writeln!(
                out,
                "    seat={}/{} coverage={}{}",
                cell(&seat.session),
                cell(&seat.name),
                cell(&coverage),
                if row.approximate { " approximate" } else { "" }
            );
        }
        if let Some(drift) = &seat.drift {
            let _ = writeln!(out, "    drift-only-model={}", cell(drift));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::provider;

    #[test]
    fn no_observations_have_no_evidence_source() {
        assert_eq!(super::evidence(&[]).source, "none");
    }

    #[test]
    fn current_incarnation_total_excludes_retired_lifetimes() {
        let seat = |retired, lifetime| super::Seat {
            profile: "p".to_owned(),
            session: "s".to_owned(),
            name: "n".to_owned(),
            slot: "main".to_owned(),
            source: crate::tool::UsageSource::Unsupported,
            rows: Vec::new(),
            drift: None,
            lifetime: Some(lifetime),
            retired,
        };
        let (current, retired) = (seat(false, 100), seat(true, 900));
        assert!(
            super::summary(&[&current, &retired], &[], false, false)
                .contains("current-incarnation-lifetime=100s")
        );
    }

    #[test]
    fn closed_observed_id_grammars_and_near_misses() {
        for id in [
            "claude-fable-5-1-20260901",
            "claude-opus-5-5",
            "claude-sonnet-5-5",
            "claude-haiku-5-5",
        ] {
            assert_eq!(provider(id), Some("Anthropic"));
        }
        for id in ["muse-spark-1.3", "muse-wonder-1.0"] {
            assert_eq!(provider(id), Some("Meta"));
        }
        assert_eq!(provider("gpt-5.6-luna"), Some("OpenAI"));
        for id in [
            "Opus 5 (1M context)",
            "Grok 4.6",
            "claude-opus-",
            "xclaude-fable-5",
            "gpt-sol",
            "muse-spark-",
            "gemini-4-argon",
        ] {
            assert_eq!(provider(id), None);
        }
    }
}
