//! Layer 0 (base): its modules, which src/lib.rs re-exports by name.

pub mod attention;
pub mod error;
pub mod event_text;
pub mod json;
pub mod provenance;
pub mod redact;
pub mod sanitize;
pub mod time;
pub mod token;
pub mod words;
