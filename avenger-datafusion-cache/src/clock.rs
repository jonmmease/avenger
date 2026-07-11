//! Injectable time source.
//!
//! All cache timestamps flow through [`CacheClock`] so hosts that cannot use
//! [`std::time::Instant`] (notably `wasm32-unknown-unknown`) can inject their
//! own source. Timestamps are durations since an arbitrary per-clock epoch;
//! only differences and ordering are meaningful.

use std::time::Duration;

/// A monotonic time source for cache bookkeeping.
///
/// Implementations must be monotonic non-decreasing. The epoch is arbitrary
/// but fixed for the lifetime of the clock instance.
pub trait CacheClock: Send + Sync + std::fmt::Debug {
    /// Current time as a duration since this clock's epoch.
    fn now(&self) -> Duration;
}

/// Default [`CacheClock`] backed by [`std::time::Instant`].
///
/// The epoch is the moment the clock was constructed.
#[derive(Debug)]
pub struct StdClock {
    origin: std::time::Instant,
}

impl StdClock {
    /// Create a clock whose epoch is "now".
    pub fn new() -> Self {
        Self {
            origin: std::time::Instant::now(),
        }
    }
}

impl Default for StdClock {
    fn default() -> Self {
        Self::new()
    }
}

impl CacheClock for StdClock {
    fn now(&self) -> Duration {
        self.origin.elapsed()
    }
}

/// Test-only deterministic clock, shared across the crate's test modules.
#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use std::sync::Mutex;

    /// Deterministic test clock advanced manually.
    #[derive(Debug, Default)]
    pub(crate) struct ManualClock {
        now: Mutex<Duration>,
    }

    impl ManualClock {
        /// Move the clock forward.
        pub(crate) fn advance(&self, by: Duration) {
            let mut now = self.now.lock().unwrap();
            *now += by;
        }
    }

    impl CacheClock for ManualClock {
        fn now(&self) -> Duration {
            *self.now.lock().unwrap()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn std_clock_is_monotonic() {
        let clock = StdClock::new();
        let a = clock.now();
        let b = clock.now();
        assert!(b >= a);
    }

    #[test]
    fn manual_clock_advances() {
        let clock = test_support::ManualClock::default();
        assert_eq!(clock.now(), Duration::ZERO);
        clock.advance(Duration::from_secs(3));
        assert_eq!(clock.now(), Duration::from_secs(3));
    }
}
