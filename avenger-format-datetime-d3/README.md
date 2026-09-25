# Datetime formatting

`avenger-format-datetime-d3` formats civil dates and instants with D3 datetime patterns and locale definitions. Display timezones are explicit IANA names, independent of locale.

```rust
use avenger_format_datetime_d3::{
    DateTimeFormatContext, PreparedDateTimeFormat, ResolvedDateTimeLocale,
};

fn main() -> Result<(), avenger_format_datetime_d3::DateTimeFormatError> {
    let locale = ResolvedDateTimeLocale::en_us();
    let formatter = PreparedDateTimeFormat::new(
        Some("%A %-d %B %Y %H:%M %Z"),
        Default::default(),
        DateTimeFormatContext::new(&locale, chrono_tz::America::New_York),
    )?;
    let instant = chrono::DateTime::from_timestamp(1_704_067_200, 0).unwrap();
    let text = formatter.format_zoned(instant)?;
    assert_eq!(text, "Sunday 31 December 2023 19:00 -0500");
    Ok(())
}
```

Prepare a formatter once and reuse it. An omitted scalar pattern uses the locale's `%c` pattern. Both formatting methods return `Result<String, DateTimeFormatError>`:

- `format_naive` preserves civil calendar fields. It rejects leap seconds, `%Q`, `%s`, `%Z`, and explicit timezone overrides.
- `format_zoned` accepts an instant represented in UTC. It uses the display zone for calendar fields and offsets, preserving the epoch value for `%Q` and `%s`. It returns an error if the display date exceeds Chrono's range. Fractional epoch milliseconds are clipped toward zero to match JavaScript Date. `%f` formats milliseconds followed by three zeros.

`DateTimeLocaleRegistry::with_builtins()` contains U.S. English (`en-US`). Register other complete D3 definitions with `register_custom_locale` or `register_custom_locale_json`. Locale JSON uses `dateTime`, `date`, `time`, `periods`, `days`, `shortDays`, `months`, and `shortMonths`. Array lengths and recursive `%c`/`%x`/`%X` expansions are validated. To customize a resolved locale, clone `definition()`, edit it, and register it. Existing prepared formatters retain their locale data.

`PreparedTimeMultiFormat` implements Vega's automatic calendar-sensitive label selection. `TimeMultiFormatSpec` overrides individual unit patterns, with empty strings using the defaults. A nonempty `date` pattern takes precedence over its `day` alias. Civil formatting selects by calendar fields. Zoned formatting accounts for repeated and skipped midnight boundaries, returning an error if a required boundary is outside Chrono's range.

Patterns use the documented [D3 datetime directives](https://d3js.org/d3-time-format). For example, `%Y-%m-%d` formats an ISO date, `%b %-d` an abbreviated month and day, `%x` the locale date, and `%Z` the offset. Plain text is literal. The crate formats existing values and does not parse date strings.

Two behaviors intentionally differ from D3: unknown directives and incomplete `%` sequences produce errors, and `%j` always uses the calendar day of the year. D3's elapsed-day calculation can be one day behind after a midnight offset change, such as January 1, 1986 in Kathmandu.

The [reference generator](../tools/format-reference/README.md) pins D3 and Vega to generate compatibility fixtures. Additional locale definitions live in test fixtures. The locale files include the upstream license.

`D3DateTimeFormatProvider` adapts these formats to the shared `avenger-format` traits. Register it explicitly in a `DateTimeFormatRegistry`; the shared crate does not select a provider. Configuration carries a locale name, complete custom D3 locale definitions, and a display timezone.

Every request supplies an explicit specification: a string for a D3 pattern, or an object for `TimeMultiFormatSpec`. An empty object selects Vega's calendar-sensitive patterns; an empty string produces empty labels. Callers choose ordinary patterns such as `%c` or `%Y-%m-%d`. Named options are rejected; timezone overrides use the shared request's `timezone` field. Errors from preparation or individual values propagate through the shared error type.

Use `prepare_naive` for dates and civil datetimes, or `prepare_zoned` for UTC instants. They return separate formatter traits, each with a `format` method for its input type. Civil preparation rejects timezone overrides and instant-only directives, including those in locale expansions or any multi-format branch. Formatting can still report value-dependent errors such as leap seconds or dates outside the supported range.
