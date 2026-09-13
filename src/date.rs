//! Dates: from what a caller wrote to the serial number a workbook stores.
//!
//! A workbook has no date cells. It has numbers, and a format that makes one
//! look like a date, so writing a date means writing the number Excel would
//! have written. Which number that is depends on the workbook's
//! [`DateSystem`], and nothing but the workbook part says which is in force,
//! so this cannot be done until the package is open.
//!
//! Two spellings are accepted, because both are what a caller has to hand: an
//! ISO 8601 date or datetime, and a serial itself. They are the same value in
//! two forms, so they produce the same bytes.
//!
//! The 1900 system carries a day that never happened. Lotus 1-2-3 treated 1900
//! as a leap year, Excel kept the bug so that serials agreed, and the result
//! is that serial 60 is the 29th of February 1900 and every serial below 61
//! either sits in that fiction or on the wrong side of it. Rather than pick an
//! answer for a caller in that range, a date before 1900-03-01 is refused and
//! the message says why.

use crate::error::{Error, Result};
use crate::workbook::DateSystem;

/// The serial of 1970-01-01 in the 1900 system: the days from the day the
/// system counts from, 1899-12-30, counting the phantom leap day.
const UNIX_EPOCH_1900: i64 = 25_569;

/// How far apart the two systems are. The 1904 system starts later and skips
/// the phantom day, so every date is 1462 lower in it.
const BETWEEN_THE_SYSTEMS: i64 = 1_462;

/// The first serial the 1900 system counts truthfully: 1900-03-01, the day
/// after the leap day that never was.
const FIRST_TRUE_DAY_1900: f64 = 61.0;

/// The serial `text` names under `dates`, or why it is not a date.
///
/// `text` is an ISO 8601 date, an ISO 8601 datetime, or a serial. Anything
/// else is a usage error: it is the caller's spelling that is wrong, not the
/// package.
pub fn serial(text: &str, dates: DateSystem) -> Result<f64> {
    let serial = match text.trim().parse::<f64>() {
        // A serial is already the stored form, so it is taken as given. It is
        // still held to the floor below: a caller cannot reach the phantom day
        // by spelling it as a number either.
        Ok(number) if number.is_finite() => number,
        Ok(_) | Err(_) => from_iso(text, dates)?,
    };
    within_the_system(serial, dates)
}

/// The serial an ISO 8601 date or datetime stands for.
fn from_iso(text: &str, dates: DateSystem) -> Result<f64> {
    let (date, time) = match text.split_once('T') {
        None => (text, None),
        Some((date, time)) => (date, Some(time)),
    };
    let day = days_from_civil(civil(date, text)?);
    let fraction = match time {
        None => 0.0,
        Some(time) => day_fraction(time, text)?,
    };
    let from_1900 = (day + UNIX_EPOCH_1900) as f64 + fraction;
    Ok(match dates {
        DateSystem::Date1900 => from_1900,
        DateSystem::Date1904 => from_1900 - BETWEEN_THE_SYSTEMS as f64,
    })
}

/// A serial the workbook's system can hold, or why it cannot.
///
/// The floor is where each system starts saying true things. Below it the 1900
/// system is describing a February that had 29 days, and the 1904 system is
/// describing days before the one it counts from, which Excel has no serial
/// for at all.
fn within_the_system(serial: f64, dates: DateSystem) -> Result<f64> {
    match dates {
        DateSystem::Date1900 if serial < FIRST_TRUE_DAY_1900 => Err(Error::usage(format!(
            "serial {serial} is before 1900-03-01, and this workbook is on the 1900 date \
             system. That system counts a 29th of February 1900, a day that never happened, \
             which Excel keeps for compatibility with Lotus 1-2-3, so no serial below \
             {FIRST_TRUE_DAY_1900} names the day a calendar would. Write a date from \
             1900-03-01 onwards, or store the number itself with write type number."
        ))),
        DateSystem::Date1904 if serial < 0.0 => Err(Error::usage(format!(
            "serial {serial} is before 1904-01-01, and this workbook is on the 1904 date \
             system, which counts from that day and has no serial for the ones before it. \
             Write a date from 1904-01-01 onwards, or store the number itself with write \
             type number."
        ))),
        _ => Ok(serial),
    }
}

/// A year, a month and a day, as an ISO date spells them.
type Civil = (i64, i64, i64);

