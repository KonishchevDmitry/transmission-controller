use std::cmp::Ordering;

use time;
use regex::Regex;

use common::GenericResult;

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct Time {
    pub hour: u8,
    pub minute: u8,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct Period {
    pub start: Time,
    pub end: Time,
}

pub type Duration = i64;
pub type Timestamp = i64;
pub type DayPeriods = Vec<Period>;
pub type WeekPeriods = Vec<DayPeriods>;

#[allow(clippy::ptr_arg)]
pub fn is_now_in(periods: &WeekPeriods) -> bool {
    is_in(periods, &time::now())
}

#[allow(clippy::ptr_arg)]
pub fn is_in(periods: &WeekPeriods, now: &time::Tm) -> bool {
    let cur = Time{
        hour: now.tm_hour as u8,
        minute: now.tm_min as u8,
    };

    for period in &periods[now.tm_wday as usize] {
        if cur < period.start {
            break;
        }

        if cur <= period.end {
            return true;
        }
    }

    false
}

pub fn parse_duration(string: &str) -> GenericResult<Duration> {
    let re = Regex::new(r"^(?P<number>[1-9]\d*)(?P<unit>[mhd])$").unwrap();
    let captures = re.captures(string).ok_or(format!(
        "Invalid time specification: {}", string))?;

    let mut duration = captures.name("number").unwrap().as_str().parse::<Duration>().unwrap();
    duration *= match captures.name("unit").unwrap().as_str() {
        "m" => 60,
        "h" => 60 * 60,
        "d" => 60 * 60 * 24,
        _ => unreachable!(),
    };

    Ok(duration)
}

pub fn parse_periods(period_strings: &[String]) -> GenericResult<WeekPeriods> {
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
        let captures = period_re.captures(period_string).ok_or(format!(
            "Invalid period specification: {}", period_string))?;

        let start_day = captures.name("start_day").unwrap().as_str().parse::<u8>().unwrap();
        let end_day = match captures.name("end_day") {
            Some(day) => {
                let day = day.as_str().parse::<u8>().unwrap();
                if day < start_day {
                    return Err!("Invalid period of days in '{}'", period_string);
                }
                day
            },
            None => start_day,
        };

        let start_hour = captures.name("start_hour").unwrap().as_str().parse::<u8>().unwrap();
        let start_minute = captures.name("start_minute").unwrap().as_str().parse::<u8>().unwrap();
        let end_hour = captures.name("end_hour").unwrap().as_str().parse::<u8>().unwrap();
        let end_minute = captures.name("end_minute").unwrap().as_str().parse::<u8>().unwrap();

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

        let period = Period {
            start: Time { hour: start_hour, minute: start_minute },
            end: Time { hour: end_hour, minute: end_minute },
        };

        if period.start > period.end {
            return Err!("Invalid period of time in '{}'", period_string);
        }

        for day in start_day .. end_day + 1 {
            // Convert "Monday-Sunday [1-7]" into "Sunday-Saturday [0-6]" which is used in
            // time::Tm::tm_wday.
            let tm_wday = day % 7;
            week_periods[tm_wday as usize].push(period);
        }
    }

    for day_periods in &mut week_periods {
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


#[cfg(test)]
mod tests {
    use time;
    use super::*;

    impl Time {
        fn new(hour: u8, minute: u8) -> Time {
            Time { hour: hour, minute: minute }
        }
    }

    impl Period {
        fn new(start: Time, end: Time) -> Period {
            Period { start: start, end: end }
        }
    }

    #[test]
    fn test_is_in() {
        use time::Tm;

        let weekend_periods = vec![Period::new(Time::new(0, 0), Time::new(8, 59))];
        let weekdays_periods = vec![Period::new(Time::new(0, 0), Time::new(5, 19)),
                                    Period::new(Time::new(6, 20), Time::new(7, 9))];

        let periods = vec![
            weekend_periods.clone(),
            weekdays_periods.clone(),
            weekdays_periods.clone(),
            weekdays_periods.clone(),
            weekdays_periods.clone(),
            weekdays_periods.clone(),
            weekend_periods.clone(),
        ];

        let empty = time::empty_tm();
        assert!(is_in(&periods, &empty));

        for wday in 0..7 {
            let day = Tm { tm_wday: wday, .. empty };

            let now = Tm { tm_hour: 6, .. day };
            assert_eq!(is_in(&periods, &now), match wday {
                0 | 6 => true,
                1..=5 => false,
                _ => unreachable!(),
            });

            let now = Tm { tm_wday: wday, tm_hour: 7, .. day };
            assert!(is_in(&periods, &now));

            let now = Tm { tm_wday: wday, tm_hour: 8, tm_min: 59, .. day };
            assert_eq!(is_in(&periods, &now), match wday {
                0 | 6 => true,
                1..=5 => false,
                _ => unreachable!(),
            });

            let now = Tm { tm_wday: wday, tm_hour: 9, .. day };
            assert!(!is_in(&periods, &now));
        }
    }

    #[test]
    fn test_parse_periods() {
        let period_strings = ["1-5/6:20-7:09", "1-5/0:00-5:19", "6-7/0:00-8:59"]
            .iter().map(|s| s!(*s)).collect::<Vec<_>>();

        let weekend_periods = vec![Period::new(Time::new(0, 0), Time::new(8, 59))];
        let weekdays_periods = vec![Period::new(Time::new(0, 0), Time::new(5, 19)),
                                    Period::new(Time::new(6, 20), Time::new(7, 9))];

        assert_eq!(
            parse_periods(&period_strings).unwrap(),
            vec![
                weekend_periods.clone(),
                weekdays_periods.clone(),
                weekdays_periods.clone(),
                weekdays_periods.clone(),
                weekdays_periods.clone(),
                weekdays_periods.clone(),
                weekend_periods.clone(),
            ]
        );
    }
}
