//! `ae board --follow`: the one-shot board, then only what is NEW.
//!
//! One pure state machine, one thin driver. The driver re-locates every
//! selected seat per poll and reads through the ONE door; this module decides,
//! from the held state and the polled lstat facts, whether that read starts at
//! the commit point, starts at zero with a LOUD rescan, or does not happen at
//! all. Identity is (dev, inode) ONLY: a replaced or shrunk transcript is a new
//! generation, never silently deduplicated. No clock, no sleep, no I/O — the
//! unit tests drive the three arms with injected snapshots.

use std::collections::BTreeMap;
use std::time::SystemTime;

use super::{Coverage, Observation, SeatSeed, Streamed, reader_for};
use crate::tool::ToolKind;

/// The follow's poll cadence: the driver sleeps this between polls.
pub(crate) const POLL_SECS: u64 = 5;

/// One located transcript's identity facts: what follow offsets bind to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Located {
    /// The located file's dev+inode — the WHOLE identity.
    pub(crate) identity: (u64, u64),
    /// The located file's byte length at this poll.
    pub(crate) len: u64,
    /// The located file's mtime, when the OS reports one.
    pub(crate) mtime: Option<SystemTime>,
}

impl Located {
    /// The lstat facts of one located file.
    pub(crate) fn of(metadata: &std::fs::Metadata) -> Self {
        Self {
            identity: super::identity_of(metadata),
            len: metadata.len(),
            mtime: metadata.modified().ok(),
        }
    }
}

/// What one successful locate found: everything a read and its rows need.
pub(crate) struct Loaded {
    /// Row identity of the file: path plus dev+inode.
    pub(crate) file: String,
    /// Which harness wrote it; picks the reader.
    pub(crate) source: ToolKind,
    /// The lstat facts this poll's arms decide on.
    pub(crate) observed: Located,
}

/// One seat's polled snapshot: what the locate found, and — unless the plan
/// held — what the door read. A refusal is a reason, never a silent skip.
pub(crate) struct Snapshot {
    pub(crate) actor: String,
    pub(crate) located: Result<Loaded, String>,
    pub(crate) streamed: Option<Result<Streamed, &'static str>>,
}

/// The bytes this poll must read for a seat, decided BEFORE any read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Plan {
    /// Same identity, length equal to the commit point, same mtime: no read.
    Hold,
    /// Read from this ABSOLUTE offset: the commit point for an append, zero
    /// for a first sight or a rescan.
    Read(u64),
}

/// The arm ONE rule picks from the held state and this poll's facts.
enum Arm {
    /// This actor was never read: stream from zero, no rescan line.
    First,
    /// Same identity, more bytes: continue from the commit point.
    Append,
    /// A new generation: zero, and say why.
    Rescan(&'static str),
    /// Nothing new.
    Hold,
}

/// The held per-seat state: the identity offsets bind to, the commit point
/// after the last complete line, and the mtime last seen.
#[derive(Debug, Clone, Copy)]
struct Seat {
    identity: (u64, u64),
    mtime: Option<SystemTime>,
    committed: u64,
}

/// Decide the arm from the held state and this poll's lstat facts. Identity
/// change and shrink are `replaced`; a same-length rewrite is `rewritten`.
fn classify(held: Option<&Seat>, observed: &Located) -> Arm {
    let Some(held) = held else {
        return Arm::First;
    };
    if held.identity != observed.identity || observed.len < held.committed {
        return Arm::Rescan("transcript replaced — rescanned");
    }
    if observed.len == held.committed {
        return match (held.mtime, observed.mtime) {
            (Some(before), Some(now)) if before != now => {
                Arm::Rescan("transcript rewritten — rescanned")
            }
            _ => Arm::Hold,
        };
    }
    Arm::Append
}

/// The follow state machine: one held seat per actor plus the steady coverage
/// reasons already printed for each.
pub(crate) struct Follow {
    since: Option<i64>,
    seats: BTreeMap<String, Seat>,
    printed: BTreeMap<String, Vec<String>>,
}

impl Follow {
    /// Seed from the first pass: its reads set the offsets the follow
    /// continues from, and its coverage rows are what a later poll must not
    /// repeat. No row prints twice and none is missed.
    pub(crate) fn seeded(seeds: &[SeatSeed], first_pass: &[Coverage], since: Option<i64>) -> Self {
        let mut printed: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for item in first_pass {
            printed
                .entry(item.actor.clone())
                .or_default()
                .push(item.reason.clone());
        }
        Self {
            since,
            seats: seeds
                .iter()
                .map(|seed| {
                    (
                        seed.actor.clone(),
                        Seat {
                            identity: seed.identity,
                            mtime: seed.mtime,
                            committed: seed.committed,
                        },
                    )
                })
                .collect(),
            printed,
        }
    }

