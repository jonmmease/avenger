# Datetime formatting

`avenger-format-datetime` formats civil dates and instants with D3 datetime patterns and D3 locale definitions. It accepts concrete IANA display timezones independently of locale. It has no chart or Typst dependency.

```rust
use avenger_format_datetime::{
    DateTimeFormatContext, DateTimeLocaleRegistry, PreparedDateTimeFormat,
};

fn main() -> Result<(), avenger_format_datetime::DateTimeFormatError> {
    let locale = DateTimeLocaleRegistry::with_builtins().resolve("fr-FR")?;
    let formatter = PreparedDateTimeFormat::new(
        Some("%A %-d %B %Y %H:%M %Z"),
        Default::default(),
        DateTimeFormatContext::new(&locale, chrono_tz::Europe::Paris),
    )?;
    let instant = chrono::DateTime::from_timestamp(1_704_067_200, 0).unwrap();
    let text = formatter.format_zoned(instant).text;
    Ok(())
}
```

Prepare a formatter once and reuse it for labels or tooltip values. `format_naive` preserves civil calendar fields and rejects `%Q`, `%s`, `%Z`, or an explicit timezone override. `validate_naive` checks that constraint before a batch. Zoned formatting preserves the original instant for epoch directives and uses the selected zone for calendar fields and offsets. Fractional epoch milliseconds are clipped toward zero to match JavaScript Date. `%f` formats milliseconds followed by three zeros.

Locale JSON uses D3's `dateTime`, `date`, `time`, `periods`, `days`, `shortDays`, `months`, and `shortMonths` properties. The registry includes `en-US`, `de-DE`, `fr-FR`, and `ja-JP`. Register a complete custom definition with `register_custom_locale` or `register_custom_locale_json`. Locale arrays and recursive `%c`/`%x`/`%X` expansions are validated at construction. Locales do not infer timezones.

`PreparedTimeMultiFormat` implements Vega's automatic calendar-sensitive label selection and object-valued format overrides. Explicit scalar patterns remain separate from those defaults. The host resolves local time to a concrete IANA name before constructing the context.

The supported grammar is the documented [D3 datetime directive set](https://d3js.org/d3-time-format). LDML patterns and style lengths are removed. For example, use `%Y-%m-%d` for an ISO date, `%b %-d` for an abbreviated month and day, `%x` for the locale date pattern, and `%Z` for an offset. A pattern without directives is literal text. The crate formats existing values and does not implement temporal data parsing.

The [reference generator](../tools/format-reference/README.md) pins the upstream packages and generates the Rust test fixtures. The locale files include the upstream license.
