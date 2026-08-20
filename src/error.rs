//! The one top-level presentation error.
//!
//! #80: "start with one top-level presentation error; domain error types are
//! permitted where recovery/semantics differ — a single crate-wide enum is a
//! default, not law". This is that default. It is hand-written because "no
//! error dependency exists until a real error does": `thiserror` earns its
//! place when the enum has enough variants to make the boilerplate cost real.

use std::fmt;
use std::io;

/// Result alias for everything this crate presents to a caller.
pub type Result<T> = std::result::Result<T, Error>;

/// Anything `ae` failed at, in the shape it will be shown to a human.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// Writing output failed — a closed pipe, a full disk, a gone terminal.
    Io(io::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(source) => write!(f, "i/o: {source}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(source) => Some(source),
        }
    }
}

impl From<io::Error> for Error {
    fn from(source: io::Error) -> Self {
        Self::Io(source)
    }
}

#[cfg(test)]
mod tests {
    use super::Error;
    use std::error::Error as _;
    use std::io;

    #[test]
    fn an_io_error_presents_with_its_cause_and_keeps_its_source() {
        let err = Error::from(io::Error::from(io::ErrorKind::BrokenPipe));
        let shown = err.to_string();
        assert!(
            shown.starts_with("i/o: "),
            "unexpected presentation: {shown}"
        );
        assert!(
            err.source().is_some(),
            "the underlying io::Error was dropped"
        );
    }
}
