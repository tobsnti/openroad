//! The two clocks a client is told about.
//!
//! The wall clock goes out packed as the original packs it ([`ServerTime`]),
//! in UTC. The in-game clock advances one game minute every 36 steps (1.2 s),
//! the rate of the captured vSRO server (117 consecutive 0x3027 pairs: 0.8334
//! game minutes per second [V capture, peer vSRO]), so a game day lasts
//! 28.8 minutes. It counts from the server's start: the original's epoch for
//! its day counter is unknown.

use packets::agent::prelude::ServerTime;

/// Steps per game minute: 1.2 s of 1/30 s steps.
pub const GAME_MINUTE_STEPS: u64 = 36;

/// Game day, hour and minute at `step`.
pub fn game_time(step: u64) -> (u16, u8, u8) {
    let minutes = step / GAME_MINUTE_STEPS;
    let day = u16::try_from(minutes / 1440).unwrap_or(u16::MAX);
    (day, (minutes / 60 % 24) as u8, (minutes % 60) as u8)
}

/// `unix_seconds` in the original's packing: years since 2000 in six bits,
/// then month, day, hour, minute and second.
pub fn server_time(unix_seconds: u64) -> ServerTime {
    let (days, of_day) = (unix_seconds / 86_400, unix_seconds % 86_400);
    let (year, month, day) = civil_date(days as i64);
    let fields = [
        (year - 2000) as u32,
        month,
        day,
        (of_day / 3600) as u32,
        (of_day / 60 % 60) as u32,
        (of_day % 60) as u32,
    ];
    let packed = fields
        .iter()
        .zip([0, 6, 10, 15, 20, 26])
        .fold(0, |packed, (value, shift)| packed | value << shift);
    ServerTime { packed }
}

/// Year, month and day of the `days`-th day since 1970-01-01, by Howard
/// Hinnant's `civil_from_days`.
fn civil_date(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let (era, of_era) = (z.div_euclid(146_097), z.rem_euclid(146_097));
    let year_of_era = (of_era - of_era / 1460 + of_era / 36_524 - of_era / 146_096) / 365;
    let day_of_year = of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let mp = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wall_clock_packs_as_the_client_unpacks_it() {
        // 2026-11-12 16:05:09 UTC
        let time = server_time(1_794_499_509);
        let read = (
            time.year(),
            time.month(),
            time.day(),
            time.hour(),
            time.minute(),
            time.second(),
        );
        assert_eq!(read, (2026, 11, 12, 16, 5, 9));
    }

    #[test]
    fn a_game_day_is_1440_game_minutes() {
        assert_eq!(game_time(0), (0, 0, 0));
        assert_eq!(game_time(GAME_MINUTE_STEPS * (1440 + 61)), (1, 1, 1));
    }
}
