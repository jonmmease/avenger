# Chrono datetime formatting

`avenger-format-datetime-chrono` implements the datetime formatter traits from `avenger-format` with Chrono's [strftime patterns](https://docs.rs/chrono/latest/chrono/format/strftime/index.html).

```rust
use avenger_format::DateTimeFormatProvider;
use avenger_format_datetime_chrono::ChronoDateTimeFormatProvider;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let provider = ChronoDateTimeFormatProvider::new()
        .with_timezone("America/New_York".parse()?);
    let date_formatter = provider.prepare_date("%Y-%m-%d")?;
    let date = chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    assert_eq!(date_formatter.format(date)?, "2024-01-01");
    let formatter = provider.prepare_zoned("%Y-%m-%d %H:%M:%S%.6f %:z")?;
    let zoned = chrono::DateTime::from_timestamp(1_704_067_200, 123_456_789).unwrap();
    assert_eq!(formatter.format(zoned)?, "2023-12-31 19:00:00.123456 -05:00");
    Ok(())
}
```

The provider owns its locale and display timezone. Pass a string pattern directly to a preparation method. Preparation resolves the pattern and locale once, including locale expansions. Prepared formatters retain these settings when the provider is changed or dropped.

`with_timezone()` accepts a resolved `chrono_tz::Tz`. Use a constant such as `chrono_tz::America::New_York`, or parse a name with `.parse()?`. The default timezone is UTC. Serialized providers use IANA timezone names and reject invalid names during deserialization.

- `prepare_date()` accepts `chrono::NaiveDate` and rejects time, epoch, and timezone fields.
- `prepare_naive()` accepts `chrono::NaiveDateTime` and rejects epoch and timezone fields.
- `prepare_zoned()` accepts `chrono::DateTime<chrono::Utc>` and uses the provider's resolved display timezone.

Date and naive preparation ignore timezone configuration. Parsing-only directives such as `%#z` fail during preparation for all three paths.

By default, the provider supports only Chrono's `POSIX` English conventions and does not enable Chrono's locale database. Enable `all-locales` to use the full database:

```toml
avenger-format-datetime-chrono = { version = "0.1", features = ["all-locales"] }
```

This feature enables Chrono's `unstable-locales` feature. Select a locale with `.with_locale("fr-FR")`. Names accept either separator: `en-US` and `en_US` select the same locale. An omitted locale still uses `POSIX`. Without `all-locales`, selecting any name other than `POSIX` returns an error during preparation.

Formatting preserves the input's submillisecond precision and Chrono's leap-second representation. `%f` prints nanoseconds, `%3f` prints milliseconds, `%6f` prints microseconds, and `%z` prints a numeric timezone offset. Formatting errors and display dates outside the supported calendar range return `DateTimeFormatError`.
