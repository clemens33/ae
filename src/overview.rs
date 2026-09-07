//! The orchestrator's compact fleet overview.
//!
//! Collection stays in [`crate::watchdog_daemon`]: this module receives the
//! same [`crate::brief::Card`] facts as `ae brief --all` and owns only stable,
//! bounded presentation. No clock and no filesystem read enters this module.

use crate::brief::{self, Card, Need};

/// The final line of every overview turn.
pub const TRAILER: &str = "— overview; declare done.";

/// The maximum width of every rendered agent or collapsed-session line.
pub const WIDTH: usize = 100;

/// Indentation of every need below its session heading.
const DETAIL_INDENT: usize = 4;
const MAX_NEED_LINES: usize = 3;
const DETAIL_WIDTH: usize = 60;

#[derive(Debug, Clone)]
struct Row {
    order: usize,
    text: String,
}

#[derive(Debug, Clone)]
struct NeedRow {
    age_secs: Option<i64>,
    order: usize,
    text: String,
}

#[derive(Debug, Clone)]
struct NeedSession {
    oldest_age_secs: Option<i64>,
    order: usize,
    name: String,
    needs: Vec<NeedRow>,
}

/// Render `cards` as `NEEDS YOU`, `WORKING`, and `QUIET` sections.
///
/// `own_session` is omitted before any row is classified. Human needs are
/// grouped per session, oldest session first, then oldest need first. Rust's
/// stable sort preserves fleet and roster creation order among ties.
#[must_use]
pub fn render(cards: &[Card], own_session: &str) -> String {
    let mut needs = Vec::new();
    let mut working = Vec::new();
    let mut quiet = Vec::new();
    let mut order = 0_usize;

    for card in cards
        .iter()
        .filter(|card| card.name != own_session && card.status == "running")
    {
        let mut session_has_news = false;
        let mut session_needs = card
            .human_needs()
            .filter_map(|need| {
                let (owner, state, detail, age_secs) = human_need_parts(need)?;
                let row = NeedRow {
                    age_secs,
                    order,
                    text: needs_line(owner, state, &detail, age_secs),
                };
                order = order.saturating_add(1);
                Some(row)
            })
            .collect::<Vec<_>>();
        session_needs.sort_by(|left, right| {
            age_order(right.age_secs)
                .cmp(&age_order(left.age_secs))
                .then_with(|| left.order.cmp(&right.order))
        });
        if !session_needs.is_empty() {
            needs.push(NeedSession {
                oldest_age_secs: session_needs.first().and_then(|need| need.age_secs),
                order,
                name: card.name.clone(),
                needs: session_needs,
            });
            session_has_news = true;
            order = order.saturating_add(1);
        }

        let mut ask_count_shown = false;
        for agent in &card.agents {
            if agent.state == "working" {
                let open_asks = if ask_count_shown {
                    0
                } else {
                    card.open_ask_count()
                };
                working.push(Row {
                    order,
                    text: working_line(&card.name, &agent.name, working_detail(card), open_asks),
                });
                ask_count_shown = true;
                session_has_news = true;
            }
            order = order.saturating_add(1);
        }

        if !session_has_news {
            quiet.push(quiet_chunk(card));
        }
    }

    needs.sort_by(|left, right| {
        age_order(right.oldest_age_secs)
            .cmp(&age_order(left.oldest_age_secs))
            .then_with(|| left.order.cmp(&right.order))
    });
    working.sort_by_key(|row| row.order);

    let mut sections = Vec::new();
    push_need_sessions(&mut sections, &needs);
    push_rows(&mut sections, "WORKING", &working);
    if !quiet.is_empty() {
        let mut section = String::from("QUIET");
        for line in packed_quiet(&quiet) {
            section.push('\n');
            section.push_str(&line);
        }
        sections.push(section);
    }
    sections.join("\n")
}

/// The exact body pasted into the orchestrator pane.
#[must_use]
pub fn nudge_body(rendered: &str) -> String {
    if rendered.is_empty() {
        TRAILER.to_owned()
    } else {
        format!("{rendered}\n{TRAILER}")
    }
}

