//! The orchestrator's compact fleet overview.
//!
//! Collection stays in [`crate::watchdog_daemon`]: this module receives the
//! same [`crate::brief::Card`] facts as `ae brief --all` and owns only stable,
//! bounded presentation. No clock and no filesystem read enters this module.

use crate::attention::Reason;
use crate::brief::{self, AgentLine, Card, Need};

/// The final line of every overview turn.
pub const TRAILER: &str = "— overview; declare done.";

/// The maximum width of every rendered agent or collapsed-session line.
pub const WIDTH: usize = 100;

/// The maximum free-text detail shown on one line.
const DETAIL_WIDTH: usize = 60;

#[derive(Debug, Clone)]
struct Row {
    rank: i64,
    order: usize,
    text: String,
}

/// Render `cards` as `NEEDS YOU`, `WORKING`, and `QUIET` sections.
///
/// `own_session` is omitted before any row is classified. Rows are sorted by
/// attention severity; Rust's stable sort preserves the fleet and roster
/// creation order among ties.
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
        for agent in &card.agents {
            if let Some((rank, state, detail, age)) = need_for(card, agent) {
                needs.push(Row {
                    rank,
                    order,
                    text: needs_line(&identity(&card.name, &agent.name), &state, &detail, age),
                });
                session_has_news = true;
            } else if agent.state == "working" {
                working.push(Row {
                    rank: 0,
                    order,
                    text: working_line(&card.name, &agent.name, working_detail(card)),
                });
                session_has_news = true;
            }
            order = order.saturating_add(1);
        }

        // Hostile or mid-write state can leave a need whose owner is absent
        // from the roster. Keep the claim visible, still one line per named
        // owner, instead of collapsing the session as quiet.
        for need in card.needs.iter().filter(|need| !need_has_agent(need, card)) {
            let (owner, state, detail, age, rank) = need_parts(need);
            needs.push(Row {
                rank,
                order,
                text: needs_line(&identity(&card.name, owner), state, &detail, age),
            });
            session_has_news = true;
            order = order.saturating_add(1);
        }

        if !session_has_news {
            quiet.push(quiet_chunk(card));
        }
    }

    needs.sort_by(|left, right| {
        right
            .rank
            .cmp(&left.rank)
            .then_with(|| left.order.cmp(&right.order))
    });
    working.sort_by_key(|row| row.order);

    let mut sections = Vec::new();
    push_rows(&mut sections, "NEEDS YOU", &needs);
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
/// advance from `20m` to `22m` without spending a seat turn; a state, reason,
/// request, goal, topic, or attention change cannot.
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
        hash_attention(&mut value, card.attention);
        hash_optional(&mut value, card.goal.as_deref());
        for topic in &card.topics {
            hash_field(&mut value, "topic");
            hash_field(&mut value, &topic.topic);
            hash_field(&mut value, &topic.text);
        }
        for agent in &card.agents {
            hash_field(&mut value, "agent");
            hash_field(&mut value, &agent.name);
            hash_field(&mut value, &agent.state);
            hash_field(&mut value, &agent.reason);
            hash_attention(&mut value, agent.attention);
        }
        for need in &card.needs {
            match need {
                Need::Declared {
                    owner,
                    state,
                    reason,
                    ..
                } => {
                    hash_field(&mut value, "declared");
                    hash_field(&mut value, owner);
                    hash_field(&mut value, state);
                    hash_field(&mut value, reason);
                }
                Need::Unanswered {
                    kind,
                    reference,
                    from,
                    to,
                    question,
                    ..
                } => {
                    hash_field(&mut value, "unanswered");
                    hash_field(&mut value, kind);
                    hash_field(&mut value, reference);
                    hash_field(&mut value, from);
                    hash_field(&mut value, to);
                    hash_field(&mut value, question);
                }
            }
        }
        hash_field(&mut value, "end-session");
    }
    format!("{value:016x}")
}

