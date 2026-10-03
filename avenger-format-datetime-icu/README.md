# ICU datetime formatting

Datetime formatting for `avenger-format`, using ICU4X's bundled locale data and calendar systems.

- `IcuPatternDateTimeFormatProvider` formats explicit [Unicode datetime patterns](https://unicode.org/reports/tr35/tr35-dates.html#Date_Format_Patterns), such as `yyyy-MM-dd`.
- `IcuSemanticDateTimeFormatProvider` selects locale-appropriate layouts from named options in a `{...}` block, with literal prefix and suffix text. Its field selection follows [Unicode semantic skeletons](https://unicode.org/reports/tr35/tr35-dates.html#Semantic_Skeletons).

Both providers prepare reusable formatters for Chrono dates and datetimes, with configurable locales, calendars, and display timezones.

```rust
use avenger_format::DateTimeFormatProvider;
use avenger_format_datetime_icu::{
    IcuPatternDateTimeFormatProvider, IcuSemanticDateTimeFormatProvider,
};
use chrono::NaiveDate;

let date = NaiveDate::from_ymd_opt(2024, 3, 11).unwrap();

let pattern = IcuPatternDateTimeFormatProvider::new()
    .prepare_date("yyyy-MM-dd")?;
assert_eq!(pattern.format(date)?, "2024-03-11");

let semantic = IcuSemanticDateTimeFormatProvider::new()
    .with_locale("fr-FR")
    .prepare_date("As of {dateFields=year-month-day}")?;
assert_eq!(semantic.format(date)?, "As of 11 mars 2024");
# Ok::<(), avenger_format::DateTimeFormatError>(())
```
