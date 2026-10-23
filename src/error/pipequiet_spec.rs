//! #220, the private contract under the program boundary: a stdout adapter that
//! marks a gone reader, a status a presentation site already decided, and the
//! boundary that settles the mark. Only the mark is settled: a gone STDERR
//! reader, an unmarked `BrokenPipe` and any other error stay errors.

use std::io::{self, ErrorKind, Write};

use super::{Error, Stdout, settle, with_status};

/// An inner stream that fails as told and counts what reaches it.
struct Inner {
    write: Option<io::Error>,
    flush: Option<io::Error>,
    bytes: Vec<u8>,
    writes: usize,
    flushes: usize,
}

impl Inner {
    fn failing_write(kind: ErrorKind, what: &str) -> Self {
        Self {
            write: Some(io::Error::new(kind, what)),
            ..Self::fine()
        }
    }

    fn failing_flush(kind: ErrorKind, what: &str) -> Self {
        Self {
            flush: Some(io::Error::new(kind, what)),
            ..Self::fine()
        }
    }

    fn fine() -> Self {
        Self {
            write: None,
            flush: None,
            bytes: Vec::new(),
            writes: 0,
            flushes: 0,
        }
    }
}

impl Write for Inner {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.writes += 1;
        if let Some(why) = &self.write {
            Err(io::Error::new(why.kind(), why.to_string()))
        } else {
            self.bytes.extend_from_slice(buf);
            Ok(buf.len())
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        self.flushes += 1;
        match &self.flush {
            Some(why) => Err(io::Error::new(why.kind(), why.to_string())),
            None => Ok(()),
        }
    }
}

/// The error a gone stdout reader produces through the adapter.
fn gone_reader() -> io::Error {
    let mut inner = Inner::failing_write(ErrorKind::BrokenPipe, "reader gone");
    let mut stdout = Stdout::new(&mut inner);
    stdout.write_all(b"line\n").expect_err("the reader is gone")
}

#[test]
fn bytes_reach_a_readable_stdout_unchanged() {
    let mut inner = Inner::fine();
    {
        let mut stdout = Stdout::new(&mut inner);
        assert_eq!(stdout.write(b"abc").expect("a reader"), 3);
        stdout.flush().expect("a reader");
    }
    assert_eq!(inner.bytes, b"abc");
    assert_eq!((inner.writes, inner.flushes), (1, 1));
}

#[test]
fn a_gone_reader_met_at_a_write_is_marked_and_nothing_more_reaches_the_stream() {
    let mut inner = Inner::failing_write(ErrorKind::BrokenPipe, "reader gone");
    {
        let mut stdout = Stdout::new(&mut inner);
        let first = stdout.write_all(b"one\n").expect_err("the reader is gone");
        assert_eq!(first.kind(), ErrorKind::BrokenPipe);
        assert!(matches!(settle(Err(Error::from(first))), Ok(0)));
        let again = stdout
            .write_all(b"two\n")
            .expect_err("still gone, never a discard");
        assert_eq!(again.kind(), ErrorKind::BrokenPipe);
        assert!(matches!(settle(Err(Error::from(again))), Ok(0)));
        let flushed = stdout.flush().expect_err("still gone");
        assert_eq!(flushed.kind(), ErrorKind::BrokenPipe);
        assert!(matches!(settle(Err(Error::from(flushed))), Ok(0)));
    }
    assert_eq!(
        (inner.writes, inner.flushes),
        (1, 0),
        "after the reader left, nothing reaches the stream"
    );
}

#[test]
fn a_gone_reader_met_at_a_flush_is_marked_and_nothing_more_reaches_the_stream() {
    let mut inner = Inner::failing_flush(ErrorKind::BrokenPipe, "reader gone");
    {
        let mut stdout = Stdout::new(&mut inner);
        assert_eq!(stdout.write(b"held").expect("accepted"), 4);
        let flushed = stdout.flush().expect_err("the reader is gone");
        assert_eq!(flushed.kind(), ErrorKind::BrokenPipe);
        assert!(matches!(settle(Err(Error::from(flushed))), Ok(0)));
        let again = stdout
            .write_all(b"more")
            .expect_err("still gone, never a discard");
        assert_eq!(again.kind(), ErrorKind::BrokenPipe);
    }
    assert_eq!((inner.writes, inner.flushes), (1, 1));
}

