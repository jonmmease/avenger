use avenger_format_number_d3::{
    format_number, prepare_number_float_format, prepare_number_prefix_format,
    prepare_number_step_format, DigitSpec, FormatError, NumberFormatOverrides,
    NumberLocaleRegistry, NumberLocaleSpec, NumberTypesetting, PreparedNumberFormat,
    ResolvedNumberLocale,
};

#[test]
fn exponent_parts_match_localized_text() {
    let mut registry = NumberLocaleRegistry::with_builtins();
    registry
        .register_custom_locale_json("de-DE", include_str!("fixtures/locales/de-DE.json"))
        .unwrap();
    let locale = registry.resolve("de-DE").unwrap();
    let formatter = PreparedNumberFormat::new(Some("+.3~e"), Default::default(), &locale).unwrap();
    for (value, text, mantissa) in [(1200.0, "+1,2e+3", "+1,2"), (-1200.0, "−1,2e+3", "−1,2")] {
        let result = formatter.format(value);
        assert_eq!(result.text, text);
        assert!(
            matches!(result.typesetting, NumberTypesetting::Exponent { mantissa: actual, exponent: 3, .. } if actual == mantissa)
        );
    }
}

#[test]
fn exponent_parts_require_unadorned_decimal_notation() {
    let locale = ResolvedNumberLocale::en_us();
    for (spec, value) in [
        ("x", 483.0),
        ("x", 2785.0),
        ("c", 1e30),
        ("$.2e", 1200.0),
        ("12.2e", 1200.0),
        ("(.2e", -1200.0),
        (".2e", f64::INFINITY),
    ] {
        let result = format_number(value, Some(spec), Default::default(), &locale).unwrap();
        assert_eq!(
            result.typesetting,
            NumberTypesetting::Plain,
            "{spec}: {value}"
        );
    }
    let result = format_number(1e21, Some("f"), Default::default(), &locale).unwrap();
    assert_eq!(result.text, "1e+21");
    assert_eq!(
        result.typesetting,
        NumberTypesetting::Exponent {
            mantissa: "1".into(),
            exponent: 21
        }
    );
}

#[test]
fn overrides_apply_before_defaults_and_zero_padding() {
    let locale = ResolvedNumberLocale::en_us();
    let without_zero = NumberFormatOverrides {
        zero: Some(false),
        ..Default::default()
    };
    for (spec, expected) in [("08.2f", "    1.20"), ("*<08.2f", "1.20****")] {
        let result = format_number(1.2, Some(spec), without_zero.clone(), &locale).unwrap();
        assert_eq!(result.text, expected);
    }
    let result = format_number(
        1234.5,
        Some("$,.2f"),
        NumberFormatOverrides {
            symbol: Some(None),
            group: Some(false),
            ..Default::default()
        },
        &locale,
    )
    .unwrap();
    assert_eq!(result.text, "1234.50");
    let result = format_number(
        1.2,
        Some(".^10.2f"),
        NumberFormatOverrides {
            width: Some(None),
            ..Default::default()
        },
        &locale,
    )
    .unwrap();
    assert_eq!(result.text, "1.20");
    for (spec, expected) in [(".5f", "1234.56"), (".5g", "1.2e+3")] {
        let result = format_number(
            1234.56,
            Some(spec),
            NumberFormatOverrides::default().with_precision(2),
            &locale,
        )
        .unwrap();
        assert_eq!(result.text, expected);
    }
    let format = prepare_number_step_format(
        0.1,
        1.0,
        Some(".3f"),
        NumberFormatOverrides {
            digit_spec: Some(DigitSpec::Auto),
            ..Default::default()
        },
        &locale,
    )
    .unwrap();
    assert_eq!(format.format(0.3).text, "0.3");
}

