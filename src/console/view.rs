//! The console lane as text, and the memory a follow needs so each tick prints
//! only what is new.
//!
//! One code path serves the one-shot and every follow tick: [`Printed::step`]
//! takes the WHOLE lane, prints what it has not printed yet, and says when a
//! card or prompt it printed no longer stands. Every field — actor, target,
//! reason, coverage, tag, body — reaches the terminal through the board's
//! terminal renderer, applied once over the finished text.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use super::lane::{Item, Kind, Lane, Seat};
use crate::board::{clock_text, terminal_text};

/// The first line of every console: what it is, and what it is not yet.
#[must_use]
pub fn header(session: &str, seats: &[Seat]) -> String {
    let names: Vec<&str> = seats.iter().map(|seat| seat.name.as_str()).collect();
    terminal_text(&format!(
        "console: {session} — lead pair {} · preview of existing data: transcript turns are mirrors, chat-bridge replies are 600-character summaries, nothing here submits or answers · scope: current conversations plus recorded predecessors, board limits; agent-to-agent turns are not shown\n",
        names.join(", ")
    ))
}

/// The tag a row wears after its time.
pub(super) fn tag(kind: &Kind) -> String {
    let prior = |generation: u8| match generation {
        0 => String::new(),
        n => format!(" · prior {n}"),
    };
    match kind {
        Kind::Pane { seat, generation } => {
            format!("{seat} pane (transcript){}", prior(*generation))
        }
        Kind::Assistant { seat, generation } => {
            format!("{seat} assistant (transcript){}", prior(*generation))
        }
        Kind::Inbound { from, to } => format!("{from} → {to}"),
        Kind::Reply { from, to } => format!("{from} → {to} · preview (600-char summary)"),
        Kind::Said { who } => format!("said {who}"),
        Kind::Card { seat } => format!("DECISION {seat} · waiting-user"),
        Kind::NeedsYou { seat } => format!("NEEDS YOU {seat} · needs you in its pane"),
        Kind::Asked { to, id, uncertain } => match uncertain {
            true => format!("you → {to} · {id} · uncertain: check the {to} pane"),
            false => format!("you → {to} · {id}"),
        },
        Kind::NotDelivered { to, id } => format!("you → {to} · {id} · not delivered"),
        Kind::Closed { id } => format!("you closed {id}"),
        Kind::Answer {
            seat,
            id,
            follow_up,
            late,
            gap,
        } => {
            let mut tag = format!("{seat} answers {id}");
            if *follow_up > 0 {
                let _ = write!(tag, " · follow-up {follow_up}");
            }
            if *late {
                tag.push_str(" · late (closed)");
            }
            if let Some(gap) = gap {
                let _ = write!(tag, " · {gap} · preview (600-char summary)");
            }
            tag
        }
        Kind::Unadmitted { from, id, why } => {
            format!("{from} reply to {id} · not admitted: {why} · preview (600-char summary)")
        }
    }
}

/// The memory key of a console-thread row: its record's position in the read.
const CONSOLE_KEY: &str = "console record ";

/// What a console has already printed: rows by multiplicity (two records of
/// equal words in one second are two rows), the cards and prompts it printed
/// that may yet stop standing, the lane-derived gaps it named, and the day.
#[derive(Debug, Default)]
pub struct Printed {
    shown: BTreeMap<String, usize>,
    open: BTreeMap<String, String>,
    gaps: BTreeSet<String>,
    day: Option<String>,
}

