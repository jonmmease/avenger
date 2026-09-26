use avenger_format_config::{
    ChronoDateTimeFormatConfig, D3DateTimeFormatConfig, D3NumberFormatConfig, DateTimeFormatConfig,
    NumberFormatConfig,
};
use avenger_scenegraph::marks::text::SceneTextMark;

#[test]
fn saved_settings_round_trip_and_reuse_bindings() {
    let number = NumberFormatConfig::D3(
        D3NumberFormatConfig::new()
            .with_locale("custom")
            .with_custom_locale(
                "custom",
                serde_json::from_str(r#"{"decimal":"~","thousands":"_","grouping":[3]}"#).unwrap(),
            ),
    );
    for datetime in [
        DateTimeFormatConfig::D3(D3DateTimeFormatConfig::new().with_timezone("Asia/Tokyo")),
        DateTimeFormatConfig::Chrono(ChronoDateTimeFormatConfig::new().with_locale("fr-FR")),
    ] {
        let mark = SceneTextMark {
            number_format: Some(number.clone()),
            datetime_format: Some(datetime),
            ..Default::default()
        };
        let serialized = serde_json::to_string(&mark).unwrap();
        let restored: SceneTextMark = serde_json::from_str(&serialized).unwrap();
        assert_eq!(mark, restored);
        let first = mark.number_format.as_ref().unwrap().binding();
        let second = restored.number_format.as_ref().unwrap().binding();
        assert_eq!(first.cache_id(), second.cache_id());
        assert_eq!(
            second.prepare(",.1f").unwrap().format(1234.5).text,
            "1_234~5"
        );
        assert_eq!(
            mark.datetime_format.as_ref().unwrap().binding().cache_id(),
            restored
                .datetime_format
                .as_ref()
                .unwrap()
                .binding()
                .cache_id()
        );
        assert!(!serialized.contains("cache_id"));
    }
    assert!(serde_json::from_str::<NumberFormatConfig>(r#"{"provider":"unknown"}"#).is_err());
    assert!(serde_json::from_str::<DateTimeFormatConfig>(r#"{"provider":"unknown"}"#).is_err());
}

#[test]
fn equal_precision_settings_have_equal_scene_hashes() {
    use std::hash::{DefaultHasher, Hash, Hasher};
    let mark = |zero| SceneTextMark {
        number_format: Some(
            D3NumberFormatConfig::new()
                .with_precision(avenger_format_config::D3NumberPrecision::Step {
                    step: zero,
                    reference_value: zero,
                })
                .into(),
        ),
        ..Default::default()
    };
    let hash = |mark: &SceneTextMark| {
        let mut hasher = DefaultHasher::new();
        mark.hash(&mut hasher);
        hasher.finish()
    };
    let positive = mark(0.0);
    let negative = mark(-0.0);
    assert_eq!(positive, negative);
    assert_eq!(hash(&positive), hash(&negative));
}