/// Return a stable dependency-free FNV-1a 64 digest of the semantic fleet
/// facts that decide an overview, never their elapsed ages. The display can
/// advance from `20m` to `22m` without spending a seat turn; a visible state,
/// leadership reason, open-request count, goal, or topic change cannot.
#[must_use]
pub fn semantic_hash(cards: &[Card], own_session: &str) -> String {
    let mut value = 0xcbf2_9ce4_8422_2325_u64;
    for card in cards
        .iter()
        .filter(|card| card.name != own_session && card.status == "running")
    {
        hash_field(&mut value, "session");
        hash_field(&mut value, &card.name);
        hash_field(&mut value, card.status);
        hash_optional(&mut value, card.goal.as_deref());
        for topic in &card.topics {
            hash_field(&mut value, "topic");
            hash_field(&mut value, &topic.topic);
            hash_field(&mut value, &topic.text);
        }
        let has_working = card.agents.iter().any(|agent| agent.state == "working");
        for agent in card.agents.iter().filter(|agent| agent.state == "working") {
            hash_field(&mut value, "working-agent");
            hash_field(&mut value, &agent.name);
            hash_field(&mut value, &agent.state);
        }
        let mut has_human_needs = false;
        for need in card.human_needs() {
            has_human_needs = true;
            if let Need::Declared {
                owner,
                state,
                reason,
                ..
            } = need
            {
                hash_field(&mut value, "declared");
                hash_field(&mut value, owner);
                hash_field(&mut value, state);
                hash_field(&mut value, reason);
            }
        }
        if !has_human_needs && !has_working {
            hash_field(&mut value, "quiet-state");
            hash_field(&mut value, quiet_state(card));
        }
        hash_field(&mut value, "open-asks");
        hash_field(&mut value, &card.open_ask_count().to_string());
        hash_field(&mut value, "end-session");
    }
    format!("{value:016x}")
}

fn hash_optional(value: &mut u64, field: Option<&str>) {
    match field {
        Some(field) => {
            hash_field(value, "some");
            hash_field(value, field);
        }
        None => hash_field(value, "none"),
    }
}

fn hash_field(value: &mut u64, field: &str) {
    let len = u64::try_from(field.len()).unwrap_or(u64::MAX);
    hash_bytes(value, &len.to_le_bytes());
    hash_bytes(value, field.as_bytes());
}

fn hash_bytes(value: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *value ^= u64::from(*byte);
        *value = value.wrapping_mul(0x0000_0100_0000_01b3);
    }
}

fn push_rows(sections: &mut Vec<String>, header: &str, rows: &[Row]) {
    if rows.is_empty() {
        return;
    }
    let mut section = header.to_owned();
    for row in rows {
        section.push('\n');
        section.push_str(&row.text);
    }
    sections.push(section);
}

fn push_need_sessions(sections: &mut Vec<String>, sessions: &[NeedSession]) {
    if sessions.is_empty() {
        return;
    }
    let mut section = String::from("NEEDS YOU");
    for session in sessions {
        section.push('\n');
        section.push_str(&need_session_line(&session.name, session.needs.len()));
        for need in &session.needs {
            section.push('\n');
            section.push_str(&need.text);
        }
    }
    sections.push(section);
}

fn human_need_parts(need: &Need) -> Option<(&str, &str, String, Option<i64>)> {
    match need {
        Need::Declared {
            owner,
            state,
            age_secs,
            reason,
        } => Some((
            owner,
            state,
            if reason.is_empty() {
                "no reason given".to_owned()
            } else {
                clean_text(reason)
            },
            *age_secs,
        )),
        Need::Unanswered { .. } => None,
    }
}

fn age_order(age_secs: Option<i64>) -> i64 {
    age_secs.unwrap_or(i64::MIN)
}

fn working_detail(card: &Card) -> &str {
    card.goal
        .as_deref()
        .filter(|goal| !goal.is_empty())
        .or_else(|| {
            card.topics
                .first()
                .map(|topic| topic.text.as_str())
                .filter(|text| !text.is_empty())
        })
        .unwrap_or("-")
}

