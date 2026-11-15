# GoogleSQL datetime formatting

`avenger-format-datetime-googlesql` formats dates, times, datetimes, and instants with the format strings of GoogleSQL's `FORMAT_DATE`, `FORMAT_TIME`, `FORMAT_DATETIME`, and `FORMAT_TIMESTAMP`, and parses them as `PARSE_DATE`, `PARSE_TIME`, `PARSE_DATETIME`, and `PARSE_TIMESTAMP` do, with GoogleSQL's text, values, and error messages. `GoogleSqlDateTimeFormatProvider` prepares the formats for the labels of charts, with the syntax name `googlesql`.

```rust
use avenger_format_datetime_googlesql::{parse_date, Format, Kind, Locale};

let googlesql = Locale::default();
let format = Format::new("%A, %B %e %Y, Q%Q", Kind::Date, &googlesql).unwrap();
let date = chrono::NaiveDate::from_ymd_opt(2024, 3, 5).unwrap();
assert_eq!(format.format_date(date), "Tuesday, March  5 2024, Q1");
let date = parse_date("%b %e %Y", "mar 5 2024", &googlesql).unwrap();
assert_eq!(date, chrono::NaiveDate::from_ymd_opt(2024, 3, 5).unwrap());
```

A parse takes a field that the format does not have from 1970-01-01 00:00:00. Whitespace in the format matches any amount of whitespace, numbers read up to their width without padding, and names match full or abbreviated, in any case. GoogleSQL rejects a weekday together with a week number, and an ISO week, as not supported yet, and so does the crate.

GoogleSQL has one locale, en-US, which is the default `Locale`. A `Locale` from a d3 time locale, such as `Locale::bundled("fr-FR")`, changes the names of days and months, the periods of the day of `%p` and `%P`, and the composites `%c`, `%x`, and `%X`, for formats and parses, as an extension of GoogleSQL. Its composites are the d3 locale's, so the bundled en-US locale writes `%x` as `3/5/2024` where the default writes `03/05/24`. A parse reads the longest name that matches, in any case, and a short name that ends with `.` also without it. The provider selects a locale by name, as the d3 provider does: `with_locale("fr-FR")` selects a custom definition from `with_custom_locale` or a bundled locale, and a provider without a locale uses GoogleSQL's names and composites. The default build bundles en-US, and the `all-locales` feature bundles all of d3's locales.

The elements that a type of value lacks, such as the hour of a `DATE`, print as written, as in GoogleSQL. `%P` prints `am` or `pm`, as GoogleSQL's documentation says. An element outside GoogleSQL's documented list, or with a width, such as `%10d`, prints as written, as the C library of GoogleSQL's Linux build prints it.

The tests compare the formats and the parses with GoogleSQL's `execute_query`, in `tests/data/googlesql_format.tsv` and `tests/data/googlesql_parse.tsv`, and the formats of each bundled locale with the d3 provider.
