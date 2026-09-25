# Formatter interfaces

`NumberFormatProvider` prepares a reusable `PreparedNumberFormatter` from a specifier, named options, locale configuration, and numeric context. Providers own their syntax and locale data. Callers receive `FormattedNumber`, which carries plain text and optional scientific-notation parts for a text renderer.

`NumberFormatConfig` serializes a provider name, optional locale name, and custom locale definitions. Runtime providers are registered separately in `NumberFormatRegistry`. Provider selection is required. Create a configuration with `NumberFormatConfig::new("d3")` and register `D3NumberFormatProvider` from `avenger-format-number-d3` to use D3. Empty registries and unknown provider names return an error when preparing a formatter.

Callers choose tick spacing and pass it with a reference value through `NumberFormatContext::Step`. The provider selects precision and units without generating ticks. `Scalar`, `Continuous`, and `Discrete` contexts distinguish individual values, automatic precision, and numeric categories.

Share a registry through `Arc`. To replace a provider, clone the registry and register the replacement. Existing prepared formatters and registry snapshots retain their behavior. Label caches should include `cache_id()` and the complete formatting configuration in their keys. Provider implementations must remain immutable after registration.

`DateTimeFormatProvider` prepares a `PreparedDateTimeFormatter` for civil dates, civil datetimes, and UTC instants. The formatter returns plain text or a value error. Civil inputs retain their calendar fields; timezone settings only affect instants. `validate_naive()` checks whether a prepared pattern supports civil values before formatting a batch.

`DateTimeFormatConfig` selects a registered provider, locale data, and an optional IANA display timezone (UTC when absent). `DateTimeFormatRequest` carries a provider-specific string or structured specification, named options, and an optional per-call timezone override. An explicit per-call timezone is invalid for civil input. Providers own pattern syntax, locale definitions, validation, and precision.

The datetime contexts describe the caller’s purpose: `Scalar` uses the provider’s ordinary pattern, `Data` formats dates, civil datetimes, and instants with suitable identifying fields, and `Tick` selects calendar-sensitive labels. Tick generation remains in the scale layer.

`DateTimeFormatRegistry` follows the same explicit registration and snapshot rules as the number registry. It starts empty, and configuration has no implicit provider. To use D3, register `D3DateTimeFormatProvider` from `avenger-format-datetime-d3` and select it with `DateTimeFormatConfig::new("d3")`. A different provider can define its own request syntax without changing consumers.
