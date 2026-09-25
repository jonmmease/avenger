use avenger_format_number::*;

#[test]
fn currency_defaults_and_explicit_precision() {
    let locale = ResolvedNumberLocale::en_us();
    for (spec, value, expected) in [
        ("C[JPY]", 1234.5, "¥1235"),
        ("C[USD]", 12.3456, "$12.35"),
        ("C[BHD]", 12.3456, "BHD\u{a0}12.346"),
        ("C[CLF]", 1.23456, "CLF\u{a0}1.2346"),
        ("C[RSD]", 1.6, "RSD\u{a0}2"),
        (".2C[JPY]", 1.5, "¥1.50"),
        ("010.2C[USD]", -12.0, "−$00012.00"),
        ("(C[USD]", -12.0, "($12.00)"),
        ("C[USD]", -0.0, "$0.00"),
        ("+C[USD]", -0.0, "−$0.00"),
        ("C[USD]", f64::INFINITY, "$Infinity"),
        ("C[USD]", f64::NAN, "$NaN"),
    ] {
        let result = format_number(value, Some(spec), Default::default(), &locale).unwrap();
        assert_eq!(result.text, expected, "{spec} {value}");
        assert_eq!(result.typesetting, NumberTypesetting::Plain);
    }
    let result = format_number(
        12.3456,
        Some("C"),
        NumberFormatOverrides::default()
            .with_currency("USD")
            .with_precision(1),
        &locale,
    )
    .unwrap();
    assert_eq!(result.text, "$12.3");
}

#[test]
fn currency_symbols_spacing_and_accounting_are_localized() {
    let mut registry = NumberLocaleRegistry::with_builtins();
    let metadata: std::collections::BTreeMap<String, NumberLocaleExtensions> =
        serde_json::from_str(include_str!("fixtures/currency-locales.json")).unwrap();
    for (id, json) in [
        ("de-DE", include_str!("fixtures/locales/de-DE.json")),
        ("fr-FR", include_str!("fixtures/locales/fr-FR.json")),
        ("ja-JP", include_str!("fixtures/locales/ja-JP.json")),
    ] {
        registry.register_custom_locale_json(id, json).unwrap();
        registry
            .register_extensions(id, metadata[id].clone())
            .unwrap();
    }
    for (id, display, spec, expected) in [
        ("de-DE", CurrencyDisplay::Symbol, "(C[EUR]", "−12,00\u{a0}€"),
        ("ja-JP", CurrencyDisplay::Symbol, "C[JPY]", "−￥12"),
        ("en-US", CurrencyDisplay::Symbol, "C[USD]", "−$12.00"),
        ("en-US", CurrencyDisplay::Code, "C[USD]", "−USD\u{a0}12.00"),
        ("en-US", CurrencyDisplay::NarrowSymbol, "C[USD]", "−$12.00"),
        (
            "fr-FR",
            CurrencyDisplay::Symbol,
            "(C[USD]",
            "(12,00\u{a0}$US)",
        ),
        (
            "fr-FR",
            CurrencyDisplay::NarrowSymbol,
            "(C[USD]",
            "(12,00\u{a0}$)",
        ),
        ("fr-FR", CurrencyDisplay::Code, "C[USD]", "−12,00\u{a0}USD"),
    ] {
        let locale = registry.resolve(id).unwrap();
        let overrides = NumberFormatOverrides {
            currency_display: Some(display),
            ..Default::default()
        };
        assert_eq!(
            format_number(-12.0, Some(spec), overrides, &locale)
                .unwrap()
                .text,
            expected
        );
    }
    let mut metadata = NumberLocaleExtensions::en_us();
    metadata.currency_symbols.get_mut("USD").unwrap().symbol = Some("US-$".into());
    metadata.currency.standard.negative_prefix.clear();
    metadata.currency.standard.negative_suffix = "¤-".into();
    registry.register_extensions("en-US", metadata).unwrap();
    let locale = registry.resolve("en-US").unwrap();
    assert_eq!(
        format_number(1.0, Some("C[USD]"), Default::default(), &locale)
            .unwrap()
            .text,
        "US-$1.00"
    );
    assert_eq!(
        format_number(-1.0, Some("C[USD]"), Default::default(), &locale)
            .unwrap()
            .text,
        "1.00\u{a0}US-$−"
    );
    assert_eq!(
        format_number(1.0, Some("$,.2f"), Default::default(), &locale)
            .unwrap()
            .text,
        "$1.00"
    );
}

#[test]
fn currency_options_and_metadata_fail_before_formatting() {
    let locale = ResolvedNumberLocale::en_us();
    for spec in ["C[]", "C[usd]", "C[US]", "C[USDD]", "C[USD", "S[USD]"] {
        assert!(
            matches!(
                PreparedNumberFormat::new(Some(spec), Default::default(), &locale),
                Err(FormatError::Parse(_))
            ),
            "{spec}"
        );
    }
    assert!(matches!(
        PreparedNumberFormat::new(Some("C[ZZZ]"), Default::default(), &locale),
        Err(FormatError::InvalidCurrencyCode(_))
    ));
    assert!(matches!(
        PreparedNumberFormat::new(Some("C"), Default::default(), &locale),
        Err(FormatError::MissingCurrencyCode)
    ));
    for (spec, overrides) in [
        ("$C[USD]", Default::default()),
        ("f", NumberFormatOverrides::default().with_currency("USD")),
    ] {
        assert!(matches!(
            PreparedNumberFormat::new(Some(spec), overrides, &locale),
            Err(FormatError::InvalidFormat(_))
        ));
    }
    let mut registry = NumberLocaleRegistry::with_builtins();
    let original = registry.resolve("en-US").unwrap();
    for invalid in ["", "¤¤"] {
        let mut metadata = NumberLocaleExtensions::en_us();
        metadata.currency.standard.positive_prefix = invalid.into();
        assert!(matches!(
            registry.register_extensions("en-US", metadata),
            Err(FormatError::InvalidLocaleData(_))
        ));
        assert_eq!(registry.resolve("en-US").unwrap(), original);
    }
}
