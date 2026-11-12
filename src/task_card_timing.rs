//! Local task-card date labels, independent from accumulated Doing duration.
use crate::task_timeline::Timeline;
use chrono::{Datelike, Timelike};
use icu_datetime::{
    DateTimeFormatter, fieldsets,
    input::{Date, DateTime, Time},
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Labels {
    pub text: String,
    pub tooltip: String,
}

pub fn duration(ms: u64, french: bool) -> String {
    let seconds = ms.saturating_add(500) / 1000;
    if seconds < 60 {
        return format!("{seconds} s");
    }
    let minutes = seconds.saturating_add(30) / 60;
    if minutes < 60 {
        return format!("{minutes} min");
    }
    let hours = minutes / 60;
    if hours < 24 {
        return format!("{hours} h {:02}", minutes % 60);
    }
    format!(
        "{} {} {} h",
        hours / 24,
        if french { "j" } else { "d" },
        hours % 24
    )
}

struct Formatters {
    clock: DateTimeFormatter<fieldsets::T>,
    day: DateTimeFormatter<fieldsets::MD>,
    full: DateTimeFormatter<fieldsets::YMDT>,
}
impl Formatters {
    fn new(locale: &str) -> Option<Self> {
        use icu_datetime::options::{Alignment, TimePrecision, YearStyle};
        let locale: icu_locale_core::Locale = locale.replace('_', "-").parse().ok()?;
        Some(Self {
            clock: DateTimeFormatter::try_new(
                locale.clone().into(),
                fieldsets::T::short()
                    .with_alignment(Alignment::Column)
                    .with_time_precision(TimePrecision::Minute),
            )
            .ok()?,
            day: DateTimeFormatter::try_new(locale.clone().into(), fieldsets::MD::medium()).ok()?,
            full: DateTimeFormatter::try_new(
                locale.into(),
                fieldsets::YMD::short()
                    .with_year_style(YearStyle::Full)
                    .with_time_hms(),
            )
            .ok()?,
        })
    }
}
thread_local! { static FORMATTERS: std::cell::RefCell<Option<(String,Formatters)>> = const { std::cell::RefCell::new(None) }; }
fn formatted<R>(locale: &str, f: impl FnOnce(&Formatters) -> R) -> Option<R> {
    FORMATTERS.with_borrow_mut(|cache| {
        if cache.as_ref().is_none_or(|(key, _)| key != locale) {
            let fallback = if locale.to_ascii_lowercase().starts_with("fr") {
                "fr-CA"
            } else {
                "en-US"
            };
            *cache = Some((
                locale.to_owned(),
                Formatters::new(locale).or_else(|| Formatters::new(fallback))?,
            ));
        }
        Some(f(&cache.as_ref()?.1))
    })
}
pub fn when<T: chrono::TimeZone>(
    date: &chrono::DateTime<T>,
    now: &chrono::DateTime<T>,
    locale: &str,
) -> Option<String> {
    let input = DateTime {
        date: Date::try_new_iso(date.year(), date.month() as u8, date.day() as u8).ok()?,
        time: Time::try_new(
            date.hour() as u8,
            date.minute() as u8,
            date.second() as u8,
            0,
        )
        .ok()?,
    };
    formatted(locale, |f| {
        let clock = f.clock.format(&input).to_string();
        let result = if date.date_naive() == now.date_naive() {
            clock
        } else {
            format!("{} {clock}", f.day.format(&input))
        };
        result.replace('\u{202f}', " ")
    })
}
fn full<T: chrono::TimeZone>(date: &chrono::DateTime<T>, locale: &str) -> Option<String> {
    let input = DateTime {
        date: Date::try_new_iso(date.year(), date.month() as u8, date.day() as u8).ok()?,
        time: Time::try_new(
            date.hour() as u8,
            date.minute() as u8,
            date.second() as u8,
            0,
        )
        .ok()?,
    };
    formatted(locale, |f| {
        f.full.format(&input).to_string().replace('\u{202f}', " ")
    })
}

pub fn labels(t: &Timeline, column: usize, now: u64, french: bool, locale: &str) -> Labels {
    let locale = if locale
        .replace('_', "-")
        .parse::<icu_locale_core::Locale>()
        .is_ok()
    {
        locale
    } else if french {
        "fr-CA"
    } else {
        "en-US"
    };
    let Some(now) = i64::try_from(now)
        .ok()
        .and_then(chrono::DateTime::from_timestamp_millis)
    else {
        return Labels::default();
    };
    let now = now.with_timezone(&chrono::Local);
    let date = |value: Option<u64>| {
        value
            .filter(|v| *v != 0 && *v <= 8_640_000_000_000_000)
            .and_then(|v| chrono::DateTime::from_timestamp_millis(v as i64))
            .map(|d| d.with_timezone(&chrono::Local))
    };
    let added = date(t.created_at_ms.filter(|v| *v != 0).or(if column == 0 {
        t.column_since_ms
    } else {
        None
    }));
    let start = date(t.started_at_ms.filter(|v| *v != 0).or(t.doing_since_ms));
    let done = if column == 3 {
        date(t.done_at_ms)
    } else {
        None
    };
    let short = |d: &chrono::DateTime<chrono::Local>| when(d, &now, locale).unwrap_or_default();
    let full = |d: &chrono::DateTime<chrono::Local>| full(d, locale).unwrap_or_default();
    let added_word = if french { "Ajoutée" } else { "Added" };
    let started_word = if french { "Commencée" } else { "Started" };
    let done_word = if french { "Terminée" } else { "Done" };
    let text = if column == 0 {
        added
            .as_ref()
            .map(|d| format!("{added_word} {}", short(d)))
            .unwrap_or_default()
    } else if let Some(done) = &done {
        if let Some(start) = &start {
            let elapsed = done
                .timestamp_millis()
                .saturating_sub(start.timestamp_millis())
                .max(0) as u64;
            format!(
                "{started_word} {} · {} {} · {}",
                short(start),
                if french { "terminée" } else { "done" },
                short(done),
                duration(elapsed, french)
            )
        } else {
            format!("{done_word} {}", short(done))
        }
    } else {
        start
            .as_ref()
            .map(|d| format!("{started_word} {}", short(d)))
            .unwrap_or_default()
    };
    let mut parts = Vec::new();
    if let Some(d) = &added {
        parts.push(format!("{added_word} {}", full(d)));
    }
    if let Some(d) = &start {
        parts.push(format!("{started_word} {}", full(d)));
    }
    if let Some(d) = &done {
        parts.push(format!(
            "{} {}",
            if french { "Terminée" } else { "Finished" },
            full(d)
        ));
    }
    Labels {
        text,
        tooltip: parts.join("\n"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    #[test]
    fn card_timing_rounds_like_reference_and_retains_unknown_dates() {
        assert_eq!(duration(59_499, false), "59 s");
        assert_eq!(duration(59_500, false), "1 min");
        assert_eq!(duration(3_599_000, false), "1 h 00");
        assert_eq!(duration(25 * 3_600_000, true), "1 j 1 h");
        assert_eq!(
            labels(&Timeline::default(), 3, 1_800_000_000_000, true, "fr-CA"),
            Labels::default()
        );
        let t = Timeline {
            created_at_ms: Some(1_800_000_000_000),
            started_at_ms: Some(1_800_000_060_000),
            done_at_ms: Some(1_800_000_660_000),
            ..Default::default()
        };
        let label = labels(&t, 3, 1_800_000_660_000, true, "fr-CA");
        assert!(label.text.ends_with("10 min"));
        assert_eq!(labels(&t, 3, 1_800_000_660_000, true, "@@@"), label);
        assert!(label.tooltip.contains("Ajoutée"));
        assert_eq!(label.tooltip.lines().count(), 3);
        assert!(
            !labels(&t, 1, 1_800_000_660_000, false, "en-US")
                .text
                .contains("done")
        );
    }
    #[test]
    fn card_timing_uses_local_day_and_language() {
        let zone = chrono::FixedOffset::west_opt(5 * 3600).unwrap();
        let date = zone.with_ymd_and_hms(2026, 11, 12, 14, 5, 0).unwrap();
        assert_eq!(when(&date, &date, "en-US").unwrap(), "02:05 PM");
        assert!(full(&date, "en-US").unwrap().contains("2026"));
        assert!(full(&date, "en-US").unwrap().contains("05:00"));
        assert_eq!(when(&date, &date, "fr-CA").unwrap(), "14 h 05");
        let next = date + chrono::Duration::days(1);
        assert!(when(&date, &next, "en-US").unwrap().contains("Nov"));
        assert!(when(&date, &next, "fr-CA").unwrap().contains("nov."));
    }
}

#[derive(Hash, PartialEq, Eq)]
struct Key {
    column: usize,
    created: Option<u64>,
    since: Option<u64>,
    started: Option<u64>,
    doing: Option<u64>,
    done: Option<u64>,
}
#[derive(Default)]
struct Cache {
    day: Option<(chrono::NaiveDate, i32, bool, &'static str)>,
    values: std::collections::HashMap<Key, Labels>,
}
thread_local! { static CACHE: std::cell::RefCell<Cache> = std::cell::RefCell::new(Cache::default()); }
/// Bounded UI-thread cache: work-period changes do not change date labels.
pub fn cached(t: &Timeline, column: usize, french: bool) -> Labels {
    let now = chrono::Local::now();
    let locale = crate::regional_date::preferred_locale(french);
    let day = (
        now.date_naive(),
        now.offset().local_minus_utc(),
        french,
        locale,
    );
    let key = Key {
        column,
        created: t.created_at_ms,
        since: t.column_since_ms,
        started: t.started_at_ms,
        doing: t.doing_since_ms,
        done: t.done_at_ms,
    };
    CACHE.with_borrow_mut(|cache| {
        if cache.day.as_ref() != Some(&day) {
            cache.values.clear();
            cache.day = Some(day);
        }
        if let Some(value) = cache.values.get(&key) {
            return value.clone();
        }
        let value = labels(
            t,
            column,
            now.timestamp_millis().max(0) as u64,
            french,
            locale,
        );
        if cache.values.len() >= 10_000 {
            cache.values.clear();
        }
        cache.values.insert(key, value.clone());
        value
    })
}
