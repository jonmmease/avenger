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

Both providers' `default_calendar_patterns()` return `CalendarPatterns`, which label each value by the coarsest calendar boundary it falls on: the year on Jan 1, the month name on the 1st of a month, the weekday or date at midnight, and the time otherwise. The semantic provider's are localized field sets. The pattern provider's put the locale's names in a fixed layout, such as `Mar 3`, `Tue 5`, and `3 PM` in `en-US`. Boundaries follow Gregorian months, so calendar patterns work in calendars that share them and only number years differently: `buddhist` (Thai solar Buddhist, the `th-TH` default), `japanese` (Japanese imperial eras), and `roc` (Republic of China, or Minguo). In `japanese`, the pattern provider's `y` is the year within the era, so add the era with `.with_year("G y")`. Preparing calendar patterns through either provider returns an error for calendars whose months start on other days, such as `hebrew` (Hebrew lunisolar) or `persian` (Persian Solar Hijri).
