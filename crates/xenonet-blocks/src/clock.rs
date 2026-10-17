//! core/blocks/clock.js: the clock the blocks keep time with. A core takes
//! the time from whoever runs it (`now`, or a `Clock`), so a test can move
//! it by hand; this is the system's, for the binary, whose timers are
//! tokio's.

/// What a block reads the time from, in ms since 1970.
pub trait Clock {
    fn now(&self) -> f64;
}

/// The system's clock: Date.now(), whole milliseconds.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> f64 {
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0.0, |d| d.as_millis() as f64)
    }
}

/// A clock that says what it's told, for tests.
#[derive(Clone, Copy, Debug, Default)]
pub struct FixedClock(pub f64);

impl Clock for FixedClock {
    fn now(&self) -> f64 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_system_clock_is_whole_milliseconds_since_1970() {
        let t = SystemClock.now();
        assert!(t > 1.7e12 && t.fract() == 0.0);
        assert_eq!(FixedClock(5.0).now(), 5.0);
    }
}
