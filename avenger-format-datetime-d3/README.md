# Datetime formatting

`avenger-format-datetime-d3` formats dates, naive datetimes, and zoned datetimes with D3 datetime patterns and locale definitions. Display timezones are resolved IANA zones, independent of locale.

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

Prepare a formatter once and reuse it. The low-level `PreparedDateTimeFormat::new(None, context)` uses the locale's `%c` pattern. Its formatting methods return `Result<String, DateTimeFormatError>`:

- `format_date` accepts `chrono::NaiveDate` and preserves its calendar date. It rejects time, epoch, and timezone directives.
- `format_naive` accepts `chrono::NaiveDateTime` and preserves its calendar fields. It rejects leap seconds, `%Q`, `%s`, and `%Z`.
- `format_zoned` accepts a zoned datetime represented in UTC. It uses the display zone for calendar fields and offsets, preserving the epoch value for `%Q` and `%s`. It returns an error if the display date exceeds Chrono's range. Fractional epoch milliseconds are clipped toward zero to match JavaScript Date. `%f` formats milliseconds followed by three zeros.

By default, the provider and `DateTimeLocaleRegistry::with_builtins()` include only U.S. English (`en-US`). Enable `all-locales` to embed all 34 D3 locales with `include_str!`:

```toml
avenger-format-datetime-d3 = { version = "0.1", features = ["all-locales"] }
```

The provider can then select a bundled locale with `.with_locale("fr-FR")`. Custom definitions take precedence over bundled definitions. Register complete D3 definitions with `register_custom_locale` or `register_custom_locale_json`. Locale JSON uses `dateTime`, `date`, `time`, `periods`, `days`, `shortDays`, `months`, and `shortMonths`. Array lengths and recursive `%c`/`%x`/`%X` expansions are validated. To customize a resolved locale, clone `definition()`, edit it, and register it. Existing prepared formatters retain their locale data.

Patterns use the documented [D3 datetime directives](https://d3js.org/d3-time-format). For example, `%Y-%m-%d` formats an ISO date, `%b %-d` an abbreviated month and day, `%x` the locale date, and `%Z` the offset. Plain text is literal. The crate formats existing values and does not parse date strings.

Two behaviors intentionally differ from D3: unknown directives and incomplete `%` sequences produce errors, and `%j` always uses the calendar day of the year. D3's elapsed-day calculation can be one day behind after a midnight offset change, such as January 1, 1986 in Kathmandu.

The [reference generator](../tools/format-reference/README.md) pins the reference packages to generate compatibility fixtures. The files in `locales/` are copied unchanged from [d3-time-format 4.1.0](https://github.com/d3/d3-time-format/tree/76cfef1da81b70404c0226ec9db9b0d56fa09461/locale), with the upstream license. The additional locales are excluded from builds without `all-locales`.

`D3DateTimeFormatProvider` implements `DateTimeFormatProvider` from `avenger-format`. It owns a locale name, typed custom `DateTimeLocaleSpec` definitions, and a display timezone. Its builders select a locale, add a custom definition, and set the timezone. Prepared formatters retain their resolved settings when the provider is changed or dropped.

`with_timezone()` accepts a resolved `chrono_tz::Tz`. Use a timezone constant or parse a name with `.parse()?`. The default timezone is UTC. Serialized providers use IANA timezone names and reject invalid names during deserialization.

The provider treats hyphens and underscores as equivalent in locale names, including custom definitions. An exact custom name takes precedence when both forms are registered.

Pass one explicit pattern string to `prepare_date()`, `prepare_naive()`, or `prepare_zoned()`. Preparation validates the pattern for the corresponding input type, including directives in locale expansions. Date and naive preparation ignore timezone configuration. An empty pattern produces an empty label. Callers choose ordinary patterns such as `%c` or `%Y-%m-%d`. Errors from preparation or individual values propagate through the shared error type.
