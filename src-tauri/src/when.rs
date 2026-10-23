//! Spoken times ("by Monday", "last week", "in September") as dates. The small model only finds the words;
//! the arithmetic happens here, where it can't be wrong.

use chrono::{Datelike, Duration, NaiveDate, Weekday};

const WEEKDAYS: [(&str, Weekday); 7] = [
    ("monday", Weekday::Mon),
    ("tuesday", Weekday::Tue),
    ("wednesday", Weekday::Wed),
    ("thursday", Weekday::Thu),
    ("friday", Weekday::Fri),
    ("saturday", Weekday::Sat),
    ("sunday", Weekday::Sun),
];
const MONTHS: [&str; 12] =
    ["january", "february", "march", "april", "may", "june", "july", "august", "september", "october", "november", "december"];

fn words(s: &str) -> Vec<String> {
    s.to_lowercase().split(|c: char| !c.is_alphanumeric() && c != '-').filter(|w| !w.is_empty()).map(String::from).collect()
}

fn month_end(y: i32, m: u32) -> NaiveDate {
    let (ny, nm) = if m == 12 { (y + 1, 1) } else { (y, m + 1) };
    NaiveDate::from_ymd_opt(ny, nm, 1).unwrap() - Duration::days(1)
}

fn week_start(d: NaiveDate) -> NaiveDate {
    d - Duration::days(d.weekday().num_days_from_monday() as i64)
}

fn day_number(w: &str) -> Option<u32> {
    let n: u32 = w.trim_end_matches(|c: char| c.is_alphabetic()).parse().ok()?;
    (1..=31).contains(&n).then_some(n)
}

/// When something is due: the first day the phrase can mean from today on.
pub fn due(phrase: &str, today: NaiveDate) -> Option<NaiveDate> {
    if let Ok(d) = NaiveDate::parse_from_str(phrase.trim(), "%Y-%m-%d") {
        return (d >= today).then_some(d);
    }
    let w = words(phrase);
    let has = |x: &str| w.iter().any(|v| v == x);
    if has("today") || has("tonight") {
        return Some(today);
    }
    if has("tomorrow") {
        return Some(today + Duration::days(1));
    }
    if has("end") && has("month") {
        return Some(month_end(today.year(), today.month()));
    }
    if has("end") && has("week") {
        // this week's Friday, or next week's once it's passed
        let friday = week_start(today) + Duration::days(4);
        return Some(if friday >= today { friday } else { friday + Duration::days(7) });
    }
    if has("next") && has("week") {
        return Some(week_start(today) + Duration::days(7));
    }
    if let Some((_, wd)) = WEEKDAYS.iter().find(|(n, _)| has(n)) {
        let ahead = (wd.num_days_from_monday() as i64 - today.weekday().num_days_from_monday() as i64).rem_euclid(7);
        return Some(today + Duration::days(if ahead == 0 { 7 } else { ahead }));
    }
    // "the 24th", "24 October", "October 24th"
    let month = w.iter().find_map(|v| MONTHS.iter().position(|m| m == v));
    let day = w.iter().find_map(|v| day_number(v));
    match (month, day) {
        (Some(m), Some(d)) => {
            let this = NaiveDate::from_ymd_opt(today.year(), m as u32 + 1, d)?;
            Some(if this >= today { this } else { NaiveDate::from_ymd_opt(today.year() + 1, m as u32 + 1, d)? })
        }
        (None, Some(d)) if has("the") || w.iter().any(|v| v.ends_with("th") || v.ends_with("st") || v.ends_with("nd") || v.ends_with("rd")) => {
            let this = NaiveDate::from_ymd_opt(today.year(), today.month(), d);
            match this.filter(|t| *t >= today) {
                Some(t) => Some(t),
                None => {
                    let (y, m) = if today.month() == 12 { (today.year() + 1, 1) } else { (today.year(), today.month() + 1) };
                    NaiveDate::from_ymd_opt(y, m, d)
                }
            }
        }
        _ => None,
    }
}

