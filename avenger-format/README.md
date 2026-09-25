# Formatter interfaces

`NumberFormatProvider` prepares a reusable `PreparedNumberFormatter` from an explicit specifier, named options, and locale configuration. Providers own their syntax and locale data. Callers receive `FormattedNumber`, which carries plain text and optional scientific-notation parts for a text renderer.

`NumberFormatConfig` serializes a provider name, optional locale name, and custom locale definitions. Runtime providers are registered separately in `NumberFormatRegistry`. Provider selection is required. Create a configuration with `NumberFormatConfig::new("d3")` and register `D3NumberFormatProvider` from `avenger-format-number-d3` to use D3. Empty registries and unknown provider names return an error when preparing a formatter.

Use `with_locale()` and `with_custom_locale()` to configure number locales. Prepare ordinary formats with `registry.prepare(&config, ",.2f")`. Pass an owned or borrowed `NumberFormatRequest` when named options are needed.

Callers choose number format strings and tick spacing. The D3 provider accepts `auto_precision` or a pair of `step` and `reference_value` options to infer precision and units without generating ticks.

Share a registry through `Arc`. To replace a provider, clone the registry and register the replacement. Existing prepared formatters and registry snapshots retain their behavior. Label caches should include `cache_id()`, the format specification, and the complete formatting configuration in their keys. Provider implementations must remain immutable after registration.

`DateTimeFormatProvider` prepares separate formatters for civil dates and datetimes through `prepare_naive()`, and for UTC instants through `prepare_zoned()`. Preparation validates the specification for the input type. Formatting returns plain text or a value error. Civil inputs retain their calendar fields. Timezone settings only affect instants.

`DateTimeFormatConfig` selects a registered provider, locale data, and an optional IANA display timezone (UTC when absent). Its builders select a locale, add custom locale definitions, and set the display timezone. Pass a format string or provider-specific JSON specification separately to the registry's preparation method, such as `registry.prepare_zoned(&config, "%B %-d, %Y")`. Providers own pattern syntax, locale definitions, validation, and precision.

`DateTimeFormatRegistry` follows the same explicit registration and snapshot rules as the number registry. It starts empty, and configuration has no implicit provider. Register `D3DateTimeFormatProvider` from `avenger-format-datetime-d3` or `ChronoDateTimeFormatProvider` from `avenger-format-datetime-chrono`, then select the registered name with `DateTimeFormatConfig::new()`. Both providers accept hyphens or underscores in locale names. D3 accepts string patterns and structured calendar formats. Chrono accepts its own string patterns.
