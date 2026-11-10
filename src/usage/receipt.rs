//! Actual IO receipts of the usage and quota doors (#288 B3, lead rulings
//! ae-20261108T141228Z-383a41ad and ae-20261108T144928Z-d9d1c43b).
//!
//! Each counted door site adds ONE line just before its system call: a
//! `stat` (an lstat or an fstat), an `open`, a directory `listing`, or the
//! bytes a `read` returned. Each is one syscall however many links the kernel
//! follows inside it; the reader never canonicalizes. The counter only
//! counts: it never changes what a door does, returns or refuses.
//!
//! The counts are per thread and only grow; nothing resets them. A caller
//! reads them through one scoped [`Guard`]: the start value when its unit
//! begins, the delta when it ends. A tool-statistics unit runs all of its
//! door calls on the one thread that holds its guard (the loader thread),
//! so a delta is exactly that unit's IO and never another unit's or another
//! caller's; the `guard_counts_only_its_own_thread` test pins that a door
//! call on another thread is not in the delta.

use std::cell::Cell;

/// What the doors did on this thread.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Counts {
    pub(crate) bytes: u64,
    pub(crate) stats: u64,
    pub(crate) listings: u64,
    pub(crate) opens: u64,
}

impl Counts {
    const ZERO: Self = Self {
        bytes: 0,
        stats: 0,
        listings: 0,
        opens: 0,
    };
}

thread_local! {
    static COUNTS: Cell<Counts> = const { Cell::new(Counts::ZERO) };
}

fn bump(add: impl FnOnce(&mut Counts)) {
    COUNTS.with(|cell| {
        let mut counts = cell.get();
        add(&mut counts);
        cell.set(counts);
    });
}

/// `n` status calls are about to be made.
pub(crate) fn stat(n: usize) {
    bump(|counts| {
        counts.stats = counts
            .stats
            .saturating_add(u64::try_from(n).unwrap_or(u64::MAX));
    });
}

/// A file is about to be opened.
pub(crate) fn open() {
    bump(|counts| counts.opens = counts.opens.saturating_add(1));
}

/// A directory is about to be listed.
pub(crate) fn listing() {
    bump(|counts| counts.listings = counts.listings.saturating_add(1));
}

/// A read returned `n` bytes.
pub(crate) fn read(n: usize) {
    bump(|counts| {
        counts.bytes = counts
            .bytes
            .saturating_add(u64::try_from(n).unwrap_or(u64::MAX));
    });
}

/// One unit's view of the counts: what this thread's doors did since
/// [`Guard::start`].
#[derive(Debug)]
pub(crate) struct Guard {
    start: Counts,
}

impl Guard {
    pub(crate) fn start() -> Self {
        Self {
            start: COUNTS.with(Cell::get),
        }
    }

    pub(crate) fn delta(&self) -> Counts {
        let now = COUNTS.with(Cell::get);
        Counts {
            bytes: now.bytes.wrapping_sub(self.start.bytes),
            stats: now.stats.wrapping_sub(self.start.stats),
            listings: now.listings.wrapping_sub(self.start.listings),
            opens: now.opens.wrapping_sub(self.start.opens),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Counts, Guard, listing, open, read, stat};

    #[test]
    fn guard_counts_only_its_own_thread() {
        let guard = Guard::start();
        let other = std::thread::spawn(|| {
            stat(5);
            open();
            listing();
            read(100);
        });
        assert!(other.join().is_ok(), "the other thread's door calls ran");
        assert_eq!(
            guard.delta(),
            Counts::default(),
            "another thread is not in the delta"
        );
        stat(2);
        read(7);
        assert_eq!(
            guard.delta(),
            Counts {
                bytes: 7,
                stats: 2,
                listings: 0,
                opens: 0,
            }
        );
        let later = Guard::start();
        open();
        assert_eq!(
            later.delta().opens,
            1,
            "a later guard starts from the current count"
        );
        assert_eq!(
            guard.delta().opens,
            1,
            "an earlier guard still sees its whole span"
        );
    }
}
