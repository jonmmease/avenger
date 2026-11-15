# GoogleSQL datetime formatting

`avenger-format-datetime-googlesql` formats dates, times, datetimes, and instants with the format strings of GoogleSQL's `FORMAT_DATE`, `FORMAT_TIME`, `FORMAT_DATETIME`, and `FORMAT_TIMESTAMP`, and parses them as `PARSE_DATE`, `PARSE_TIME`, `PARSE_DATETIME`, and `PARSE_TIMESTAMP` do, with GoogleSQL's text, values, and error messages. `GoogleSqlDateTimeFormatProvider` prepares the formats for the labels of charts, with the syntax name `googlesql`.

```rust
use avenger_format_datetime_googlesql::{Format, Kind};

let format = Format::new("%A, %B %e %Y, Q%Q", Kind::Date).unwrap();
let date = chrono::NaiveDate::from_ymd_opt(2024, 3, 5).unwrap();
assert_eq!(format.format_date(date), "Tuesday, March  5 2024, Q1");
let date = avenger_format_datetime_googlesql::parse_date("%b %e %Y", "mar 5 2024").unwrap();
assert_eq!(date, chrono::NaiveDate::from_ymd_opt(2024, 3, 5).unwrap());
```

A parse takes a field that the format does not have from 1970-01-01 00:00:00. Whitespace in the format matches any amount of whitespace, numbers read up to their width without padding, and names match full or abbreviated, in any case. GoogleSQL rejects a weekday together with a week number, and an ISO week, as not supported yet, and so does the crate.

GoogleSQL has one locale, en-US. The elements that a type of value lacks, such as the hour of a `DATE`, print as written, as in GoogleSQL. `%P` prints `am` or `pm`, as GoogleSQL's documentation says. An element outside GoogleSQL's documented list, or with a width, such as `%10d`, prints as written, as the C library of GoogleSQL's Linux build prints it.

The tests compare the formats and the parses with GoogleSQL's `execute_query`, in `tests/data/googlesql_format.tsv` and `tests/data/googlesql_parse.tsv`.
