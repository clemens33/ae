//! Assembling the identity v2 roster block.
//!
//! The core side of `_meta-init`: given the seats a launch resolved, render the
//! `schema=2` + `seat.<slot>` + `profile.<slot>` + `harness_session.<slot>` +
//! `agent_bin.<slot>` + `config_home.<slot>` + `config_home_base.<slot>` lines that
//! [`crate::meta::init`] publishes in one rename.
//! `agent.<slot>` is never written, and never read into a seat: a v1 meta is a
//! session to start over from, not one to migrate.

use crate::meta::Anomaly;

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
    /// The canonical config-home row, where this seat has started before.
    pub config_home: Option<String>,
    /// The canonical effective `HOME` paired with an implicit config home.
    pub config_home_base: Option<String>,
}

/// Render the v2 roster block for `seats`, in the order given.
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
        if let Some(home) = &seat.config_home {
            let _ = writeln!(out, "config_home.{}={}", seat.slot, home);
        }
        if let Some(base) = &seat.config_home_base {
            let _ = writeln!(out, "config_home_base.{}={}", seat.slot, base);
        }
    }
    out
}

/// Whether an anomaly makes the ROSTER itself untrustworthy, at the provenance
/// grain:
pub(crate) fn roster_doubting(a: &Anomaly) -> bool {
    const IDENTITY_PREFIXES: [&str; 7] = [
        "agent.",
        "agent_bin.",
        "seat.",
        "profile.",
        "harness_session.",
        "config_home.",
        "config_home_base.",
    ];
    match a {
        Anomaly::LegacyRoster { .. }
        | Anomaly::DuplicateName { .. }
        | Anomaly::InconsistentConfigHome { .. }
        | Anomaly::MalformedRosterEntry { .. }
        | Anomaly::MalformedLine { .. } => true,
        Anomaly::DuplicateKey { key, .. } | Anomaly::UnknownKey { key, .. } => {
            IDENTITY_PREFIXES.iter().any(|p| key.starts_with(p))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{SeatLines, render};
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
            config_home: None,
            config_home_base: None,
        }
    }

    #[test]
    fn render_writes_the_v2_block_and_parses_back_to_the_same_seats() {
        let mut seats = [
            seat("main", "lead", "fable5", Some("claude"), Some("e795")),
            seat("worker.0", "colead", "gpt56sol", Some("codex"), None),
        ];
        seats[0].config_home = Some("/accounts/claude".to_owned());
        seats[1].config_home = Some("implicit:/accounts/codex".to_owned());
        seats[1].config_home_base = Some("/people/colead".to_owned());
        let block = render(&seats);
        assert_eq!(
            block,
            "schema=2\n\
             seat.main=lead\nprofile.main=fable5\nagent_bin.main=claude\nharness_session.main=e795\nconfig_home.main=/accounts/claude\n\
             seat.worker.0=colead\nprofile.worker.0=gpt56sol\nagent_bin.worker.0=codex\nconfig_home.worker.0=implicit:/accounts/codex\nconfig_home_base.worker.0=/people/colead\n"
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
        assert_eq!(
            roster[0].config_home,
            crate::meta::RecordedConfigHome::Path("/accounts/claude".into())
        );
        assert_eq!(roster[1].name, "colead");
        assert_eq!(roster[1].harness_session, None);
        assert_eq!(
            roster[1].config_home_base,
            crate::meta::RecordedConfigHomeBase::Path("/people/colead".into())
        );
        assert!(meta.anomalies().is_empty());
    }
}