fn need_session_line(session: &str, count: usize) -> String {
    let suffix = format!(" ({count})");
    let room = WIDTH.saturating_sub(2 + suffix.chars().count());
    format!("  {}{suffix}", clipped(session, room))
}

fn needs_line(owner: &str, state: &str, detail: &str, age_secs: Option<i64>) -> String {
    let mut prefix = " ".repeat(DETAIL_INDENT);
    push_field(&mut prefix, &clipped(owner, 18), 10);
    push_field(&mut prefix, &clipped(state, 14), 12);
    push_field(&mut prefix, &brief::age(age_secs), 4);
    let first_width = WIDTH.saturating_sub(prefix.chars().count());
    let mut lines = wrap_detail(detail, first_width, WIDTH - DETAIL_INDENT, MAX_NEED_LINES);
    if lines.is_empty() {
        lines.push(String::new());
    }
    let first = lines.remove(0);
    let mut out = format!("{prefix}{first}");
    let indent = " ".repeat(DETAIL_INDENT);
    for line in lines {
        out.push('\n');
        out.push_str(&indent);
        out.push_str(&line);
    }
    out
}

fn wrap_detail(
    detail: &str,
    first_width: usize,
    continuation_width: usize,
    max_lines: usize,
) -> Vec<String> {
    let mut lines = Vec::new();
    let mut rest = detail;
    let mut width = first_width;
    while !rest.is_empty() && lines.len() < max_lines {
        let take = rest.chars().count().min(width);
        let boundary = (rest.chars().count() > width)
            .then(|| {
                rest.char_indices()
                    .take(take + 1)
                    .filter(|(_, ch)| ch.is_whitespace())
                    .map(|(index, _)| index)
                    .filter(|index| rest[..*index].chars().count() <= width)
                    .filter(|index| rest[..*index].chars().count() >= width / 2)
                    .fold(None, |_, index| Some(index))
            })
            .flatten();
        let end = boundary.unwrap_or_else(|| {
            rest.char_indices()
                .nth(take)
                .map_or(rest.len(), |(index, _)| index)
        });
        let mut line = rest[..end].trim_end().to_owned();
        rest = rest[end..].trim_start();
        if !rest.is_empty() && lines.len() + 1 == max_lines {
            line = line.chars().take(width.saturating_sub(1)).collect();
            line.push('…');
            rest = "";
        }
        lines.push(line);
        width = continuation_width;
    }
    lines
}

fn clean_text(text: &str) -> String {
    let mut clean = String::with_capacity(text.len());
    let mut in_run = false;
    for ch in text.chars() {
        if ch.is_control() || ch.is_whitespace() {
            if !in_run {
                clean.push(' ');
                in_run = true;
            }
        } else {
            clean.push(ch);
            in_run = false;
        }
    }
    clean.trim().to_owned()
}

fn working_line(session: &str, agent: &str, detail: &str, open_asks: usize) -> String {
    let mut prefix = String::from("  ");
    push_field(&mut prefix, &clipped(session, 28), 12);
    push_field(&mut prefix, &clipped(agent, 20), 12);
    let remaining = WIDTH.saturating_sub(prefix.chars().count());
    let suffix = match open_asks {
        0 => String::new(),
        1 => " (1 open ask)".to_owned(),
        count => format!(" ({count} open asks)"),
    };
    let detail_width = DETAIL_WIDTH.min(remaining.saturating_sub(suffix.chars().count()));
    prefix.push_str(&clipped(detail, detail_width));
    prefix.push_str(&suffix);
    prefix
}

fn quiet_chunk(card: &Card) -> String {
    let age = match card.agents.as_slice() {
        [] => None,
        [only] => only.age_secs,
        agents => agents.iter().filter_map(|agent| agent.age_secs).min(),
    };
    clipped(
        &format!("{} ({} {})", card.name, quiet_state(card), brief::age(age)),
        WIDTH - 2,
    )
}