#[test]
fn step_format_selects_precision_and_shared_si_units() {
    let locale = ResolvedNumberLocale::en_us();
    for (step, reference, spec, value, expected) in [
        (0.01, 1.0, None, 0.3, "0.30"),
        (0.1, 1.0, None, 0.3, "0.3"),
        (-0.01, -1.0, Some("g"), 0.3, "0.30"),
        (0.1, 1.0, Some(".3f"), 0.3, "0.300"),
        (50_000.0, 1_100_000.0, Some("s"), 900_000.0, "0.90M"),
        (50_000.0, 1_100_000.0, Some("s"), 1_100_000.0, "1.10M"),
    ] {
        let format =
            prepare_number_step_format(step, reference, spec, Default::default(), &locale).unwrap();
        assert_eq!(
            format.format(value).text,
            expected,
            "step={step}, spec={spec:?}"
        );
    }
}

#[test]
fn undefined_step_preserves_default_precision() {
    let locale = ResolvedNumberLocale::en_us();
    for step in [0.0, f64::NAN, f64::INFINITY] {
        let format =
            prepare_number_step_format(step, 1.0, Some("f"), Default::default(), &locale).unwrap();
        assert_eq!(format.format(0.3).text, "0.300000");
    }
}

#[test]
fn float_trimming_preserves_numerals_affixes_and_padding() {
    let locale = ResolvedNumberLocale::new(
        "custom",
        NumberLocaleSpec {
            decimal: "💠".into(),
            currency: ["¤".into(), " euros0".into()],
            numerals: Some(["⓪", "①", "②", "③", "④", "⑤", "⑥", "⑦", "⑧", "⑨"].map(str::to_owned)),
            ..Default::default()
        },
    )
    .unwrap();
    for (spec, value, expected) in [
        ("f", 1.23, "①💠②③"),
        ("$f", 1.2, "¤①💠② euros⓪"),
        ("%", 0.12, "①②%"),
        ("e", 1200.0, "①💠②e+③"),
        ("6f", 1.2, "  ①💠②"),
        (".3f", 1.2, "①💠②⓪⓪"),
    ] {
        let result = prepare_number_float_format(Some(spec), &locale)
            .unwrap()
            .format(value);
        assert_eq!(result.text, expected, "{spec}");
        assert_eq!(result.typesetting, NumberTypesetting::Plain);
    }
    let locale = ResolvedNumberLocale::en_us();
    let result = prepare_number_float_format(Some("e"), &locale)
        .unwrap()
        .format(1200.0);
    assert_eq!(result.text, "1.2e+3");
    assert_eq!(
        result.typesetting,
        NumberTypesetting::Exponent {
            mantissa: "1.2".into(),
            exponent: 3
        }
    );
}

#[test]
fn locale_registration_validates_before_replacing_a_definition() {
    let mut registry = NumberLocaleRegistry::default();
    assert!(matches!(
        registry.resolve("custom"),
        Err(FormatError::LocaleNotFound(_))
    ));
    registry
        .register_custom_locale_json("custom", r#"{"decimal": ","}"#)
        .unwrap();
    let original = registry.resolve("custom").unwrap();
    let prepared = PreparedNumberFormat::new(Some(".1f"), Default::default(), &original).unwrap();
    for invalid in [
        r#"{"grouping": [3, 0]}"#,
        r#"{"currency": ["$"]}"#,
        "not json",
    ] {
        assert!(matches!(
            registry.register_custom_locale_json("custom", invalid),
            Err(FormatError::InvalidLocaleData(_))
        ));
        assert_eq!(registry.resolve("custom").unwrap(), original);
    }
    registry
        .register_custom_locale("custom", NumberLocaleSpec::en_us())
        .unwrap();
    assert_eq!(prepared.format(1.5).text, "1,5");
    let replacement = registry.resolve("custom").unwrap();
    assert_eq!(
        format_number(1.5, Some(".1f"), Default::default(), &replacement)
            .unwrap()
            .text,
        "1.5"
    );
}

#[test]
fn undefined_prefix_reference_uses_no_si_multiplier() {
    let locale = ResolvedNumberLocale::en_us();
    for value in [0.0, f64::NAN, f64::INFINITY] {
        let format = prepare_number_prefix_format(".1", value, &locale).unwrap();
        assert_eq!(format.format(1.2).text, "1.2");
    }
    let format =
        prepare_number_step_format(0.0, 0.0, Some("s"), Default::default(), &locale).unwrap();
    assert_eq!(format.format(0.0).text, "0.000000");
}
