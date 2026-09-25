# Number formatter interfaces

`NumberFormatProvider` prepares a reusable `PreparedNumberFormatter` from a specifier, named options, locale configuration, and numeric context. Providers own their syntax and locale data. Callers receive `FormattedNumber`, which carries plain text and optional scientific-notation parts for a text renderer.

`NumberFormatConfig` serializes a provider name, optional locale name, and custom locale definitions. Runtime providers are registered separately in `NumberFormatRegistry`. Unknown provider names return an error. The default provider name is `d3`, supplied by `avenger-format-number-d3`.

Callers choose tick spacing and pass it with a reference value through `NumberFormatContext::Step`. The provider selects precision and units without generating ticks. `Scalar`, `Continuous`, and `Discrete` contexts distinguish individual values, automatic precision, and numeric categories.

Share a registry through `Arc`. To replace a provider, clone the registry and register the replacement. Existing prepared formatters and registry snapshots retain their behavior. Label caches should include `cache_id()` and the complete formatting configuration in their keys. Provider implementations must remain immutable after registration.