#[test]
fn any_other_stdout_error_passes_through_unmarked_with_its_kind_and_words() {
    for (mut inner, at) in [
        (
            Inner::failing_write(ErrorKind::Other, "disk said no"),
            "write",
        ),
        (
            Inner::failing_flush(ErrorKind::Other, "disk said no"),
            "flush",
        ),
    ] {
        let mut stdout = Stdout::new(&mut inner);
        let why = match at {
            "write" => stdout.write_all(b"x").expect_err("the stream fails"),
            _ => stdout.flush().expect_err("the stream fails"),
        };
        assert_eq!(why.kind(), ErrorKind::Other, "{at}");
        assert_eq!(why.to_string(), "disk said no", "{at}");
        let settled = settle(Err(Error::from(why)));
        assert!(
            matches!(&settled, Err(Error::Io(cause)) if cause.kind() == ErrorKind::Other),
            "{at}: {settled:?}"
        );
        assert_eq!(
            settled.err().as_ref().map(ToString::to_string),
            Some("i/o: disk said no".to_owned()),
            "{at}: the presentation is unchanged"
        );
    }
}

#[test]
fn the_boundary_settles_only_the_mark() {
    assert!(matches!(settle(Ok(0)), Ok(0)));
    assert!(matches!(settle(Ok(2)), Ok(2)));
    // A BrokenPipe the adapter never saw — a gone STDERR reader — stays an
    // error, presented as before.
    let unmarked = io::Error::new(ErrorKind::BrokenPipe, "stderr reader gone");
    let settled = settle(Err(Error::from(unmarked)));
    assert!(
        matches!(&settled, Err(Error::Io(cause)) if cause.kind() == ErrorKind::BrokenPipe),
        "{settled:?}"
    );
    assert_eq!(
        settled.err().as_ref().map(ToString::to_string),
        Some("i/o: stderr reader gone".to_owned())
    );
}

#[test]
fn a_status_already_decided_is_what_a_gone_reader_settles_to() {
    assert!(matches!(
        settle(with_status(1, Err(gone_reader())).map_err(Error::from)),
        Ok(1)
    ));
    assert!(matches!(
        settle(with_status(2, Err(gone_reader())).map_err(Error::from)),
        Ok(2)
    ));
    assert!(
        matches!(
            settle(with_status(0, Err(gone_reader())).map_err(Error::from)),
            Ok(0)
        ),
        "status 0 attaches nothing"
    );
    // The innermost site decided first, and an outer site does not overrule it.
    let nested = with_status(2, with_status(1, Err::<u8, _>(gone_reader())));
    assert!(matches!(settle(nested.map_err(Error::from)), Ok(1)));
    let after_zero = with_status(2, with_status(0, Err::<u8, _>(gone_reader())));
    assert!(matches!(settle(after_zero.map_err(Error::from)), Ok(2)));
}

#[test]
fn a_status_attaches_to_nothing_but_the_mark() {
    assert!(matches!(with_status(1, Ok::<u8, io::Error>(5)), Ok(5)));
    for kind in [ErrorKind::BrokenPipe, ErrorKind::Other] {
        let attached = with_status(1, Err::<u8, _>(io::Error::new(kind, "not stdout's reader")));
        let why = attached.expect_err("still an error");
        assert_eq!(why.kind(), kind);
        assert_eq!(why.to_string(), "not stdout's reader");
        let settled = settle(Err(Error::from(why)));
        assert!(
            matches!(&settled, Err(Error::Io(cause)) if cause.kind() == kind),
            "{kind:?}: {settled:?}"
        );
    }
}