impl Printed {
    /// Print what `lane` holds that this console has not: `board_gaps` is how
    /// many leading coverage rows are the board's own — its follow already
    /// names each once, so they always print — and the rest are named once.
    pub fn step(&mut self, lane: &Lane, board_gaps: usize, settled: bool) -> String {
        let mut out = String::new();
        for (index, gap) in lane.coverage.iter().enumerate() {
            if index < board_gaps || self.gaps.insert(gap.clone()) {
                let _ = writeln!(out, "coverage incomplete: {gap}");
            }
        }
        let (mut in_lane, mut standing) = (BTreeMap::new(), BTreeMap::new());
        for item in &lane.items {
            let key = match &item.record {
                Some(record) => format!("{CONSOLE_KEY}{record}"),
                None => format!("{}|{:?}|{}", item.micros, item.kind, item.body),
            };
            let nth: &mut usize = in_lane.entry(key.clone()).or_default();
            *nth += 1;
            if matches!(item.kind, Kind::Card { .. } | Kind::NeedsYou { .. }) {
                standing.insert(key.clone(), tag(&item.kind));
            }
            let printed = self.shown.entry(key).or_default();
            if *nth > *printed {
                *printed += 1;
                self.row(item, &mut out);
            }
        }
        // Only a lane whose journal was READ can say a card stopped standing; a
        // card that closes and stands again is news, so it is forgotten as printed.
        if settled {
            for (key, label) in &self.open {
                if !standing.contains_key(key) {
                    let _ = writeln!(out, "-- closed: {label} is no longer standing\n");
                    self.shown.remove(key);
                }
            }
            self.open = standing;
        }
        terminal_text(&out)
    }

    /// Forget the printed console-thread rows, whose positions a rewritten journal
    /// (`before` records, `now` now) no longer names; every other memory stays.
    pub fn rebase(&mut self, before: usize, now: usize) -> String {
        self.shown.retain(|key, _| !key.starts_with(CONSOLE_KEY));
        terminal_text(&format!(
            "-- journal rewritten ({before} records before, {now} now): the console thread is shown again as the journal stands\n\n"
        ))
    }

    fn row(&mut self, item: &Item, out: &mut String) {
        let (day, time) = clock_text(item.micros);
        if self.day.as_deref() != Some(day.as_str()) {
            let _ = writeln!(out, "# {day} UTC");
            self.day = Some(day);
        }
        let _ = writeln!(out, "## {time} {}", tag(&item.kind));
        for line in item.body.lines() {
            let _ = writeln!(out, "  {line}");
        }
        out.push('\n');
    }
}

#[cfg(test)]
mod tests {
    use super::{Printed, header};
    use crate::console::lane::{Item, Kind, Lane, Seat};

    const T: i64 = 1_790_748_060_000_000;

    fn item(kind: Kind, micros: i64, body: &str) -> Item {
        Item {
            micros,
            kind,
            body: body.to_owned(),
            record: None,
        }
    }

    fn said(who: &str) -> Kind {
        Kind::Said {
            who: who.to_owned(),
        }
    }

    fn card(seat: &str) -> Kind {
        Kind::Card {
            seat: seat.to_owned(),
        }
    }

    fn lane(items: Vec<Item>, coverage: &[&str]) -> Lane {
        Lane {
            items,
            coverage: coverage.iter().map(|gap| (*gap).to_owned()).collect(),
        }
    }

    #[test]
    fn every_kind_wears_its_tag_and_a_body_keeps_its_tabs_and_indentation() {
        let kinds = [
            (
                Kind::Pane {
                    seat: "lead".to_owned(),
                    generation: 1,
                },
                "lead pane (transcript) · prior 1",
            ),
            (
                Kind::Assistant {
                    seat: "colead".to_owned(),
                    generation: 0,
                },
                "colead assistant (transcript)",
            ),
            (
                Kind::Inbound {
                    from: "telegram:42".to_owned(),
                    to: "lead".to_owned(),
                },
                "telegram:42 → lead",
            ),
            (
                Kind::Reply {
                    from: "lead".to_owned(),
                    to: "telegram:42".to_owned(),
                },
                "lead → telegram:42 · preview (600-char summary)",
            ),
            (said("ae"), "said ae"),
            (card("lead"), "DECISION lead · waiting-user"),
            (
                Kind::NeedsYou {
                    seat: "colead".to_owned(),
                },
                "NEEDS YOU colead · needs you in its pane",
            ),
        ];
        for (kind, tag) in kinds {
            let mut printed = Printed::default();
            let got = printed.step(
                &lane(vec![item(kind, T, "a\n\tb\n  fn c() {}")], &[]),
                0,
                true,
            );
            assert_eq!(
                got,
                format!("# 2026-09-30 UTC\n## 06:01:00 {tag}\n  a\n  \tb\n    fn c() {{}}\n\n")
            );
        }
    }

