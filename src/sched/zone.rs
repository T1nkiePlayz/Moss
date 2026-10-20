//! Time zones for schedules and the pure `next_run` computation. A zone is an IANA name (`Europe/Berlin`) or a
//! fixed offset written `UTC+HH:MM` (what old clients' `tz_offset_min` is stored as).
use chrono::{DateTime, Duration as CD, FixedOffset, LocalResult, NaiveDate, NaiveTime, Offset, TimeZone, Utc};
use chrono_tz::Tz;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Zone { Iana(Tz), Fixed(FixedOffset) }

pub fn parse_hhmm(s: &str) -> Option<NaiveTime> { NaiveTime::parse_from_str(s, "%H:%M").ok() }

impl Zone {
    pub const UTC: Zone = Zone::Iana(Tz::UTC);

    /// Accepts `UTC`, `UTC+05:30` / `UTC-08:00` or any IANA name.
    pub fn parse(s: &str) -> Option<Zone> {
        let s = s.trim();
        if let Some(rest) = s.strip_prefix("UTC").filter(|r| r.starts_with(['+', '-'])) {
            let sign = if rest.starts_with('-') { -1 } else { 1 };
            let (h, m) = rest[1..].split_once(':')?;
            let (h, m): (i32, i32) = (h.parse().ok()?, m.parse().ok()?);
            if h > 14 || m > 59 || h * 60 + m > 840 { return None; }
            return FixedOffset::east_opt(sign * (h * 3600 + m * 60)).map(Zone::Fixed);
        }
        s.parse::<Tz>().ok().map(Zone::Iana)
    }

    pub fn fixed_minutes(min: i64) -> Option<Zone> { FixedOffset::east_opt(i32::try_from(min * 60).ok()?).map(Zone::Fixed) }

    /// The canonical stored name.
    pub fn name(&self) -> String {
        match self {
            Zone::Iana(t) => t.name().to_string(),
            Zone::Fixed(o) => {
                let s = o.local_minus_utc();
                if s == 0 { "UTC".into() } else { format!("UTC{}{:02}:{:02}", if s < 0 { '-' } else { '+' }, s.abs() / 3600, s.abs() % 3600 / 60) }
            }
        }
    }

    /// UTC offset in minutes at the given instant.
    pub fn offset_min_at(&self, at: DateTime<Utc>) -> i64 {
        match self {
            Zone::Iana(t) => t.offset_from_utc_datetime(&at.naive_utc()).fix().local_minus_utc() as i64 / 60,
            Zone::Fixed(o) => o.local_minus_utc() as i64 / 60,
        }
    }

    fn local_to_utc(&self, naive: chrono::NaiveDateTime) -> LocalResult<DateTime<Utc>> {
        match self {
            Zone::Iana(t) => t.from_local_datetime(&naive).map(|d| d.with_timezone(&Utc)),
            Zone::Fixed(o) => o.from_local_datetime(&naive).map(|d| d.with_timezone(&Utc)),
        }
    }

    fn local_date(&self, at: DateTime<Utc>) -> NaiveDate {
        match self {
            Zone::Iana(t) => at.with_timezone(t).date_naive(),
            Zone::Fixed(o) => at.with_timezone(o).date_naive(),
        }
    }

    /// The instant a wall-clock time on `date` happens: a repeated time resolves to its first occurrence, a time
    /// skipped by a DST jump resolves to the first valid wall-clock time after the gap.
    fn instant_on(&self, date: NaiveDate, t: NaiveTime) -> Option<DateTime<Utc>> {
        let base = date.and_time(t);
        for step in 0..=180 {
            match self.local_to_utc(base + CD::minutes(step)) {
                LocalResult::Single(d) => return Some(d),
                LocalResult::Ambiguous(first, _) => return Some(first),
                LocalResult::None => continue,
            }
        }
        None
    }
}

/// The next time a schedule is due after `from`. Intervals simply add; `daily_at` follows local wall-clock time.
pub fn next_run(every: Option<i64>, daily: Option<&str>, zone: &Zone, from: DateTime<Utc>) -> Option<DateTime<Utc>> {
    if let Some(m) = every { return Some(from + CD::minutes(m.max(1))); }
    let t = parse_hhmm(daily?)?;
    let today = zone.local_date(from);
    (-1..=2).filter_map(|d| zone.instant_on(today + CD::days(d), t)).find(|i| *i > from)
}

