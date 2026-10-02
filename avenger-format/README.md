# Formatting interfaces

`NumberFormatProvider` and `DateTimeFormatProvider` prepare an explicit pattern string using a provider-specific associated `Config` type. The implementation defines its pattern syntax, locale data, validation, and preparation options.

Select a concrete provider and pass its typed configuration to a preparation method. Generic preparation code can accept `P: NumberFormatProvider` or `P: DateTimeFormatProvider` and `&P::Config`.

Prepared formatters own their resolved state and remain unchanged when the input configuration is modified or dropped. Each method returns an `Arc<dyn ...>` for its prepared trait, so rendering code can use different implementations through the same interface.

| Preparation method | Prepared trait | Input |
| --- | --- | --- |
| `prepare()` | `PreparedNumberFormatter` | `f64` |
| `prepare_date()` | `PreparedDateFormatter` | `chrono::NaiveDate` |
| `prepare_naive()` | `PreparedNaiveDateTimeFormatter` | `chrono::NaiveDateTime` |
| `prepare_zoned()` | `PreparedZonedDateTimeFormatter` | `chrono::DateTime<chrono::Utc>` |

`NumberFormatBinding` and `DateTimeFormatBinding` capture a provider and its typed configuration when a consumer needs to prepare patterns at runtime. Clones share the captured settings and cache identity. Constructing a replacement binding assigns a new identity. These bindings require neither serialization nor a shared configuration type.

Number formatting returns `FormattedNumber`, which carries plain text and optional scientific-notation parts for a text renderer. Callers choose patterns and tick spacing. Provider configuration can supply precision options without generating ticks.

Datetime preparation validates the pattern for the input type. Date patterns reject time, epoch, and timezone fields. Naive datetime patterns reject epoch and timezone fields. Both paths preserve calendar fields and ignore timezone configuration. Zoned formatting displays a UTC datetime in the configured timezone. Formatting returns plain text or an error that depends on the value, such as an out-of-range display date.

Implementations are available in `avenger-format-number-d3`, `avenger-format-datetime-d3`, and `avenger-format-datetime-chrono`. Each crate exposes its configuration type and builders. Applications choose patterns and compose formatters for automatic axis labels.
