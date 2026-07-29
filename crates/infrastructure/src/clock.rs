//! The operating system clock.

use elrond_application::ports::Clock;
use time::OffsetDateTime;

/// Reads the wall clock in UTC.
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> OffsetDateTime {
        OffsetDateTime::now_utc()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_clock_moves_forward() {
        let clock = SystemClock;
        let first = clock.now();
        let second = clock.now();
        assert!(second >= first);
    }

    #[test]
    fn the_clock_reports_utc() {
        assert_eq!(SystemClock.now().offset(), time::UtcOffset::UTC);
    }
}