/// Every IANA zone name Moss accepts, plus `UTC`, sorted.
pub fn zone_names() -> Vec<&'static str> {
    let mut v: Vec<&'static str> = chrono_tz::TZ_VARIANTS.iter().map(|t| t.name()).collect();
    v.sort_unstable();
    v.dedup();
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn utc(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime<Utc> { Utc.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap() }
    fn z(s: &str) -> Zone { Zone::parse(s).unwrap() }

    #[test]
    fn parses_names_and_fixed_offsets() {
        assert_eq!(Zone::parse("UTC+05:30").unwrap().name(), "UTC+05:30");
        assert_eq!(Zone::parse("UTC-08:00").unwrap().name(), "UTC-08:00");
        assert_eq!(Zone::fixed_minutes(0).unwrap().name(), "UTC");
        assert_eq!(z("Europe/Berlin").name(), "Europe/Berlin");
        for bad in ["Mars/Base", "UTC+25:00", "UTC+aa:00", "", "UTC+1"] { assert!(Zone::parse(bad).is_none(), "{bad}"); }
        assert!(zone_names().contains(&"Pacific/Chatham") && zone_names().contains(&"UTC"));
    }

    #[test]
    fn table_of_daily_runs() {
        // (zone, daily_at, from, expected)
        let cases = [
            ("UTC", "03:00", utc(2026, 1, 1, 12, 0), utc(2026, 1, 2, 3, 0)),
            ("UTC+08:00", "03:00", utc(2026, 1, 1, 12, 0), utc(2026, 1, 1, 19, 0)),
            ("UTC+08:00", "03:00", utc(2026, 1, 1, 20, 0), utc(2026, 1, 2, 19, 0)),
            // Berlin: CET (+1) in winter, CEST (+2) in summer
            ("Europe/Berlin", "03:00", utc(2026, 1, 10, 0, 0), utc(2026, 1, 10, 2, 0)),
            ("Europe/Berlin", "03:00", utc(2026, 7, 10, 0, 0), utc(2026, 7, 10, 1, 0)),
            // Berlin spring forward 2026-03-29: 02:00-03:00 does not exist; 02:30 runs at 03:00 local (01:00 UTC)
            ("Europe/Berlin", "02:30", utc(2026, 3, 28, 12, 0), utc(2026, 3, 29, 1, 0)),
            // ...and the day after is a normal 02:30 CEST = 00:30 UTC
            ("Europe/Berlin", "02:30", utc(2026, 3, 29, 1, 0), utc(2026, 3, 30, 0, 30)),
            // Berlin fall back 2026-10-25: 02:30 happens twice, runs at the first (00:30 UTC)
            ("Europe/Berlin", "02:30", utc(2026, 10, 24, 12, 0), utc(2026, 10, 25, 0, 30)),
            // after the first occurrence ran, the repeated one (01:30 UTC) is skipped: next is the 26th, 01:30 UTC
            ("Europe/Berlin", "02:30", utc(2026, 10, 25, 0, 30), utc(2026, 10, 26, 1, 30)),
            ("Europe/Berlin", "02:30", utc(2026, 10, 25, 1, 0), utc(2026, 10, 26, 1, 30)),
            // New York spring forward 2026-03-08 (02:00 -> 03:00 EDT, UTC-4): 02:30 -> 03:00 EDT = 07:00 UTC
            ("America/New_York", "02:30", utc(2026, 3, 7, 12, 0), utc(2026, 3, 8, 7, 0)),
            ("America/New_York", "03:00", utc(2026, 3, 8, 0, 0), utc(2026, 3, 8, 7, 0)),
            // New York fall back 2026-11-01: 01:30 happens twice, first is EDT (05:30 UTC)
            ("America/New_York", "01:30", utc(2026, 10, 31, 12, 0), utc(2026, 11, 1, 5, 30)),
            ("America/New_York", "01:30", utc(2026, 11, 1, 5, 30), utc(2026, 11, 2, 6, 30)),
            // Chatham Islands: +13:45 in January (NZDT), +12:45 in July
            ("Pacific/Chatham", "03:00", utc(2026, 1, 1, 0, 0), utc(2026, 1, 1, 13, 15)),
            ("Pacific/Chatham", "03:00", utc(2026, 7, 1, 0, 0), utc(2026, 7, 1, 14, 15)),
            // local midnight and a date that rolls across the UTC day
            ("America/New_York", "00:00", utc(2026, 1, 15, 4, 0), utc(2026, 1, 15, 5, 0)),
            ("America/New_York", "00:00", utc(2026, 1, 15, 5, 0), utc(2026, 1, 16, 5, 0)),
        ];
        for (zone, at, from, want) in cases {
            assert_eq!(next_run(None, Some(at), &z(zone), from), Some(want), "{zone} {at} from {from}");
        }
    }

    #[test]
    fn every_dst_day_has_exactly_one_run_and_it_advances() {
        // walk a whole year of daily runs; each must be strictly later than the previous and ~24h apart (+-1h)
        for zone in ["Europe/Berlin", "America/New_York", "Pacific/Chatham", "Australia/Lord_Howe"] {
            let mut t = utc(2026, 1, 1, 0, 0);
            for _ in 0..365 {
                let n = next_run(None, Some("02:30"), &z(zone), t).unwrap();
                let gap = (n - t).num_minutes();
                assert!(gap > 0 && gap <= 25 * 60 + 30, "{zone} {t} -> {n}");
                t = n;
            }
        }
    }

    #[test]
    fn intervals_and_invalid_input() {
        let t = utc(2026, 1, 1, 12, 0);
        assert_eq!(next_run(Some(30), None, &Zone::UTC, t), Some(t + CD::minutes(30)));
        assert!(next_run(None, Some("25:99"), &Zone::UTC, t).is_none());
        assert!(next_run(None, None, &Zone::UTC, t).is_none());
    }

    #[test]
    fn current_offset_follows_dst() {
        assert_eq!(z("Europe/Berlin").offset_min_at(utc(2026, 1, 1, 0, 0)), 60);
        assert_eq!(z("Europe/Berlin").offset_min_at(utc(2026, 7, 1, 0, 0)), 120);
        assert_eq!(z("Pacific/Chatham").offset_min_at(utc(2026, 7, 1, 0, 0)), 765);
        assert_eq!(z("UTC-08:00").offset_min_at(utc(2026, 7, 1, 0, 0)), -480);
    }
}
