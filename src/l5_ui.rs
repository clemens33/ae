//! Layer 5 (terminal UI): its modules, which src/lib.rs re-exports by name.

pub mod app;
pub(crate) mod clipboard;
pub mod console;
pub mod doctor;
pub mod events_tail;
pub mod jump;
pub mod monitor;
pub mod next;
pub mod overview;
pub mod panes;
pub(crate) mod profile_view;
pub(crate) mod reader;
pub mod session_menu;
pub(crate) mod settings_menu;
