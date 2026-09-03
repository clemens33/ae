//! The single integration-test target (`[[test]] name = "it"`).
//!
//! One target, `mod` submodules. Cargo would otherwise build and link one test
//! binary per `tests/*.rs`, and shared helpers would have no home that is not
//! also a test target.

mod archive;
mod archive_from;
mod archive_publish;
mod archive_purge;
mod cli;
mod compact;
mod daemons;
mod deliver;
mod fixtures;
mod git;
mod helper_corpus;
mod lifecycle;
mod parity;
mod parity_self_test;
mod phase2;
mod phase3;
mod session_launch;
mod spawn;
mod teardown;
mod telegram;
mod transport;
mod watchdog_glue;
