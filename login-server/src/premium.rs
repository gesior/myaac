use chrono::Datelike;

use crate::schema::Schema;

pub const GRATIS_PREMIUM_DAYS: i64 = 65535;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PremiumMode {
    PremiumEndsAt,
    Premend,
    LastDay,
    Premdays,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PremiumInput {
    pub premium_ends_at: Option<i64>,
    pub premend: Option<i64>,
    pub lastday: Option<i64>,
    pub premdays: Option<i64>,
}

pub fn resolve(schema: &Schema, lastday_is_premium_end: bool) -> PremiumMode {
    if schema.has_premium_ends_at {
        PremiumMode::PremiumEndsAt
    } else if lastday_is_premium_end && schema.has_lastday {
        PremiumMode::LastDay
    } else if schema.has_premend {
        PremiumMode::Premend
    } else {
        PremiumMode::Premdays
    }
}

pub fn is_premium(input: &PremiumInput, mode: PremiumMode, now: i64) -> bool {
    if let Some(end) = timestamp_end(input, mode) {
        return end > now;
    }
    let premdays = input.premdays.unwrap_or(0);
    if premdays == GRATIS_PREMIUM_DAYS {
        return true;
    }
    let lastday = input.lastday.unwrap_or(0);
    premdays - elapsed_premium_days(now, lastday) > 0
}

pub fn premium_days(input: &PremiumInput, mode: PremiumMode, now: i64) -> i64 {
    if let Some(end) = timestamp_end(input, mode) {
        return php_ceil_nonneg((end - now) as f64 / 86400.0);
    }
    let premdays = input.premdays.unwrap_or(0);
    if premdays == 0 {
        return 0;
    }
    if premdays == GRATIS_PREMIUM_DAYS {
        return GRATIS_PREMIUM_DAYS;
    }
    let lastday = input.lastday.unwrap_or(0);
    php_ceil_nonneg(premdays as f64 - elapsed_premium_days(now, lastday) as f64)
}

pub fn premium_until(days: i64, now: i64) -> i64 {
    if days > 0 {
        now.saturating_add(days.saturating_mul(86400))
    } else {
        0
    }
}

fn timestamp_end(input: &PremiumInput, mode: PremiumMode) -> Option<i64> {
    match mode {
        PremiumMode::PremiumEndsAt => Some(input.premium_ends_at.unwrap_or(0)),
        PremiumMode::Premend => Some(input.premend.unwrap_or(0)),
        PremiumMode::LastDay => Some(input.lastday.unwrap_or(0)),
        PremiumMode::Premdays => None,
    }
}

/// PHP `date("z") + 365 * (year - last_year) - date("z", lastday)` in UTC.
fn elapsed_premium_days(now: i64, lastday: i64) -> i64 {
    let (now_year, now_day) = utc_year_and_day(now);
    let (last_year, last_day) = utc_year_and_day(lastday);
    now_day + 365 * (now_year - last_year) - last_day
}

fn utc_year_and_day(ts: i64) -> (i64, i64) {
    match chrono::DateTime::from_timestamp(ts, 0) {
        Some(dt) => (i64::from(dt.year()), i64::from(dt.ordinal0())),
        None => (1970, 0),
    }
}

fn php_ceil_nonneg(value: f64) -> i64 {
    let rounded = value.ceil();
    if rounded <= 0.0 {
        0
    } else {
        rounded as i64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn premdays_same_day() {
        let now = 1_577_836_800; // 2020-01-01 UTC, day 0
        let input = PremiumInput {
            premdays: Some(5),
            lastday: Some(now),
            ..PremiumInput::default()
        };
        assert!(is_premium(&input, PremiumMode::Premdays, now));
        assert_eq!(premium_days(&input, PremiumMode::Premdays, now), 5);
        assert_eq!(premium_until(5, now), now + 5 * 86400);
    }

    #[test]
    fn premdays_one_day_later() {
        let start = 1_577_836_800;
        let now = start + 86400;
        let input = PremiumInput {
            premdays: Some(1),
            lastday: Some(start),
            ..PremiumInput::default()
        };
        assert!(!is_premium(&input, PremiumMode::Premdays, now));
        assert_eq!(premium_days(&input, PremiumMode::Premdays, now), 0);
    }

    #[test]
    fn gratis_premdays() {
        let input = PremiumInput {
            premdays: Some(GRATIS_PREMIUM_DAYS),
            lastday: Some(0),
            ..PremiumInput::default()
        };
        assert!(is_premium(&input, PremiumMode::Premdays, 1_700_000_000));
        assert_eq!(
            premium_days(&input, PremiumMode::Premdays, 1_700_000_000),
            GRATIS_PREMIUM_DAYS
        );
    }

    #[test]
    fn timestamp_end_rounds_up_partial_day() {
        let now = 1_000_000;
        let input = PremiumInput {
            premium_ends_at: Some(now + 86400 + 10),
            ..PremiumInput::default()
        };
        assert!(is_premium(&input, PremiumMode::PremiumEndsAt, now));
        assert_eq!(premium_days(&input, PremiumMode::PremiumEndsAt, now), 2);
        assert!(!is_premium(
            &PremiumInput {
                premium_ends_at: Some(now),
                ..PremiumInput::default()
            },
            PremiumMode::PremiumEndsAt,
            now
        ));
    }
}
