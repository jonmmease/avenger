# Formatting interfaces

Traits for preparing reusable number, date, and datetime formatters.

Configure a provider, then pass a format string to its preparation method. Each provider defines its format syntax and locale settings. Prepared formatters retain those settings when the provider changes or is dropped.

`NumberFormatProvider` and `DateTimeFormatProvider` return prepared formatters through `Arc<dyn ...>`, so rendering code can use different providers through the same interface.

| Preparation method | Prepared trait | Value passed to `format()` |
| --- | --- | --- |
| `prepare()` | `PreparedNumberFormatter` | `f64` |
| `prepare_date()` | `PreparedDateFormatter` | `chrono::NaiveDate` |
| `prepare_naive()` | `PreparedNaiveDateTimeFormatter` | `chrono::NaiveDateTime` |
| `prepare_zoned()` | `PreparedZonedDateTimeFormatter` | `chrono::DateTime<chrono::Utc>` |

Number formatting returns `FormattedNumber`, which contains the label text and optional mantissa and exponent parts for scientific notation. Formatting accepts all `f64` values, including NaN and infinity. Callers choose formats and tick spacing.

Datetime preparation checks that the format is valid for its input type. Date formats reject time, epoch, and timezone fields. Naive datetime formats reject epoch and timezone fields. Date and naive formatters ignore the display timezone. Zoned formatters convert UTC datetimes to the configured display timezone.

Chrono inputs use the proleptic Gregorian calendar. Providers can convert the display date to another calendar. Datetime formatting returns text or an error for an unsupported value or a date outside the supported range.

Providers and usage examples are available in:

- [D3 number formatting](../avenger-format-number-d3/README.md).
- [D3 datetime formatting](../avenger-format-datetime-d3/README.md).
- [Chrono datetime formatting](../avenger-format-datetime-chrono/README.md).
- [ICU datetime formatting](../avenger-format-datetime-icu/README.md).
