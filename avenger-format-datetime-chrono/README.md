# Chrono datetime formatting

`avenger-format-datetime-chrono` formats dates and datetimes with Chrono's [strftime patterns](https://docs.rs/chrono/latest/chrono/format/strftime/index.html). It implements the [datetime formatter interfaces](../avenger-format/README.md) from `avenger-format`.

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

Use `prepare_date()`, `prepare_naive()`, or `prepare_zoned()` for the input type. Preparation validates the pattern and resolves its locale settings once. Reuse the prepared formatter for multiple values.

`with_timezone()` sets the display timezone for zoned datetimes and defaults to UTC. It accepts a `chrono_tz::Tz` constant or a name parsed with `.parse()?`. Dates and naive datetimes preserve their calendar fields.

The default locale is Chrono's `POSIX` English. Enable `all-locales` to include Chrono's locale database:

```toml
avenger-format-datetime-chrono = { version = "0.1", features = ["all-locales"] }
```

This enables Chrono's `unstable-locales` feature. Select a locale with `.with_locale("fr-FR")`. Names accept hyphens or underscores, so `fr-FR` and `fr_FR` select the same locale. Names other than `POSIX` require `all-locales`.

The provider's `default_calendar_patterns()` returns strftime patterns as `CalendarPatterns`, which label each value by the coarsest calendar boundary it falls on, as Vega labels time axes. Prepare them through the provider with `prepare_date()`, `prepare_naive()`, or `prepare_zoned()`. Names and `%p` follow the locale. Locales without AM/PM markers, such as `de_DE`, leave `%p` empty, so replace the hour and minute patterns there, for example with `.with_hour("%H:00").with_minute("%H:%M")`.

Formatting preserves nanosecond precision and Chrono's leap-second representation. Directives that Chrono supports only for parsing, such as `%#z`, return an error during preparation.