fn quiet_state(card: &Card) -> &str {
    match card.agents.as_slice() {
        [] => "no agents",
        [only] => only.state.as_str(),
        agents => {
            let first = agents[0].state.as_str();
            if agents.iter().all(|agent| agent.state == first) {
                first
            } else {
                "quiet"
            }
        }
    }
}

fn packed_quiet(chunks: &[String]) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::from("  ");
    for chunk in chunks {
        let gap = if line == "  " { 0 } else { 3 };
        if line.chars().count() + gap + chunk.chars().count() > WIDTH && line != "  " {
            lines.push(line);
            line = String::from("  ");
        }
        if line != "  " {
            line.push_str("   ");
        }
        line.push_str(chunk);
    }
    if line != "  " {
        lines.push(line);
    }
    lines
}

fn push_field(out: &mut String, field: &str, width: usize) {
    out.push_str(field);
    for _ in field.chars().count()..width {
        out.push(' ');
    }
    out.push_str("  ");
}

fn clipped(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    if max == 0 {
        return String::new();
    }
    let mut out: String = text.chars().take(max - 1).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::{TRAILER, WIDTH, nudge_body, render, semantic_hash};
    use crate::attention::Reason;
    use crate::brief::{AgentLine, Card, Need, TopicLine};

    fn agent(name: &str, state: &str, age_secs: i64, attention: Option<Reason>) -> AgentLine {
        AgentLine {
            name: name.to_owned(),
            state: state.to_owned(),
            age_secs: Some(age_secs),
            reason: String::new(),
            attention,
        }
    }

    fn card(name: &str, agents: Vec<AgentLine>) -> Card {
        let main = agents.first().map(|agent| agent.name.clone());
        Card {
            name: name.to_owned(),
            status: "running",
            main,
            attention: None,
            ae_version: None,
            branch: None,
            dirty: false,
            work_dir: None,
            goal: None,
            topics: Vec::new(),
            agents,
            needs: Vec::new(),
            degraded: false,
            memo_unreadable: false,
        }
    }

    #[test]
    fn each_nonempty_section_is_rendered_and_empty_ones_are_omitted() {
        let mut blocked = card(
            "alpha",
            vec![agent(
                "lead",
                "waiting-user",
                720,
                Some(Reason::WaitingUser),
            )],
        );
        blocked.agents[0].reason = "choose blue or green".to_owned();
        blocked.needs.push(Need::Declared {
            owner: "lead".to_owned(),
            state: "waiting-user".to_owned(),
            age_secs: Some(720),
            reason: "choose blue or green".to_owned(),
        });
        let mut working = card("beta", vec![agent("builder", "working", 30, None)]);
        working.goal = Some("ship the parser".to_owned());
        let quiet = card("gamma", vec![agent("lead", "done", 1_200, None)]);

        let text = render(&[blocked, working, quiet], "orchestrator");
        assert!(text.contains("NEEDS YOU\n"), "{text}");
        assert!(text.contains("  alpha (1)"), "{text}");
        assert!(
            text.contains("lead        waiting-user  12m   choose blue or green"),
            "{text}"
        );
        assert!(text.contains("WORKING\n"), "{text}");
        assert!(text.contains("beta"), "{text}");
        assert!(text.contains("ship the parser"), "{text}");
        assert!(text.contains("QUIET\n  gamma (done 20m)"), "{text}");

        let only_work = render(
            &[card("beta", vec![agent("builder", "working", 30, None)])],
            "x",
        );
        assert!(!only_work.contains("NEEDS YOU"), "{only_work}");
        assert!(!only_work.contains("QUIET"), "{only_work}");
    }

    #[test]
    fn unanswered_requests_are_only_a_count_on_the_working_session() {
        let mut pending = card("dotfiles", vec![agent("lead", "working", 0, None)]);
        pending.needs.push(Need::Unanswered {
            kind: "ask".to_owned(),
            reference: "ae-20260907T000000Z-9d07aac0".to_owned(),
            from: "reviewer".to_owned(),
            to: "lead".to_owned(),
            age_secs: 86_400,
            question: "ignored\r\nin\u{1b}[31m the compact overview".to_owned(),
        });
        pending.needs.push(Need::Unanswered {
            kind: "review".to_owned(),
            reference: "ae-second".to_owned(),
            from: "lead".to_owned(),
            to: "reviewer".to_owned(),
            age_secs: 60,
            question: "also hidden".to_owned(),
        });
        let text = render(&[pending], "orchestrator");
        assert!(
            text.contains("dotfiles      lead          - (2 open asks)"),
            "{text}"
        );
        assert!(!text.contains("NEEDS YOU"), "{text}");
        assert!(!text.contains("9d07aac0"), "{text}");
        assert!(!text.contains("compact overview"), "{text}");

        let without = render(
            &[card("dotfiles", vec![agent("lead", "working", 0, None)])],
            "orchestrator",
        );
        assert!(!without.contains("open ask"), "{without}");
    }

    #[test]
    fn explicit_main_not_first_and_colead_share_one_session_heading() {
        let mut pending = card(
            "alpha",
            vec![
                agent("worker", "waiting-user", 3_600, None),
                agent("captain", "waiting-user", 0, None),
                agent("colead", "blocked", 1_800, None),
            ],
        );
        pending.main = Some("captain".to_owned());
        pending.needs.push(Need::Declared {
            owner: "captain".to_owned(),
            state: "waiting-user".to_owned(),
            age_secs: Some(0),
            reason: "main decision".to_owned(),
        });
        pending.needs.push(Need::Declared {
            owner: "worker".to_owned(),
            state: "waiting-user".to_owned(),
            age_secs: Some(3_600),
            reason: "internal worker blocker".to_owned(),
        });
        pending.needs.push(Need::Declared {
            owner: "colead".to_owned(),
            state: "blocked".to_owned(),
            age_secs: Some(1_800),
            reason: "co-lead gate".to_owned(),
        });

        let text = render(&[pending], "orchestrator");
        assert!(text.contains("  alpha (2)"), "{text}");
        assert!(
            text.contains("colead      blocked       30m   co-lead gate"),
            "{text}"
        );
        assert!(
            text.contains("captain     waiting-user  0s    main decision"),
            "{text}"
        );
        assert!(!text.contains("worker"), "{text}");
        assert!(!text.contains("internal worker blocker"), "{text}");
    }

    #[test]
    fn a_180_character_reason_keeps_its_last_decision_token_within_three_lines() {
        let reason = format!("{} decision", "x".repeat(171));
        assert_eq!(reason.chars().count(), 180);
        let mut entry = card(
            "a".repeat(28).as_str(),
            vec![agent("lead", "waiting-user", 99_999, None)],
        );
        entry.needs.push(Need::Declared {
            owner: "lead".to_owned(),
            state: "waiting-user".to_owned(),
            age_secs: Some(99_999),
            reason: reason.clone(),
        });
        let text = render(&[entry], "orchestrator");
        assert!(text.contains("decision"), "{text}");
        assert!(!text.contains('…'), "{text}");
        let need_lines = text.lines().skip(2).count();
        assert!(need_lines <= 3, "{need_lines} need lines:\n{text}");
        assert!(
            text.lines().all(|line| line.chars().count() <= WIDTH),
            "{text}"
        );
    }

    #[test]
    fn hostile_controls_in_a_reason_are_flattened() {
        let mut entry = card("alpha", vec![agent("lead", "blocked", 60, None)]);
        entry.needs.push(Need::Declared {
            owner: "lead".to_owned(),
            state: "blocked".to_owned(),
            age_secs: Some(60),
            reason: "choose\r\nblue \u{1b}[31mdecision".to_owned(),
        });
        let text = render(&[entry], "orchestrator");
        assert!(text.contains("choose blue [31mdecision"), "{text}");
        assert!(
            text.chars()
                .filter(|ch| *ch != '\n')
                .all(|ch| !ch.is_control()),
            "{text:?}"
        );
    }

    #[test]
    fn quiet_sessions_collapse_and_wrap_without_one_row_per_agent() {
        let alpha = card(
            "alpha",
            vec![
                agent("lead", "done", 1_200, None),
                agent("reviewer", "done", 1_800, None),
            ],
        );
        let beta = card("beta", vec![agent("lead", "done", 7_200, None)]);
        let text = render(&[alpha, beta], "orchestrator");
        assert_eq!(text, "QUIET\n  alpha (done 20m)   beta (done 2h)");
        assert!(!text.contains("reviewer"), "{text}");
    }

    #[test]
    fn own_session_is_excluded_and_an_empty_fleet_renders_nothing() {
        let own = card("orchestrator", vec![agent("lead", "working", 0, None)]);
        assert_eq!(render(&[own], "orchestrator"), "");
    }

    #[test]
    fn sessions_and_their_needs_sort_oldest_first() {
        let mut recent = card(
            "recent",
            vec![
                agent("lead", "waiting-user", 60, None),
                agent("colead", "blocked", 1_200, None),
            ],
        );
        recent.needs = vec![
            Need::Declared {
                owner: "lead".to_owned(),
                state: "waiting-user".to_owned(),
                age_secs: Some(60),
                reason: "newer".to_owned(),
            },
            Need::Declared {
                owner: "colead".to_owned(),
                state: "blocked".to_owned(),
                age_secs: Some(1_200),
                reason: "older".to_owned(),
            },
        ];
        let mut oldest = card("oldest", vec![agent("lead", "blocked", 7_200, None)]);
        oldest.needs.push(Need::Declared {
            owner: "lead".to_owned(),
            state: "blocked".to_owned(),
            age_secs: Some(7_200),
            reason: "oldest need".to_owned(),
        });
        let text = render(&[recent, oldest], "orchestrator");
        let oldest_session = text.find("  oldest (1)").unwrap_or(usize::MAX);
        let recent_session = text.find("  recent (2)").unwrap_or(usize::MAX);
        let older_need = text.find("older").unwrap_or(usize::MAX);
        let newer_need = text.find("newer").unwrap_or(usize::MAX);
        assert!(oldest_session < recent_session, "{text}");
        assert!(older_need < newer_need, "{text}");
    }

    #[test]
    fn every_data_line_is_bounded_even_with_long_unicode_facts() {
        let long = "界".repeat(180);
        let mut entry = card(&long, vec![agent(&long, "working", 0, None)]);
        entry.goal = Some(long);
        let text = render(&[entry], "orchestrator");
        for line in text.lines().filter(|line| line.starts_with("  ")) {
            assert!(
                line.chars().count() <= WIDTH,
                "{}: {line}",
                line.chars().count()
            );
        }
    }

    #[test]
    fn body_is_exact() {
        assert_eq!(
            nudge_body("WORKING\n  alpha lead ship"),
            format!("WORKING\n  alpha lead ship\n{TRAILER}")
        );
        assert_eq!(nudge_body(""), TRAILER);
    }

    fn hash_card() -> Card {
        let mut pending = card("alpha", vec![agent("lead", "waiting-user", 1_200, None)]);
        pending.agents[0].reason = "choose blue".to_owned();
        pending.topics.push(TopicLine {
            topic: "goal".to_owned(),
            age_secs: Some(60),
            author: "lead".to_owned(),
            text: "ship overview".to_owned(),
        });
        pending.needs.push(Need::Declared {
            owner: "lead".to_owned(),
            state: "waiting-user".to_owned(),
            age_secs: Some(1_200),
            reason: "choose blue".to_owned(),
        });
        pending.needs.push(Need::Unanswered {
            kind: "review".to_owned(),
            reference: "ae-20260907T000000Z-first".to_owned(),
            from: "reviewer".to_owned(),
            to: "lead".to_owned(),
            age_secs: 60,
            question: "check it".to_owned(),
        });
        pending
    }

    #[test]
    fn semantic_hash_ignores_display_ages() {
        let original = vec![hash_card()];
        let mut later = original.clone();
        later[0].agents[0].age_secs = Some(1_320);
        later[0].topics[0].age_secs = Some(180);
        if let Need::Declared { age_secs, .. } = &mut later[0].needs[0] {
            *age_secs = Some(1_320);
        }
        if let Need::Unanswered { age_secs, .. } = &mut later[0].needs[1] {
            *age_secs = 180;
        }

        assert_ne!(
            render(&original, "orchestrator"),
            render(&later, "orchestrator"),
            "the displayed ages still advance"
        );
        assert_eq!(
            semantic_hash(&original, "orchestrator"),
            semantic_hash(&later, "orchestrator"),
            "elapsed time alone never spends a seat turn"
        );
    }

    #[test]
    fn semantic_hash_changes_when_a_leadership_reason_changes() {
        let original = vec![hash_card()];
        let mut changed_reason = original.clone();
        changed_reason[0].agents[0].reason = "choose green".to_owned();
        if let Need::Declared { reason, .. } = &mut changed_reason[0].needs[0] {
            *reason = "choose green".to_owned();
        }
        assert_ne!(
            semantic_hash(&original, "orchestrator"),
            semantic_hash(&changed_reason, "orchestrator"),
            "a reason change wakes the seat"
        );
    }

    #[test]
    fn semantic_hash_ignores_an_open_ask_body() {
        let original = vec![hash_card()];
        let mut changed_body = original.clone();
        if let Need::Unanswered { question, .. } = &mut changed_body[0].needs[1] {
            *question = "different body".to_owned();
        }
        assert_eq!(
            semantic_hash(&original, "orchestrator"),
            semantic_hash(&changed_body, "orchestrator"),
            "request bodies never spend a seat turn"
        );
    }

    #[test]
    fn semantic_hash_changes_with_the_open_ask_count() {
        let original = vec![hash_card()];
        let mut changed_count = original.clone();
        changed_count[0].needs.push(Need::Unanswered {
            kind: "ask".to_owned(),
            reference: "ae-second".to_owned(),
            from: "worker".to_owned(),
            to: "lead".to_owned(),
            age_secs: 0,
            question: "another body".to_owned(),
        });
        assert_ne!(
            semantic_hash(&original, "orchestrator"),
            semantic_hash(&changed_count, "orchestrator"),
            "a changed open-ask count wakes the seat"
        );
    }

    #[test]
    fn semantic_hash_ignores_a_hidden_worker_block() {
        let original = vec![card(
            "alpha",
            vec![
                agent("lead", "working", 0, None),
                agent("worker", "done", 60, None),
            ],
        )];
        let mut worker_blocked = original.clone();
        worker_blocked[0].agents[1].state = "blocked".to_owned();
        worker_blocked[0].agents[1].reason = "internal worker blocker".to_owned();
        worker_blocked[0].agents[1].attention = Some(Reason::Blocked);
        worker_blocked[0].attention = Some(Reason::Blocked);
        worker_blocked[0].needs.push(Need::Declared {
            owner: "worker".to_owned(),
            state: "blocked".to_owned(),
            age_secs: Some(0),
            reason: "internal worker blocker".to_owned(),
        });

        assert_eq!(
            render(&original, "orchestrator"),
            render(&worker_blocked, "orchestrator"),
            "worker needs stay absent from the overview"
        );
        assert_eq!(
            semantic_hash(&original, "orchestrator"),
            semantic_hash(&worker_blocked, "orchestrator"),
            "a hidden worker need never spends a seat turn"
        );
    }

    #[test]
    fn latest_memo_supplies_work_when_a_goal_is_absent() {
        let mut entry = card("alpha", vec![agent("lead", "working", 0, None)]);
        entry.topics.push(TopicLine {
            topic: "parking".to_owned(),
            age_secs: Some(10),
            author: "lead".to_owned(),
            text: "resume here: gate the renderer".to_owned(),
        });
        assert!(render(&[entry], "orchestrator").contains("resume here: gate the renderer"));
    }
}
