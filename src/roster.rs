//! Assembling the identity v2 roster block, and migrating a v1 meta into it.
//!
//! The core side of `_meta-init`: given the seats a launch resolved (or the
//! roster read out of a v1 meta), render the `schema=2` + `seat.<slot>` +
//! `profile.<slot>` + `harness_session.<slot>` + `agent_bin.<slot>` lines that
//! [`crate::meta::init`] publishes in one rename. `agent.<slot>` is never
//! written again — an old core meets an empty roster and fails closed (the
//! identity plan's Meta v2 section).
//!
//! MIGRATE is ONE-WAY and read-side: it parses a v1 meta with
//! [`crate::meta::Meta`], turns each `agent.<slot>=alias:name[:sid]` into a v2
//! seat whose PROFILE is the legacy alias, and REFUSES — with a full list, not
//! the first failure — when a legacy alias has no `[profiles]` entry or when two
//! seats would carry one name (v2's identity is the bare name, so a collision is
//! a roster in doubt). A refusal renders nothing, so the caller leaves the v1
//! meta byte-identical.

use crate::meta::{Anomaly, Meta, RosterSchema};

/// One seat's identity rows, ready to render.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeatLines {
    /// `main` / `worker.<n>` / `spawned.<n>`.
    pub slot: String,
    /// The agent's name — its v2 identity.
    pub name: String,
    /// The execution profile.
    pub profile: String,
    /// The recorded binary, where the meta carries one.
    pub binary: Option<String>,
    /// The captured harness session id, where the meta carries one.
    pub harness_session: Option<String>,
}

/// Render the v2 roster block for `seats`, in the order given. The block opens
/// with `schema=2` and then, per seat, `seat.`, `profile.`, `agent_bin.` (when
/// present) and `harness_session.` (when present) — every line `\n`-terminated.
/// The caller concatenates this after the base facts and hands the whole
/// document to [`crate::meta::init`].
#[must_use]
pub fn render(seats: &[SeatLines]) -> String {
    use std::fmt::Write as _;
    let mut out = String::from("schema=2\n");
    for seat in seats {
        // `writeln!` into a String is infallible; the results are consumed to
        // satisfy `-D warnings` without an `unwrap` on the capability boundary.
        let _ = writeln!(out, "seat.{}={}", seat.slot, seat.name);
        let _ = writeln!(out, "profile.{}={}", seat.slot, seat.profile);
        if let Some(binary) = &seat.binary {
            let _ = writeln!(out, "agent_bin.{}={}", seat.slot, binary);
        }
        if let Some(session) = &seat.harness_session {
            let _ = writeln!(out, "harness_session.{}={}", seat.slot, session);
        }
    }
    out
}

/// Why a v1 meta could not be migrated to v2 — collected in full so the
/// operator fixes the config once. Rendered in the order the v1 roster lists
/// its slots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MigrateRefusal {
    /// The v1 meta itself does not parse into a clean roster (a mixed-schema,
    /// malformed, or duplicate-key anomaly): there is nothing safe to migrate.
    RosterInDoubt {
        /// The anomaly text, for the operator.
        reason: String,
    },
    /// A legacy alias with no `[profiles]` entry in the target config.
    ProfileMissing {
        /// The slot whose alias is unbound.
        slot: String,
        /// The seat name.
        name: String,
        /// The alias that is now a profile and is not defined.
        profile: String,
    },
    /// Two seats would carry one name — v2's identity is the bare name.
    NameCollision {
        /// The name claimed twice.
        name: String,
    },
}

impl std::fmt::Display for MigrateRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RosterInDoubt { reason } => {
                write!(f, "the v1 roster does not parse cleanly: {reason}")
            }
            Self::ProfileMissing {
                slot,
                name,
                profile,
            } => write!(
                f,
                "seat '{name}' ({slot}) needs profile '{profile}', which is not defined in [profiles]."
            ),
            Self::NameCollision { name } => {
                write!(f, "the name '{name}' is claimed by more than one seat.")
            }
        }
    }
}

/// The refusal a launcher prints for a migrate, one line per refusal.
#[must_use]
pub fn render_refusals(refusals: &[MigrateRefusal]) -> String {
    let mut out = String::from("Error: this session's v1 roster cannot be migrated to v2:\n");
    for refusal in refusals {
        out.push_str("  - ");
        out.push_str(&refusal.to_string());
        out.push('\n');
    }
    out
}

