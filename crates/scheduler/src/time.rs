//! Schedule arithmetic on local wall-clock time (pure, unit tested).

use chrono::{Datelike, NaiveDate, NaiveDateTime, NaiveTime};
use velox_types::Schedule;

/// Parse "HH:MM" (24-hour).
pub fn parse_hhmm(s: &str) -> Option<NaiveTime> {
    let (h, m) = s.trim().split_once(':')?;
    let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
    NaiveTime::from_hms_opt(h, m, 0)
}

fn day_allowed(days: &[u8], date: NaiveDate) -> bool {
    days.is_empty() || days.contains(&(date.weekday().num_days_from_sunday() as u8))
}

/// Did the daily time `t` occur in `(prev, now]` on an allowed day?
///
/// Edge-triggered, so a queue the user stopped inside its window is not
/// restarted on the next tick, and a start time missed while the computer
/// slept still fires on wake-up (gaps are scanned day by day, at most 8).
pub fn fired(t: NaiveTime, days: &[u8], prev: NaiveDateTime, now: NaiveDateTime) -> bool {
    if now <= prev {
        return false;
    }
    let mut d = prev.date().max(now.date() - chrono::Days::new(8));
    while d <= now.date() {
        let at = d.and_time(t);
        if at > prev && at <= now && day_allowed(days, d) {
            return true;
        }
        match d.succ_opt() {
            Some(n) => d = n,
            None => break,
        }
    }
    false
}

/// What a schedule asks for between two ticks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    Start,
    Stop,
}

/// Start/stop events of `schedule` in `(prev, now]`. When both fire in the
/// same interval (e.g. after a long sleep), the later one wins.
pub fn triggers(schedule: &Schedule, prev: NaiveDateTime, now: NaiveDateTime) -> Option<Trigger> {
    if !schedule.enabled {
        return None;
    }
    let start = schedule.start_time.as_deref().and_then(parse_hhmm);
    let stop = schedule.stop_time.as_deref().and_then(parse_hhmm);
    let s = start.filter(|t| fired(*t, &schedule.days, prev, now));
    let e = stop.filter(|t| fired(*t, &schedule.days, prev, now));
    match (s, e) {
        (Some(_), None) => Some(Trigger::Start),
        (None, Some(_)) => Some(Trigger::Stop),
        (Some(a), Some(b)) => {
            // The most recent occurrence decides.
            let last = |t: NaiveTime| {
                let today = now.date().and_time(t);
                if today <= now {
                    today
                } else {
                    today - chrono::Days::new(1)
                }
            };
            Some(if last(a) >= last(b) {
                Trigger::Start
            } else {
                Trigger::Stop
            })
        }
        (None, None) => None,
    }
}

