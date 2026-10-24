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

Number formatting returns `FormattedNumber`, which contains the label text and optional mantissa and exponent parts for scientific notation. Formatting accepts all `f64` values, including NaN and infinity.

`format_all()` formats a set of values together, such as an axis's ticks. Pass `PrecisionSharing::Shared` for evenly spaced values, such as a linear scale's ticks, so the labels share one precision and one SI or compact unit. Pass `PrecisionSharing::PerValue` for values that span magnitudes, such as a log scale's ticks, so each label keeps its own. When a pattern leaves precision open, providers can derive it from the values with `TickStep::infer()`. Explicit pattern settings take precedence. Providers that don't override `format_all()` format each value independently.

Datetime preparation checks that the format is valid for its input type. Date formats reject time, epoch, and timezone fields. Naive datetime formats reject epoch and timezone fields. Date and naive formatters ignore the display timezone. Zoned formatters convert UTC datetimes to the configured display timezone and report it with `timezone()`, as the provider does with `DateTimeFormatProvider::timezone()`.

`FormatValues` holds values of one kind, in the form its formatter accepts, and `ValueKind` names the kind. Consumers that format whichever values they're given, such as `LabelEngine::format` in `avenger-typst-label`, choose the provider by kind.

`CalendarPatterns` holds one pattern per calendar boundary, in a provider's syntax, for labels such as a time axis's. Its `prepare_date()`, `prepare_naive()`, and `prepare_zoned()` prepare each pattern through a provider and return an ordinary prepared formatter. That formatter labels each value with the pattern for the coarsest boundary it falls on, as Vega labels time axes: Jan 1 uses `year`, the 1st of another month `month`, other Sundays `week`, other days `day`, and times within a day `hour`, `minute`, `second`, or `millisecond`. Boundaries use the Gregorian calendar and Sunday weeks, so preparation first calls the provider's `check_gregorian_months()`, which rejects calendars whose months start on other days. Zoned values are tested in the formatter's timezone. Providers supply defaults with `DateTimeFormatProvider::default_calendar_patterns()`, and `with_*` builders replace single patterns.

Chrono inputs use the proleptic Gregorian calendar. Providers can convert the display date to another calendar. Datetime formatting returns text or an error for an unsupported value or a date outside the supported range.

Providers and usage examples are available in:

- [D3 number formatting](../avenger-format-number-d3/README.md).
- [D3 datetime formatting](../avenger-format-datetime-d3/README.md).
- [Chrono datetime formatting](../avenger-format-datetime-chrono/README.md).
- [ICU datetime formatting](../avenger-format-datetime-icu/README.md).
