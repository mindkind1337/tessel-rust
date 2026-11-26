#[path = "../src/regional_date.rs"]
mod regional_date;

#[test]
fn regional_medium_date_short_time_matches_intl_examples() {
    assert!(regional_date::local(f64::NAN, true).is_none());
    assert!(regional_date::local(f64::INFINITY, false).is_none());
    assert!(regional_date::local(8.64e15 + 1., false).is_none());
    assert!(regional_date::local(0., true).is_some());
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/regional_date.json")).unwrap();
    for case in fixture["selections"].as_array().unwrap() {
        let preferred: Vec<String> = serde_json::from_value(case["preferred"].clone()).unwrap();
        assert_eq!(
            regional_date::locale(case["language"] == "fr", &preferred),
            case["expected"].as_str().unwrap()
        );
    }
    for case in fixture["dates"].as_array().unwrap() {
        let date =
            chrono::DateTime::from_timestamp_millis(case["timestamp"].as_i64().unwrap()).unwrap();
        let locale = case["locale"].as_str().unwrap();
        assert_eq!(
            regional_date::format(&date, locale).as_deref(),
            case["expected"].as_str(),
            "{locale}: {date}"
        );
    }
}

#[test]
fn local_history_dates_spell_out_the_month_in_the_selected_language() {
    let june = chrono::DateTime::parse_from_rfc3339("2026-06-15T12:00:00Z")
        .unwrap()
        .timestamp_millis();
    for (fr, month) in [(false, "june"), (true, "juin")] {
        let label = regional_date::long_local(june, fr).unwrap().to_lowercase();
        assert!(label.contains(month) && label.contains("2026"), "{label}");
    }
    assert!(regional_date::long_local(i64::MAX, false).is_none());
}
