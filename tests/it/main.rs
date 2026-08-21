//! The single integration-test target (`[[test]] name = "it"`).
//!
//! One target, `mod` submodules. Cargo would otherwise build and link one test
//! binary per `tests/*.rs`, and shared helpers would have no home that is not
//! also a test target.

mod cli;
mod fixtures;
mod parity;
mod parity_self_test;
mod phase2;
mod phase3;
