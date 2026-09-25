# Datetime formatting

`avenger-format-datetime-d3` formats civil dates and instants with D3 datetime patterns and locale definitions. Display timezones are explicit IANA names, independent of locale.

```rust
use avenger_format::DateTimeFormatProvider;
use avenger_format_datetime_d3::{D3DateTimeFormatConfig, D3DateTimeFormatProvider};

fn main() -> Result<(), avenger_format::DateTimeFormatError> {
    let config = D3DateTimeFormatConfig::new().with_timezone("America/New_York");
    let formatter = D3DateTimeFormatProvider.prepare_zoned(
        &config,
        "%A %-d %B %Y %H:%M %Z",
    )?;
    let instant = chrono::DateTime::from_timestamp(1_704_067_200, 0).unwrap();
    assert_eq!(formatter.format(instant)?, "Sunday 31 December 2023 19:00 -0500");
    Ok(())
}
```

Prepare a formatter once and reuse it. The low-level `PreparedDateTimeFormat::new(None, context)` uses the locale's `%c` pattern. Its formatting methods return `Result<String, DateTimeFormatError>`:

- `format_naive` preserves civil calendar fields. It rejects leap seconds, `%Q`, `%s`, and `%Z`.
- `format_zoned` accepts an instant represented in UTC. It uses the display zone for calendar fields and offsets, preserving the epoch value for `%Q` and `%s`. It returns an error if the display date exceeds Chrono's range. Fractional epoch milliseconds are clipped toward zero to match JavaScript Date. `%f` formats milliseconds followed by three zeros.

`DateTimeLocaleRegistry::with_builtins()` contains U.S. English (`en-US`). Register other complete D3 definitions with `register_custom_locale` or `register_custom_locale_json`. Locale JSON uses `dateTime`, `date`, `time`, `periods`, `days`, `shortDays`, `months`, and `shortMonths`. Array lengths and recursive `%c`/`%x`/`%X` expansions are validated. To customize a resolved locale, clone `definition()`, edit it, and register it. Existing prepared formatters retain their locale data.

Patterns use the documented [D3 datetime directives](https://d3js.org/d3-time-format). For example, `%Y-%m-%d` formats an ISO date, `%b %-d` an abbreviated month and day, `%x` the locale date, and `%Z` the offset. Plain text is literal. The crate formats existing values and does not parse date strings.

Two behaviors intentionally differ from D3: unknown directives and incomplete `%` sequences produce errors, and `%j` always uses the calendar day of the year. D3's elapsed-day calculation can be one day behind after a midnight offset change, such as January 1, 1986 in Kathmandu.

The [reference generator](../tools/format-reference/README.md) pins the reference packages to generate compatibility fixtures. Additional locale definitions live in test fixtures. The locale files include the upstream license.

`D3DateTimeFormatProvider` implements `avenger-format` with `D3DateTimeFormatConfig`. Configuration carries a locale name, typed custom `DateTimeLocaleSpec` definitions, and a display timezone. Its builders select a locale, add a custom definition, and set the timezone.

The provider treats hyphens and underscores as equivalent in locale names, including custom definitions. An exact custom name takes precedence when both forms are registered.

Pass one explicit pattern string to `prepare_naive()` or `prepare_zoned()`. Civil preparation rejects epoch and timezone directives, including directives in locale expansions. An empty pattern produces an empty label. Callers choose ordinary patterns such as `%c` or `%Y-%m-%d`. Errors from preparation or individual values propagate through the shared error type.
