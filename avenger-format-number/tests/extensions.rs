use avenger_format_number::{
    prepare_number_tick_format, NumberFormatContext, NumberFormatOverrides, NumberLocaleRegistry,
    NumberTypesetting, PreparedNumberFormat,
};

#[test]
fn compact_and_currency_extensions_preserve_magnitude_and_precision() {
    let registry = NumberLocaleRegistry::with_builtins();
    for (locale, spec, value, expected) in [
        ("en-US", ".3~S", 12.0, "12"),
        ("en-US", ".3~S", 999500.0, "1M"),
        ("de-DE", ".4~S", 1200.0, "1200"),
        ("de-DE", ".3~S", 1200000.0, "1,2\u{a0}Mio."),
        ("en-US", ".3~L", 2000000.0, "2 million"),
        ("ja-JP", ".3~S", 12000.0, "1.2万"),
        ("en-US", "C[JPY]", 1234.5, "¥1235"),
        ("en-US", "C[BHD]", 1.2345, "BHD1.234"),
        ("en-US", "010.2C[USD]", -12.0, "−$00012.00"),
        ("fr-FR", "(.2C[EUR]", -12.0, "(12,00\u{a0}€)"),
    ] {
        let locale = registry.resolve(locale).unwrap();
        let format = PreparedNumberFormat::new(
            Some(spec),
            Default::default(),
            NumberFormatContext::new(&locale),
        )
        .unwrap();
        assert_eq!(format.format(value).text, expected, "{spec} {value}");
    }
    let locale = registry.resolve("en-US").unwrap();
    let ticks = prepare_number_tick_format(
        &[900000.0, 1000000.0, 1100000.0],
        Some("S"),
        NumberFormatOverrides::default(),
        NumberFormatContext::new(&locale),
    )
    .unwrap();
    assert_eq!(
        [900000.0, 1000000.0, 1100000.0].map(|value| ticks.format(value).text),
        ["0.9M", "1.0M", "1.1M"]
    );
}

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
