//! Behavior outside the ICU4J label fixture: thread sharing and typesetting parts.

use avenger_format::{NumberFormatProvider, NumberTypesetting};
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

#[test]
fn scientific_typesetting_parts() {
    let provider = IcuNumberFormatProvider::new();
    assert_eq!(
        provider
            .prepare("E0 .0")
            .unwrap()
            .format(-1234.0)
            .typesetting,
        NumberTypesetting::Exponent {
            mantissa: "-1.2".into(),
            exponent: 3
        }
    );
    for skeleton in [
        "E00",
        "E0 percent",
        "E0 numbering-system/thai",
        "E0 000",
        "K",
    ] {
        assert_eq!(
            provider
                .prepare(skeleton)
                .unwrap()
                .format(1234.0)
                .typesetting,
            NumberTypesetting::Plain,
            "{skeleton}"
        );
    }
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

#[test]
fn rejects_unit_names_that_icu4x_lacks() {
    use icu_experimental::dimension::provider::units::categorized_display_names::*;
    use icu_experimental::dimension::provider::units::display_names::UnitsDisplayNames;
    use icu_experimental::provider::Baked;
    use icu_provider::prelude::*;

    // The locale's own names, from any of a category's data sets.
    fn localized<M>(locale: &DataLocale, unit: &str) -> bool
    where
        M: DataMarker<DataStruct = UnitsDisplayNames<'static>>,
        Baked: DataProvider<M>,
    {
        let attributes = DataMarkerAttributes::from_str_or_panic(unit);
        let request = DataRequest {
            id: DataIdentifierBorrowed::for_marker_attributes_and_locale(attributes, locale),
            ..Default::default()
        };
        DataProvider::<M>::load(&Baked, request)
            .is_ok_and(|r| !r.metadata.locale.is_some_and(|l| l.is_unknown()))
    }
    for name in [
        "bgn", "cad", "ccp", "ce", "cic", "dz", "en-Dsrt", "fur", "gsw", "haw", "jgo", "ksh",
        "lkt", "ms-Arab", "mus", "mzn", "os", "osa", "se", "trv", "wae",
    ] {
        let locale: DataLocale = name
            .parse::<icu_locale_core::LanguageIdentifier>()
            .unwrap()
            .into();
        let names = [
            localized::<UnitsNamesLengthCoreV1>(&locale, "long-meter"),
            localized::<UnitsNamesLengthExtendedV1>(&locale, "long-meter"),
            localized::<UnitsNamesDurationCoreV1>(&locale, "long-hour"),
            localized::<UnitsNamesDurationExtendedV1>(&locale, "long-hour"),
            localized::<UnitsNamesMassCoreV1>(&locale, "long-kilogram"),
            localized::<UnitsNamesMassExtendedV1>(&locale, "long-kilogram"),
        ];
        assert!(!names.contains(&true), "{name} has ICU4X unit names");
        assert!(
            matches!(
                IcuNumberFormatProvider::new()
                    .with_locale(name)
                    .prepare("unit/meter"),
                Err(avenger_format::NumberFormatError::LocaleUnavailable { .. })
            ),
            "{name}"
        );
    }
}