/// A time a search mentions, as the words used and the days they cover (up to today).
pub fn find_range(query: &str, today: NaiveDate) -> Option<(String, NaiveDate, NaiveDate)> {
    let w = words(query);
    let at = |i: usize| w.get(i).map(String::as_str).unwrap_or("");
    for i in 0..w.len() {
        let (a, b, c) = (at(i), at(i + 1), at(i + 2));
        let found = match (a, b) {
            ("today", _) => Some((1, today, today)),
            ("yesterday", _) => Some((1, today - Duration::days(1), today - Duration::days(1))),
            ("this", "week") => Some((2, week_start(today), today)),
            ("last", "week") | ("past", "week") => Some((2, week_start(today) - Duration::days(7), week_start(today) - Duration::days(1))),
            ("this", "month") => Some((2, today.with_day(1).unwrap(), today)),
            ("last", "month") | ("past", "month") => {
                let end = today.with_day(1).unwrap() - Duration::days(1);
                Some((2, end.with_day(1).unwrap(), end))
            }
            ("this", "year") => Some((2, NaiveDate::from_ymd_opt(today.year(), 1, 1).unwrap(), today)),
            ("last", "year") => Some((
                2,
                NaiveDate::from_ymd_opt(today.year() - 1, 1, 1).unwrap(),
                NaiveDate::from_ymd_opt(today.year() - 1, 12, 31).unwrap(),
            )),
            ("last" | "past", n) if c.starts_with("day") => n.parse::<i64>().ok().map(|n| (3, today - Duration::days(n), today)),
            (n, "days") if c == "ago" => n.parse::<i64>().ok().map(|n| (3, today - Duration::days(n), today - Duration::days(n))),
            (n, "weeks") if c == "ago" => n.parse::<i64>().ok().map(|n| {
                let start = week_start(today) - Duration::days(7 * n);
                (3, start, start + Duration::days(6))
            }),
            _ => None,
        };
        if let Some((len, from, to)) = found {
            return Some((w[i..i + len].join(" "), from, to));
        }
        if let Some((name, wd)) = WEEKDAYS.iter().find(|(n, _)| *n == a) {
            // the most recent one before today, or "last monday"
            let back = (today.weekday().num_days_from_monday() as i64 - wd.num_days_from_monday() as i64).rem_euclid(7);
            let back = if back == 0 { 7 } else { back };
            let d = today - Duration::days(back);
            let phrase = if i > 0 && (at(i - 1) == "last" || at(i - 1) == "on") { format!("{} {name}", at(i - 1)) } else { name.to_string() };
            return Some((phrase, d, d));
        }
        if let Some(m) = MONTHS.iter().position(|m| *m == a).filter(|_| a != "may" || (i > 0 && at(i - 1) == "in")) {
            let m = m as u32 + 1;
            let y = if m <= today.month() { today.year() } else { today.year() - 1 };
            let phrase = if i > 0 && at(i - 1) == "in" { format!("in {a}") } else { a.to_string() };
            return Some((phrase, NaiveDate::from_ymd_opt(y, m, 1).unwrap(), month_end(y, m).min(today)));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn due_dates() {
        let today = d("2026-10-23"); // a Friday
        assert_eq!(due("by Monday", today), Some(d("2026-10-26")));
        assert_eq!(due("on Friday", today), Some(d("2026-10-30")));
        assert_eq!(due("tomorrow", today), Some(d("2026-10-24")));
        assert_eq!(due("before the end of the month", today), Some(d("2026-10-31")));
        assert_eq!(due("next week", today), Some(d("2026-10-26")));
        assert_eq!(due("by the 3rd", today), Some(d("2026-11-03")));
        assert_eq!(due("November 4th", today), Some(d("2026-11-04")));
        assert_eq!(due("2026-12-01", today), Some(d("2026-12-01")));
        assert_eq!(due("soon", today), None);
    }

    #[test]
    fn search_ranges() {
        let today = d("2026-10-23");
        let r = |q: &str| find_range(q, today).map(|(p, a, b)| (p, a.to_string(), b.to_string()));
        assert_eq!(r("notes from yesterday"), Some(("yesterday".into(), "2026-10-22".into(), "2026-10-22".into())));
        assert_eq!(r("what Sarah said about pricing last week"), Some(("last week".into(), "2026-10-12".into(), "2026-10-18".into())));
        assert_eq!(r("meetings with Daniel in September"), Some(("in september".into(), "2026-09-01".into(), "2026-09-30".into())));
        assert_eq!(r("the call on Monday"), Some(("on monday".into(), "2026-10-19".into(), "2026-10-19".into())));
        assert_eq!(r("ideas from the last 10 days"), Some(("last 10 days".into(), "2026-10-13".into(), "2026-10-23".into())));
        assert_eq!(r("may we ship"), None);
        assert_eq!(r("refund policy"), None);
    }
}
