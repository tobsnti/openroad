//! Simulation time in whole steps of 1/30 s.
//!
//! The original server advances its world on a 1/30 s grid and runs AI and
//! movement on a 300 ms timer [V original server]. Its timers add milliseconds
//! truncated per step, which stretches 300 ms to about 307. Ours count steps
//! instead — 300 ms is exactly 9 — so a mob covers exactly its nominal speed;
//! a deliberate deviation (ADR 0013 §6).

use std::time::Duration;

/// Simulation steps per second.
pub const STEPS_PER_SECOND: u64 = 30;

/// The length of one step.
pub const STEP: Duration = Duration::from_nanos(1_000_000_000 / STEPS_PER_SECOND);

/// The AI and movement timer, in steps: 300 ms.
pub const AI_STEPS: u64 = 9;

/// True on the steps a timer of `period` steps fires (step 0 included).
pub fn fires(step: u64, period: u64) -> bool {
    step.is_multiple_of(period)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ai_timer_is_three_hundred_milliseconds() {
        // Exact in whole milliseconds; `STEP` itself is rounded to nanoseconds.
        assert_eq!(AI_STEPS * 1000 % STEPS_PER_SECOND, 0);
        assert_eq!(AI_STEPS * 1000 / STEPS_PER_SECOND, 300);
        let fired = (0..90).filter(|&s| fires(s, AI_STEPS)).count();
        assert_eq!(fired, 10, "ten times in three seconds");
    }
}
