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
