//! The one top-level presentation error.
//!
//! One top-level presentation error; domain error types are permitted where
//! recovery or semantics differ — a single crate-wide enum is a default, not
//! law. It is hand-written because no error dependency exists until a real
//! error does: `thiserror` earns its place when the enum has enough variants to
//! make the boilerplate cost real.

use std::fmt;
use std::io;

#[cfg(test)]
mod pipequiet_spec;

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

/// Marks only failures observed at the program's stdout boundary.
#[derive(Debug)]
struct ClosedStdout(u8, io::Error);

impl fmt::Display for ClosedStdout {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.1, f)
    }
}
impl std::error::Error for ClosedStdout {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.1)
    }
}

pub(crate) struct Stdout<'a, W> {
    inner: &'a mut W,
    closed: bool,
}

impl<'a, W: io::Write> Stdout<'a, W> {
    pub(crate) fn new(inner: &'a mut W) -> Self {
        Self {
            inner,
            closed: false,
        }
    }

    fn apply<T>(&mut self, operation: impl FnOnce(&mut W) -> io::Result<T>) -> io::Result<T> {
        let result = if self.closed {
            Err(io::ErrorKind::BrokenPipe.into())
        } else {
            operation(self.inner)
        };
        result.map_err(|error| {
            if error.kind() != io::ErrorKind::BrokenPipe {
                return error;
            }
            self.closed = true;
            io::Error::new(io::ErrorKind::BrokenPipe, ClosedStdout(0, error))
        })
    }
}

impl<W: io::Write> io::Write for Stdout<'_, W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.apply(|inner| inner.write(buf))
    }

    fn flush(&mut self) -> io::Result<()> {
        self.apply(io::Write::flush)
    }
}

pub(crate) fn with_status<T>(code: u8, result: io::Result<T>) -> io::Result<T> {
    result.map_err(|mut error| {
        if let Some(closed) = error
            .get_mut()
            .and_then(|e| e.downcast_mut::<ClosedStdout>())
            && closed.0 == 0
        {
            closed.0 = code;
        }
        error
    })
}

pub(crate) fn settle(result: Result<u8>) -> Result<u8> {
    match result {
        Err(Error::Io(error)) => match error
            .get_ref()
            .and_then(|e| e.downcast_ref::<ClosedStdout>())
        {
            Some(closed) => Ok(closed.0),
            None => Err(Error::Io(error)),
        },
        other => other,
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
