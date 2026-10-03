# D3 datetime formatting

`avenger-format-datetime-d3` formats dates and datetimes with [D3 datetime patterns](https://d3js.org/d3-time-format) and locale definitions. It implements the [datetime formatter interfaces](../avenger-format/README.md) from `avenger-format`.

```rust
use avenger_format::DateTimeFormatProvider;
use avenger_format_datetime_d3::D3DateTimeFormatProvider;

fn main() -> Result<(), avenger_format::DateTimeFormatError> {
    let provider = D3DateTimeFormatProvider::new().with_timezone(chrono_tz::America::New_York);
    let date_formatter = provider.prepare_date("%Y-%m-%d")?;
    let date = chrono::NaiveDate::from_ymd_opt(2024, 1, 1).unwrap();
    assert_eq!(date_formatter.format(date)?, "2024-01-01");
    let formatter = provider.prepare_zoned("%A %-d %B %Y %H:%M %Z")?;
    let zoned = chrono::DateTime::from_timestamp(1_704_067_200, 0).unwrap();
    assert_eq!(formatter.format(zoned)?, "Sunday 31 December 2023 19:00 -0500");
    Ok(())
}
```

Use `prepare_date()`, `prepare_naive()`, or `prepare_zoned()` for the input type. Preparation validates the pattern, including locale expansions. Reuse the prepared formatter for multiple values.

`with_timezone()` sets the display timezone for zoned datetimes and defaults to UTC. It accepts a `chrono_tz::Tz` constant or a name parsed with `.parse()?`. Dates and naive datetimes preserve their calendar fields.

The default locale is U.S. English (`en-US`). Enable `all-locales` to include all bundled D3 locales:

```toml
avenger-format-datetime-d3 = { version = "0.1", features = ["all-locales"] }
```

Select a locale with `.with_locale("fr-FR")`. Names accept hyphens or underscores. Add custom `DateTimeLocaleSpec` definitions with `with_custom_locale()`. Custom definitions take precedence over bundled definitions and work without `all-locales`.

Formatting uses millisecond precision. `%f` prints milliseconds followed by three zeros. Zoned formatting truncates fractional epoch milliseconds toward zero to match JavaScript `Date`. Naive leap-second values return an error.

Unknown directives and incomplete `%` sequences return errors. `%j` uses the calendar day of the year. This intentionally differs from D3's elapsed-day calculation, which can be one day behind after a midnight offset change.

The locale files and license come from [d3-time-format 4.1.0](https://github.com/d3/d3-time-format/tree/76cfef1da81b70404c0226ec9db9b0d56fa09461/locale). The [reference generator](../tools/format-reference/README.md) produces compatibility fixtures from D3.
