//! Tick labels, mostly compared with explicit precision, which the ICU4J fixture covers.

use avenger_format::{NumberFormatProvider, PreparedNumberFormatter, TickSpacing};
use avenger_format_number_icu::IcuNumberFormatProvider;
use std::sync::Arc;
use TickSpacing::{Uniform, Varying};

const MILLIONS: [f64; 7] = [0.0, 2e5, 4e5, 6e5, 8e5, 1e6, 1.2e6];
const TENTHS: [f64; 11] = [0.0, 0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8, 0.9, 1.0];

fn formatter(locale: &str, skeleton: &str) -> Arc<dyn PreparedNumberFormatter> {
    IcuNumberFormatProvider::new()
        .with_locale(locale)
        .prepare(skeleton)
        .unwrap_or_else(|error| panic!("{locale} {skeleton:?}: {error}"))
}

fn ticks(locale: &str, skeleton: &str, values: &[f64], spacing: TickSpacing) -> Vec<String> {
    formatter(locale, skeleton)
        .format_ticks(values, spacing)
        .into_iter()
        .map(|label| label.text)
        .collect()
}

fn each(locale: &str, skeleton: &str, values: &[f64]) -> Vec<String> {
    let formatter = formatter(locale, skeleton);
    values
        .iter()
        .map(|&value| formatter.format(value).text)
        .collect()
}

/// The skeleton with exactly `places` fraction digits.
fn with_places(skeleton: &str, places: usize) -> String {
    format!("{skeleton} .{}", "0".repeat(places))
}

#[test]
fn uniform_ticks_share_fraction_digits() {
    // Tick sets and the power of ten of the finest place they need.
    let tick_sets: [(&[f64], i32); 5] = [
        (&TENTHS, -1),
        (&MILLIONS, 5),
        (&[-0.5, -0.25, 0.0, 0.25, 0.5], -2),
        (&[1000.0, 1000.1, 1000.2, 1000.3], -1),
        (&[0.05, 1.05, 2.05], -2),
    ];
    // Skeletons and the power of ten their scale shifts that place by.
    let skeletons = [
        ("", 0),
        ("%x100", 2),
        ("permille scale/1000", 3),
        ("unit/meter unit-width-full-name", 0),
        ("scale/0.5", -1),
        (",_ +!", 0),
    ];
    for locale in ["en-US", "de-DE", "fr-FR", "ja-JP", "hi-IN"] {
        for (skeleton, shift) in skeletons {
            for (values, resolution) in tick_sets {
                let places = (-(resolution + shift)).max(0) as usize;
                assert_eq!(
                    ticks(locale, skeleton, values, Uniform),
                    each(locale, &with_places(skeleton, places), values),
                    "{locale} {skeleton:?} {values:?}"
                );
            }
        }
    }
}

#[test]
fn scientific_ticks_share_significant_digits() {
    for (skeleton, values, digits) in [
        ("E0", &MILLIONS[..], 2),
        ("EE0", &MILLIONS[..], 2),
        ("E0", &TENTHS[..], 2),
        ("E0", &[1e-7, 2e-7, 3e-7][..], 1),
        // Significant digits keep zero's integer digit.
        ("E0 integer-width/*", &MILLIONS[..], 2),
    ] {
        let explicit = format!("{skeleton} {}", "@".repeat(digits));
        assert_eq!(
            ticks("en-US", skeleton, values, Uniform),
            each("en-US", &explicit, values),
            "{skeleton} {values:?}"
        );
    }
}

#[test]
fn compact_ticks_share_one_unit() {
    for (locale, skeleton, values, places, expected) in [
        (
            "en-US",
            "K",
            &MILLIONS[..],
            1,
            &["0.0M", "0.2M", "0.4M", "0.6M", "0.8M", "1.0M", "1.2M"][..],
        ),
        // French plural forms still follow each label.
        (
            "fr-FR",
            "KK",
            &[0.0, 1e6, 2e6, 3e6][..],
            0,
            &["0 million", "1 million", "2 millions", "3 millions"][..],
        ),
        (
            "ja-JP",
            "K",
            &[0.0, 5e6, 1e7, 1.5e7, 2e7][..],
            0,
            &["0万", "500万", "1000万", "1500万", "2000万"][..],
        ),
        (
            "hi-IN",
            "K",
            &[0.0, 5e5, 1e6, 1.5e6, 2e6][..],
            0,
            &[
                "0\u{a0}लाख",
                "5\u{a0}लाख",
                "10\u{a0}लाख",
                "15\u{a0}लाख",
                "20\u{a0}लाख",
            ][..],
        ),
        // German has no short compact pattern for thousands.
        (
            "de-DE",
            "K",
            &MILLIONS[..5],
            0,
            &["0", "200.000", "400.000", "600.000", "800.000"][..],
        ),
    ] {
        let labels = ticks(locale, skeleton, values, Uniform);
        assert_eq!(labels, expected, "{locale} {skeleton}");
        // Per-value compact formatting chooses the same unit for the largest tick.
        let largest = &values[values.len() - 1..];
        assert_eq!(
            labels.last(),
            each(locale, &with_places(skeleton, places), largest).last(),
            "{locale} {skeleton}"
        );
    }
}

#[test]
fn currency_precision_follows_the_spacing_unless_explicit() {
    assert_eq!(
        ticks("en-US", "currency/USD", &MILLIONS[..3], Uniform),
        ["$0", "$200,000", "$400,000"]
    );
    for (locale, skeleton, values, places) in [
        ("en-US", "currency/USD", &MILLIONS[..], 0),
        ("en-GB", "currency/EUR", &[0.0, 0.25, 0.5][..], 2),
        ("ja-JP", "currency/JPY", &[0.0, 500.0, 1000.0][..], 0),
    ] {
        assert_eq!(
            ticks(locale, skeleton, values, Uniform),
            each(locale, &with_places(skeleton, places), values),
            "{locale} {skeleton}"
        );
    }
    let values = [0.0, 0.5, 1.0];
    assert_eq!(
        ticks("en-US", "currency/USD .00", &values, Uniform),
        each("en-US", "currency/USD .00", &values)
    );
}

#[test]
fn varying_ticks_keep_their_own_digits() {
    assert_eq!(
        ticks("en-US", "", &[1e-7, 1e-4, 0.1, 100.0, 1e6], Varying),
        ["0.0000001", "0.0001", "0.1", "100", "1,000,000"]
    );
    assert_eq!(
        ticks("en-US", "%x100", &[0.001, 0.01, 0.1, 1.0], Varying),
        ["0.1%", "1%", "10%", "100%"]
    );
    let values = [1e3, 1e4, 1e5, 1e6];
    for skeleton in ["K", "E0"] {
        assert_eq!(
            ticks("en-US", skeleton, &values, Varying),
            each("en-US", skeleton, &values),
            "{skeleton}"
        );
    }
}

#[test]
fn explicit_precision_and_usage_format_each_value() {
    let values = [0.0, 0.5, 1.0];
    for skeleton in [
        ".00",
        "@@#",
        "precision-integer",
        "K .0",
        "unit/meter usage/person-height",
    ] {
        for spacing in [Uniform, Varying] {
            assert_eq!(
                ticks("en-US", skeleton, &values, spacing),
                each("en-US", skeleton, &values),
                "{skeleton} {spacing:?}"
            );
        }
    }
}

#[test]
fn non_finite_ticks_leave_precision_unchanged() {
    assert_eq!(
        ticks("en-US", "", &[0.0, f64::NAN, 0.5, f64::INFINITY], Uniform),
        ["0.0", "NaN", "0.5", "∞"]
    );
}
