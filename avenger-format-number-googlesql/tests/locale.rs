//! The symbols of d3 number locales in GoogleSQL's patterns.
//!
//! The default build bundles only U.S. English, so the tests read the other locales' files from
//! avenger-format-locales-d3 as custom definitions, and check the bundled ones with
//! `all-locales`.

use avenger_format::{
    NumberFormatError, NumberFormatProvider, NumberTypesetting, PrecisionSharing,
};
use avenger_format_locales_d3::NumberLocaleSpec;
use avenger_format_number_googlesql::{Format, GoogleSqlNumberFormatProvider, Locale};

/// The d3 file of the number locale `name`.
fn spec(name: &str) -> NumberLocaleSpec {
    let json = match name {
        "de-DE" => include_str!("../../avenger-format-locales-d3/locales/number/de-DE.json"),
        "en-IN" => include_str!("../../avenger-format-locales-d3/locales/number/en-IN.json"),
        "fr-FR" => include_str!("../../avenger-format-locales-d3/locales/number/fr-FR.json"),
        "ar-EG" => include_str!("../../avenger-format-locales-d3/locales/number/ar-EG.json"),
        _ => panic!("a locale file: {name}"),
    };
    NumberLocaleSpec::from_json(name, json).unwrap()
}

/// The provider of the locale `name`, as a custom definition.
fn provider(name: &str) -> GoogleSqlNumberFormatProvider {
    GoogleSqlNumberFormatProvider::new()
        .with_custom_locale(name, spec(name))
        .with_locale(name)
}

fn format(name: &str, pattern: &str, value: f64) -> String {
    provider(name).prepare(pattern).unwrap().format(value).text
}

#[test]
fn de_de_writes_its_point_and_groups() {
    assert_eq!(format("de-DE", "%'.2f", 1234567.891), "1.234.567,89");
    assert_eq!(format("de-DE", "%.1e", 1234.5), "1,2e+03");
    assert_eq!(format("de-DE", "%#.0E", 1234.5), "1,E+03");
    assert_eq!(format("de-DE", "%'g", 1234.5), "1.234,5");
    assert_eq!(format("de-DE", "%'g", 1234567.891), "1,23457e+06");
    assert_eq!(format("de-DE", "%'d", 1234567.0), "1.234.567");
    assert_eq!(format("de-DE", "%'.8d", 42.0), "00.000.042");
    assert_eq!(format("de-DE", "%.2f", 1234.5), "1234,50");
    assert_eq!(format("de-DE", "$%'.2f", 1234.5), "$1.234,50");
    assert_eq!(format("de-DE", "%.1f%%", 12.25), "12,2%");
}

#[test]
fn en_in_cycles_its_group_sizes() {
    assert_eq!(format("en-IN", "%'d", 1234567.0), "12,34,567");
    assert_eq!(format("en-IN", "%'.2f", 1234567.891), "12,34,567.89");
    assert_eq!(format("en-IN", "%'d", 123.0), "123");
    assert_eq!(format("en-IN", "%'d", 1234567890.0), "1,23,45,67,890");
}

#[test]
fn fr_fr_pads_by_characters() {
    let space = '\u{a0}';
    assert_eq!(spec("fr-FR").thousands, space.to_string());
    assert_eq!(format("fr-FR", "%'.1f", 1234.5), format!("1{space}234,5"));
    assert_eq!(
        format("fr-FR", "%'12.1f", 1234.5),
        format!("     1{space}234,5")
    );
    assert_eq!(
        format("fr-FR", "%'12.1f", -1234.5),
        format!("    \u{2212}1{space}234,5")
    );
    assert_eq!(
        format("fr-FR", "%'-12.1f|", -1234.5),
        format!("\u{2212}1{space}234,5    |")
    );
    assert_eq!(
        format("fr-FR", "%'012.1f", -1234.5),
        format!("\u{2212}00001{space}234,5")
    );
}

#[test]
fn numerals_replace_decimal_digits_and_padding_zeros() {
    let ar = |pattern: &str, value: f64| format("ar-EG", pattern, value);
    assert_eq!(ar("%'.2f", 1234.5), "١٬٢٣٤٫٥٠");
    assert_eq!(ar("%05d", 42.0), "٠٠٠٤٢");
    assert_eq!(ar("%.1e", 1234.5), "١٫٢e+٠٣");
    // Hexadecimal and octal digits stay ASCII, with GoogleSQL's groups.
    assert_eq!(ar("%'x", 12345678.0), "bc:614e");
    assert_eq!(ar("%08o", 8.0), "00000010");
    let circled = NumberLocaleSpec {
        numerals: Some(["⓪", "①", "②", "③", "④", "⑤", "⑥", "⑦", "⑧", "⑨"].map(String::from)),
        ..NumberLocaleSpec::en_us()
    };
    let locale = Locale::new(&circled).unwrap();
    let format = |pattern: &str, value: f64| Format::new(pattern, &locale).unwrap().format(value);
    assert_eq!(format("%'08.1f", 1234.5), "⓪①,②③④.⑤");
    assert_eq!(format("%d", 1907.0), "①⑨⓪⑦");
}

