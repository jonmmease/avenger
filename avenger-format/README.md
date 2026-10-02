# Formatting interfaces

`NumberFormatProvider` and `DateTimeFormatProvider` prepare explicit pattern strings. Each provider owns its locale and formatting settings. The implementation defines its pattern syntax, locale data, validation, and preparation options.

Construct a provider with its settings, then pass a pattern to a preparation method. Generic preparation code can accept `P: NumberFormatProvider` or `P: DateTimeFormatProvider`. Both traits also support `dyn` dispatch.

Prepared formatters own their resolved state and remain unchanged when the provider is modified or dropped. Each method returns an `Arc<dyn ...>` for its prepared trait, so rendering code can use different implementations through the same interface.

| Preparation method | Prepared trait | Input |
| --- | --- | --- |
| `prepare()` | `PreparedNumberFormatter` | `f64` |
| `prepare_date()` | `PreparedDateFormatter` | `chrono::NaiveDate` |
| `prepare_naive()` | `PreparedNaiveDateTimeFormatter` | `chrono::NaiveDateTime` |
| `prepare_zoned()` | `PreparedZonedDateTimeFormatter` | `chrono::DateTime<chrono::Utc>` |

Number formatting returns `FormattedNumber`, which carries plain text and optional scientific-notation parts for a text renderer. Callers choose patterns and tick spacing. Providers can carry precision options without generating ticks.

Datetime preparation validates the pattern for the input type. Date patterns reject time, epoch, and timezone fields. Naive datetime patterns reject epoch and timezone fields. Both paths preserve calendar fields and ignore timezone configuration. Zoned formatting displays a UTC datetime in the configured timezone. Formatting returns plain text or an error that depends on the value, such as an out-of-range display date.

Implementations are available in `avenger-format-number-d3`, `avenger-format-datetime-d3`, and `avenger-format-datetime-chrono`. Each provider supports builders, cloning, and serialization of its settings. Applications choose patterns and compose formatters for automatic axis labels.