/// Migrate the roster of a v1 `meta` into v2 [`SeatLines`], resolving each
/// legacy alias to a profile of the same name and checking `profile_defined`.
/// The seats come out in the v1 meta's own roster order.
///
/// # Errors
///
/// Every [`MigrateRefusal`] found: the v1 roster in doubt, an alias with no
/// profile, or a name on two seats. On any refusal the caller renders nothing
/// and leaves the v1 meta untouched.
pub fn migrate(
    meta: &Meta,
    profile_defined: impl Fn(&str) -> bool,
) -> Result<Vec<SeatLines>, Vec<MigrateRefusal>> {
    // Migrate a POSITIVELY-v1 roster only, and fail closed otherwise. Three
    // gates, in order — each refuses without guessing:
    //
    // 1. A ROSTER-DOUBTING anomaly (a slot claimed by both schemas, one name on
    //    two seats, a malformed roster row, or a duplicate roster key) makes the
    //    roster untrustworthy. An UnknownKey does NOT: a real v1 meta carries
    //    session/layout/config/ae_core rows the reader files as UnknownKey, and
    //    rejecting on those would refuse every live session (colead BLOCKER-1).
    // 2. Any v2 row already present means this is not a clean v1 roster to
    //    migrate — a half-migrated meta would silently drop its v2 seats.
    // 3. No v1 row at all (an empty meta, or a clean v2 one) is nothing to
    //    migrate — a clean v2 meta must REFUSE, not read back as an empty roster.
    let doubts: Vec<&Anomaly> = meta
        .anomalies()
        .iter()
        .filter(|a| roster_doubting(a))
        .collect();
    if !doubts.is_empty() {
        return Err(vec![MigrateRefusal::RosterInDoubt {
            reason: doubts
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join("; "),
        }]);
    }
    // A schema marker other than legacy (absent, or `1`) contradicts a v1 roster
    // — `schema=2` beside `agent.*` rows is not something to migrate.
    if let Some(schema) = meta.schema()
        && schema != "1"
    {
        return Err(vec![MigrateRefusal::RosterInDoubt {
            reason: format!("the meta declares schema={schema}, not a legacy v1 roster"),
        }]);
    }
    let v1_rows = meta
        .roster()
        .iter()
        .filter(|e| e.schema == RosterSchema::V1)
        .count();
    let v2_rows = meta
        .roster()
        .iter()
        .filter(|e| e.schema == RosterSchema::V2)
        .count();
    if v2_rows > 0 {
        return Err(vec![MigrateRefusal::RosterInDoubt {
            reason: "the meta already carries v2 seat rows — not a clean v1 roster to migrate"
                .to_owned(),
        }]);
    }
    if v1_rows == 0 {
        return Err(vec![MigrateRefusal::RosterInDoubt {
            reason: "no v1 roster to migrate (the meta is empty or already v2)".to_owned(),
        }]);
    }
    let mut refusals = Vec::new();
    let mut seats = Vec::new();
    let mut seen_names: Vec<&str> = Vec::new();
    for entry in meta.roster() {
        // Only a v1 row migrates; a v2 row is already what we would write, and
        // a meta that already carries v2 rows is not a migration candidate.
        let profile = match entry.schema {
            RosterSchema::V1 => entry.profile.clone().unwrap_or_default(),
            RosterSchema::V2 => continue,
        };
        if !profile_defined(&profile) {
            refusals.push(MigrateRefusal::ProfileMissing {
                slot: entry.slot.clone(),
                name: entry.name.clone(),
                profile: profile.clone(),
            });
        }
        if seen_names.contains(&entry.name.as_str()) {
            refusals.push(MigrateRefusal::NameCollision {
                name: entry.name.clone(),
            });
        } else {
            seen_names.push(&entry.name);
        }
        seats.push(SeatLines {
            slot: entry.slot.clone(),
            name: entry.name.clone(),
            profile,
            binary: entry.binary.clone(),
            harness_session: entry.harness_session.clone(),
        });
    }
    if refusals.is_empty() {
        Ok(seats)
    } else {
        Err(refusals)
    }
}