    /// The bytes this poll reads for one seat — the arm decision, before any
    /// read. `step` re-derives the same arm from the same facts.
    pub(crate) fn plan(&self, actor: &str, observed: &Located) -> Plan {
        match classify(self.seats.get(actor), observed) {
            Arm::Hold => Plan::Hold,
            Arm::Append => Plan::Read(self.seats.get(actor).map_or(0, |seat| seat.committed)),
            Arm::First | Arm::Rescan(_) => Plan::Read(0),
        }
    }

    /// One poll: the three arms, the LOUD rescans, and coverage that prints
    /// when it changes. No clock, no sleep, no I/O.
    pub(crate) fn step(&mut self, snapshots: Vec<Snapshot>) -> Observation {
        let mut rows = Vec::new();
        let mut coverage = Vec::new();
        for snapshot in snapshots {
            let Snapshot {
                actor,
                located,
                streamed,
            } = snapshot;
            let loaded = match located {
                Err(reason) => {
                    self.steady(&actor, vec![reason], &mut coverage);
                    continue;
                }
                Ok(loaded) => loaded,
            };
            let arm = classify(self.seats.get(&actor), &loaded.observed);
            if let Arm::Rescan(reason) = arm {
                coverage.push(Coverage {
                    actor: actor.clone(),
                    reason: reason.to_owned(),
                });
            }
            if matches!(arm, Arm::Hold) {
                continue;
            }
            match streamed {
                None => {}
                Some(Err(reason)) => {
                    if let Arm::Rescan(_) = arm {
                        // The generation change is KNOWN even when its first
                        // read failed: bind the new identity at zero so the
                        // loud line names it ONCE and the retry streams the
                        // whole generation from its start.
                        self.seats.insert(
                            actor.clone(),
                            Seat {
                                identity: loaded.observed.identity,
                                mtime: loaded.observed.mtime,
                                committed: 0,
                            },
                        );
                    }
                    self.steady(&actor, vec![reason.to_owned()], &mut coverage);
                }
                Some(Ok(streamed)) => {
                    let (mut seat_rows, seat_coverage) =
                        reader_for(loaded.source)(&streamed, &actor, &loaded.file, loaded.source);
                    self.steady(
                        &actor,
                        seat_coverage.into_iter().map(|item| item.reason).collect(),
                        &mut coverage,
                    );
                    rows.append(&mut seat_rows);
                    self.seats.insert(
                        actor,
                        Seat {
                            identity: loaded.observed.identity,
                            mtime: loaded.observed.mtime,
                            committed: streamed.committed,
                        },
                    );
                }
            }
        }
        let mut rows = super::collect(rows);
        if let Some(since) = self.since {
            rows.retain(|row| row.ts >= since);
        }
        Observation {
            rows,
            coverage,
            seeds: Vec::new(),
        }
    }

    /// Emit every steady reason not already printed for this actor, then hold
    /// the new set as printed: a seat that becomes readable prints nothing, one
    /// that becomes unreadable prints its new reason once. Rescan lines never
    /// pass through here — they are loud by ruling.
    fn steady(&mut self, actor: &str, reasons: Vec<String>, out: &mut Vec<Coverage>) {
        let printed = self.printed.get(actor);
        for reason in &reasons {
            if printed.is_none_or(|printed| !printed.contains(reason)) {
                out.push(Coverage {
                    actor: actor.to_owned(),
                    reason: reason.clone(),
                });
            }
        }
        self.printed.insert(actor.to_owned(), reasons);
    }
}

#[cfg(test)]
mod tests {
    use super::{Follow, Loaded, Located, Plan, Snapshot};
    use crate::board::{Coverage, Observation, SeatSeed, Splitter};
    use crate::tool::ToolKind;
    use std::time::{Duration, SystemTime};

    fn human(ts: &str, body: &str) -> String {
        format!(
            r#"{{"type":"user","timestamp":"{ts}","message":{{"role":"user","content":"{body}"}}}}"#
        )
    }

    fn located(identity: u64, len: u64, mtime: u64) -> Located {
        Located {
            identity: (identity, 7),
            len,
            mtime: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(mtime)),
        }
    }