#[test]
fn minus_and_nan_replace_googlesqls() {
    assert_eq!(format("de-DE", "%.1f", -1.5), "\u{2212}1,5");
    assert_eq!(format("de-DE", "%x", -255.0), "\u{2212}ff");
    assert_eq!(format("de-DE", "%.1f", -0.04), "\u{2212}0,0");
    assert_eq!(format("de-DE", "%.1f", -0.0), "0,0");
    // A locale's NaN prints as written in every case; `inf` keeps GoogleSQL's text.
    for pattern in ["%f", "%F", "%G", "%d"] {
        assert_eq!(format("de-DE", pattern, f64::NAN), "NaN", "{pattern}");
    }
    assert_eq!(
        format("de-DE", "%08F", f64::NEG_INFINITY),
        "    \u{2212}INF"
    );
    assert_eq!(format("de-DE", "%e", f64::INFINITY), "inf");
    assert_eq!(format("de-DE", "%+f", f64::NAN), "+NaN");
}

#[test]
fn a_custom_locale_sets_every_symbol() {
    let custom = NumberLocaleSpec {
        decimal: "·".into(),
        thousands: "_".into(),
        grouping: vec![3, 2],
        minus: "MINUS".into(),
        nan: "missing".into(),
        ..Default::default()
    };
    let provider = GoogleSqlNumberFormatProvider::new()
        .with_custom_locale("x_custom", custom)
        .with_locale("x-custom");
    let provider: GoogleSqlNumberFormatProvider =
        serde_json::from_str(&serde_json::to_string(&provider).unwrap()).unwrap();
    let formatter = provider.prepare("%'.1f").unwrap();
    assert_eq!(formatter.format(-1234567.25).text, "MINUS12_34_567·2");
    assert_eq!(formatter.format(f64::NAN).text, "missing");
    // An empty group separator, or no group sizes, inserts nothing.
    for spec in [
        NumberLocaleSpec {
            thousands: String::new(),
            ..NumberLocaleSpec::en_us()
        },
        NumberLocaleSpec {
            grouping: Vec::new(),
            ..NumberLocaleSpec::en_us()
        },
    ] {
        let format = Format::new("%'d", &Locale::new(&spec).unwrap()).unwrap();
        assert_eq!(format.format(1234567.0), "1234567");
    }
}

#[test]
fn an_unknown_locale_is_unavailable() {
    let result = GoogleSqlNumberFormatProvider::new()
        .with_locale("xx-XX")
        .prepare("%d");
    assert!(matches!(
        result,
        Err(NumberFormatError::LocaleUnavailable { locale, .. }) if locale == "xx-XX"
    ));
}

#[test]
fn a_group_size_of_zero_is_invalid() {
    let result = GoogleSqlNumberFormatProvider::new()
        .with_custom_locale(
            "zero",
            NumberLocaleSpec {
                grouping: vec![3, 0],
                ..NumberLocaleSpec::en_us()
            },
        )
        .with_locale("zero")
        .prepare("%d");
    assert!(matches!(
        result,
        Err(NumberFormatError::InvalidLocaleData { message })
            if message.contains("`zero`") && message.contains("grouping")
    ));
}

#[test]
fn the_empty_pattern_follows_the_locale() {
    let formatter = provider("de-DE").prepare("").unwrap();
    let texts = |values: &[f64], sharing| -> Vec<String> {
        (formatter.format_all(values, sharing).into_iter())
            .map(|label| label.text)
            .collect()
    };
    use PrecisionSharing::{PerValue, Shared};
    assert_eq!(
        texts(&[0.0, 0.25, 0.5, 0.75, 1.0], Shared),
        ["0,00", "0,25", "0,50", "0,75", "1,00"]
    );
    assert_eq!(
        texts(&[-20000.0, 0.0, 20000.0, 40000.0], Shared),
        ["\u{2212}20.000", "0", "20.000", "40.000"]
    );
    assert_eq!(
        texts(&[1234.5, -0.125, f64::NAN], PerValue),
        ["1.234,5", "\u{2212}0,125", "NaN"]
    );
    assert_eq!(
        texts(&[-1e-7, 0.0, 1.5e-7], Shared),
        ["\u{2212}1,0e-07", "0", "1,5e-07"]
    );
    assert_eq!(formatter.format(-1.5e-9).text, "\u{2212}1,5e-09");
    assert_eq!(
        formatter.format(-1.5e25).typesetting,
        NumberTypesetting::Exponent {
            mantissa: "\u{2212}1,5".into(),
            exponent: 25
        }
    );
    assert_eq!(formatter.format(f64::NEG_INFINITY).text, "\u{2212}inf");
}

#[test]
fn the_bundled_en_us_differs_from_googlesql_in_minus_and_nan() {
    let en_us = GoogleSqlNumberFormatProvider::new().with_locale("en_US");
    let formatter = en_us.prepare("%'.1f").unwrap();
    assert_eq!(formatter.format(-1234.5).text, "\u{2212}1,234.5");
    assert_eq!(formatter.format(f64::NAN).text, "NaN");
}

#[cfg(feature = "all-locales")]
#[test]
fn bundled_locales_match_their_files() {
    for name in ["de-DE", "en-IN", "fr-FR", "ar-EG"] {
        let bundled = GoogleSqlNumberFormatProvider::new().with_locale(name.replace('-', "_"));
        for (pattern, value) in [("%'.2f", -1234567.891), ("%'d", 1234567.0), ("", 0.5)] {
            assert_eq!(
                bundled.prepare(pattern).unwrap().format(value),
                provider(name).prepare(pattern).unwrap().format(value),
                "{name} {pattern}"
            );
        }
    }
}