/// Is `now` inside the schedule's window (start ≤ now < stop, possibly
/// across midnight)? Used once at startup, so a queue whose start time
/// passed while the app was closed still runs inside its window.
pub fn in_window(schedule: &Schedule, now: NaiveDateTime) -> bool {
    if !schedule.enabled {
        return false;
    }
    let (Some(start), Some(stop)) = (
        schedule.start_time.as_deref().and_then(parse_hhmm),
        schedule.stop_time.as_deref().and_then(parse_hhmm),
    ) else {
        return false;
    };
    let t = now.time();
    if start <= stop {
        start <= t && t < stop && day_allowed(&schedule.days, now.date())
    } else {
        // Overnight window: the start day decides.
        (t >= start && day_allowed(&schedule.days, now.date()))
            || (t < stop
                && now
                    .date()
                    .pred_opt()
                    .is_some_and(|d| day_allowed(&schedule.days, d)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> NaiveDateTime {
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").unwrap()
    }

    fn sched(start: Option<&str>, stop: Option<&str>, days: &[u8]) -> Schedule {
        Schedule {
            enabled: true,
            start_time: start.map(Into::into),
            stop_time: stop.map(Into::into),
            days: days.to_vec(),
        }
    }

    #[test]
    fn parses_times() {
        assert_eq!(parse_hhmm("02:30"), NaiveTime::from_hms_opt(2, 30, 0));
        assert_eq!(parse_hhmm(" 23:59 "), NaiveTime::from_hms_opt(23, 59, 0));
        assert_eq!(parse_hhmm("24:00"), None);
        assert_eq!(parse_hhmm("7"), None);
        assert_eq!(parse_hhmm("aa:bb"), None);
    }

    #[test]
    fn fires_once_when_crossing_the_time() {
        let t = parse_hhmm("02:00").unwrap();
        // 2026-10-05 is a Monday.
        assert!(fired(
            t,
            &[],
            at("2026-10-05 01:59:59"),
            at("2026-10-05 02:00:00")
        ));
        assert!(!fired(
            t,
            &[],
            at("2026-10-05 02:00:00"),
            at("2026-10-05 02:00:01")
        ));
        assert!(!fired(
            t,
            &[],
            at("2026-10-05 01:00:00"),
            at("2026-10-05 01:59:00")
        ));
        // Across midnight.
        let t = parse_hhmm("00:00").unwrap();
        assert!(fired(
            t,
            &[],
            at("2026-10-05 23:59:59"),
            at("2026-10-06 00:00:01")
        ));
    }

    #[test]
    fn respects_days_of_week() {
        let t = parse_hhmm("02:00").unwrap();
        // Monday = 1.
        assert!(fired(
            t,
            &[1],
            at("2026-10-05 01:59:00"),
            at("2026-10-05 02:01:00")
        ));
        assert!(!fired(
            t,
            &[0, 6],
            at("2026-10-05 01:59:00"),
            at("2026-10-05 02:01:00")
        ));
    }

    #[test]
    fn missed_while_asleep_still_fires() {
        let t = parse_hhmm("02:00").unwrap();
        assert!(fired(
            t,
            &[],
            at("2026-10-04 23:00:00"),
            at("2026-10-05 06:00:00")
        ));
        // A month-long gap only scans the last 8 days but still fires.
        assert!(fired(
            t,
            &[],
            at("2026-09-01 23:00:00"),
            at("2026-10-05 06:00:00")
        ));
    }

    #[test]
    fn start_and_stop_triggers() {
        let s = sched(Some("02:00"), Some("07:00"), &[]);
        assert_eq!(
            triggers(&s, at("2026-10-05 01:59:00"), at("2026-10-05 02:00:00")),
            Some(Trigger::Start)
        );
        assert_eq!(
            triggers(&s, at("2026-10-05 06:59:00"), at("2026-10-05 07:00:00")),
            Some(Trigger::Stop)
        );
        assert_eq!(
            triggers(&s, at("2026-10-05 03:00:00"), at("2026-10-05 03:00:01")),
            None
        );
        // Slept from 01:00 to 08:00: the stop is the latest event.
        assert_eq!(
            triggers(&s, at("2026-10-05 01:00:00"), at("2026-10-05 08:00:00")),
            Some(Trigger::Stop)
        );
        // Slept from 06:00 to 03:00 next day: the start is the latest event.
        assert_eq!(
            triggers(&s, at("2026-10-05 06:00:00"), at("2026-10-06 03:00:00")),
            Some(Trigger::Start)
        );
        let off = Schedule {
            enabled: false,
            ..s
        };
        assert_eq!(
            triggers(&off, at("2026-10-05 01:59:00"), at("2026-10-05 02:00:00")),
            None
        );
    }

    #[test]
    fn windows_including_overnight() {
        let day = sched(Some("09:00"), Some("17:00"), &[]);
        assert!(in_window(&day, at("2026-10-05 12:00:00")));
        assert!(!in_window(&day, at("2026-10-05 17:00:00")));
        assert!(!in_window(&day, at("2026-10-05 08:00:00")));
        let night = sched(Some("23:00"), Some("06:00"), &[1]); // Monday nights only
        assert!(in_window(&night, at("2026-10-05 23:30:00")));
        assert!(in_window(&night, at("2026-10-06 05:00:00"))); // Tuesday morning, Monday's window
        assert!(!in_window(&night, at("2026-10-07 05:00:00"))); // Wednesday morning
        assert!(!in_window(
            &sched(Some("09:00"), None, &[]),
            at("2026-10-05 12:00:00")
        ));
    }
}
