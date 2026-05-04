#![no_std]

use core::time::Duration;

pub mod schedule;

pub const WEEKDAY_NONE: u8 = 0;
pub const MONDAY: u8 = 1;
pub const TUESDAY: u8 = 2;
pub const WEDNESDAY: u8 = 3;
pub const THURSDAY: u8 = 4;
pub const FRIDAY: u8 = 5;
pub const SATURDAY: u8 = 6;
pub const SUNDAY: u8 = 7;

const SECONDS_PER_DAY: u64 = 24 * 60 * 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DateTime {
    pub year: i32,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

impl DateTime {
    pub const fn new(year: i32, month: u8, day: u8, hour: u8, minute: u8, second: u8) -> Self {
        Self {
            year,
            month,
            day,
            hour,
            minute,
            second,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeRecord {
    pub hour: u8,
    pub minute: u8,
    /// Enabled weekdays. Use 1 = Monday through 7 = Sunday. Use 0 for unused slots.
    pub weekdays: [u8; 7],
}

impl TimeRecord {
    pub const fn new(hour: u8, minute: u8, weekdays: [u8; 7]) -> Self {
        Self {
            hour,
            minute,
            weekdays,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    InvalidDate,
    InvalidTime,
    InvalidWeekday(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NextWake {
    pub duration: Duration,
    pub hour: u8,
    pub minute: u8,
    pub weekday: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClosestWake {
    pub difference: Duration,
    pub hour: u8,
    pub minute: u8,
    pub weekday: u8,
    pub is_future: bool,
}

pub fn sleep_duration_until_next(
    now: DateTime,
    records: &[TimeRecord],
) -> Result<Option<Duration>, Error> {
    Ok(next_wake(now, records)?.map(|wake| wake.duration))
}

pub fn next_wake(now: DateTime, records: &[TimeRecord]) -> Result<Option<NextWake>, Error> {
    validate_datetime(now)?;

    let current_weekday = weekday_from_date(now.year, now.month, now.day)?;
    let now_seconds = seconds_since_midnight(now.hour, now.minute, now.second);
    let mut best: Option<NextWake> = None;

    for record in records {
        validate_record(*record)?;

        let target_seconds = seconds_since_midnight(record.hour, record.minute, 0);
        for weekday in record.weekdays {
            if weekday == WEEKDAY_NONE {
                continue;
            }

            let delta = forward_delta_seconds(current_weekday, weekday, now_seconds, target_seconds);

            best = Some(match best {
                Some(existing) if existing.duration.as_secs() <= delta => existing,
                _ => NextWake {
                    duration: Duration::from_secs(delta),
                    hour: record.hour,
                    minute: record.minute,
                    weekday,
                },
            });
        }
    }

    Ok(best)
}

pub fn closest_wake(now: DateTime, records: &[TimeRecord]) -> Result<Option<ClosestWake>, Error> {
    validate_datetime(now)?;

    let current_weekday = weekday_from_date(now.year, now.month, now.day)?;
    let now_seconds = seconds_since_midnight(now.hour, now.minute, now.second);
    let mut best: Option<ClosestWake> = None;

    for record in records {
        validate_record(*record)?;

        let target_seconds = seconds_since_midnight(record.hour, record.minute, 0);
        for weekday in record.weekdays {
            if weekday == WEEKDAY_NONE {
                continue;
            }

            let forward = forward_delta_seconds(current_weekday, weekday, now_seconds, target_seconds);
            let backward = backward_delta_seconds(current_weekday, weekday, now_seconds, target_seconds);
            let (difference, is_future) = if forward <= backward {
                (forward, true)
            } else {
                (backward, false)
            };

            best = Some(match best {
                Some(existing) if existing.difference.as_secs() <= difference => existing,
                _ => ClosestWake {
                    difference: Duration::from_secs(difference),
                    hour: record.hour,
                    minute: record.minute,
                    weekday,
                    is_future,
                },
            });
        }
    }

    Ok(best)
}

pub fn weekday_from_date(year: i32, month: u8, day: u8) -> Result<u8, Error> {
    validate_date(year, month, day)?;

    // Zeller's congruence for the Gregorian calendar.
    // h: 0 = Saturday, 1 = Sunday, 2 = Monday, ..., 6 = Friday.
    let (y, m) = if month < 3 {
        (year - 1, month as i32 + 12)
    } else {
        (year, month as i32)
    };

    let k = y % 100;
    let j = y / 100;
    let h = (day as i32 + ((13 * (m + 1)) / 5) + k + (k / 4) + (j / 4) + 5 * j) % 7;
    let h = ((h + 7) % 7) as u8;

    Ok(match h {
        0 => SATURDAY,
        1 => SUNDAY,
        2 => MONDAY,
        3 => TUESDAY,
        4 => WEDNESDAY,
        5 => THURSDAY,
        _ => FRIDAY,
    })
}

pub const fn is_leap_year(year: i32) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

pub const fn days_in_month(year: i32, month: u8) -> Option<u8> {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => Some(31),
        4 | 6 | 9 | 11 => Some(30),
        2 if is_leap_year(year) => Some(29),
        2 => Some(28),
        _ => None,
    }
}

fn validate_datetime(dt: DateTime) -> Result<(), Error> {
    validate_date(dt.year, dt.month, dt.day)?;
    validate_time(dt.hour, dt.minute, dt.second)
}

fn validate_record(record: TimeRecord) -> Result<(), Error> {
    validate_time(record.hour, record.minute, 0)?;
    for weekday in record.weekdays {
        if weekday > SUNDAY {
            return Err(Error::InvalidWeekday(weekday));
        }
    }
    Ok(())
}

fn validate_date(year: i32, month: u8, day: u8) -> Result<(), Error> {
    let Some(max_day) = days_in_month(year, month) else {
        return Err(Error::InvalidDate);
    };

    if day == 0 || day > max_day {
        return Err(Error::InvalidDate);
    }

    Ok(())
}

fn validate_time(hour: u8, minute: u8, second: u8) -> Result<(), Error> {
    if hour > 23 || minute > 59 || second > 59 {
        return Err(Error::InvalidTime);
    }

    Ok(())
}

fn seconds_since_midnight(hour: u8, minute: u8, second: u8) -> u64 {
    hour as u64 * 60 * 60 + minute as u64 * 60 + second as u64
}

fn forward_delta_seconds(current_weekday: u8, target_weekday: u8, now_seconds: u64, target_seconds: u64) -> u64 {
    let mut days_until = days_until_weekday(current_weekday, target_weekday);
    if days_until == 0 && target_seconds < now_seconds {
        days_until = 7;
    }

    let mut delta = days_until as u64 * SECONDS_PER_DAY;
    if target_seconds >= now_seconds {
        delta += target_seconds - now_seconds;
    } else {
        delta -= now_seconds - target_seconds;
    }
    delta
}

fn backward_delta_seconds(current_weekday: u8, target_weekday: u8, now_seconds: u64, target_seconds: u64) -> u64 {
    let mut days_since = days_since_weekday(current_weekday, target_weekday);
    if days_since == 0 && target_seconds > now_seconds {
        days_since = 7;
    }

    let mut delta = days_since as u64 * SECONDS_PER_DAY;
    if now_seconds >= target_seconds {
        delta += now_seconds - target_seconds;
    } else {
        delta -= target_seconds - now_seconds;
    }
    delta
}

fn days_until_weekday(current: u8, target: u8) -> u8 {
    (target + 7 - current) % 7
}

fn days_since_weekday(current: u8, target: u8) -> u8 {
    (current + 7 - target) % 7
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn computes_weekday_from_date() {
        assert_eq!(weekday_from_date(2026, 5, 4), Ok(MONDAY));
        assert_eq!(weekday_from_date(1970, 1, 1), Ok(THURSDAY));
        assert_eq!(weekday_from_date(2000, 1, 1), Ok(SATURDAY));
        assert_eq!(weekday_from_date(2024, 2, 29), Ok(THURSDAY));
    }

    #[test]
    fn rejects_invalid_dates_and_times() {
        assert_eq!(weekday_from_date(2026, 2, 29), Err(Error::InvalidDate));

        let record = TimeRecord::new(24, 0, [MONDAY, 0, 0, 0, 0, 0, 0]);
        let now = DateTime::new(2026, 5, 4, 8, 0, 0);
        assert_eq!(
            sleep_duration_until_next(now, &[record]),
            Err(Error::InvalidTime)
        );

        let record = TimeRecord::new(8, 0, [8, 0, 0, 0, 0, 0, 0]);
        assert_eq!(
            sleep_duration_until_next(now, &[record]),
            Err(Error::InvalidWeekday(8))
        );
    }

    #[test]
    fn returns_none_when_no_active_weekdays_exist() {
        let now = DateTime::new(2026, 5, 4, 8, 0, 0);
        let record = TimeRecord::new(9, 0, [0; 7]);
        assert_eq!(sleep_duration_until_next(now, &[record]), Ok(None));
    }

    #[test]
    fn selects_same_day_future_time() {
        let now = DateTime::new(2026, 5, 4, 8, 15, 30);
        let record = TimeRecord::new(8, 20, [MONDAY, 0, 0, 0, 0, 0, 0]);
        assert_eq!(
            sleep_duration_until_next(now, &[record]),
            Ok(Some(Duration::from_secs(4 * 60 + 30)))
        );
    }

    #[test]
    fn exact_current_minute_is_due_when_seconds_are_zero() {
        let now = DateTime::new(2026, 5, 4, 8, 15, 0);
        let record = TimeRecord::new(8, 15, [MONDAY, 0, 0, 0, 0, 0, 0]);
        assert_eq!(
            sleep_duration_until_next(now, &[record]),
            Ok(Some(Duration::from_secs(0)))
        );
    }

    #[test]
    fn current_minute_has_passed_when_seconds_are_nonzero() {
        let now = DateTime::new(2026, 5, 4, 8, 15, 1);
        let record = TimeRecord::new(8, 15, [MONDAY, 0, 0, 0, 0, 0, 0]);
        assert_eq!(
            sleep_duration_until_next(now, &[record]),
            Ok(Some(Duration::from_secs(7 * SECONDS_PER_DAY - 1)))
        );
    }

    #[test]
    fn wraps_to_next_week_when_today_time_passed() {
        let now = DateTime::new(2026, 5, 4, 8, 16, 0);
        let record = TimeRecord::new(8, 15, [MONDAY, 0, 0, 0, 0, 0, 0]);
        assert_eq!(
            sleep_duration_until_next(now, &[record]),
            Ok(Some(Duration::from_secs(7 * SECONDS_PER_DAY - 60)))
        );
    }

    #[test]
    fn selects_closest_record_across_all_records_and_days() {
        let now = DateTime::new(2026, 5, 4, 23, 55, 0);
        let records = [
            TimeRecord::new(7, 0, [TUESDAY, 0, 0, 0, 0, 0, 0]),
            TimeRecord::new(23, 58, [MONDAY, 0, 0, 0, 0, 0, 0]),
            TimeRecord::new(0, 1, [TUESDAY, 0, 0, 0, 0, 0, 0]),
        ];

        assert_eq!(
            sleep_duration_until_next(now, &records),
            Ok(Some(Duration::from_secs(3 * 60)))
        );
    }

    #[test]
    fn next_wake_returns_selected_time_record() {
        let now = DateTime::new(2026, 5, 4, 23, 55, 0);
        let records = [
            TimeRecord::new(7, 0, [TUESDAY, 0, 0, 0, 0, 0, 0]),
            TimeRecord::new(23, 58, [MONDAY, 0, 0, 0, 0, 0, 0]),
        ];

        assert_eq!(
            next_wake(now, &records),
            Ok(Some(NextWake {
                duration: Duration::from_secs(3 * 60),
                hour: 23,
                minute: 58,
                weekday: MONDAY,
            }))
        );
    }

    #[test]
    fn closest_wake_selects_near_future_entry() {
        let now = DateTime::new(2026, 5, 4, 9, 23, 30);
        let record = TimeRecord::new(9, 25, [MONDAY, 0, 0, 0, 0, 0, 0]);

        assert_eq!(
            closest_wake(now, &[record]),
            Ok(Some(ClosestWake {
                difference: Duration::from_secs(90),
                hour: 9,
                minute: 25,
                weekday: MONDAY,
                is_future: true,
            }))
        );
    }

    #[test]
    fn closest_wake_selects_near_past_entry() {
        let now = DateTime::new(2026, 5, 4, 9, 26, 30);
        let record = TimeRecord::new(9, 25, [MONDAY, 0, 0, 0, 0, 0, 0]);

        assert_eq!(
            closest_wake(now, &[record]),
            Ok(Some(ClosestWake {
                difference: Duration::from_secs(90),
                hour: 9,
                minute: 25,
                weekday: MONDAY,
                is_future: false,
            }))
        );
    }

    #[test]
    fn closest_wake_considers_previous_week() {
        let now = DateTime::new(2026, 5, 4, 0, 1, 0);
        let record = TimeRecord::new(23, 59, [SUNDAY, 0, 0, 0, 0, 0, 0]);

        assert_eq!(
            closest_wake(now, &[record]),
            Ok(Some(ClosestWake {
                difference: Duration::from_secs(2 * 60),
                hour: 23,
                minute: 59,
                weekday: SUNDAY,
                is_future: false,
            }))
        );
    }

    #[test]
    fn ignores_empty_weekday_slots() {
        let now = DateTime::new(2026, 5, 4, 23, 55, 0);
        let record = TimeRecord::new(0, 1, [0, 0, TUESDAY, 0, 0, 0, 0]);
        assert_eq!(
            sleep_duration_until_next(now, &[record]),
            Ok(Some(Duration::from_secs(6 * 60)))
        );
    }
}
