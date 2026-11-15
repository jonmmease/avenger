# GoogleSQL datetime formatting

`avenger-format-datetime-googlesql` formats dates, times, datetimes, and instants with the format strings of GoogleSQL's `FORMAT_DATE`, `FORMAT_TIME`, `FORMAT_DATETIME`, and `FORMAT_TIMESTAMP`, and gives the text that GoogleSQL gives.

```rust
use avenger_format_datetime_googlesql::{Format, Kind};

let format = Format::new("%A, %B %e %Y, Q%Q", Kind::Date).unwrap();
let date = chrono::NaiveDate::from_ymd_opt(2024, 3, 5).unwrap();
assert_eq!(format.format_date(date), "Tuesday, March  5 2024, Q1");
```

GoogleSQL has one locale, en-US. The elements that a type of value lacks, such as the hour of a `DATE`, print as written, as in GoogleSQL. `%P` prints `am` or `pm`, as GoogleSQL's documentation says. An element outside GoogleSQL's documented list, or with a width, such as `%10d`, prints as written, as the C library of GoogleSQL's Linux build prints it.

The tests compare the output with GoogleSQL's `execute_query`, in `tests/data/googlesql_format.tsv`.
