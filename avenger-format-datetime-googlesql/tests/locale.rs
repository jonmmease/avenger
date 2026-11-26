//! Locales: the formats agree with the d3 provider in every bundled locale, and their names and
//! composites parse back.

use avenger_format::DateTimeFormatProvider;
use avenger_format_datetime_d3::D3DateTimeFormatProvider;
use avenger_format_datetime_googlesql::{parse_datetime, Format, Kind, Locale};
use avenger_format_locales_d3::{DateTimeLocaleSpec, BUNDLED};
use chrono::{NaiveDate, NaiveDateTime};

/// A morning and an afternoon of each month, which cover each weekday.
fn datetimes() -> Vec<NaiveDateTime> {
    (1..=12)
        .flat_map(|month| {
            let date = NaiveDate::from_ymd_opt(2024, month, month + 3).unwrap();
            [date.and_hms_opt(2, 8, 9), date.and_hms_opt(14, 38, 59)]
        })
        .map(Option::unwrap)
        .collect()
}

#[test]
fn formats_agree_with_the_d3_provider_in_every_bundled_locale() {
    // The elements that both grammars share. A four-digit year writes the same in both.
    let formats = [
        "%A",
        "%a",
        "%B",
        "%b",
        "%p",
        "%c",
        "%x",
        "%X",
        "%e %d.%m.%Y %I:%M:%S",
    ];
    for (name, _) in BUNDLED {
        let locale = Locale::bundled(name).unwrap();
        let d3 = D3DateTimeFormatProvider::new().with_locale(*name);
        for format in formats {
            let ours = Format::new(format, Kind::Datetime, &locale).unwrap();
            let theirs = d3.prepare_naive(format).unwrap();
            for datetime in datetimes() {
                assert_eq!(
                    ours.format_datetime(datetime),
                    theirs.format(datetime).unwrap(),
                    "{name} {format} {datetime}"
                );
            }
        }
    }
}

#[test]
fn names_and_composites_parse_back_in_every_bundled_locale() {
    // Whether the text reads in upper case: the names do, and a composite's other text reads as
    // written.
    let formats = [
        ("%A %d %B %Y %H:%M:%S", true),
        ("%a %d %b %Y %H:%M:%S", true),
        ("%c", false),
        ("%x %X", false),
    ];
    for (name, _) in BUNDLED {
        let locale = Locale::bundled(name).unwrap();
        for (format, upper) in formats {
            let prepared = Format::new(format, Kind::Datetime, &locale).unwrap();
            for datetime in datetimes() {
                let text = prepared.format_datetime(datetime);
                let upper = upper.then(|| text.to_uppercase());
                for text in [Some(text), upper].into_iter().flatten() {
                    assert_eq!(
                        parse_datetime(format, &text, &locale),
                        Ok(datetime),
                        "{name} {format} {text:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn a_name_reads_its_longest_form_and_a_short_name_reads_without_its_period() {
    let french = Locale::bundled("fr-FR").unwrap();
    let date = |text: &str| parse_datetime("%b %Y", text, &french).map(|d| d.date());
    let july = NaiveDate::from_ymd_opt(2024, 7, 1).unwrap();
    assert_eq!(date("juil. 2024"), Ok(july));
    assert_eq!(date("JUIL 2024"), Ok(july));
    assert_eq!(date("juillet 2024"), Ok(july));
    assert_eq!(
        date("janv 2024"),
        Ok(NaiveDate::from_ymd_opt(2024, 1, 1).unwrap())
    );
}

#[test]
fn only_the_default_locale_writes_googlesqls_composites() {
    let datetime = NaiveDate::from_ymd_opt(2024, 3, 5)
        .unwrap()
        .and_hms_opt(2, 8, 9)
        .unwrap();
    let text = |format: &str, locale: &Locale| {
        Format::new(format, Kind::Datetime, locale)
            .unwrap()
            .format_datetime(datetime)
    };
    let googlesql = Locale::default();
    let english = Locale::bundled("en-US").unwrap();
    assert_eq!(
        text("%c|%x|%P", &googlesql),
        "Tue Mar  5 02:08:09 2024|03/05/24|am"
    );
    assert_eq!(
        text("%c|%x|%P", &english),
        "3/5/2024, 2:08:09 AM|3/5/2024|am"
    );
}

#[test]
fn a_locales_composites_are_googlesql_formats() {
    let error = |spec: DateTimeLocaleSpec| Locale::new(&spec).unwrap_err().to_string();
    let english = DateTimeLocaleSpec::default;
    assert_eq!(
        error(DateTimeLocaleSpec {
            date_time: "%A %c".into(),
            ..english()
        }),
        "invalid locale data: field `dateTime` has `%c`, which refers to itself"
    );
    assert_eq!(
        error(DateTimeLocaleSpec {
            date: "%d %H".into(),
            ..english()
        }),
        "invalid locale data: field `date` has `%H`, which needs a part that the value does not have"
    );
    assert_eq!(
        error(DateTimeLocaleSpec {
            time: "%L".into(),
            ..english()
        }),
        "invalid locale data: field `time` has `%L`, which is not an element of GoogleSQL's formats"
    );
}

#[test]
fn a_locale_and_a_zone_rebuild_from_their_spec_and_name() {
    use avenger_format_datetime_googlesql::Zone;
    for locale in [Locale::default(), Locale::bundled("en-US").unwrap()] {
        assert_eq!(Locale::new(&locale.spec()).unwrap(), locale);
    }
    for name in [
        "America/New_York",
        "UTC",
        "+05:30",
        "-08",
        "UTC+5",
        "+00:45",
    ] {
        let zone = Zone::parse(name).unwrap();
        assert_eq!(Zone::parse(&zone.name()), Some(zone), "{name}");
    }
}
