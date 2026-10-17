use std::time::{Duration, SystemTime};

use jiff::{Timestamp, civil::Weekday, tz::TimeZone};
use timeago::Formatter;

use crate::domain::{model::Account, vo::AccountId};

pub fn display_name(account: Option<&Account>, fallback_id: AccountId) -> String {
    let Some(meta) = account.and_then(Account::platform_meta) else {
        return format!("User{}", fallback_id);
    };

    let full_name = meta.full_name();
    let username = meta.username();

    match username {
        Some(u) => format!("{} (@{})", full_name, u),
        None => full_name,
    }
}

/// Formats a timestamp: relative time within 24 hours, absolute time beyond.
pub fn format_time_dyn(ts: impl Into<Option<Timestamp>>) -> String {
    let Some(ts) = ts.into() else {
        return "None".to_string();
    };
    let now = Timestamp::now();
    if now.duration_since(ts).as_secs() < 86400 {
        format_relative_time(ts)
    } else {
        ts.to_zoned(TimeZone::system()).to_string()
    }
}

pub fn format_relative_time(ts: Timestamp) -> String {
    let then = SystemTime::UNIX_EPOCH + Duration::from_secs(ts.as_second() as u64);
    let duration = SystemTime::now().duration_since(then).unwrap_or_default();
    Formatter::new().convert(duration)
}

pub fn format_time_only(ts: impl Into<Option<Timestamp>>) -> String {
    let Some(ts) = ts.into() else {
        return String::new();
    };
    ts.to_zoned(TimeZone::system())
        .strftime("%H:%M")
        .to_string()
}

/// Date label: today, yesterday, weekday, month and day, or full date with the year.
pub fn format_date_label(ts: impl Into<Option<Timestamp>>) -> String {
    let Some(ts) = ts.into() else {
        return String::new();
    };
    let now = Timestamp::now();
    let secs = now.as_second() - ts.as_second();
    if secs < 0 {
        return String::from("today");
    }
    let days = secs / 86400;

    if days < 1 {
        "today".into()
    } else if days < 2 {
        "yesterday".into()
    } else if days < 7 {
        let wd = ts.to_zoned(TimeZone::system()).weekday();
        match wd {
            Weekday::Monday => "Monday",
            Weekday::Tuesday => "Tuesday",
            Weekday::Wednesday => "Wednesday",
            Weekday::Thursday => "Thursday",
            Weekday::Friday => "Friday",
            Weekday::Saturday => "Saturday",
            Weekday::Sunday => "Sunday",
        }
        .into()
    } else {
        let z = ts.to_zoned(TimeZone::system());
        let now_z = jiff::Zoned::now();
        if z.year() == now_z.year() {
            z.strftime("%b %-d").to_string()
        } else {
            z.strftime("%b %-d, %Y").to_string()
        }
    }
}
