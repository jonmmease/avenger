# Formatting interfaces

`NumberFormatProvider` and `DateTimeFormatProvider` prepare an explicit pattern string using a provider-specific associated `Config` type. The implementation defines its pattern syntax, locale data, validation, and preparation options.

Select a concrete provider and pass its typed configuration to `prepare()`, `prepare_naive()`, or `prepare_zoned()`. Generic preparation code can accept `P: NumberFormatProvider` or `P: DateTimeFormatProvider` and `&P::Config`.

Prepared formatters own their resolved state and remain unchanged when the input configuration is modified or dropped. They are returned as `Arc<dyn PreparedNumberFormatter>`, `Arc<dyn PreparedCivilDateTimeFormatter>`, or `Arc<dyn PreparedInstantFormatter>`, so rendering code can use different implementations through the same interface.

Number formatting returns `FormattedNumber`, which carries plain text and optional scientific-notation parts for a text renderer. Callers choose patterns and tick spacing. Provider configuration can supply precision options without generating ticks.

Datetime preparation validates the pattern for the input type. Civil inputs retain their calendar fields. Instant formatting displays a UTC instant in the configured timezone. Formatting returns plain text or an error that depends on the value, such as an out-of-range display date.

Implementations are available in `avenger-format-number-d3`, `avenger-format-datetime-d3`, and `avenger-format-datetime-chrono`. Each crate exposes its configuration type and builders. Applications choose patterns and compose formatters for automatic axis labels.
