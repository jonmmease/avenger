use avenger_format_number::{
    NumberFormatContext, NumberLocaleRegistry, NumberTypesetting, PreparedNumberFormat,
};

#[test]
fn exponent_parts_match_localized_text() {
    let locale = NumberLocaleRegistry::with_builtins()
        .resolve("de-DE")
        .unwrap();
    let formatter = PreparedNumberFormat::new(
        Some("+.3~e"),
        Default::default(),
        NumberFormatContext::new(&locale),
    )
    .unwrap();
    for (value, text, mantissa) in [(1200.0, "+1,2e+3", "+1,2"), (-1200.0, "−1,2e+3", "−1,2")] {
        let result = formatter.format(value);
        assert_eq!(result.text, text);
        assert!(
            matches!(result.typesetting, NumberTypesetting::Exponent { mantissa: actual, exponent: 3, .. } if actual == mantissa)
        );
    }
}