/// The date `date` spells, checked against the calendar.
fn civil(date: &str, whole: &str) -> Result<Civil> {
    let mut fields = date.split('-');
    // A negative year would put an empty first field here. ISO 8601 spells one
    // with a leading sign and a fixed width, which is not a date any workbook
    // holds, so it falls through to the usage error like any other spelling.
    let parsed = (|| {
        let year: i64 = number(fields.next()?, 4)?;
        let month: i64 = number(fields.next()?, 2)?;
        let day: i64 = number(fields.next()?, 2)?;
        match fields.next() {
            None => Some((year, month, day)),
            Some(_) => None,
        }
    })();
    let (year, month, day) = parsed.ok_or_else(|| not_a_date(whole))?;
    if !(1..=12).contains(&month) || day < 1 || day > days_in(year, month) {
        return Err(Error::usage(format!(
            "'{whole}' is not a date: there is no {day:02} of month {month:02} in {year:04}"
        )));
    }
    Ok((year, month, day))
}

/// The part of a day `time` stands for: what an ISO time of day is as a
/// fraction of twenty-four hours.
fn day_fraction(time: &str, whole: &str) -> Result<f64> {
    let mut fields = time.split(':');
    let parsed = (|| {
        let hour: i64 = number(fields.next()?, 2)?;
        let minute: i64 = number(fields.next()?, 2)?;
        let seconds: f64 = match fields.next() {
            None => 0.0,
            // Only the seconds may carry a fraction, and it is the one field
            // whose width the schema lets vary past its two digits.
            Some(field) => match field.split_once('.') {
                None => number::<i64>(field, 2)? as f64,
                Some((whole, rest)) => {
                    let digits: i64 = number(whole, 2)?;
                    match rest.chars().all(|ch| ch.is_ascii_digit()) && !rest.is_empty() {
                        true => digits as f64 + format!("0.{rest}").parse::<f64>().ok()?,
                        false => return None,
                    }
                }
            },
        };
        match fields.next() {
            None => Some((hour, minute, seconds)),
            Some(_) => None,
        }
    })();
    let (hour, minute, seconds) = parsed.ok_or_else(|| not_a_date(whole))?;
    if hour > 23 || minute > 59 || seconds >= 60.0 {
        return Err(Error::usage(format!(
            "'{whole}' is not a date: there is no {hour:02}:{minute:02}:{seconds:02} in a day"
        )));
    }
    Ok((hour as f64 * 3600.0 + minute as f64 * 60.0 + seconds) / 86_400.0)
}

/// A field of exactly `width` ASCII digits, as the number it spells.
///
/// The width is fixed because ISO 8601 fixes it, and because a variable one
/// would make `2026-9-3` a date here and not in the package. A field carrying
/// anything but digits is not a number: `+1` and `1e2` are numbers to Rust and
/// are not fields of a date.
fn number<T: std::str::FromStr>(field: &str, width: usize) -> Option<T> {
    let digits = field.len() == width && field.chars().all(|ch| ch.is_ascii_digit());
    match digits {
        true => field.parse().ok(),
        false => None,
    }
}

fn not_a_date(text: &str) -> Error {
    Error::usage(format!(
        "'{text}' is not a date; write type date takes an ISO 8601 date such as \
         2026-09-13, a datetime such as 2026-09-13T14:30:00, or the serial itself. \
         A datetime carries no time zone, because a serial cannot hold one."
    ))
}

/// Whether `year` is a leap year in the proleptic Gregorian calendar.
fn leap(year: i64) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

/// How many days month `month` of `year` has.
fn days_in(year: i64, month: i64) -> i64 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap(year) => 29,
        2 => 28,
        _ => 0,
    }
}

