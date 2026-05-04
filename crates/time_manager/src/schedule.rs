use crate::{TimeRecord, FRIDAY, MONDAY, SATURDAY, SUNDAY, THURSDAY, TUESDAY, WEDNESDAY};

pub const WORKING_DAYS: [u8; 7] = [MONDAY, TUESDAY, WEDNESDAY, THURSDAY, FRIDAY, 0, 0];
pub const EVERY_DAY: [u8; 7] = [
    MONDAY, TUESDAY, WEDNESDAY, THURSDAY, FRIDAY, SATURDAY, SUNDAY,
];

pub const WAKE_SCHEDULE: [TimeRecord; 5] = [
    TimeRecord::new(9, 25, [MONDAY, WEDNESDAY, FRIDAY, 0, 0, 0, 0]),
    TimeRecord::new(9, 54, [TUESDAY, 0, 0, 0, 0, 0, 0]),
    TimeRecord::new(11, 27, WORKING_DAYS),
    TimeRecord::new(17, 30, WORKING_DAYS),
    TimeRecord::new(20, 57, EVERY_DAY),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{sleep_duration_until_next, DateTime};
    use core::time::Duration;

    #[test]
    fn schedule_selects_925_on_monday() {
        let now = DateTime::new(2026, 5, 4, 9, 20, 0);

        assert_eq!(
            sleep_duration_until_next(now, &WAKE_SCHEDULE),
            Ok(Some(Duration::from_secs(5 * 60)))
        );
    }

    #[test]
    fn schedule_selects_954_on_tuesday() {
        let now = DateTime::new(2026, 5, 5, 9, 50, 0);

        assert_eq!(
            sleep_duration_until_next(now, &WAKE_SCHEDULE),
            Ok(Some(Duration::from_secs(4 * 60)))
        );
    }

    #[test]
    fn schedule_uses_working_day_midday_time() {
        let now = DateTime::new(2026, 5, 6, 11, 0, 0);

        assert_eq!(
            sleep_duration_until_next(now, &WAKE_SCHEDULE),
            Ok(Some(Duration::from_secs(27 * 60)))
        );
    }

    #[test]
    fn schedule_skips_weekend_for_working_day_time() {
        let now = DateTime::new(2026, 5, 9, 17, 0, 0);

        assert_eq!(
            sleep_duration_until_next(now, &WAKE_SCHEDULE),
            Ok(Some(Duration::from_secs(3 * 60 * 60 + 57 * 60)))
        );
    }

    #[test]
    fn schedule_uses_daily_evening_time_on_sunday() {
        let now = DateTime::new(2026, 5, 10, 20, 50, 0);

        assert_eq!(
            sleep_duration_until_next(now, &WAKE_SCHEDULE),
            Ok(Some(Duration::from_secs(7 * 60)))
        );
    }
}
