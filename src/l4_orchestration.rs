//! Layer 4 (seat and session orchestration): its modules, which src/lib.rs re-exports by name.

pub mod autoreseat;
pub mod autotrust;
pub mod effort;
pub mod interrupt;
/// The whole `end`/`stop`/`compact` operations, in one place.
pub mod lifecycle;
pub mod migrate;
pub mod orchestrator;
pub mod orchestrator_autostart;
pub mod rename;
pub mod request_verb;
pub mod reseat;
pub mod restore;
pub(crate) mod seat_relaunch;
pub mod seatcompact;
pub(crate) mod seatcompact_run;
pub mod seatpack;
pub mod session_launch;
pub mod session_notice;
pub mod spawn;
pub mod teardown;
pub mod telegram_lifecycle;
pub mod watchdog_daemon;
pub mod watchdog_glue;
pub mod watchdog_lifecycle;
pub mod worktrees;
