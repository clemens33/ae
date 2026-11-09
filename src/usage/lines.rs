//! A resumable, cursor-owning JSONL framer for the tool-statistics reader.
//!
//! Pure: the caller hands the file's bytes from [`Lines::cursor`] on, in any
//! chunking; every LF-ended line is emitted once, LF excluded. An unterminated
//! tail of at most the cap is NOT consumed (the cursor stays after the last LF
//! and the next step hands it again). A tail without LF that already exceeds
//! the cap is one [`Framed::OverCap`], consumed to the chunk end, and later
//! chunks pass in `skipping` up to its LF without another count, so every
//! step with more than `cap` new bytes advances the cursor. `LineReader` is
//! untouched: its final unterminated line is emitted, this one is not.

/// The product's line cap.
pub const LINE_CAP: usize = 1 << 20;

/// One framed line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Framed<'a> {
    /// A whole line, LF excluded, at most the cap.
    Line(&'a [u8]),
    /// A line over the cap, counted once, its bytes never handed out.
    OverCap,
}

/// The framer's state: bytes consumed and whether an over-cap line is open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lines {
    cap: usize,
    cursor: u64,
    skipping: bool,
}

impl Lines {
    /// A framer at offset 0 whose longest line is `cap` bytes.
    #[must_use]
    pub const fn new(cap: usize) -> Self {
        Self {
            cap,
            cursor: 0,
            skipping: false,
        }
    }

    /// A framer resumed at a persisted `cursor`, an over-cap line still open
    /// when `skipping` (#288 P3).
    #[must_use]
    pub const fn resume(cap: usize, cursor: u64, skipping: bool) -> Self {
        Self {
            cap,
            cursor,
            skipping,
        }
    }

    /// Bytes consumed: where the next step's bytes start.
    #[must_use]
    pub const fn cursor(&self) -> u64 {
        self.cursor
    }

    /// Whether an over-cap line is open across steps.
    #[must_use]
    pub const fn skipping(&self) -> bool {
        self.skipping
    }

    /// Frame `bytes`, the file's bytes from [`Self::cursor`] on; returns the
    /// bytes consumed (the cursor's advance).
    pub fn step(&mut self, bytes: &[u8], sink: &mut dyn FnMut(Framed<'_>)) -> u64 {
        let mut pos = 0;
        while let Some(at) = bytes[pos..].iter().position(|byte| *byte == b'\n') {
            let line = &bytes[pos..pos + at];
            if self.skipping {
                // The end of a line already counted over the cap.
                self.skipping = false;
            } else if line.len() > self.cap {
                sink(Framed::OverCap);
            } else {
                sink(Framed::Line(line));
            }
            pos += at + 1;
        }
        let tail = bytes.len() - pos;
        if self.skipping {
            pos = bytes.len();
        } else if tail > self.cap {
            sink(Framed::OverCap);
            self.skipping = true;
            pos = bytes.len();
        }
        let consumed = u64::try_from(pos).unwrap_or(u64::MAX);
        self.cursor = self.cursor.saturating_add(consumed);
        consumed
    }
}

#[cfg(test)]
#[path = "lines_spec.rs"]
mod spec;
