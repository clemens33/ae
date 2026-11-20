//! #287 slice 3 driver tests: the kept upkeep stop and what ends it.

use super::App;
use crate::stats_store::disk::{Refused, Reset};

const STOP: &str = "Statistics: the store was damaged again (x); upkeep stopped";

fn stopped_app() -> App {
    let mut app = App::new(None, None, None, None);
    app.stats_upkeep_stopped(STOP.to_owned());
    app.open_settings();
    app
}

#[test]
fn a_reset_that_swapped_the_store_ends_the_kept_stop_in_any_open() {
    for outcome in [Ok(Reset::Rebuilt { epoch: 2 }), Ok(Reset::Empty)] {
        let mut app = stopped_app();
        assert_eq!(app.settings_bodies.stats.stopped.as_deref(), Some(STOP));
        // A generation no open has: the fresh upkeep still ends the stop.
        app.reset_landed(u64::MAX, outcome);
        assert_eq!(app.settings_bodies.stats.stopped, None);
        app.open_settings();
        assert_eq!(app.settings_bodies.stats.stopped, None, "not latched again");
    }
}

#[test]
fn a_reset_that_swapped_nothing_keeps_the_stop() {
    for outcome in [Ok(Reset::Busy), Err(Refused::Io("held".to_owned()))] {
        let mut app = stopped_app();
        let generation = app.model.settings().map(|open| open.generation);
        app.reset_landed(generation.unwrap_or_default(), outcome);
        assert_eq!(app.settings_bodies.stats.stopped.as_deref(), Some(STOP));
        assert_eq!(app.stats_upkeep.stopped.as_deref(), Some(STOP));
    }
}