/// Whether an anomaly makes the ROSTER itself untrustworthy for migration, at
/// the provenance grain (colead round-2 BLOCKER-2):
///
/// - the identity-doubting three (a slot both schemas claim, one name on two
///   seats, a malformed roster row) — always;
/// - ANY `MalformedLine`: a bare `agent.<slot>` or `seat.<slot>` with no `=`
///   is filed as one, and it is a raw seat claim the migration would silently
///   drop — the line's key is not in the anomaly, so every one refuses;
/// - a duplicate key, or an `UnknownKey`, ON an identity prefix — a v1-attached
///   `profile.<slot>` / `harness_session.<slot>` is a v2 fact beside a v1 row;
/// - an `UnknownKey` elsewhere is tolerated: a real v1 meta is full of them
///   (`session`/`layout`/`config`/`ae_core` rows).
fn roster_doubting(a: &Anomaly) -> bool {
    const IDENTITY_PREFIXES: [&str; 5] = [
        "agent.",
        "agent_bin.",
        "seat.",
        "profile.",
        "harness_session.",
    ];
    match a {
        Anomaly::MixedSchemaSlot { .. }
        | Anomaly::DuplicateName { .. }
        | Anomaly::MalformedRosterEntry { .. }
        | Anomaly::MalformedLine { .. } => true,
        Anomaly::DuplicateKey { key, .. } | Anomaly::UnknownKey { key, .. } => {
            IDENTITY_PREFIXES.iter().any(|p| key.starts_with(p))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{MigrateRefusal, SeatLines, migrate, render, render_refusals};
    use crate::meta::Meta;

    fn seat(
        slot: &str,
        name: &str,
        profile: &str,
        bin: Option<&str>,
        sid: Option<&str>,
    ) -> SeatLines {
        SeatLines {
            slot: slot.to_owned(),
            name: name.to_owned(),
            profile: profile.to_owned(),
            binary: bin.map(ToOwned::to_owned),
            harness_session: sid.map(ToOwned::to_owned),
        }
    }

    #[test]
    fn render_writes_the_v2_block_and_parses_back_to_the_same_seats() {
        let seats = [
            seat("main", "lead", "fable5", Some("claude"), Some("e795")),
            seat("worker.0", "colead", "gpt56sol", Some("codex"), None),
        ];
        let block = render(&seats);
        assert_eq!(
            block,
            "schema=2\n\
             seat.main=lead\nprofile.main=fable5\nagent_bin.main=claude\nharness_session.main=e795\n\
             seat.worker.0=colead\nprofile.worker.0=gpt56sol\nagent_bin.worker.0=codex\n"
        );
        // The core's own reader reads it back to exactly these seats.
        let meta = Meta::parse(&block);
        assert_eq!(meta.schema(), Some("2"));
        let roster = meta.roster();
        assert_eq!(roster.len(), 2);
        assert_eq!(roster[0].name, "lead");
        assert_eq!(roster[0].profile.as_deref(), Some("fable5"));
        assert_eq!(roster[0].harness_session.as_deref(), Some("e795"));
        assert_eq!(roster[0].binary.as_deref(), Some("claude"));
        assert_eq!(roster[1].name, "colead");
        assert_eq!(roster[1].harness_session, None);
        assert!(meta.anomalies().is_empty());
    }

    #[test]
    fn migrate_turns_v1_rows_into_seats_whose_profile_is_the_alias() {
        let meta = Meta::parse(
            "mode=local\n\
             agent.main=fable5:lead:e795\nagent_bin.main=claude\n\
             agent.worker.0=gpt56sol:colead\nagent_bin.worker.0=codex\n",
        );
        let defined = |p: &str| ["fable5", "gpt56sol"].contains(&p);
        let seats = migrate(&meta, defined).expect("migratable");
        assert_eq!(
            seats,
            [
                seat("main", "lead", "fable5", Some("claude"), Some("e795")),
                seat("worker.0", "colead", "gpt56sol", Some("codex"), None),
            ]
        );
        // Round trip: the rendered v2 block reads back to the same names/profiles.
        let v2 = Meta::parse(&render(&seats));
        assert_eq!(v2.roster()[0].reference(), "lead");
        assert_eq!(v2.roster()[0].profile.as_deref(), Some("fable5"));
    }

    #[test]
    fn migrate_collects_every_refusal_and_never_the_first_only() {
        let meta = Meta::parse(
            "agent.main=fable5:lead\n\
             agent.worker.0=ghost:helper\n\
             agent.worker.1=fable5:lead\n",
        );
        // `ghost` is undefined; `lead` is used twice.
        let refusals = migrate(&meta, |p| p == "fable5").unwrap_err();
        assert_eq!(
            refusals,
            [
                MigrateRefusal::ProfileMissing {
                    slot: "worker.0".to_owned(),
                    name: "helper".to_owned(),
                    profile: "ghost".to_owned()
                },
                MigrateRefusal::NameCollision {
                    name: "lead".to_owned()
                },
            ]
        );
        let rendered = render_refusals(&refusals);
        assert!(
            rendered.starts_with("Error: this session's v1 roster cannot be migrated to v2:\n")
        );
        assert_eq!(rendered.lines().count(), 3);
    }

    #[test]
    fn migrate_refuses_a_v1_roster_in_doubt_rather_than_guessing() {
        // A mixed-schema slot (an anomaly) makes the whole roster untrustworthy.
        let meta = Meta::parse("agent.main=cl:lead\nseat.main=lead\n");
        let refusals = migrate(&meta, |_| true).unwrap_err();
        assert!(matches!(
            refusals.as_slice(),
            [MigrateRefusal::RosterInDoubt { .. }]
        ));
        // A clean single-seat roster with its profile present migrates — a
        // control so the refusal is not over-strong.
        let meta = Meta::parse("agent.main=cl:solo\n");
        assert_eq!(
            migrate(&meta, |p| p == "cl").expect("clean single seat"),
            [seat("main", "solo", "cl", None, None)]
        );
    }

    #[test]
    fn migrate_reads_a_real_live_meta_shaped_v1_roster() {
        // Colead BLOCKER-1: a real v1 meta is FULL of non-roster keys the reader
        // files as UnknownKey. Those must NOT block migration; only the v1 agent
        // rows migrate, in order, and every UnknownKey is ignored.
        let meta = Meta::parse(
            "session=aedev\nmode=local\nlayout=vertical\n\
             config=/home/x/.ae/config\nae_path=/home/x/bin/ae\n\
             watchdog=on\nae_core=/home/x/.ae/core/current\n\
             main_pane=%3\n\
             agent.main=fable5:lead:e795\nagent_bin.main=claude\n\
             agent.worker.0=gpt56sol:colead\nagent_bin.worker.0=codex\n",
        );
        assert!(
            !meta.anomalies().is_empty(),
            "the non-roster rows ARE recorded as anomalies — the point is they don't block"
        );
        let defined = |p: &str| ["fable5", "gpt56sol"].contains(&p);
        assert_eq!(
            migrate(&meta, defined).expect("a live v1 meta migrates through its UnknownKey rows"),
            [
                seat("main", "lead", "fable5", Some("claude"), Some("e795")),
                seat("worker.0", "colead", "gpt56sol", Some("codex"), None),
            ]
        );
    }

    #[test]
    fn migrate_fails_closed_on_anything_not_positively_a_v1_roster() {
        // A CLEAN v2 meta is not a migration candidate — it must refuse, never
        // read back as `Ok([])` (an empty schema=2 block).
        let v2 = Meta::parse("schema=2\nseat.main=lead\nprofile.main=fable5\n");
        assert!(v2.anomalies().is_empty(), "the v2 meta is clean");
        assert!(
            matches!(
                migrate(&v2, |_| true).unwrap_err().as_slice(),
                [MigrateRefusal::RosterInDoubt { .. }]
            ),
            "a clean v2 meta refuses migration"
        );
        // An empty meta has no roster to migrate.
        let empty = Meta::parse("mode=local\n");
        assert!(matches!(
            migrate(&empty, |_| true).unwrap_err().as_slice(),
            [MigrateRefusal::RosterInDoubt { .. }]
        ));
        // A half-migrated meta (a v1 row beside a v2 seat) refuses rather than
        // silently dropping the v2 seat.
        let half = Meta::parse("agent.main=cl:lead\nseat.worker.0=helper\nprofile.worker.0=cl\n");
        assert!(matches!(
            migrate(&half, |_| true).unwrap_err().as_slice(),
            [MigrateRefusal::RosterInDoubt { .. }]
        ));
    }

    #[test]
    fn migrate_refuses_at_the_provenance_grain_never_dropping_a_claimed_seat() {
        // Colead round-2 BLOCKER-2: each of these has a VALID v1 row the old
        // predicate would have migrated alone, silently losing the other claim.
        let doubtful = [
            // a bare agent.<slot> (no `=`) is a raw seat claim filed as MalformedLine
            "agent.main\nagent.worker.0=cl:colead\n",
            // a bare seat.<slot> beside a valid v1 row
            "seat.main\nagent.worker.0=cl:colead\n",
            // a v2 schema marker over v1 rows
            "schema=2\nagent.main=cl:lead\n",
            // a v2 fact attached to a v1 slot
            "agent.main=cl:lead\nprofile.main=cl\n",
            "agent.main=cl:lead\nharness_session.main=abc\n",
        ];
        for text in doubtful {
            let meta = Meta::parse(text);
            assert!(
                matches!(
                    migrate(&meta, |_| true).unwrap_err().as_slice(),
                    [MigrateRefusal::RosterInDoubt { .. }]
                ),
                "{text:?} must refuse"
            );
        }
        // Controls: an unrelated unknown key, and a legacy schema=1 marker, migrate.
        for text in [
            "agent.main=cl:lead\nwatchdog=on\n",
            "schema=1\nagent.main=cl:lead\n",
        ] {
            let meta = Meta::parse(text);
            assert_eq!(
                migrate(&meta, |_| true).unwrap_or_else(|e| panic!("{text:?}: {e:?}")),
                [seat("main", "lead", "cl", None, None)]
            );
        }
    }
}
