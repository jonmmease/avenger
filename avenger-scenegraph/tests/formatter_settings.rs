use avenger_format_config::{
    ChronoDateTimeFormatProvider, D3DateTimeFormatProvider, D3NumberFormatProvider,
    DateTimeFormatConfig, NumberFormatConfig,
};
use avenger_scenegraph::marks::text::SceneTextMark;

#[test]
fn saved_formatter_settings_round_trip() {
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
    }
}
