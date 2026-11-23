//! The one clock `ae web` reads: every deadline, expiry, rate limit and
//! snapshot age asks it, never `Instant::now()` directly, so a test moves time
//! by hand.

use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

/// Monotonic time for deadlines and expiry, wall time for what a page shows.
pub trait Clock: Send + Sync {
    /// The monotonic now.
    fn now(&self) -> Instant;
    /// The wall-clock now, in epoch seconds.
    fn epoch(&self) -> i64;
}

/// The system's clocks.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }

    fn epoch(&self) -> i64 {
        crate::time::Timestamp::now().epoch()
    }
}

/// A clock that moves only when told: both readings advance together.
#[derive(Debug)]
pub struct ManualClock {
    base: Instant,
    epoch: i64,
    moved: Mutex<Duration>,
}

impl ManualClock {
    /// A clock that reads `epoch` on the wall now.
    #[must_use]
    pub fn new(epoch: i64) -> Self {
        Self {
            base: Instant::now(),
            epoch,
            moved: Mutex::new(Duration::ZERO),
        }
    }

    /// Move both readings forward by `by`.
    pub fn advance(&self, by: Duration) {
        let mut moved = self.moved.lock().unwrap_or_else(PoisonError::into_inner);
        *moved += by;
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Instant {
        let moved = self.moved.lock().unwrap_or_else(PoisonError::into_inner);
        self.base + *moved
    }

    fn epoch(&self) -> i64 {
        let moved = *self.moved.lock().unwrap_or_else(PoisonError::into_inner);
        self.epoch
            .saturating_add(i64::try_from(moved.as_secs()).unwrap_or(i64::MAX))
    }
}

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{Clock, SystemClock};

    /// The wall second as the standard library reads it, apart from the crate.
    fn wall() -> i64 {
        let since = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_else(|error| panic!("wall clock before 1970: {error}"));
        i64::try_from(since.as_secs()).unwrap_or(i64::MAX)
    }

    #[test]
    fn the_system_clock_reads_the_wall_second() {
        let before = wall();
        let read = SystemClock.epoch();
        let after = wall();
        assert!(
            (before..=after).contains(&read),
            "{before} <= {read} <= {after}"
        );
    }
}
