# Chrono datetime formatting

`avenger-format-datetime-chrono` implements the datetime formatter traits from `avenger-format` with Chrono's [strftime patterns](https://docs.rs/chrono/latest/chrono/format/strftime/index.html).

```rust
use avenger_format::DateTimeFormatProvider;
use avenger_format_datetime_chrono::{ChronoDateTimeFormatConfig, ChronoDateTimeFormatProvider};

fn main() -> Result<(), avenger_format::DateTimeFormatError> {
    let config = ChronoDateTimeFormatConfig::new()
        .with_locale("en-US")
        .with_timezone("America/New_York");
    let date_formatter = ChronoDateTimeFormatProvider.prepare_date(&config, "%Y-%m-%d")?;
    let date = chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    assert_eq!(date_formatter.format(date)?, "2024-01-01");
    let formatter = ChronoDateTimeFormatProvider.prepare_zoned(&config, "%Y-%m-%d %H:%M:%S%.6f %:z")?;
    let zoned = chrono::DateTime::from_timestamp(1_704_067_200, 123_456_789).unwrap();
    assert_eq!(formatter.format(zoned)?, "2023-12-31 19:00:00.123456 -05:00");
    Ok(())
}
```

Pass a string pattern directly to a preparation method. Preparation resolves the pattern and locale once, including locale expansions.

- `prepare_date()` accepts `chrono::NaiveDate` and rejects time, epoch, and timezone fields.
- `prepare_naive()` accepts `chrono::NaiveDateTime` and rejects epoch and timezone fields.
- `prepare_zoned()` accepts `chrono::DateTime<chrono::Utc>` and uses the IANA display timezone in `ChronoDateTimeFormatConfig`, or UTC when absent.

Date and naive preparation ignore timezone configuration. Parsing-only directives such as `%#z` fail during preparation for all three paths.

Locale names accept either separator: `en-US` and `en_US` select the same locale. An omitted locale uses `POSIX`. The crate enables Chrono's `unstable-locales` feature for its built-in locale data.

Formatting preserves the input's submillisecond precision and Chrono's leap-second representation. `%f` prints nanoseconds, `%3f` prints milliseconds, `%6f` prints microseconds, and `%z` prints a numeric timezone offset. Formatting errors and display dates outside the supported calendar range return `DateTimeFormatError`.
