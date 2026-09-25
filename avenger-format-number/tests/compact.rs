use avenger_format_number::*;
use std::collections::BTreeMap;

fn locales() -> NumberLocaleRegistry {
    let mut registry = NumberLocaleRegistry::with_builtins();
    let extensions: BTreeMap<String, NumberLocaleExtensions> =
        serde_json::from_str(include_str!("fixtures/compact-locales.json")).unwrap();
    for (id, json) in [
        ("de-DE", include_str!("fixtures/locales/de-DE.json")),
        ("fr-FR", include_str!("fixtures/locales/fr-FR.json")),
        ("ja-JP", include_str!("fixtures/locales/ja-JP.json")),
    ] {
        registry.register_custom_locale_json(id, json).unwrap();
        registry
            .register_extensions(id, extensions[id].clone())
            .unwrap();
    }
    registry
}

#[test]
fn compact_rounding_and_localized_patterns() {
    let registry = locales();
    for (id, spec, value, expected) in [
        ("en-US", ".3~S", 12.0, "12"),
        ("en-US", ".3~S", 999.4, "999"),
        ("en-US", ".3~S", 999.5, "1K"),
        ("en-US", ".3~S", -999.5, "−1K"),
        ("en-US", ".3~S", 999500.0, "1M"),
        ("en-US", ".3~S", 0.0, "0"),
        ("en-US", ".3~S", f64::INFINITY, "Infinity"),
        ("en-US", ".3~S", f64::NAN, "NaN"),
        ("de-DE", ".3~S", 999500.0, "1\u{a0}Mio."),
        ("de-DE", ".3~L", 1e6, "1 Million"),
        ("de-DE", ".3~L", 2e6, "2 Millionen"),
        ("fr-FR", ".3L", 1e6, "1,00 million"),
        ("fr-FR", ".3~L", 1.2e6, "1,2 million"),
        ("fr-FR", ".3~L", 2e6, "2 millions"),
        ("fr-FR", ".3~L", 1000.0, "mille"),
        ("ja-JP", ".3~S", 12000.0, "1.2万"),
    ] {
        let locale = registry.resolve(id).unwrap();
        let result = format_number(value, Some(spec), Default::default(), &locale).unwrap();
        assert_eq!(result.text, expected, "{id} {spec} {value}");
        assert_eq!(result.typesetting, NumberTypesetting::Plain);
    }
    assert!(matches!(
        PreparedNumberFormat::new(
            Some("#S"),
            Default::default(),
            &ResolvedNumberLocale::en_us()
        ),
        Err(FormatError::InvalidFormat(_))
    ));
}

#[test]
fn shared_tier_preserves_precision_and_selects_each_labels_plural() {
    let registry = locales();
    let boundary = prepare_number_step_format(
        0.1,
        999.5,
        Some(".3S"),
        Default::default(),
        &ResolvedNumberLocale::en_us(),
    )
    .unwrap();
    assert_eq!(boundary.format(999.5).text, "1.00K");

    for (id, spec, expected) in [
        ("en-US", "S", ["0.9M", "1.0M", "1.1M"]),
        ("en-US", ".3S", ["0.900M", "1.00M", "1.10M"]),
        (
            "de-DE",
            ".3~L",
            ["0,9 Millionen", "1 Million", "1,1 Millionen"],
        ),
    ] {
        let locale = registry.resolve(id).unwrap();
        let format = prepare_number_step_format(
            100000.0,
            1100000.0,
            Some(spec),
            Default::default(),
            &locale,
        )
        .unwrap();
        assert_eq!(
            [900000.0, 1000000.0, 1100000.0].map(|v| format.format(v).text),
            expected
        );
    }
}

#[test]
fn extension_registration_validates_before_replacement() {
    let mut registry = locales();
    let original = registry.resolve("en-US").unwrap();
    let prepared = PreparedNumberFormat::new(Some(".3~S"), Default::default(), &original).unwrap();
    for exponent in [i32::MIN, -400, 0, 309, i32::MAX] {
        let mut invalid = NumberLocaleExtensions::en_us();
        invalid.compact_short[0].exponent = exponent;
        assert!(matches!(
            registry.register_extensions("en-US", invalid),
            Err(FormatError::InvalidLocaleData(_))
        ));
        assert_eq!(registry.resolve("en-US").unwrap(), original);
    }
    for pattern in ["K", "{0}{0}K"] {
        let mut invalid = NumberLocaleExtensions::en_us();
        invalid.compact_short[0].other = pattern.into();
        assert!(registry.register_extensions("en-US", invalid).is_err());
    }
    let mut duplicate = NumberLocaleExtensions::en_us();
    duplicate.compact_short[1].exponent = 3;
    assert!(registry.register_extensions("en-US", duplicate).is_err());
    assert_eq!(registry.resolve("en-US").unwrap(), original);
    let mut replacement = NumberLocaleExtensions::en_us();
    replacement.compact_short[0].other = "{0} thousand".into();
    replacement.compact_short.reverse();
    registry.register_extensions("en-US", replacement).unwrap();
    let updated = registry.resolve("en-US").unwrap();
    assert_eq!(prepared.format(1200.0).text, "1.2K");
    assert_eq!(
        format_number(1200.0, Some(".3~S"), Default::default(), &updated)
            .unwrap()
            .text,
        "1.2 thousand"
    );
    for locale in [&original, &updated] {
        assert_eq!(
            format_number(1200.0, Some("$,.2f"), Default::default(), locale)
                .unwrap()
                .text,
            "$1,200.00"
        );
    }
}
