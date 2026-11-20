//! S5 state text from independently specified progress, never inferred from counts.
use crate::app::model::SettingsTab;
use crate::app::settings::{ResetLine, SettingsBodies, StatsScope, StoreLine};
use crate::stats::{self, Source};
use crate::stats_fixture as f;
use crate::stats_store::disk::{Glance, Progress, Refused, ThisProgress};

fn published(all_pending: u64, this_pending: u64, updating: bool) -> Glance {
    let bytes = f::birth(
        f::S,
        1,
        &f::fields("codex", None, Some("sol"), Some("high")),
    )
    .into_bytes();
    let mut sources = vec![Source::live(
        "alpha",
        format!("session_id={}\n", f::S).as_bytes(),
        Ok(bytes),
    )];
    if all_pending != this_pending {
        sources.push(Source::live(
            "other",
            format!("session_id={}\n", f::T).as_bytes(),
            Ok(Vec::new()),
        ));
    }
    let listed = sources.len() as u64;
    let view = stats::fold(sources, Some(f::S));
    Glance::Ready {
        view,
        epoch: 1,
        r#gen: 3,
        updating,
        all: Progress {
            listed,
            pending: all_pending,
        },
        this: ThisProgress::Ready(Progress {
            listed: 1,
            pending: this_pending,
        }),
    }
}
fn text(glance: &Glance, scope: StatsScope, reset: Option<ResetLine>) -> String {
    let (view, mut store) = StoreLine::of(glance);
    store.reset = reset;
    let mut bodies = SettingsBodies::default();
    bodies.stats.view = view;
    bodies.stats.store = store;
    bodies.stats.scope = scope;
    super::settings_body(
        SettingsTab::Statistics,
        &bodies,
        super::Paint::of(None),
        512,
        false,
    )
    .into_iter()
    .map(|(line, _)| line.trim().to_owned())
    .collect::<Vec<_>>()
    .join("\n")
}
#[test]
fn s5_ready_coverage_and_pending_are_independent_and_all_pending_retains_numbers() {
    let complete = text(&published(0, 0, false), StatsScope::All, None);
    assert!(complete.contains("Recorded worker spawns: 1"));
    assert!(complete.contains("Coverage: complete"));
    for updating in [false, true] {
        let glance = published(1, 0, updating);
        let all = text(&glance, StatsScope::All, None);
        assert!(
            all.contains("Recorded worker spawns: 1"),
            "pending sources retain published evidence: {all}"
        );
        assert!(
            all.contains("Partial: 1 of 2 sources not yet up to date"),
            "{all}"
        );
        assert!(!all.contains("Coverage: complete"), "{all}");
        let this = text(&glance, StatsScope::ThisSession, None);
        assert!(
            !this.contains("sources not yet up to date"),
            "root progress cannot be borrowed by the current session: {this}"
        );
        if updating {
            assert!(this.contains("Partial: an update is in progress"));
            assert!(!this.contains("Coverage: complete"));
        } else {
            assert!(this.contains("Coverage: complete"));
        }
    }
    let current_pending = published(1, 1, false);
    assert!(
        text(&current_pending, StatsScope::ThisSession, None)
            .contains("Partial: 1 of 1 sources not yet up to date")
    );
    assert!(text(&current_pending, StatsScope::All, None).contains("Recorded worker spawns: 1"));
    assert!(!text(&current_pending, StatsScope::All, None).contains("Coverage: complete"));
}
#[test]
fn s5_updating_without_pending_is_partial_and_keeps_numbers() {
    let shown = text(&published(0, 0, true), StatsScope::All, None);
    assert!(shown.contains("Recorded worker spawns: 1"));
    assert!(shown.contains("Partial: an update is in progress"));
    assert!(!shown.contains("Coverage: complete"));
}
#[test]
fn s5_building_damage_and_io_never_draw_zeros_as_complete() {
    for (glance, required) in [
        (
            Glance::Building,
            "Statistics are being built: no source read yet",
        ),
        (
            Glance::Refused(Refused::Damaged {
                file: "stats/view".into(),
                reason: "version".into(),
            }),
            "Statistics are being built: stats/view version",
        ),
        (
            Glance::Refused(Refused::OverCap {
                file: "stats/view".into(),
                cap: 1 << 20,
            }),
            "Statistics are being built:",
        ),
        (
            Glance::Refused(Refused::Io("stats/view permission denied".into())),
            "Statistics cannot be read: stats/view permission denied",
        ),
    ] {
        for scope in [StatsScope::All, StatsScope::ThisSession] {
            let shown = text(&glance, scope, None);
            assert!(shown.contains(required), "{shown}");
            assert!(
                !shown.contains("Recorded worker spawns:"),
                "unusable view must not invent numbers: {shown}"
            );
            assert!(!shown.contains("Coverage: complete"), "{shown}");
        }
    }
}
#[test]
fn s5_session_unregistered_and_damaged_leave_all_numbers_available() {
    for session in [
        ThisProgress::Unregistered,
        ThisProgress::Damaged(Refused::Damaged {
            file: "stats/session/current.view".into(),
            reason: "missing".into(),
        }),
    ] {
        let mut glance = published(0, 0, false);
        if let Glance::Ready { view, this, .. } = &mut glance {
            *this = session;
            view.this = stats::ThisSession::NoUuid;
        }
        let all = text(&glance, StatsScope::All, None);
        assert!(all.contains("Recorded worker spawns: 1"), "{all}");
        let current = text(&glance, StatsScope::ThisSession, None);
        assert!(current.contains("being built"), "{current}");
        assert!(!current.contains("Recorded worker spawns:"));
        assert!(!current.contains("Coverage: complete"));
        assert!(
            !current.contains("has no session id"),
            "an unavailable view is not an absent identity"
        );
    }
}
#[test]
fn s5_no_uuid_keeps_the_named_identity_state_and_never_uses_all_as_current() {
    let mut glance = published(0, 0, false);
    if let Glance::Ready { view, this, .. } = &mut glance {
        *this = ThisProgress::NoUuid;
        view.this = stats::ThisSession::NoUuid;
    }
    let current = text(&glance, StatsScope::ThisSession, None);
    assert!(current.contains("This session has no session id: nothing to show"));
    assert!(!current.contains("Recorded worker spawns:"));
}
#[test]
fn s5_reset_refusals_are_named_on_top_of_the_held_numbers() {
    let glance = published(0, 0, false);
    for (reset, required) in [
        (
            ResetLine::Busy,
            "Statistics: the store is busy; nothing was removed. Press r again.",
        ),
        (
            ResetLine::Waiting,
            "rebuild waits: statistics upkeep is not running",
        ),
        (
            ResetLine::Refused("stats.lock permission denied".into()),
            "stats.lock permission denied",
        ),
    ] {
        let shown = text(&glance, StatsScope::All, Some(reset));
        assert!(shown.contains(required), "{shown}");
        assert!(
            shown.contains("Recorded worker spawns: 1"),
            "refusal keeps the published answer: {shown}"
        );
    }
}
