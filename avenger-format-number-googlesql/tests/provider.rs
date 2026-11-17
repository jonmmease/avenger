//! The provider's chart-specific rules: the errors of `prepare`, the integer conversions of
//! floats, and the empty pattern.

use avenger_format::{
    NumberFormatError, NumberFormatProvider, NumberTypesetting, PrecisionSharing,
};
use avenger_format_number_googlesql::GoogleSqlNumberFormatProvider;

fn format(pattern: &str, value: f64) -> String {
    let formatter = GoogleSqlNumberFormatProvider::new()
        .prepare(pattern)
        .unwrap();
    formatter.format(value).text
}

fn format_all(pattern: &str, values: &[f64], sharing: PrecisionSharing) -> Vec<String> {
    let formatter = GoogleSqlNumberFormatProvider::new()
        .prepare(pattern)
        .unwrap();
    (formatter.format_all(values, sharing).into_iter())
        .map(|label| label.text)
        .collect()
}

/// The message and position of the error that `prepare` returns for `pattern`.
fn error(pattern: &str) -> (String, Option<usize>) {
    match GoogleSqlNumberFormatProvider::new().prepare(pattern) {
        Err(NumberFormatError::InvalidPattern { message, position }) => (message, position),
        Err(other) => panic!("`{pattern}` gives an invalid pattern, not {other:?}"),
        Ok(_) => panic!("`{pattern}` is an error"),
    }
}

/// Every message says which conversions a chart's pattern takes.
fn assert_lists_conversions(message: &str) {
    assert!(
        message.contains("`%d`") && message.contains("`%G`"),
        "the message lists the conversions: {message}"
    );
}

#[test]
fn syntax_is_googlesql() {
    assert_eq!(GoogleSqlNumberFormatProvider::new().syntax(), "googlesql");
}