fn hash_attention(value: &mut u64, attention: Option<Reason>) {
    hash_optional(value, attention.map(Reason::as_str));
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

fn need_for(card: &Card, agent: &AgentLine) -> Option<(i64, String, String, Option<i64>)> {
    let explicit = card
        .needs
        .iter()
        .filter(|need| need_owner(need) == agent.name)
        .max_by_key(|need| need_rank(need));
    let watchdog = agent.attention.map(|reason| {
        (
            reason.rank(),
            reason.as_str().to_owned(),
            watchdog_detail(reason, agent),
            agent.age_secs,
        )
    });
    let explicit = explicit.map(|need| {
        let (_, state, detail, age, rank) = need_parts(need);
        (rank, state.to_owned(), detail, age)
    });
    match (watchdog, explicit) {
        // On an equal class the explicit row carries the useful request id or
        // declaration reason, while the rollup can only repeat the class.
        (Some(left), Some(right)) if right.0 >= left.0 => Some(right),
        (Some(left), _) => Some(left),
        (None, right) => right,
    }
}

fn watchdog_detail(reason: Reason, agent: &AgentLine) -> String {
    if matches!(reason, Reason::WaitingUser | Reason::Blocked) && !agent.reason.is_empty() {
        agent.reason.clone()
    } else {
        reason.as_str().to_owned()
    }
}

fn need_has_agent(need: &Need, card: &Card) -> bool {
    card.agents
        .iter()
        .any(|agent| agent.name == need_owner(need))
}

fn need_owner(need: &Need) -> &str {
    match need {
        Need::Declared { owner, .. } => owner,
        Need::Unanswered { to, .. } => to,
    }
}

fn need_rank(need: &Need) -> i64 {
    match need {
        Need::Declared { state, .. } if state == "waiting-user" => Reason::WaitingUser.rank(),
        Need::Declared { state, .. } if state == "blocked" => Reason::Blocked.rank(),
        Need::Declared { .. } => 0,
        Need::Unanswered { .. } => Reason::Unanswered.rank(),
    }
}

fn need_parts(need: &Need) -> (&str, &str, String, Option<i64>, i64) {
    match need {
        Need::Declared {
            owner,
            state,
            age_secs,
            reason,
        } => (
            owner,
            state,
            if reason.is_empty() {
                "no reason given".to_owned()
            } else {
                reason.clone()
            },
            *age_secs,
            need_rank(need),
        ),
        Need::Unanswered {
            kind,
            reference,
            from,
            to,
            age_secs,
            ..
        } => (
            to,
            "unanswered",
            format!("{kind} {reference} from {from}"),
            Some(*age_secs),
            need_rank(need),
        ),
    }
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

fn needs_line(identity: &str, state: &str, detail: &str, age_secs: Option<i64>) -> String {
    let identity = clipped(identity, 28);
    let state = clipped(state, 14);
    let suffix = format!(" ({})", brief::age(age_secs));
    let mut prefix = String::from("  ");
    push_field(&mut prefix, &identity, 18);
    push_field(&mut prefix, &state, 14);
    let remaining = WIDTH
        .saturating_sub(prefix.chars().count())
        .saturating_sub(suffix.chars().count());
    let detail = clipped(detail, DETAIL_WIDTH.min(remaining));
    format!("{prefix}{detail}{suffix}")
}

fn working_line(session: &str, agent: &str, detail: &str) -> String {
    let mut prefix = String::from("  ");
    push_field(&mut prefix, &clipped(session, 28), 12);
    push_field(&mut prefix, &clipped(agent, 20), 12);
    let remaining = WIDTH.saturating_sub(prefix.chars().count());
    prefix.push_str(&clipped(detail, DETAIL_WIDTH.min(remaining)));
    prefix
}

fn quiet_chunk(card: &Card) -> String {
    let (state, age) = match card.agents.as_slice() {
        [] => ("no agents", None),
        [only] => (only.state.as_str(), only.age_secs),
        agents => {
            let first = agents[0].state.as_str();
            let common = agents.iter().all(|agent| agent.state == first);
            let age = agents.iter().filter_map(|agent| agent.age_secs).min();
            (if common { first } else { "quiet" }, age)
        }
    };
    clipped(
        &format!("{} ({state} {})", card.name, brief::age(age)),
        WIDTH - 2,
    )
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

fn identity(session: &str, agent: &str) -> String {
    format!("{session}:{agent}")
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
        Card {
            name: name.to_owned(),
            status: "running",
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
        assert!(text.contains("alpha:lead"), "{text}");
        assert!(text.contains("choose blue or green (12m)"), "{text}");
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
    fn unanswered_rows_name_the_request_and_sender() {
        let mut pending = card("dotfiles", vec![agent("lead", "working", 0, None)]);
        pending.needs.push(Need::Unanswered {
            kind: "ask".to_owned(),
            reference: "ae-20260907T000000Z-9d07aac0".to_owned(),
            from: "reviewer".to_owned(),
            to: "lead".to_owned(),
            age_secs: 86_400,
            question: "ignored in the compact overview".to_owned(),
        });
        let text = render(&[pending], "orchestrator");
        assert!(
            text.contains("ask ae-20260907T000000Z-9d07aac0 from reviewer (1d)"),
            "{text}"
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
    fn needs_sort_by_rank_then_keep_input_and_roster_order() {
        let stale = card(
            "first",
            vec![
                agent("one", "working", 60, Some(Reason::Stale)),
                agent("two", "working", 120, Some(Reason::Stale)),
            ],
        );
        let dead = card(
            "second",
            vec![agent("lead", "working", 10, Some(Reason::Dead))],
        );
        let text = render(&[stale, dead], "orchestrator");
        let rows: Vec<&str> = text.lines().skip(1).collect();
        assert!(rows[0].contains("second:lead"), "{text}");
        assert!(rows[1].contains("first:one"), "{text}");
        assert!(rows[2].contains("first:two"), "{text}");
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

    #[test]
    fn semantic_hash_ignores_clock_only_changes_but_tracks_state_and_requests() {
        let mut pending = card("alpha", vec![agent("lead", "done", 1_200, None)]);
        pending.topics.push(TopicLine {
            topic: "goal".to_owned(),
            age_secs: Some(60),
            author: "lead".to_owned(),
            text: "ship overview".to_owned(),
        });
        pending.needs.push(Need::Unanswered {
            kind: "review".to_owned(),
            reference: "ae-20260907T000000Z-first".to_owned(),
            from: "reviewer".to_owned(),
            to: "lead".to_owned(),
            age_secs: 60,
            question: "check it".to_owned(),
        });
        let original = vec![pending];
        let mut later = original.clone();
        later[0].agents[0].age_secs = Some(1_320);
        later[0].topics[0].age_secs = Some(180);
        if let Need::Unanswered { age_secs, .. } = &mut later[0].needs[0] {
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

        later[0].agents[0].state = "working".to_owned();
        assert_ne!(
            semantic_hash(&original, "orchestrator"),
            semantic_hash(&later, "orchestrator"),
            "a real state change wakes the seat"
        );
        later[0].agents[0].state = "done".to_owned();
        if let Need::Unanswered { reference, .. } = &mut later[0].needs[0] {
            *reference = "ae-20260907T000000Z-second".to_owned();
        }
        assert_ne!(
            semantic_hash(&original, "orchestrator"),
            semantic_hash(&later, "orchestrator"),
            "a different open request wakes the seat"
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
