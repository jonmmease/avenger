use avenger_format_config::{
    ChronoDateTimeFormatProvider, D3DateTimeFormatProvider, D3NumberFormatProvider,
    DateTimeFormatConfig, NumberFormatConfig,
};
use avenger_scenegraph::marks::text::SceneTextMark;
use std::sync::Arc;

#[test]
fn saved_settings_round_trip_and_reuse_providers() {
    let number = NumberFormatConfig::D3(
        D3NumberFormatProvider::new()
            .with_locale("custom")
            .with_custom_locale(
                "custom",
                serde_json::from_str(r#"{"decimal":"~","thousands":"_","grouping":[3]}"#).unwrap(),
            ),
    );
    for datetime in [
        DateTimeFormatConfig::D3(
            D3DateTimeFormatProvider::new().with_timezone(chrono_tz::Asia::Tokyo),
        ),
        DateTimeFormatConfig::Chrono(ChronoDateTimeFormatProvider::new().with_locale("fr-FR")),
    ] {
        let mark = SceneTextMark {
            number_format: Some(number.clone()),
            datetime_format: Some(datetime),
            ..Default::default()
        };
        let serialized = serde_json::to_string(&mark).unwrap();
        let restored: SceneTextMark = serde_json::from_str(&serialized).unwrap();
        assert_eq!(mark, restored);
        let first = mark.number_format.as_ref().unwrap().provider();
        let second = restored.number_format.as_ref().unwrap().provider();
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(
            second.prepare(",.1f").unwrap().format(1234.5).text,
            "1_234~5"
        );
        assert!(Arc::ptr_eq(
            &mark.datetime_format.as_ref().unwrap().provider(),
            &restored.datetime_format.as_ref().unwrap().provider(),
        ));
    }
    assert!(serde_json::from_str::<NumberFormatConfig>(r#"{"provider":"unknown"}"#).is_err());
    assert!(serde_json::from_str::<DateTimeFormatConfig>(r#"{"provider":"unknown"}"#).is_err());
}