/// The days from 1970-01-01 to the given date, negative before it.
///
/// Howard Hinnant's `days_from_civil`, which is exact over the whole proleptic
/// Gregorian calendar and needs no table: March is treated as the first month
/// of the year, so a leap day lands at the end and the arithmetic never has to
/// know whether it happened.
fn days_from_civil((year, month, day): Civil) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let shifted = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * shifted + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;

    /// Serials transcribed from Excel rather than derived from the code: each
    /// is what Excel shows for that date on the 1900 system.
    const KNOWN: [(&str, f64); 6] = [
        ("1900-03-01", 61.0),
        ("1970-01-01", 25_569.0),
        ("2000-02-29", 36_585.0),
        ("2026-09-13", 46_278.0),
        ("2100-03-01", 73_110.0),
        ("9999-12-31", 2_958_465.0),
    ];

    fn of(text: &str) -> f64 {
        serial(text, DateSystem::Date1900).unwrap_or_else(|err| panic!("{text}: {err}"))
    }

    #[test]
    fn a_date_is_the_serial_excel_shows_for_it() {
        for (text, expected) in KNOWN {
            assert_eq!(of(text), expected, "{text}");
        }
    }

    /// The two systems are a fixed distance apart, over every date both of
    /// them hold. The 1904 system starts later, so the earliest of these is
    /// not one of them.
    #[test]
    fn the_1904_system_is_1462_lower_all_the_way_along() {
        for (text, expected) in KNOWN {
            if expected < 1462.0 {
                continue;
            }
            let shifted = serial(text, DateSystem::Date1904).unwrap_or_else(|err| panic!("{err}"));
            assert_eq!(shifted, expected - 1462.0, "{text}");
        }
        assert_eq!(
            serial("1904-01-01", DateSystem::Date1900).expect("a date in both"),
            1462.0,
            "the day the 1904 system counts from is 1462 in the 1900 system"
        );
    }

    #[test]
    fn a_serial_and_the_date_it_stands_for_are_the_same_value() {
        for (text, expected) in KNOWN {
            assert_eq!(of(&expected.to_string()), of(text), "{text}");
        }
    }

    #[test]
    fn a_time_of_day_is_the_part_of_the_day_it_is() {
        assert_eq!(of("2026-09-13T00:00:00"), 46_278.0);
        assert_eq!(of("2026-09-13T12:00"), 46_278.5);
        assert_eq!(of("2026-09-13T06:00:00"), 46_278.25);
        assert_eq!(of("2026-09-13T23:59:59"), 46_278.0 + 86_399.0 / 86_400.0);
        assert_eq!(of("2026-09-13T00:00:00.5"), 46_278.0 + 0.5 / 86_400.0);
    }

    #[test]
    fn a_leap_day_is_a_day_only_where_there_is_one() {
        assert!(serial("2024-02-29", DateSystem::Date1900).is_ok());
        for absent in ["2023-02-29", "2100-02-29", "1900-02-29"] {
            let err = serial(absent, DateSystem::Date1900).expect_err(absent);
            assert_eq!(err.code(), ErrorCode::Usage, "{absent}");
        }
    }

    /// The gap the 1900 system carries is refused rather than guessed at, and
    /// the message says why there is one.
    #[test]
    fn a_date_before_the_phantom_leap_day_is_refused_with_the_reason() {
        for early in ["1900-01-01", "1900-02-28", "1899-12-31", "60", "0", "-5"] {
            let err = serial(early, DateSystem::Date1900).expect_err(early);
            assert_eq!(err.code(), ErrorCode::Usage, "{early}");
            assert!(
                err.message().contains("29th of February 1900"),
                "{early}: {}",
                err.message()
            );
        }
    }

    /// The 1904 system has no phantom day, so it holds the dates the 1900
    /// system refuses, down to the day it counts from.
    #[test]
    fn the_1904_system_counts_from_its_own_first_day() {
        assert_eq!(
            serial("1904-01-01", DateSystem::Date1904).expect("day zero"),
            0.0
        );
        assert_eq!(
            serial("1904-01-02", DateSystem::Date1904).expect("day one"),
            1.0
        );
        let err = serial("1903-12-31", DateSystem::Date1904).expect_err("before day zero");
        assert_eq!(err.code(), ErrorCode::Usage);
        assert!(err.message().contains("1904-01-01"), "{err}");
    }

    #[test]
    fn a_spelling_that_is_not_a_date_says_what_one_looks_like() {
        for text in [
            "",
            "today",
            "2026-09",
            "2026-9-13",
            "2026-09-13T",
            "2026-09-13T25:00",
            "2026-09-13T12:60",
            "2026-09-13T12:00:60",
            "2026-09-13Z",
            "2026-09-13T12:00:00Z",
            "2026-09-13T12:00:00+01:00",
            "2026/09/13",
            "13-09-2026",
            "NaN",
            "inf",
        ] {
            let err = serial(text, DateSystem::Date1900).expect_err(text);
            assert_eq!(err.code(), ErrorCode::Usage, "{text}");
        }
    }

    #[test]
    fn whitespace_around_a_serial_is_not_part_of_it() {
        assert_eq!(of(" 46278 "), 46_278.0);
    }
}
