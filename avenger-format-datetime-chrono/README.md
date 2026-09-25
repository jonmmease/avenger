# Chrono datetime formatting

`avenger-format-datetime-chrono` implements the datetime formatter traits from `avenger-format` with Chrono's [strftime patterns](https://docs.rs/chrono/latest/chrono/format/strftime/index.html).

```rust
use std::sync::Arc;
use avenger_format::{DateTimeFormatConfig, DateTimeFormatRegistry, DateTimeFormatRequest};
use avenger_format_datetime_chrono::ChronoDateTimeFormatProvider;

fn main() -> Result<(), avenger_format::DateTimeFormatError> {
    let mut registry = DateTimeFormatRegistry::default();
    registry.register("chrono", Arc::new(ChronoDateTimeFormatProvider));
    let config = DateTimeFormatConfig::new("chrono")
        .with_locale("en-US")
        .with_timezone("America/New_York");
    let formatter = registry.prepare_zoned(
        &config,
        &DateTimeFormatRequest::new("%Y-%m-%d %H:%M:%S%.6f %:z"),
    )?;
    let instant = chrono::DateTime::from_timestamp(1_704_067_200, 123_456_789).unwrap();
    assert_eq!(formatter.format(instant)?, "2023-12-31 19:00:00.123456 -05:00");
    Ok(())
}
```

Every request supplies a string pattern. Preparation resolves the pattern and locale once. `prepare_naive()` accepts dates and civil datetimes, with dates interpreted as midnight. It rejects epoch and timezone fields, including those in locale patterns, and explicit timezone overrides. Configuration's display timezone does not affect civil values. `prepare_zoned()` accepts UTC instants and resolves an IANA display timezone, using the request override before the configuration and UTC when neither is set. Parsing-only directives such as `%#z` fail during preparation.

Locale names accept either separator: `en-US` and `en_US` select the same locale. An omitted locale uses `POSIX`. The crate enables Chrono's `unstable-locales` feature for its built-in locale data. Custom locale definitions, named options, and structured specifications are unsupported.

Formatting preserves the input's submillisecond precision and Chrono's leap-second representation. `%f` prints nanoseconds, `%3f` prints milliseconds, `%6f` prints microseconds, and `%z` prints a numeric timezone offset. Formatting errors and display dates outside the supported calendar range return `DateTimeFormatError`.