    fn loaded(observed: Located) -> Loaded {
        Loaded {
            file: format!("f#{}:{}", observed.identity.0, observed.identity.1),
            source: ToolKind::Claude,
            observed,
        }
    }

    /// One poll EXACTLY as the driver drives it: plan, read the file's tail
    /// from the planned offset (the door's own shape), step.
    fn poll(follow: &mut Follow, actor: &str, observed: Located, full: &str) -> Observation {
        let from = match follow.plan(actor, &observed) {
            Plan::Read(from) => from,
            Plan::Hold => panic!("this poll should read"),
        };
        let mut splitter = Splitter::at(from);
        let at = usize::try_from(from).expect("test offsets fit a usize");
        splitter.feed(&full.as_bytes()[at..]);
        follow.step(vec![Snapshot {
            actor: actor.to_owned(),
            located: Ok(loaded(observed)),
            streamed: Some(Ok(splitter.finish())),
        }])
    }

    fn hold(follow: &mut Follow, actor: &str, observed: Located) -> Observation {
        assert_eq!(follow.plan(actor, &observed), Plan::Hold);
        follow.step(vec![Snapshot {
            actor: actor.to_owned(),
            located: Ok(loaded(observed)),
            streamed: None,
        }])
    }

    fn failing(actor: &str, observed: Located, reason: &'static str) -> Snapshot {
        Snapshot {
            actor: actor.to_owned(),
            located: Ok(loaded(observed)),
            streamed: Some(Err(reason)),
        }
    }

    #[test]
    fn an_append_streams_from_the_commit_point_with_absolute_offsets() {
        let mut follow = Follow::seeded(&[], &[], None);
        let first = format!("{}\n", human("2026-09-16T09:00:00Z", "one"));
        let batch = poll(
            &mut follow,
            "s:lead",
            located(1, first.len() as u64, 1),
            &first,
        );
        assert_eq!(batch.rows.len(), 1);
        assert_eq!(batch.rows[0].offset, 0);
        assert!(batch.coverage.is_empty(), "a first sight is not a rescan");
        let full = format!("{first}{}\n", human("2026-09-16T09:01:00Z", "two"));
        let batch = poll(
            &mut follow,
            "s:lead",
            located(1, full.len() as u64, 2),
            &full,
        );
        assert_eq!(batch.rows.len(), 1, "only the new record prints");
        assert_eq!(batch.rows[0].body, "two");
        assert_eq!(batch.rows[0].offset, first.len() as u64, "absolute offset");
    }

