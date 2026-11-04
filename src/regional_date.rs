//! Localized medium dates and short times for model-list metadata.
use chrono::{Datelike, Timelike};
use icu_datetime::{
    DateTimeFormatter, fieldsets,
    input::{Date, DateTime, Time},
};

pub fn locale(french: bool, preferred: &[String]) -> &str {
    let language = if french { "fr" } else { "en" };
    preferred
        .iter()
        .find(|tag| {
            tag.split(['-', '_'])
                .next()
                .is_some_and(|l| l.eq_ignore_ascii_case(language))
        })
        .map(String::as_str)
        .unwrap_or(if french { "fr-CA" } else { "en-US" })
}

pub fn format<T: chrono::TimeZone>(date: &chrono::DateTime<T>, locale: &str) -> Option<String> {
    let locale: icu_locale_core::Locale = locale.replace('_', "-").parse().ok()?;
    let formatter =
        DateTimeFormatter::try_new(locale.into(), fieldsets::YMD::medium().with_time_hm()).ok()?;
    let input = DateTime {
        date: Date::try_new_iso(date.year(), date.month() as u8, date.day() as u8).ok()?,
        time: Time::try_new(date.hour() as u8, date.minute() as u8, 0, 0).ok()?,
    };
    // V8's Date.toLocaleString normalizes the narrow day-period separator.
    Some(
        formatter
            .format(&input)
            .to_string()
            .replace('\u{202f}', " "),
    )
}

pub fn local(timestamp: f64, french: bool) -> Option<String> {
    if !timestamp.is_finite() || timestamp.abs() > 8.64e15 {
        return None;
    }
    static PREFERRED: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    let preferred = PREFERRED.get_or_init(system_languages);
    let date = chrono::DateTime::from_timestamp_millis(timestamp as i64)?;
    format(
        &date.with_timezone(&chrono::Local),
        locale(french, preferred),
    )
}

#[cfg(windows)]
fn system_languages() -> Vec<String> {
    use windows::{
        Win32::Globalization::{GetUserPreferredUILanguages, MUI_LANGUAGE_NAME},
        core::PWSTR,
    };
    let mut count = 0;
    let mut length = 0;
    // The Windows MULTI_SZ is queried with a fixed allocation ceiling.
    if unsafe { GetUserPreferredUILanguages(MUI_LANGUAGE_NAME, &mut count, None, &mut length) }
        .is_err()
        || length == 0
        || length > 16_384
    {
        return Vec::new();
    }
    let mut buffer = vec![0u16; length as usize];
    if unsafe {
        GetUserPreferredUILanguages(
            MUI_LANGUAGE_NAME,
            &mut count,
            Some(PWSTR(buffer.as_mut_ptr())),
            &mut length,
        )
    }
    .is_err()
    {
        return Vec::new();
    }
    buffer
        .split(|c| *c == 0)
        .filter(|s| !s.is_empty())
        .take(32)
        .filter_map(|s| String::from_utf16(s).ok())
        .collect()
}

#[cfg(not(windows))]
fn system_languages() -> Vec<String> {
    ["LANGUAGE", "LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(|key| std::env::var(key).ok())
        .flat_map(|value| {
            value
                .split(':')
                .take(32)
                .map(|s| s.split(['.', '@']).next().unwrap_or("").replace('_', "-"))
                .collect::<Vec<_>>()
        })
        .filter(|s| !s.is_empty() && s.len() <= 128)
        .take(32)
        .collect()
}
