//! Layer 3 (services): its modules, which src/lib.rs re-exports by name.

pub mod autoupgrade;
pub mod board;
pub mod brief;
pub mod brief_retry;
pub mod carry;
pub(crate) mod compact;
pub mod deliver;
pub mod deskcmd;
pub mod deskstore;
pub(crate) mod fleet_read;
pub mod go_usage;
pub mod init;
pub mod install;
pub mod launch;
pub mod listing;
pub mod muse_usage;
pub mod opencode_variant;
pub mod quota;
pub mod relay;
pub mod render;
pub mod reply;
pub mod run;
pub mod send;
pub mod session;
pub mod stats;
#[cfg(test)]
pub(crate) mod stats_fixture;
pub mod stats_store;
pub mod telegram;
pub mod tool_stats;
#[cfg(test)]
pub(crate) mod tool_stats_fixture;
pub mod upgrade;
pub mod usage;