    #[test]
    fn seeded_offsets_continue_the_first_pass_and_an_unchanged_file_holds() {
        let first = format!("{}\n", human("2026-09-16T09:00:00Z", "one"));
        let mut follow = Follow::seeded(
            &[SeatSeed {
                actor: "s:lead".to_owned(),
                identity: (1, 7),
                mtime: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(1)),
                committed: first.len() as u64,
            }],
            &[],
            None,
        );
        let full = format!("{first}{}\n", human("2026-09-16T09:01:00Z", "two"));
        let observed = located(1, full.len() as u64, 2);
        let batch = poll(&mut follow, "s:lead", observed, &full);
        assert_eq!(batch.rows.len(), 1);
        assert_eq!(batch.rows[0].offset, first.len() as u64, "the seed binds");
        let batch = hold(&mut follow, "s:lead", observed);
        assert!(batch.rows.is_empty());
        assert!(batch.coverage.is_empty());
    }

    #[test]
    fn identity_change_same_size_rewrite_and_shrink_each_rescan_loudly() {
        let mut follow = Follow::seeded(&[], &[], None);
        let full = format!("{}\n", human("2026-09-16T09:00:00Z", "words"));
        let len = full.len() as u64;
        poll(&mut follow, "s:lead", located(1, len, 1), &full);

        let batch = poll(&mut follow, "s:lead", located(2, len, 2), &full);
        assert_eq!(batch.coverage[0].reason, "transcript replaced — rescanned");
        assert_eq!(batch.rows.len(), 1, "every row prints again");
        assert_eq!(batch.rows[0].offset, 0);

        let batch = poll(&mut follow, "s:lead", located(2, len, 3), &full);
        assert_eq!(batch.coverage[0].reason, "transcript rewritten — rescanned");
        assert_eq!(batch.rows.len(), 1);

        let small = &full[..(len / 2) as usize];
        let batch = poll(&mut follow, "s:lead", located(2, len / 2, 4), small);
        assert_eq!(batch.coverage[0].reason, "transcript replaced — rescanned");
    }

    #[test]
    fn a_failed_rescan_names_the_generation_once_and_retries_from_zero() {
        let mut follow = Follow::seeded(&[], &[], None);
        let first = format!("{}\n", human("2026-09-16T09:00:00Z", "one"));
        poll(
            &mut follow,
            "s:lead",
            located(1, first.len() as u64, 1),
            &first,
        );
        let replaced = format!("{}\n", human("2026-09-16T09:01:00Z", "two"));
        let observed = located(2, replaced.len() as u64, 2);
        let batch = follow.step(vec![Snapshot {
            actor: "s:lead".to_owned(),
            located: Ok(loaded(observed)),
            streamed: Some(Err("transcript unreadable")),
        }]);
        assert_eq!(batch.coverage[0].reason, "transcript replaced — rescanned");
        let batch = poll(&mut follow, "s:lead", observed, &replaced);
        assert_eq!(batch.rows.len(), 1, "the new generation reads from zero");
        assert_eq!(batch.rows[0].offset, 0);
        assert!(
            batch.coverage.is_empty(),
            "the generation change is named once, not every poll"
        );
    }

    #[test]
    fn a_torn_tail_holds_the_commit_point_and_the_retry_completes_it() {
        let mut follow = Follow::seeded(&[], &[], None);
        let first = format!("{}\n", human("2026-09-16T09:00:00Z", "one"));
        let commit = first.len() as u64;
        poll(&mut follow, "s:lead", located(1, commit, 1), &first);

        let record = human("2026-09-16T09:01:00Z", "torn then whole");
        let torn = format!("{first}{}", &record[..record.len() / 2]);
        let batch = poll(
            &mut follow,
            "s:lead",
            located(1, torn.len() as u64, 2),
            &torn,
        );
        assert!(batch.rows.is_empty(), "a torn tail is never trusted");
        assert_eq!(batch.coverage[0].reason, "torn last record");

        let whole = format!("{first}{record}\n");
        let batch = poll(
            &mut follow,
            "s:lead",
            located(1, whole.len() as u64, 3),
            &whole,
        );
        assert_eq!(batch.rows.len(), 1, "the retry completes the record");
        assert_eq!(batch.rows[0].offset, commit, "the commit point held");
        assert_eq!(batch.rows[0].body, "torn then whole");
    }

    #[test]
    fn coverage_prints_when_it_changes_never_every_poll() {
        let mut follow = Follow::seeded(&[], &[], None);
        let full = format!("{}\n", human("2026-09-16T09:00:00Z", "words"));
        let observed = located(1, full.len() as u64, 1);
        let batch = follow.step(vec![failing("s:lead", observed, "transcript unreadable")]);
        assert_eq!(batch.coverage.len(), 1, "the new reason prints once");
        let batch = follow.step(vec![failing("s:lead", observed, "transcript unreadable")]);
        assert!(batch.coverage.is_empty(), "the same reason does not repeat");
        let batch = poll(&mut follow, "s:lead", observed, &full);
        assert!(
            batch.coverage.is_empty(),
            "becoming readable prints nothing"
        );
        assert_eq!(batch.rows.len(), 1);
        let later = located(1, full.len() as u64 + 1, 2);
        let batch = follow.step(vec![failing("s:lead", later, "transcript unreadable")]);
        assert_eq!(batch.coverage.len(), 1, "a fresh failure prints again");
    }

    #[test]
    fn a_reason_the_first_pass_printed_does_not_repeat() {
        let printed = Coverage {
            actor: "s:lead".to_owned(),
            reason: "transcript unreadable".to_owned(),
        };
        let mut follow = Follow::seeded(&[], std::slice::from_ref(&printed), None);
        let batch = follow.step(vec![failing(
            "s:lead",
            located(1, 10, 1),
            "transcript unreadable",
        )]);
        assert!(batch.coverage.is_empty(), "the first pass already said it");
    }

    #[test]
    fn since_applies_to_every_batch() {
        let mut follow = Follow::seeded(&[], &[], Some(1_789_549_200_000_000));
        let full = format!(
            "{}\n{}\n",
            human("2026-09-16T08:59:59Z", "too early"),
            human("2026-09-16T09:00:00Z", "exactly since")
        );
        let batch = poll(
            &mut follow,
            "s:lead",
            located(1, full.len() as u64, 1),
            &full,
        );
        let bodies: Vec<&str> = batch.rows.iter().map(|row| row.body.as_str()).collect();
        assert_eq!(bodies, ["exactly since"]);
    }
}
