# Formatting interfaces

`NumberFormatProvider` and `DateTimeFormatProvider` prepare explicit format specifications. Each provider owns its locale and formatting settings. The implementation defines its specification syntax, locale data, validation, and preparation options.

Construct a provider with its settings, then pass a specification to a preparation method. Generic preparation code can accept `P: NumberFormatProvider` or `P: DateTimeFormatProvider`. Both traits also support `dyn` dispatch.

Prepared formatters own their resolved state and remain unchanged when the provider is modified or dropped. Each method returns an `Arc<dyn ...>` for its prepared trait, so rendering code can use different implementations through the same interface.

| Preparation method | Prepared trait | Input |
| --- | --- | --- |
| `prepare()` | `PreparedNumberFormatter` | `f64` |
| `prepare_date()` | `PreparedDateFormatter` | `chrono::NaiveDate` |
| `prepare_naive()` | `PreparedNaiveDateTimeFormatter` | `chrono::NaiveDateTime` |
| `prepare_zoned()` | `PreparedZonedDateTimeFormatter` | `chrono::DateTime<chrono::Utc>` |

Number formatting returns `FormattedNumber`, which carries plain text and optional scientific-notation parts for a text renderer. Callers choose formats and tick spacing. Providers can carry precision options without generating ticks.

Datetime preparation validates the specification for the input type. Date formats reject time, epoch, and timezone fields. Naive datetime formats reject epoch and timezone fields. Both paths ignore timezone configuration and preserve the input date and time value. A provider can display that date in another calendar system. Chrono input dates use the proleptic Gregorian calendar. Zoned formatting displays a UTC datetime in the configured timezone before any calendar conversion. Formatting returns plain text or an error that depends on the value, such as an out-of-range display date.

Implementations are available in `avenger-format-number-d3`, `avenger-format-datetime-d3`, `avenger-format-datetime-chrono`, and `avenger-format-datetime-icu`. Each provider supports builders, cloning, and serialization of its settings. The ICU crate offers separate `IcuPatternDateTimeFormatProvider` and `IcuSemanticDateTimeFormatProvider` implementations for explicit patterns and locale-driven layouts. Applications choose specifications and compose formatters for automatic axis labels.
