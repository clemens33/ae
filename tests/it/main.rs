//! The single integration-test target (`[[test]] name = "it"`).
//!
//! One target, `mod` submodules. Cargo would otherwise build and link one test
//! binary per `tests/*.rs`, and shared helpers would have no home that is not
//! also a test target.

mod archive;
mod archive_from;
mod archive_publish;
mod archive_purge;
mod capture;
mod cli;
mod compact;
mod daemons;
mod deliver;
mod doctor;
mod doors;
mod entry;
mod fixtures;
mod gate;
mod git;
mod install;
mod lifecycle;
mod monitor;
mod parity;
mod phase2;
mod phase3;
mod run;
mod session_launch;
mod shape;
mod spawn;
mod teardown;
mod telegram;
mod transport;
mod watchdog_glue;
