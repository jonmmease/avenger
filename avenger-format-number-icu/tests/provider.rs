//! Behavior outside the ICU4J label fixture: thread sharing.

use avenger_format::NumberFormatProvider;
use avenger_format_number_icu::IcuNumberFormatProvider;

#[test]
fn prepared_formatters_are_shareable() {
    let formatter = IcuNumberFormatProvider::new().prepare(".00").unwrap();
    let expected = formatter.format(1.8).text;
    let actual = std::thread::spawn(move || formatter.format(1.8).text)
        .join()
        .unwrap();
    assert_eq!(actual, expected);
}

/// ICU4X 2.3 has no number data for these CLDR locales, so preparation rejects them instead of
/// formatting with root data. This fails when ICU4X adds a locale; then accept it again.
#[test]
fn rejects_locales_that_icu4x_formats_with_root_data() {
    use icu_decimal::DecimalFormatter;
    use icu_locale_core::Locale;
    for name in ["az-Cyrl", "pa-Arab", "uz-Arab"] {
        let locale: Locale = name.parse().unwrap();
        let root = DecimalFormatter::try_new(locale.into(), Default::default()).unwrap();
        assert_eq!(
            root.format(&"-1234.5".parse().unwrap()).to_string(),
            "-1,234.5",
            "{name}"
        );
        assert!(
            matches!(
                IcuNumberFormatProvider::new().with_locale(name).prepare(""),
                Err(avenger_format::NumberFormatError::LocaleUnavailable { .. })
            ),
            "{name}"
        );
    }
}