#[test]
fn provider_round_trips_through_serde() {
    let provider = GoogleSqlNumberFormatProvider::new();
    let json = serde_json::to_string(&provider).unwrap();
    assert_eq!(json, r#"{"locale":null,"locales":{}}"#);
    assert_eq!(
        serde_json::from_str::<GoogleSqlNumberFormatProvider>("{}").unwrap(),
        provider
    );
}

#[test]
fn literal_text_and_percent_signs_surround_the_conversion() {
    assert_eq!(format("$%.2f", 4.567), "$4.57");
    assert_eq!(format("%.1f%%", 12.25), "12.2%");
    assert_eq!(format("x = %.1f mm", 2.0), "x = 2.0 mm");
    assert_eq!(format("%%%d%%", 5.0), "%5%");
    assert_eq!(format("é %d ü", 5.0), "é 5 ü");
}

#[test]
fn a_pattern_without_a_conversion_is_an_error() {
    let (message, position) = error("100%%");
    assert!(message.contains("no conversion"), "{message}");
    assert_lists_conversions(&message);
    assert_eq!(position, None);
}

#[test]
fn a_second_conversion_is_an_error() {
    let (message, position) = error("%d to %.1f");
    assert!(
        message.contains("`%.1f`") && message.contains("second"),
        "{message}"
    );
    assert_lists_conversions(&message);
    assert_eq!(position, Some(6));
}

#[test]
fn a_star_width_is_an_error() {
    let (message, position) = error("a %*d");
    assert!(message.contains("`*` width"), "{message}");
    assert_lists_conversions(&message);
    assert_eq!(position, Some(3));
}

#[test]
fn a_star_precision_is_an_error() {
    let (message, position) = error("%.*f");
    assert!(message.contains("`*` precision"), "{message}");
    assert_lists_conversions(&message);
    assert_eq!(position, Some(2));
}

#[test]
fn a_specifier_other_than_a_number_conversion_is_an_error() {
    for (pattern, element) in [
        ("%s", "`%s`"),
        ("%t", "`%t`"),
        ("%T", "`%T`"),
        ("%p", "`%p`"),
        ("%P", "`%P`"),
        ("%u", "`%u`"),
        ("%a", "`%a`"),
        ("n = %-5ld", "`%-5l`"),
        ("%é", "`%é`"),
    ] {
        let (message, position) = error(pattern);
        assert!(message.contains(element), "{pattern}: {message}");
        assert!(
            message.contains("not a conversion of a number"),
            "{message}"
        );
        assert_lists_conversions(&message);
        assert_eq!(position, Some(pattern.find('%').unwrap()), "{pattern}");
    }
}

#[test]
fn a_trailing_lone_percent_is_an_error() {
    for (pattern, element, at) in [("%d %", "`%`", 3), ("%d %'5", "`%'5`", 3)] {
        let (message, position) = error(pattern);
        assert!(message.contains(element), "{message}");
        assert!(
            message.contains("lone") && message.contains("`%%`"),
            "{message}"
        );
        assert_eq!(position, Some(at));
    }
}

#[test]
fn a_huge_width_is_an_error() {
    let (message, position) = error("%99999d");
    assert!(message.contains("width"), "{message}");
    assert_eq!(position, Some(0));
}

#[test]
fn integer_conversions_round_floats_half_to_even() {
    assert_eq!(format("%d", 2.5), "2");
    assert_eq!(format("%d", 3.5), "4");
    assert_eq!(format("%d", -2.5), "-2");
    assert_eq!(format("%d", -0.4), "0");
    assert_eq!(format("%d", 1e20), "100000000000000000000");
    assert_eq!(format("%'d", 1234567.4), "1,234,567");
    assert_eq!(format("%i", 41.6), "42");
    assert_eq!(format("%x", 255.4), "ff");
    assert_eq!(format("%o", 7.5), "10");
}

#[test]
fn integer_conversions_print_any_magnitude_exactly() {
    assert_eq!(
        format("%d", 1e300),
        format!("{:.0}", 1e300),
        "every digit of 1e300"
    );
    assert_eq!(format("%x", 2f64.powi(64)), "10000000000000000");
    assert_eq!(
        format("%X", 2f64.powi(100) * 3.0),
        format!("3{}", "0".repeat(25))
    );
    assert_eq!(format("%o", 2f64.powi(66)), format!("1{}", "0".repeat(22)));
    assert_eq!(format("%'x", 2f64.powi(64)), "1:0000:0000:0000:0000");
}

#[test]
fn integer_conversions_print_non_finite_values_as_f() {
    assert_eq!(format("%d", f64::NAN), "nan");
    assert_eq!(format("%X", f64::INFINITY), "inf");
    assert_eq!(format("%05x", f64::NEG_INFINITY), " -inf");
}

#[test]
fn unrecorded_flag_combinations_follow_c() {
    assert_eq!(format("%08.3d", 42.0), "     042");
    assert_eq!(format("%#06x", 42.0), "0x002a");
}

#[test]
fn the_empty_pattern_prints_a_single_value_with_its_shortest_decimals() {
    for (value, expected) in [
        (0.0, "0"),
        (1.0, "1"),
        (0.1, "0.1"),
        (1234.5, "1,234.5"),
        (1234567.891, "1,234,567.891"),
        (-1234.5, "-1,234.5"),
        (-0.0, "0"),
        (1e25, "1e+25"),
        (-1e25, "-1e+25"),
        (1e-9, "1e-09"),
        (0.000001, "0.000001"),
        (999999999999999900000.0, "999,999,999,999,999,900,000"),
        (f64::NAN, "nan"),
        (f64::INFINITY, "inf"),
        (f64::NEG_INFINITY, "-inf"),
    ] {
        assert_eq!(format("", value), expected, "{value}");
    }
}

#[test]
fn the_empty_pattern_sets_scientific_notation_as_a_power_of_ten() {
    let formatter = GoogleSqlNumberFormatProvider::new().prepare("").unwrap();
    assert_eq!(
        formatter.format(-1.5e25).typesetting,
        NumberTypesetting::Exponent {
            mantissa: "-1.5".into(),
            exponent: 25
        }
    );
    assert_eq!(formatter.format(1.5).typesetting, NumberTypesetting::Plain);
    let written = GoogleSqlNumberFormatProvider::new().prepare("%e").unwrap();
    assert_eq!(written.format(1.5e25).typesetting, NumberTypesetting::Plain);
}

#[test]
fn the_empty_pattern_prints_shared_ticks_with_the_decimals_of_their_step() {
    use PrecisionSharing::Shared;
    assert_eq!(
        format_all("", &[0.0, 0.25, 0.5, 0.75, 1.0], Shared),
        ["0.00", "0.25", "0.50", "0.75", "1.00"]
    );
    assert_eq!(
        format_all("", &[0.0, 20000.0, 40000.0], Shared),
        ["0", "20,000", "40,000"]
    );
    assert_eq!(
        format_all("", &[0.0, 0.1, 0.2, 0.30000000000000004], Shared),
        ["0.0", "0.1", "0.2", "0.3"]
    );
    assert_eq!(
        format_all("", &[-1e-17, 0.5, 1.0, f64::NAN], Shared),
        ["0.0", "0.5", "1.0", "nan"]
    );
    assert_eq!(
        format_all("", &[0.0, 1e21, 2e21], Shared),
        ["0", "1e+21", "2e+21"]
    );
    assert_eq!(format_all("", &[f64::NAN], Shared), ["nan"]);
}

#[test]
fn the_empty_pattern_prints_values_per_value_as_single_values() {
    assert_eq!(
        format_all("", &[1.0, 10.5, 1234.25, 1e-9], PrecisionSharing::PerValue),
        ["1", "10.5", "1,234.25", "1e-09"]
    );
}

#[test]
fn a_written_pattern_formats_as_written_in_every_mode() {
    for sharing in [PrecisionSharing::Shared, PrecisionSharing::PerValue] {
        assert_eq!(
            format_all("%.1f", &[0.0, 0.25, 0.5], sharing),
            ["0.0", "0.2", "0.5"]
        );
        assert_eq!(format_all("%g", &[0.0, 20000.0], sharing), ["0", "20000"]);
    }
}