    #[test]
    fn a_hostile_byte_in_any_field_is_inert_and_only_whitespace_survives() {
        let hostile = "x\u{1b}]52;c;AAAA\u{7}y\rz\u{9b}w";
        let kind = Kind::Inbound {
            from: format!("telegram:{hostile}"),
            to: format!("lead{hostile}"),
        };
        let gap = format!("aedev:lead — {hostile}");
        let mut printed = Printed::default();
        let got = printed.step(
            &lane(vec![item(kind, T, hostile)], &[gap.as_str()]),
            1,
            true,
        );
        assert!(
            !got.chars().any(|ch| ch.is_control() && ch != '\n'),
            "{got:?}"
        );
        assert!(
            got.contains("x\u{fffd}]52;c;AAAA\u{fffd}y\u{fffd}z\u{fffd}w"),
            "{got}"
        );
        let head = header(
            &format!("s{hostile}"),
            &[Seat {
                slot: "main".to_owned(),
                name: format!("l{hostile}"),
            }],
        );
        assert!(!head.trim_end().chars().any(char::is_control), "{head:?}");
    }

    #[test]
    fn a_follow_tick_prints_only_what_is_new_and_equal_records_are_two_rows() {
        let mut printed = Printed::default();
        let one = lane(
            vec![item(said("lead"), T, "same")],
            &["say — 1 line from other seats not shown"],
        );
        assert!(
            printed
                .step(&one, 0, true)
                .contains("## 06:01:00 said lead")
        );
        assert_eq!(
            printed.step(&one, 0, true),
            "",
            "nothing new, nothing printed"
        );
        let two = lane(
            vec![
                item(said("lead"), T, "same"),
                item(said("lead"), T, "same"),
                item(said("lead"), T + 5_000_000, "later"),
            ],
            &["say — 1 line from other seats not shown"],
        );
        let got = printed.step(&two, 0, true);
        assert_eq!(got.matches("said lead").count(), 2, "{got}");
        assert!(got.contains("06:01:05 said lead\n  later"), "{got}");
        assert!(!got.contains("coverage"), "a named gap prints once: {got}");
    }

    #[test]
    fn the_boards_own_gaps_print_on_every_tick_and_a_lane_gap_only_once() {
        let mut printed = Printed::default();
        let both = lane(vec![], &["board gap", "lane gap"]);
        assert_eq!(
            printed.step(&both, 1, true),
            "coverage incomplete: board gap\ncoverage incomplete: lane gap\n"
        );
        assert_eq!(
            printed.step(&both, 1, true),
            "coverage incomplete: board gap\n",
            "only the board's own gap comes back"
        );
    }

    #[test]
    fn a_card_that_stops_standing_is_closed_out_loud_and_a_newer_one_prints() {
        let mut printed = Printed::default();
        let asks = lane(vec![item(card("lead"), T, "ship v2?")], &[]);
        assert!(printed.step(&asks, 0, true).contains("DECISION lead"));
        assert_eq!(printed.step(&asks, 0, true), "");
        let answered = lane(vec![], &[]);
        assert_eq!(
            printed.step(&answered, 0, true),
            "-- closed: DECISION lead · waiting-user is no longer standing\n\n"
        );
        assert_eq!(printed.step(&answered, 0, true), "", "closed once");
        let again = lane(vec![item(card("lead"), T + 60_000_000, "and now?")], &[]);
        assert!(
            printed
                .step(&again, 0, true)
                .contains("## 06:02:00 DECISION lead")
        );
    }

    #[test]
    fn an_unread_journal_closes_nothing_and_a_standing_item_that_closed_and_returns_prints_again() {
        let needs = Kind::NeedsYou {
            seat: "lead".to_owned(),
        };
        for (kind, tag) in [(card("lead"), "DECISION lead"), (needs, "NEEDS YOU lead")] {
            let (mut printed, empty) = (Printed::default(), lane(vec![], &[]));
            let asks = lane(vec![item(kind, T, "ship it?")], &[]);
            assert!(printed.step(&asks, 0, true).contains(tag));
            assert_eq!(printed.step(&empty, 0, false), "", "unread proves nothing");
            assert_eq!(
                printed.step(&asks, 0, true),
                "",
                "still standing, still printed"
            );
            assert!(printed.step(&empty, 0, true).contains("-- closed"));
            assert!(printed.step(&asks, 0, true).contains(tag), "back, so news");
        }
    }
}
