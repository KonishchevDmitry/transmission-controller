use std::cmp::Ordering;

use time;
use regex::Regex;

use common::GenericResult;

#[derive(Debug, Copy, Clone)]
pub struct Time {
    pub hour: u8,
    pub minute: u8,
}

#[derive(Debug, Copy, Clone)]
pub struct Period {
    pub start: Time,
    pub end: Time,
}

pub type Duration = i64;
pub type Timestamp = i64;
pub type DayPeriods = Vec<Period>;
pub type WeekPeriods = Vec<DayPeriods>;

pub fn is_now_in(periods: &WeekPeriods) -> bool {
    let now = time::now();
    let cur = Time{
        hour: now.tm_hour as u8,
        minute: now.tm_min as u8,
    };

    for period in &periods[now.tm_wday as usize] {
        if period.start > cur {
            break;
        }

        if period.end < cur {
            continue;
        }

        return true;
    }

    false
}

pub fn parse_duration(string: &str) -> GenericResult<Duration> {
    let re = Regex::new(r"^(?P<number>[1-9]\d*)(?P<unit>[mhd])$").unwrap();
    let captures = try!(re.captures(string).ok_or(format!(
        "Invalid time specification: {}", string)));

    let mut duration = captures.name("number").unwrap().parse::<Duration>().unwrap();
    duration *= match captures.name("unit").unwrap() {
        "m" => 60,
        "h" => 60 * 60,
        "d" => 60 * 60 * 24,
        _ => unreachable!(),
    };

    Ok(duration)
}

pub fn parse_periods(period_strings: &Vec<String>) -> GenericResult<WeekPeriods> {
    let mut week_periods = Vec::with_capacity(7);
    for _ in 0..7 {
        week_periods.push(Vec::new());
    }

    let period_re = Regex::new(r"(?x)^
        \s*(?P<start_day>[1-7])
        (?:\s*-\s*(?P<end_day>[1-7]))
        \s*/
        \s*(?P<start_hour>\d{1,2})\s*:\s*(?P<start_minute>\d{2})
        \s*-
        \s*(?P<end_hour>\d{1,2})\s*:\s*(?P<end_minute>\d{2})
        \s*$
    ").unwrap();

    for period_string in period_strings {
        let captures = try!(period_re.captures(period_string).ok_or(format!(
            "Invalid period specification: {}", period_string)));

        let start_day = captures.name("start_day").unwrap().parse::<u8>().unwrap();
        let end_day = match captures.name("end_day") {
            Some(day) => {
                let day = day.parse::<u8>().unwrap();
                if day < start_day {
                    return Err!("Invalid period of days in '{}'", period_string);
                }
                day
            },
            None => start_day,
        };

        let start_hour = captures.name("start_hour").unwrap().parse::<u8>().unwrap();
        let start_minute = captures.name("start_minute").unwrap().parse::<u8>().unwrap();
        let end_hour = captures.name("end_hour").unwrap().parse::<u8>().unwrap();
        let end_minute = captures.name("end_minute").unwrap().parse::<u8>().unwrap();

        for hour in &[start_hour, end_hour] {
            if *hour > 24 {
                return Err!("Invalid hour in '{}' period: {}", period_string, hour);
            }
        }

        for minute in &[start_minute, end_minute] {
            if *minute > 59 {
                return Err!("Invalid minute in '{}' period: {}", period_string, minute);
            }
        }

        let period = Period{
            start: Time{hour: start_hour, minute: start_minute},
            end: Time{hour: end_hour, minute: end_minute},
        };

        if period.start > period.end {
            return Err!("Invalid period of time in '{}'", period_string);
        }

        for day in start_day .. end_day + 1 {
            week_periods[day as usize - 1].push(period);
        }
    }

    for mut day_periods in &mut week_periods {
        day_periods.sort_by(|a, b| a.start.cmp(&b.start));

        let mut prev: Option<Time> = None;
        for period in day_periods {
            if let Some(prev) = prev {
                if prev >= period.start {
                    return Err!("Periods mustn't overlap");
                }
            }

            prev = Some(period.end);
        }
    }

    Ok(week_periods)
}


impl PartialEq for Time {
    fn eq(&self, other: &Time) -> bool {
        self.hour == other.hour && self.minute == other.minute
    }
}

impl Eq for Time {}

impl PartialOrd for Time {
    fn partial_cmp(&self, other: &Time) -> Option<Ordering> {
        Some(Ord::cmp(self, other))
    }
}

impl Ord for Time {
    fn cmp(&self, other: &Time) -> Ordering {
        let mut result = self.hour.cmp(&other.hour);
        if result == Ordering::Equal {
            result = self.minute.cmp(&other.minute);
        }
        result
    }
}
