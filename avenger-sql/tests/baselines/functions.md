# Functions

## `COUNT`

```googlesql
COUNT(value: T) -> INT64 where T: ANY
```

Returns the number of rows, or of the rows whose value is not `NULL`.

`COUNT(*)` counts every row of the group, and `COUNT(value)` the rows whose value is not `NULL`. With `DISTINCT`, it counts the distinct values.

**Parameters**

+ `value`: The value whose rows to count.

**Clauses**: `DISTINCT`, `OVER`

**Examples**

```googlesql
FROM UNNEST([1, 2, 2, 3, NULL]) AS x |> AGGREGATE COUNT(*) AS row_count, COUNT(x) AS value_count, COUNT(DISTINCT x) AS distinct_count
```

| row_count | value_count | distinct_count |
| --- | --- | --- |
| 5 | 4 | 3 |


## `COUNTIF`

```googlesql
COUNTIF(condition: BOOL) -> INT64
```

Returns the number of rows whose condition is `TRUE`.

**Parameters**

+ `condition`: The condition of each row. `FALSE` and `NULL` don't count.

**Clauses**: `DISTINCT`, `OVER`

**Examples**

```googlesql
FROM UNNEST([1, 2, 2, 3, NULL]) AS x |> AGGREGATE COUNTIF(x > 1) AS n
```

| n |
| --- |
| 3 |


## `SUM`

```googlesql
SUM(value: T) -> T where T: INT64 | NUMERIC | BIGNUMERIC | FLOAT64
```

Returns the sum of the values that are not `NULL`.

Returns `NULL` when the group has no value that isn't `NULL`.

**Parameters**

+ `value`: The value of each row.

**Clauses**: `DISTINCT`, `OVER`

**Examples**

```googlesql
FROM UNNEST([1, 2, 2, 3, NULL]) AS x |> AGGREGATE SUM(x) AS total
```

| total |
| --- |
| 8 |


## `AVG`

```googlesql
AVG(value: INT64) -> FLOAT64
AVG(value: T) -> T where T: FLOAT64 | NUMERIC | BIGNUMERIC
```

Returns the mean of the values that are not `NULL`.

The mean of `INT64` values is a `FLOAT64`. Returns `NULL` when the group has no value that isn't `NULL`.

**Parameters**

+ `value`: The value of each row.

**Clauses**: `DISTINCT`, `OVER`

**Examples**

```googlesql
FROM UNNEST([1, 2, 2, 3, NULL]) AS x |> AGGREGATE AVG(x) AS mean
```

| mean |
| --- |
| 2.0 |


## `MIN`

```googlesql
MIN(value: T) -> T where T: ANY
```

Returns the least of the values that are not `NULL`.

**Parameters**

+ `value`: The value of each row.

**Clauses**: `DISTINCT`, `OVER`

**Examples**

```googlesql
FROM UNNEST([1, 2, 2, 3, NULL]) AS x |> AGGREGATE MIN(x) AS least
```

| least |
| --- |
| 1 |


## `MAX`

```googlesql
MAX(value: T) -> T where T: ANY
```

Returns the greatest of the values that are not `NULL`.

**Parameters**

+ `value`: The value of each row.

**Clauses**: `DISTINCT`, `OVER`

**Examples**

```googlesql
FROM UNNEST([1, 2, 2, 3, NULL]) AS x |> AGGREGATE MAX(x) AS greatest
```

| greatest |
| --- |
| 3 |


## `ANY_VALUE`

```googlesql
ANY_VALUE(value: T) -> T where T: ANY
```

Returns a value of the group, which can be any of them.

**Parameters**

+ `value`: The value of each row.

**Clauses**: `OVER`

**Examples**

```googlesql
FROM UNNEST([7, 7]) AS x |> AGGREGATE ANY_VALUE(x) AS one
```

| one |
| --- |
| 7 |


## `ARRAY_AGG`

```googlesql
ARRAY_AGG(value: T) -> ARRAY<T> where T: ANY
```

Returns an array of the values of the group.

`ORDER BY` orders the elements, and `IGNORE NULLS` leaves out the `NULL` values.

**Note:** A `NULL` value is an element of the array, where GoogleSQL fails unless the call ignores `NULL`s.

**Parameters**

+ `value`: The value of each row.

**Clauses**: `DISTINCT`, `IGNORE NULLS` and `RESPECT NULLS`, `ORDER BY`, `OVER`

**Examples**

```googlesql
FROM UNNEST([1, 2, 2, 3, NULL]) AS x |> AGGREGATE ARRAY_AGG(x IGNORE NULLS ORDER BY x DESC) AS values
```

| values |
| --- |
| [3, 2, 2, 1] |


## `LOGICAL_AND`

```googlesql
LOGICAL_AND(value: BOOL) -> BOOL
```

Returns whether every value that is not `NULL` is `TRUE`.

**Parameters**

+ `value`: The value of each row.

**Clauses**: `OVER`

**Examples**

```googlesql
FROM UNNEST([1, 2, 2, 3, NULL]) AS x |> AGGREGATE LOGICAL_AND(x > 0) AS all_positive
```

| all_positive |
| --- |
| TRUE |


## `LOGICAL_OR`

```googlesql
LOGICAL_OR(value: BOOL) -> BOOL
```

Returns whether any value is `TRUE`.

**Parameters**

+ `value`: The value of each row.

**Clauses**: `OVER`

**Examples**

```googlesql
FROM UNNEST([1, 2, 2, 3, NULL]) AS x |> AGGREGATE LOGICAL_OR(x > 2) AS any_above_2
```

| any_above_2 |
| --- |
| TRUE |


## `STRING_AGG`

```googlesql
STRING_AGG(value: STRING[, delimiter: CONST STRING]) -> STRING
```

Joins the strings of the group into one, with a delimiter between them.

`ORDER BY` orders the strings. Returns `NULL` when the group has no string that isn't `NULL`.

**Parameters**

+ `value`: The string of each row. A `NULL` value is left out.
+ `delimiter`: The string between two values, a constant. When omitted, `','`.

**Clauses**: `DISTINCT`, `ORDER BY`, `OVER`

**Examples**

```googlesql
FROM UNNEST(['b', 'a', 'c']) AS s |> AGGREGATE STRING_AGG(s, '-' ORDER BY s) AS joined
```

| joined |
| --- |
| 'a-b-c' |


## `ROW_NUMBER`

```googlesql
ROW_NUMBER() -> INT64
OVER ([PARTITION BY expression[, ...]] [ORDER BY expression[, ...]])
```

Returns the number of each row in its window, from 1.

Rows that tie in the window's order take distinct numbers, in an order that isn't fixed.

**Examples**

```googlesql
FROM UNNEST([1, 2, 2, 3]) AS x |> EXTEND ROW_NUMBER() OVER (ORDER BY x) AS n
```

| x | n |
| --- | --- |
| 1 | 1 |
| 2 | 2 |
| 2 | 3 |
| 3 | 4 |


## `RANK`

```googlesql
RANK() -> INT64
OVER ([PARTITION BY expression[, ...]] ORDER BY expression[, ...])
```

Returns the rank of each row in its window, from 1, with gaps after ties.

Rows that tie take one rank, and the next rank skips the tied rows.

**Examples**

```googlesql
FROM UNNEST([1, 2, 2, 3]) AS x |> EXTEND RANK() OVER (ORDER BY x) AS rank
```

| x | rank |
| --- | --- |
| 1 | 1 |
| 2 | 2 |
| 2 | 2 |
| 3 | 4 |


## `DENSE_RANK`

```googlesql
DENSE_RANK() -> INT64
OVER ([PARTITION BY expression[, ...]] ORDER BY expression[, ...])
```

Returns the rank of each row in its window, from 1, without gaps after ties.

**Examples**

```googlesql
FROM UNNEST([1, 2, 2, 3]) AS x |> EXTEND DENSE_RANK() OVER (ORDER BY x) AS rank
```

| x | rank |
| --- | --- |
| 1 | 1 |
| 2 | 2 |
| 2 | 2 |
| 3 | 3 |


## `PERCENT_RANK`

```googlesql
PERCENT_RANK() -> FLOAT64
OVER ([PARTITION BY expression[, ...]] ORDER BY expression[, ...])
```

Returns the rank of each row as a fraction of its window, from 0 to 1.

The fraction is `(rank - 1) / (rows - 1)`, and 0 for a window of one row.

**Examples**

```googlesql
FROM UNNEST([1, 2, 2, 3]) AS x |> EXTEND PERCENT_RANK() OVER (ORDER BY x) AS fraction
```

| x | fraction |
| --- | --- |
| 1 | 0.0 |
| 2 | 0.3333333333333333 |
| 2 | 0.3333333333333333 |
| 3 | 1.0 |


## `CUME_DIST`

```googlesql
CUME_DIST() -> FLOAT64
OVER ([PARTITION BY expression[, ...]] ORDER BY expression[, ...])
```

Returns the fraction of the rows of its window that come before each row or tie with it.

**Examples**

```googlesql
FROM UNNEST([1, 2, 2, 3]) AS x |> EXTEND CUME_DIST() OVER (ORDER BY x) AS fraction
```

| x | fraction |
| --- | --- |
| 1 | 0.25 |
| 2 | 0.75 |
| 2 | 0.75 |
| 3 | 1.0 |


## `NTILE`

```googlesql
NTILE(buckets: INT64) -> INT64
OVER ([PARTITION BY expression[, ...]] ORDER BY expression[, ...])
```

Divides the rows of each window into buckets, and returns the bucket of each row, from 1.

The buckets differ in size by at most one row, and the first buckets take the extra rows.

**Parameters**

+ `buckets`: The number of buckets, which is positive.

**Examples**

```googlesql
FROM UNNEST([1, 2, 2, 3]) AS x |> EXTEND NTILE(2) OVER (ORDER BY x) AS bucket
```

| x | bucket |
| --- | --- |
| 1 | 1 |
| 2 | 1 |
| 2 | 2 |
| 3 | 2 |


## `LAG`

```googlesql
LAG(value: T[, offset: INT64][, default_value: T]) -> T where T: ANY
OVER ([PARTITION BY expression[, ...]] ORDER BY expression[, ...])
```

Returns the value of a row before each row in its window.

**Parameters**

+ `value`: The value of the row.
+ `offset`: How many rows before the row to read, which is not negative. When omitted, 1.
+ `default_value`: The result when no row is that far from the row. When omitted, `NULL`.

**Examples**

```googlesql
FROM UNNEST([1, 2, 3, 4]) AS x |> EXTEND LAG(x) OVER (ORDER BY x) AS previous
```

| x | previous |
| --- | --- |
| 1 | NULL |
| 2 | 1 |
| 3 | 2 |
| 4 | 3 |

```googlesql
FROM UNNEST([1, 2, 3, 4]) AS x |> EXTEND LAG(x, 2, 0) OVER (ORDER BY x) AS two_before
```

| x | two_before |
| --- | --- |
| 1 | 0 |
| 2 | 0 |
| 3 | 1 |
| 4 | 2 |


## `LEAD`

```googlesql
LEAD(value: T[, offset: INT64][, default_value: T]) -> T where T: ANY
OVER ([PARTITION BY expression[, ...]] ORDER BY expression[, ...])
```

Returns the value of a row after each row in its window.

**Parameters**

+ `value`: The value of the row.
+ `offset`: How many rows after the row to read, which is not negative. When omitted, 1.
+ `default_value`: The result when no row is that far from the row. When omitted, `NULL`.

**Examples**

```googlesql
FROM UNNEST([1, 2, 3, 4]) AS x |> EXTEND LEAD(x, 1, 0) OVER (ORDER BY x) AS next
```

| x | next |
| --- | --- |
| 1 | 2 |
| 2 | 3 |
| 3 | 4 |
| 4 | 0 |


## `FIRST_VALUE`

```googlesql
FIRST_VALUE(value: T) -> T where T: ANY
OVER ([PARTITION BY expression[, ...]] ORDER BY expression[, ...] [frame])
```

Returns the value of the first row of each row's window frame.

`IGNORE NULLS` returns the first value that isn't `NULL`.

**Parameters**

+ `value`: The value of each row.

**Clauses**: `IGNORE NULLS` and `RESPECT NULLS`

**Examples**

```googlesql
FROM UNNEST([1, 2, 3, 4]) AS x |> EXTEND FIRST_VALUE(x) OVER (ORDER BY x) AS first
```

| x | first |
| --- | --- |
| 1 | 1 |
| 2 | 1 |
| 3 | 1 |
| 4 | 1 |


## `LAST_VALUE`

```googlesql
LAST_VALUE(value: T) -> T where T: ANY
OVER ([PARTITION BY expression[, ...]] ORDER BY expression[, ...] [frame])
```

Returns the value of the last row of each row's window frame.

With an `ORDER BY` and no frame, the frame ends at the row and its ties, so the last value is the row's own. `IGNORE NULLS` returns the last value that isn't `NULL`.

**Parameters**

+ `value`: The value of each row.

**Clauses**: `IGNORE NULLS` and `RESPECT NULLS`

**Examples**

```googlesql
FROM UNNEST([1, 2, 3, 4]) AS x |> EXTEND LAST_VALUE(x) OVER (ORDER BY x) AS last
```

| x | last |
| --- | --- |
| 1 | 1 |
| 2 | 2 |
| 3 | 3 |
| 4 | 4 |


## `ARRAY_LENGTH`

```googlesql
ARRAY_LENGTH(value: ARRAY<T>) -> INT64 where T: ANY
```

Returns the number of elements of an array.

Returns `NULL` for a `NULL` array.

**Parameters**

+ `value`: The array.

A call can write `SAFE.` before the name.

**Examples**

```googlesql
ARRAY_LENGTH([1, 2, 3])
```

Result: `3`


## `ARRAY_CONCAT`

```googlesql
ARRAY_CONCAT(arrays: ARRAY<T>, ...) -> ARRAY<T> where T: ANY
```

Joins arrays into one.

**Parameters**

+ `arrays`: The arrays, of one element type, in order.

A call can write `SAFE.` before the name.

**Examples**

```googlesql
ARRAY_CONCAT([1, 2], [3])
```

Result: `[1, 2, 3]`


## `ARRAY_REVERSE`

```googlesql
ARRAY_REVERSE(value: ARRAY<T>) -> ARRAY<T> where T: ANY
```

Returns the elements of an array in reverse order.

**Parameters**

+ `value`: The array.

A call can write `SAFE.` before the name.

**Examples**

```googlesql
ARRAY_REVERSE([1, 2, 3])
```

Result: `[3, 2, 1]`


## `GENERATE_ARRAY`

```googlesql
GENERATE_ARRAY(start_value: INT64, end_value: INT64[, step: INT64]) -> ARRAY<INT64>
```

Returns an array of the numbers from a start to an end in steps.

The array holds the end when a whole number of steps reaches it, and is empty when the steps lead away from the end.

**Parameters**

+ `start_value`: The first number.
+ `end_value`: The last number that the array can hold.
+ `step`: The difference between two numbers, which is not 0. When omitted, 1.

A call can write `SAFE.` before the name.

**Examples**

```googlesql
GENERATE_ARRAY(1, 5)
```

Result: `[1, 2, 3, 4, 5]`

```googlesql
GENERATE_ARRAY(1, 10, 3)
```

Result: `[1, 4, 7, 10]`

```googlesql
GENERATE_ARRAY(5, 1)
```

Result: `[]`


## `IF`

```googlesql
IF(condition: BOOL, true_result: T, else_result: T) -> T where T: ANY
```

Returns one of two values by a condition.

The two results take one type, their common supertype.

**Parameters**

+ `condition`: The condition. `NULL` counts as `FALSE`.
+ `true_result`: The result when the condition is `TRUE`.
+ `else_result`: The result when the condition is `FALSE` or `NULL`.

A call can write `SAFE.` before the name.

**Examples**

```googlesql
IF(1 < 2, 'yes', 'no')
```

Result: `'yes'`

```googlesql
IF(NULL, 1, 2)
```

Result: `2`


## `IFNULL`

```googlesql
IFNULL(value: T, null_result: T) -> T where T: ANY
```

Returns a value, or another value when it is `NULL`.

**Parameters**

+ `value`: The value.
+ `null_result`: The result when the value is `NULL`.

A call can write `SAFE.` before the name.

**Examples**

```googlesql
IFNULL(NULL, 0)
```

Result: `0`

```googlesql
IFNULL(5, 0)
```

Result: `5`


## `COALESCE`

```googlesql
COALESCE(values: T, ...) -> T where T: ANY
```

Returns the first of its values that is not `NULL`.

Returns `NULL` when every value is `NULL`.

**Parameters**

+ `values`: The values, of one common supertype.

A call can write `SAFE.` before the name.

**Examples**

```googlesql
COALESCE(NULL, 'b', 'c')
```

Result: `'b'`


## `NULLIF`

```googlesql
NULLIF(value: T, value_to_match: T) -> T where T: ANY
```

Returns `NULL` when two values are equal, or the first value.

**Parameters**

+ `value`: The value.
+ `value_to_match`: The value whose equality makes the result `NULL`.

A call can write `SAFE.` before the name.

**Examples**

```googlesql
NULLIF(0, 0)
```

Result: `NULL`

```googlesql
NULLIF(5, 0)
```

Result: `5`


## `CURRENT_DATE`

```googlesql
CURRENT_DATE([time_zone: CONST STRING]) -> DATE
```

Returns the current date.

**Parameters**

+ `time_zone`: The time zone, a string literal such as `'America/Los_Angeles'`. When omitted, UTC, the session's default.

A call can write `SAFE.` before the name.

## `CURRENT_DATETIME`

```googlesql
CURRENT_DATETIME([time_zone: CONST STRING]) -> DATETIME
```

Returns the current date and time, in civil time.

**Parameters**

+ `time_zone`: The time zone, a string literal such as `'America/Los_Angeles'`. When omitted, UTC, the session's default.

A call can write `SAFE.` before the name.

## `CURRENT_TIMESTAMP`

```googlesql
CURRENT_TIMESTAMP() -> TIMESTAMP
```

Returns the current instant.

A call can write `SAFE.` before the name.

## `DATE`

```googlesql
DATE(year: INT64, month: INT64, day: INT64) -> DATE
DATE(value: TIMESTAMP[, time_zone: CONST STRING]) -> DATE
DATE(value: DATETIME) -> DATE
DATE(value: DATE) -> DATE
DATE(value: STRING) -> DATE
```

Returns a date from its parts, or the date of a value.

**Parameters**

+ `year`: The year.
+ `month`: The month, from 1 to 12.
+ `day`: The day of the month.
+ `value`: A `TIMESTAMP`, whose date is its date in the time zone, a `DATETIME`, a `DATE`, or a `STRING` such as `'2024-05-17'`.
+ `time_zone`: The time zone, a string literal such as `'America/Los_Angeles'`. When omitted, UTC, the session's default.

**Examples**

```googlesql
DATE(2024, 5, 17)
```

Result: `DATE '2024-05-17'`

```googlesql
DATE(TIMESTAMP '2024-05-18 03:00:00+00', 'America/Los_Angeles')
```

Result: `DATE '2024-05-17'`

```googlesql
DATE(DATETIME '2024-05-17 23:00:00')
```

Result: `DATE '2024-05-17'`


## `DATETIME`

```googlesql
DATETIME(year: INT64, month: INT64, day: INT64, hour: INT64, minute: INT64, second: INT64) -> DATETIME
DATETIME(value: DATE[, time: TIME]) -> DATETIME
DATETIME(value: TIMESTAMP[, time_zone: CONST STRING]) -> DATETIME
DATETIME(value: DATETIME) -> DATETIME
DATETIME(value: STRING) -> DATETIME
```

Returns a date and time from its parts, or the date and time of a value.

**Parameters**

+ `year`: The year.
+ `month`: The month, from 1 to 12.
+ `day`: The day of the month.
+ `hour`: The hour, from 0 to 23.
+ `minute`: The minute, from 0 to 59.
+ `second`: The second, from 0 to 59.
+ `value`: A `DATE`, which starts at midnight, a `TIMESTAMP`, whose civil time is its time in the time zone, a `DATETIME`, or a `STRING` such as `'2024-05-17 13:45:10'`.
+ `time`: The time of day of the date. When omitted, midnight.
+ `time_zone`: The time zone, a string literal such as `'America/Los_Angeles'`. When omitted, UTC, the session's default.

**Examples**

```googlesql
DATETIME(2024, 5, 17, 13, 45, 10)
```

Result: `DATETIME '2024-05-17 13:45:10'`

```googlesql
DATETIME(DATE '2024-05-17')
```

Result: `DATETIME '2024-05-17 00:00:00'`


## `TIMESTAMP`

```googlesql
TIMESTAMP(value: STRING[, time_zone: CONST STRING]) -> TIMESTAMP
TIMESTAMP(value: DATE[, time_zone: CONST STRING]) -> TIMESTAMP
TIMESTAMP(value: DATETIME[, time_zone: CONST STRING]) -> TIMESTAMP
TIMESTAMP(value: TIMESTAMP) -> TIMESTAMP
```

Returns the instant of a value.

**Parameters**

+ `value`: A `STRING` such as `'2024-05-17 13:45:10'`, a `DATE`, which starts at midnight, a `DATETIME`, each in the time zone, or a `TIMESTAMP`.
+ `time_zone`: The time zone, a string literal such as `'America/Los_Angeles'`. When omitted, UTC, the session's default.

**Examples**

```googlesql
TIMESTAMP('2024-05-17 13:45:10')
```

Result: `TIMESTAMP '2024-05-17 13:45:10+00'`

```googlesql
TIMESTAMP(DATE '2024-05-17', 'America/Los_Angeles')
```

Result: `TIMESTAMP '2024-05-17 07:00:00+00'`


## `DATE_TRUNC`

```googlesql
DATE_TRUNC(value: DATE, part: DATE PART) -> DATE
DATE_TRUNC(value: DATETIME, part: DATE OR TIME PART) -> DATETIME
DATE_TRUNC(value: TIMESTAMP, part: DATE OR TIME PART[, time_zone: CONST STRING]) -> TIMESTAMP
```

Truncates a date, or a date and time, to the start of a part.

**Parameters**

+ `value`: The `DATE`, `DATETIME` or `TIMESTAMP`.
+ `part`: The part: `DAY`, `WEEK`, which starts on Sunday, `WEEK(day)`, which starts on the day, `ISOWEEK`, `MONTH`, `QUARTER`, `YEAR` or `ISOYEAR`, and for a `DATETIME` or a `TIMESTAMP`, a part of the time, from `MICROSECOND` to `HOUR`.
+ `time_zone`: The time zone, a string literal such as `'America/Los_Angeles'`. When omitted, UTC, the session's default.

A call can write `SAFE.` before the name.

**Examples**

```googlesql
DATE_TRUNC(DATE '2024-05-17', MONTH)
```

Result: `DATE '2024-05-01'`

```googlesql
DATE_TRUNC(DATE '2024-05-17', WEEK)
```

Result: `DATE '2024-05-12'`

```googlesql
DATE_TRUNC(DATE '2024-05-17', WEEK(MONDAY))
```

Result: `DATE '2024-05-13'`


## `DATETIME_TRUNC`

```googlesql
DATETIME_TRUNC(value: DATETIME, part: DATE OR TIME PART) -> DATETIME
DATETIME_TRUNC(value: TIMESTAMP, part: DATE OR TIME PART[, time_zone: CONST STRING]) -> TIMESTAMP
```

Truncates a date and time to the start of a part.

**Parameters**

+ `value`: The `DATETIME`, or a `TIMESTAMP`, which truncates in the time zone.
+ `part`: The part, from `MICROSECOND` to `ISOYEAR`, as for `DATE_TRUNC`.
+ `time_zone`: The time zone, a string literal such as `'America/Los_Angeles'`. When omitted, UTC, the session's default.

A call can write `SAFE.` before the name.

**Examples**

```googlesql
DATETIME_TRUNC(DATETIME '2024-05-17 13:45:10', HOUR)
```

Result: `DATETIME '2024-05-17 13:00:00'`


## `TIMESTAMP_TRUNC`

```googlesql
TIMESTAMP_TRUNC(value: TIMESTAMP, part: DATE OR TIME PART[, time_zone: CONST STRING]) -> TIMESTAMP
TIMESTAMP_TRUNC(value: DATETIME, part: DATE OR TIME PART) -> DATETIME
```

Truncates an instant to the start of a part, in a time zone.

**Parameters**

+ `value`: The `TIMESTAMP`, or a `DATETIME`.
+ `part`: The part, from `MICROSECOND` to `ISOYEAR`, as for `DATE_TRUNC`.
+ `time_zone`: The time zone, a string literal such as `'America/Los_Angeles'`. When omitted, UTC, the session's default.

A call can write `SAFE.` before the name.

**Examples**

```googlesql
TIMESTAMP_TRUNC(TIMESTAMP '2024-05-17 13:45:10+00', HOUR)
```

Result: `TIMESTAMP '2024-05-17 13:00:00+00'`

```googlesql
TIMESTAMP_TRUNC(TIMESTAMP '2024-05-18 03:00:00+00', DAY, 'America/Los_Angeles')
```

Result: `TIMESTAMP '2024-05-17 07:00:00+00'`


## `DATE_ADD`

```googlesql
DATE_ADD(value: DATE, interval_value: INTERVAL) -> DATE
DATE_ADD(value: DATETIME, interval_value: INTERVAL) -> DATETIME
DATE_ADD(value: TIMESTAMP, interval_value: INTERVAL) -> TIMESTAMP
```

Adds an interval to a date.

Months and years keep the day of the month, or take the last day of a shorter month.

**Parameters**

+ `value`: The value.
+ `interval_value`: The interval, as in `INTERVAL 1 MONTH`.

A call can write `SAFE.` before the name.

**Examples**

```googlesql
DATE_ADD(DATE '2024-01-31', INTERVAL 1 MONTH)
```

Result: `DATE '2024-02-29'`


## `DATE_SUB`

```googlesql
DATE_SUB(value: DATE, interval_value: INTERVAL) -> DATE
DATE_SUB(value: DATETIME, interval_value: INTERVAL) -> DATETIME
DATE_SUB(value: TIMESTAMP, interval_value: INTERVAL) -> TIMESTAMP
```

Subtracts an interval from a date.

Months and years keep the day of the month, or take the last day of a shorter month.

**Parameters**

+ `value`: The value.
+ `interval_value`: The interval, as in `INTERVAL 1 MONTH`.

A call can write `SAFE.` before the name.

**Examples**

```googlesql
DATE_SUB(DATE '2024-03-01', INTERVAL 1 DAY)
```

Result: `DATE '2024-02-29'`


## `DATETIME_ADD`

```googlesql
DATETIME_ADD(value: DATETIME, interval_value: INTERVAL) -> DATETIME
DATETIME_ADD(value: TIMESTAMP, interval_value: INTERVAL) -> TIMESTAMP
```

Adds an interval to a date and time.

Months and years keep the day of the month, or take the last day of a shorter month.

**Parameters**

+ `value`: The value.
+ `interval_value`: The interval, as in `INTERVAL 1 MONTH`.

A call can write `SAFE.` before the name.

**Examples**

```googlesql
DATETIME_ADD(DATETIME '2024-05-17 13:45:10', INTERVAL 30 MINUTE)
```

Result: `DATETIME '2024-05-17 14:15:10'`


## `DATETIME_SUB`

```googlesql
DATETIME_SUB(value: DATETIME, interval_value: INTERVAL) -> DATETIME
DATETIME_SUB(value: TIMESTAMP, interval_value: INTERVAL) -> TIMESTAMP
```

Subtracts an interval from a date and time.

Months and years keep the day of the month, or take the last day of a shorter month.

**Parameters**

+ `value`: The value.
+ `interval_value`: The interval, as in `INTERVAL 1 MONTH`.

A call can write `SAFE.` before the name.

**Examples**

```googlesql
DATETIME_SUB(DATETIME '2024-05-17 13:45:10', INTERVAL 2 HOUR)
```

Result: `DATETIME '2024-05-17 11:45:10'`


## `TIMESTAMP_ADD`

```googlesql
TIMESTAMP_ADD(value: TIMESTAMP, interval_value: INTERVAL) -> TIMESTAMP
TIMESTAMP_ADD(value: DATETIME, interval_value: INTERVAL) -> DATETIME
```

Adds an interval to an instant.

Months and years keep the day of the month, or take the last day of a shorter month.

**Parameters**

+ `value`: The value.
+ `interval_value`: The interval, as in `INTERVAL 1 MONTH`.

A call can write `SAFE.` before the name.

**Examples**

```googlesql
TIMESTAMP_ADD(TIMESTAMP '2024-05-17 13:45:10+00', INTERVAL 30 MINUTE)
```

Result: `TIMESTAMP '2024-05-17 14:15:10+00'`


## `TIMESTAMP_SUB`

```googlesql
TIMESTAMP_SUB(value: TIMESTAMP, interval_value: INTERVAL) -> TIMESTAMP
TIMESTAMP_SUB(value: DATETIME, interval_value: INTERVAL) -> DATETIME
```

Subtracts an interval from an instant.

Months and years keep the day of the month, or take the last day of a shorter month.

**Parameters**

+ `value`: The value.
+ `interval_value`: The interval, as in `INTERVAL 1 MONTH`.

A call can write `SAFE.` before the name.

**Examples**

```googlesql
TIMESTAMP_SUB(TIMESTAMP '2024-05-17 13:45:10+00', INTERVAL 1 DAY)
```

Result: `TIMESTAMP '2024-05-16 13:45:10+00'`


## `DATE_DIFF`

```googlesql
DATE_DIFF(end_date: T, start_date: T, part: DATE PART) -> INT64 where T: DATE | DATETIME | TIMESTAMP
```

Returns the number of boundaries of a part between two dates.

Counts the starts of the part after the start date, up to and with the end date, so `DATE_DIFF` of January 1 and the December 31 before it in `YEAR` is 1. The result is negative when the end date comes first.

**Parameters**

+ `end_date`: The later date, a `DATE`, a `DATETIME` or a `TIMESTAMP`.
+ `start_date`: The earlier date, of the type of `end_date`.
+ `part`: The part: `DAY`, `WEEK`, `WEEK(day)`, `ISOWEEK`, `MONTH`, `QUARTER`, `YEAR` or `ISOYEAR`.

A call can write `SAFE.` before the name.

**Examples**

```googlesql
DATE_DIFF(DATE '2024-03-01', DATE '2024-01-01', DAY)
```

Result: `60`

```googlesql
DATE_DIFF(DATE '2024-03-01', DATE '2024-01-31', MONTH)
```

Result: `2`

```googlesql
DATE_DIFF(DATE '2024-01-01', DATE '2023-12-31', YEAR)
```

Result: `1`


## `TIMESTAMP_DIFF`

```googlesql
TIMESTAMP_DIFF(end_timestamp: T, start_timestamp: T, part: TIME PART OR DAY) -> INT64 where T: TIMESTAMP | DATETIME
```

Returns the number of whole parts of the time between two instants.

The result truncates toward zero, and is negative when the end comes first.

**Parameters**

+ `end_timestamp`: The later instant, a `TIMESTAMP` or a `DATETIME`.
+ `start_timestamp`: The earlier instant, of the type of `end_timestamp`.
+ `part`: The part, of a fixed length: `MICROSECOND`, `MILLISECOND`, `SECOND`, `MINUTE`, `HOUR` or `DAY`, which is 24 hours.

A call can write `SAFE.` before the name.

**Examples**

```googlesql
TIMESTAMP_DIFF(TIMESTAMP '2024-05-17 13:45:10+00', TIMESTAMP '2024-05-17 12:00:00+00', MINUTE)
```

Result: `105`

```googlesql
TIMESTAMP_DIFF(TIMESTAMP '2024-05-17 13:45:10+00', TIMESTAMP '2024-05-17 12:00:00+00', HOUR)
```

Result: `1`


## `FORMAT_DATE`

```googlesql
FORMAT_DATE(format: STRING, value: DATE) -> STRING
FORMAT_DATE(format: STRING, value: DATETIME) -> STRING
FORMAT_DATE(format: STRING, value: TIMESTAMP[, time_zone: CONST STRING]) -> STRING
```

Formats a date as a string.

**Note:** The format follows `strftime`, as DataFusion's `to_char` does, so GoogleSQL's own elements, such as `%Q`, are not elements of it.

**Parameters**

+ `format`: The format, whose elements, such as `%Y` and `%m`, are those of `strftime`.
+ `value`: The `DATE`, or a `DATETIME` or a `TIMESTAMP`, whose date it formats.
+ `time_zone`: The time zone, a string literal such as `'America/Los_Angeles'`. When omitted, UTC, the session's default.

**Examples**

```googlesql
FORMAT_DATE('%Y/%m/%d', DATE '2024-05-07')
```

Result: `'2024/05/07'`

```googlesql
FORMAT_DATE('%b %e', DATE '2024-05-07')
```

Result: `'May  7'`


## `FORMAT_DATETIME`

```googlesql
FORMAT_DATETIME(format: STRING, value: DATETIME) -> STRING
FORMAT_DATETIME(format: STRING, value: TIMESTAMP[, time_zone: CONST STRING]) -> STRING
```

Formats a date and time as a string.

**Parameters**

+ `format`: The format, whose elements, such as `%Y` and `%m`, are those of `strftime`.
+ `value`: The `DATETIME`, or a `TIMESTAMP` in the time zone.
+ `time_zone`: The time zone, a string literal such as `'America/Los_Angeles'`. When omitted, UTC, the session's default.

**Examples**

```googlesql
FORMAT_DATETIME('%Y-%m-%d %H:%M', DATETIME '2024-05-17 13:45:10')
```

Result: `'2024-05-17 13:45'`


## `FORMAT_TIMESTAMP`

```googlesql
FORMAT_TIMESTAMP(format: STRING, value: TIMESTAMP[, time_zone: CONST STRING]) -> STRING
FORMAT_TIMESTAMP(format: STRING, value: DATETIME) -> STRING
```

Formats an instant as a string, in a time zone.

**Parameters**

+ `format`: The format, whose elements, such as `%Y` and `%m`, are those of `strftime`.
+ `value`: The `TIMESTAMP`, or a `DATETIME`.
+ `time_zone`: The time zone, a string literal such as `'America/Los_Angeles'`. When omitted, UTC, the session's default.

**Examples**

```googlesql
FORMAT_TIMESTAMP('%Y-%m-%d %H:%M:%S', TIMESTAMP '2024-05-17 13:45:10+00')
```

Result: `'2024-05-17 13:45:10'`

```googlesql
FORMAT_TIMESTAMP('%H:%M', TIMESTAMP '2024-05-17 13:45:10+00', 'America/Los_Angeles')
```

Result: `'06:45'`


## `PARSE_DATE`

```googlesql
PARSE_DATE(format: STRING, value: STRING) -> DATE
```

Parses a date from a string in a format.

The string matches the whole format.

**Parameters**

+ `format`: The format, whose elements, such as `%Y` and `%m`, are those of `strftime`.
+ `value`: The string.

**Examples**

```googlesql
PARSE_DATE('%Y%m%d', '20240517')
```

Result: `DATE '2024-05-17'`


## `PARSE_DATETIME`

```googlesql
PARSE_DATETIME(format: STRING, value: STRING) -> DATETIME
```

Parses a date and time from a string in a format.

**Parameters**

+ `format`: The format, whose elements, such as `%Y` and `%m`, are those of `strftime`.
+ `value`: The string.

**Examples**

```googlesql
PARSE_DATETIME('%Y-%m-%d %H:%M', '2024-05-17 13:45')
```

Result: `DATETIME '2024-05-17 13:45:00'`


## `PARSE_TIMESTAMP`

```googlesql
PARSE_TIMESTAMP(format: STRING, value: STRING[, time_zone: CONST STRING]) -> TIMESTAMP
```

Parses an instant from a string in a format and a time zone.

**Parameters**

+ `format`: The format, whose elements, such as `%Y` and `%m`, are those of `strftime`.
+ `value`: The string. An offset in it, such as `+02:00`, takes the place of the time zone.
+ `time_zone`: The time zone, a string literal such as `'America/Los_Angeles'`. When omitted, UTC, the session's default.

**Examples**

```googlesql
PARSE_TIMESTAMP('%Y-%m-%d %H:%M:%S', '2024-05-17 13:45:10')
```

Result: `TIMESTAMP '2024-05-17 13:45:10+00'`

```googlesql
PARSE_TIMESTAMP('%Y-%m-%d %H:%M:%S', '2024-05-17 13:45:10', 'America/Los_Angeles')
```

Result: `TIMESTAMP '2024-05-17 20:45:10+00'`


## `UNIX_SECONDS`

```googlesql
UNIX_SECONDS(value: TIMESTAMP) -> INT64
```

Returns the number of seconds since 1970-01-01 00:00:00 UTC of an instant.

Truncates toward zero.

**Parameters**

+ `value`: The instant.

A call can write `SAFE.` before the name.

**Examples**

```googlesql
UNIX_SECONDS(TIMESTAMP '2024-01-01 00:00:00+00')
```

Result: `1704067200`


## `UNIX_MILLIS`

```googlesql
UNIX_MILLIS(value: TIMESTAMP) -> INT64
```

Returns the number of milliseconds since 1970-01-01 00:00:00 UTC of an instant.

Truncates toward zero.

**Parameters**

+ `value`: The instant.

A call can write `SAFE.` before the name.

**Examples**

```googlesql
UNIX_MILLIS(TIMESTAMP '2024-01-01 00:00:00+00')
```

Result: `1704067200000`


## `UNIX_MICROS`

```googlesql
UNIX_MICROS(value: TIMESTAMP) -> INT64
```

Returns the number of microseconds since 1970-01-01 00:00:00 UTC of an instant.

Truncates toward zero.

**Parameters**

+ `value`: The instant.

A call can write `SAFE.` before the name.

**Examples**

```googlesql
UNIX_MICROS(TIMESTAMP '2024-01-01 00:00:00+00')
```

Result: `1704067200000000`


## `TIMESTAMP_SECONDS`

```googlesql
TIMESTAMP_SECONDS(value: INT64) -> TIMESTAMP
```

Returns the instant of a number of seconds since 1970-01-01 00:00:00 UTC.

**Parameters**

+ `value`: The number.

A call can write `SAFE.` before the name.

**Examples**

```googlesql
TIMESTAMP_SECONDS(1704067200)
```

Result: `TIMESTAMP '2024-01-01 00:00:00+00'`


## `TIMESTAMP_MILLIS`

```googlesql
TIMESTAMP_MILLIS(value: INT64) -> TIMESTAMP
```

Returns the instant of a number of milliseconds since 1970-01-01 00:00:00 UTC.

**Parameters**

+ `value`: The number.

A call can write `SAFE.` before the name.

**Examples**

```googlesql
TIMESTAMP_MILLIS(1704067200000)
```

Result: `TIMESTAMP '2024-01-01 00:00:00+00'`


## `TIMESTAMP_MICROS`

```googlesql
TIMESTAMP_MICROS(value: INT64) -> TIMESTAMP
```

Returns the instant of a number of microseconds since 1970-01-01 00:00:00 UTC.

**Parameters**

+ `value`: The number.

A call can write `SAFE.` before the name.

**Examples**

```googlesql
TIMESTAMP_MICROS(1704067200000000)
```

Result: `TIMESTAMP '2024-01-01 00:00:00+00'`


## `ABS`

```googlesql
ABS(x: T) -> T where T: INT64 | NUMERIC | BIGNUMERIC | FLOAT64
```

Returns the absolute value of a number.

**Parameters**

+ `x`: The number.

A call can write `SAFE.` before the name.

**Examples**

```googlesql
ABS(-3)
```

Result: `3`

```googlesql
ABS(-2.5)
```

Result: `2.5`


## `SIGN`

```googlesql
SIGN(x: T) -> T where T: INT64 | NUMERIC | BIGNUMERIC | FLOAT64
```

Returns the sign of a number: -1, 0, or 1.

The result has the type of the number, so the sign of a `FLOAT64` is `-1.0`, `0.0` or `1.0`.

**Parameters**

+ `x`: The number.

A call can write `SAFE.` before the name.

**Examples**

```googlesql
SIGN(-3)
```

Result: `-1`

```googlesql
SIGN(2.5)
```

Result: `1.0`


## `ROUND`

```googlesql
ROUND(x: T[, digits: INT64]) -> T where T: FLOAT64 | NUMERIC | BIGNUMERIC
```

Rounds a number to a number of decimal places.

Rounds halfway values away from zero.

**Parameters**

+ `x`: The number. An `INT64` rounds as a `FLOAT64`.
+ `digits`: The number of decimal places to keep. A negative number rounds to the left of the decimal point. When omitted, 0.

A call can write `SAFE.` before the name.

**Examples**

```googlesql
ROUND(2.5)
```

Result: `3.0`

```googlesql
ROUND(-2.5)
```

Result: `-3.0`

```googlesql
ROUND(1.2345, 2)
```

Result: `1.23`


## `TRUNC`

```googlesql
TRUNC(x: T[, digits: INT64]) -> T where T: FLOAT64 | NUMERIC | BIGNUMERIC
```

Truncates a number toward zero, to a number of decimal places.

Drops the digits after the kept places, so a negative number truncates toward zero.

**Parameters**

+ `x`: The number. An `INT64` rounds as a `FLOAT64`.
+ `digits`: The number of decimal places to keep. A negative number rounds to the left of the decimal point. When omitted, 0.

A call can write `SAFE.` before the name.

**Examples**

```googlesql
TRUNC(-2.7)
```

Result: `-2.0`

```googlesql
TRUNC(1.2345, 2)
```

Result: `1.23`


## `FLOOR`

```googlesql
FLOOR(x: T) -> T where T: FLOAT64 | NUMERIC | BIGNUMERIC
```

Returns the greatest integral value that is not greater than a number.

**Parameters**

+ `x`: The number.

A call can write `SAFE.` before the name.

**Examples**

```googlesql
FLOOR(1.7)
```

Result: `1.0`

```googlesql
FLOOR(-1.2)
```

Result: `-2.0`


## `CEIL`

```googlesql
CEIL(x: T) -> T where T: FLOAT64 | NUMERIC | BIGNUMERIC
```

Returns the least integral value that is not less than a number.

**Parameters**

+ `x`: The number.

A call can write `SAFE.` before the name.

**Examples**

```googlesql
CEIL(1.2)
```

Result: `2.0`

```googlesql
CEIL(-1.7)
```

Result: `-1.0`


## `CEILING`

```googlesql
CEILING(x: T) -> T where T: FLOAT64 | NUMERIC | BIGNUMERIC
```

Returns the least integral value that is not less than a number, as `CEIL` does.

**Parameters**

+ `x`: The number.

A call can write `SAFE.` before the name.

**Examples**

```googlesql
CEILING(1.2)
```

Result: `2.0`


## `SQRT`

```googlesql
SQRT(x: T) -> T where T: FLOAT64 | NUMERIC | BIGNUMERIC
```

Returns the square root of a number.

**Note:** The square root of a negative number is `NaN`, where GoogleSQL fails. `SAFE.SQRT` returns `NULL` for it, as in GoogleSQL.

**Parameters**

+ `x`: The number.

A call can write `SAFE.` before the name.

**Examples**

```googlesql
SQRT(16)
```

Result: `4.0`

```googlesql
SAFE.SQRT(-1)
```

Result: `NULL`


## `EXP`

```googlesql
EXP(x: T) -> T where T: FLOAT64 | NUMERIC | BIGNUMERIC
```

Returns e raised to the power of a number.

**Parameters**

+ `x`: The number.

A call can write `SAFE.` before the name.

**Examples**

```googlesql
EXP(0)
```

Result: `1.0`

```googlesql
EXP(1)
```

Result: `2.718281828459045`


## `LN`

```googlesql
LN(x: T) -> T where T: FLOAT64 | NUMERIC | BIGNUMERIC
```

Returns the natural logarithm of a number.

**Note:** The logarithm of zero is `-inf`, and of a negative number `NaN`, where GoogleSQL fails. `SAFE.LN` returns `NULL` for them.

**Parameters**

+ `x`: The number, which is positive.

A call can write `SAFE.` before the name.

**Examples**

```googlesql
LN(1)
```

Result: `0.0`

```googlesql
LN(10)
```

Result: `2.302585092994046`


## `LOG`

```googlesql
LOG(x: T[, base: T]) -> T where T: FLOAT64 | NUMERIC | BIGNUMERIC
```

Returns the logarithm of a number, natural or in a base.

**Note:** A number or a base out of range returns `NaN` or infinity, where GoogleSQL fails. `SAFE.LOG` returns `NULL` for them.

**Parameters**

+ `x`: The number, which is positive.
+ `base`: The base of the logarithm, which is positive and not 1. When omitted, the logarithm is natural, as `LN` returns.

A call can write `SAFE.` before the name.

**Examples**

```googlesql
LOG(100, 10)
```

Result: `2.0`

```googlesql
LOG(8, 2)
```

Result: `3.0`

```googlesql
LOG(100)
```

Result: `4.605170185988092`


## `LOG10`

```googlesql
LOG10(x: T) -> T where T: FLOAT64 | NUMERIC | BIGNUMERIC
```

Returns the base 10 logarithm of a number.

**Note:** The logarithm of zero is `-inf`, and of a negative number `NaN`, where GoogleSQL fails. `SAFE.LOG10` returns `NULL` for them.

**Parameters**

+ `x`: The number, which is positive.

A call can write `SAFE.` before the name.

**Examples**

```googlesql
LOG10(1000)
```

Result: `3.0`


## `POW`

```googlesql
POW(x: T, y: T) -> T where T: FLOAT64 | NUMERIC | BIGNUMERIC
```

Returns a number raised to a power.

**Note:** A negative base with a fractional power is `NaN`, where GoogleSQL fails. `SAFE.POW` returns `NULL` for it.

**Parameters**

+ `x`: The base.
+ `y`: The power.

A call can write `SAFE.` before the name.

**Examples**

```googlesql
POW(2, 10)
```

Result: `1024.0`

```googlesql
POW(2, -1)
```

Result: `0.5`


## `POWER`

```googlesql
POWER(x: T, y: T) -> T where T: FLOAT64 | NUMERIC | BIGNUMERIC
```

Returns a number raised to a power, as `POW` does.

**Note:** A negative base with a fractional power is `NaN`, where GoogleSQL fails. `SAFE.POW` returns `NULL` for it.

**Parameters**

+ `x`: The base.
+ `y`: The power.

A call can write `SAFE.` before the name.

**Examples**

```googlesql
POWER(3, 2)
```

Result: `9.0`


## `MOD`

```googlesql
MOD(x: T, y: T) -> T where T: INT64 | NUMERIC | BIGNUMERIC
```

Returns the remainder of dividing one number by another.

The remainder has the sign of the dividend. A divisor of 0 is an error.

**Parameters**

+ `x`: The dividend.
+ `y`: The divisor.

A call can write `SAFE.` before the name.

**Examples**

```googlesql
MOD(7, 3)
```

Result: `1`

```googlesql
MOD(-7, 3)
```

Result: `-1`


## `DIV`

```googlesql
DIV(x: T, y: T) -> T where T: INT64 | NUMERIC | BIGNUMERIC
```

Divides one number by another, and truncates the quotient toward zero.

A divisor of 0 is an error.

**Parameters**

+ `x`: The dividend.
+ `y`: The divisor.

A call can write `SAFE.` before the name.

**Examples**

```googlesql
DIV(7, 2)
```

Result: `3`

```googlesql
DIV(-7, 2)
```

Result: `-3`


## `SAFE_DIVIDE`

```googlesql
SAFE_DIVIDE(x: T, y: T) -> T where T: FLOAT64 | NUMERIC | BIGNUMERIC
```

Divides one number by another, or returns `NULL` for a divisor of 0.

**Parameters**

+ `x`: The dividend.
+ `y`: The divisor.

A call can write `SAFE.` before the name.

**Examples**

```googlesql
SAFE_DIVIDE(10, 4)
```

Result: `2.5`

```googlesql
SAFE_DIVIDE(1, 0)
```

Result: `NULL`


## `GREATEST`

```googlesql
GREATEST(values: T, ...) -> T where T: ANY
```

Returns the greatest of its values.

Returns `NULL` when any value is `NULL`.

**Parameters**

+ `values`: The values, of one type that orders.

A call can write `SAFE.` before the name.

**Examples**

```googlesql
GREATEST(1, 3, 2)
```

Result: `3`

```googlesql
GREATEST(1, NULL)
```

Result: `NULL`


## `LEAST`

```googlesql
LEAST(values: T, ...) -> T where T: ANY
```

Returns the least of its values.

Returns `NULL` when any value is `NULL`.

**Parameters**

+ `values`: The values, of one type that orders.

A call can write `SAFE.` before the name.

**Examples**

```googlesql
LEAST(1, 3, 2)
```

Result: `1`

```googlesql
LEAST('b', 'a')
```

Result: `'a'`


## `CONCAT`

```googlesql
CONCAT(values: T, ...) -> T where T: STRING | BYTES
```

Joins strings, or bytes, into one.

Returns `NULL` when any value is `NULL`.

**Parameters**

+ `values`: The strings, or the bytes, in order.

A call can write `SAFE.` before the name.

**Examples**

```googlesql
CONCAT('a', 'b', 'c')
```

Result: `'abc'`

```googlesql
CONCAT('a', NULL)
```

Result: `NULL`


## `LENGTH`

```googlesql
LENGTH(value: STRING) -> INT64
```

Returns the number of characters of a string.

**Parameters**

+ `value`: The string.

A call can write `SAFE.` before the name.

**Examples**

```googlesql
LENGTH('héllo')
```

Result: `5`


## `LOWER`

```googlesql
LOWER(value: STRING) -> STRING
```

Returns a string with its letters in lowercase.

**Parameters**

+ `value`: The string.

A call can write `SAFE.` before the name.

**Examples**

```googlesql
LOWER('Hello')
```

Result: `'hello'`


## `UPPER`

```googlesql
UPPER(value: STRING) -> STRING
```

Returns a string with its letters in uppercase.

**Parameters**

+ `value`: The string.

A call can write `SAFE.` before the name.

**Examples**

```googlesql
UPPER('Hello')
```

Result: `'HELLO'`


## `SUBSTR`

```googlesql
SUBSTR(value: STRING, position: INT64[, length: INT64]) -> STRING
```

Returns a part of a string.

**Parameters**

+ `value`: The string.
+ `position`: Where the part starts, from 1. A negative position counts from the end, with -1 for the last character, and 0 or a position before the start is 1.
+ `length`: The most characters that the part holds. A negative length is an error. When omitted, the part runs to the end of the string.

A call can write `SAFE.` before the name.

**Examples**

```googlesql
SUBSTR('apple', 2)
```

Result: `'pple'`

```googlesql
SUBSTR('apple', 2, 2)
```

Result: `'pp'`

```googlesql
SUBSTR('apple', -3, 2)
```

Result: `'pl'`


## `SUBSTRING`

```googlesql
SUBSTRING(value: STRING, position: INT64[, length: INT64]) -> STRING
```

Returns a part of a string, as `SUBSTR` does.

**Parameters**

+ `value`: The string.
+ `position`: Where the part starts, from 1. A negative position counts from the end, with -1 for the last character, and 0 or a position before the start is 1.
+ `length`: The most characters that the part holds. A negative length is an error. When omitted, the part runs to the end of the string.

A call can write `SAFE.` before the name.

**Examples**

```googlesql
SUBSTRING('apple', 2, 3)
```

Result: `'ppl'`


## `TRIM`

```googlesql
TRIM(value: STRING[, characters: STRING]) -> STRING
```

Removes characters from the start and the end of a string.

**Parameters**

+ `value`: The string.
+ `characters`: The characters to remove, each on its own. When omitted, whitespace.

A call can write `SAFE.` before the name.

**Examples**

```googlesql
TRIM('  a  ')
```

Result: `'a'`

```googlesql
TRIM('xxaxx', 'x')
```

Result: `'a'`


## `LTRIM`

```googlesql
LTRIM(value: STRING[, characters: STRING]) -> STRING
```

Removes characters from the start of a string.

**Parameters**

+ `value`: The string.
+ `characters`: The characters to remove, each on its own. When omitted, whitespace.

A call can write `SAFE.` before the name.

**Examples**

```googlesql
LTRIM('xxaxx', 'x')
```

Result: `'axx'`


## `RTRIM`

```googlesql
RTRIM(value: STRING[, characters: STRING]) -> STRING
```

Removes characters from the end of a string.

**Parameters**

+ `value`: The string.
+ `characters`: The characters to remove, each on its own. When omitted, whitespace.

A call can write `SAFE.` before the name.

**Examples**

```googlesql
RTRIM('xxaxx', 'x')
```

Result: `'xxa'`


## `REPLACE`

```googlesql
REPLACE(value: STRING, from_pattern: STRING, to_pattern: STRING) -> STRING
```

Replaces every occurrence of a string in another.

**Parameters**

+ `value`: The string.
+ `from_pattern`: The string to replace. An empty string replaces nothing.
+ `to_pattern`: The string that takes its place.

A call can write `SAFE.` before the name.

**Examples**

```googlesql
REPLACE('apple', 'p', 'b')
```

Result: `'abble'`


## `STARTS_WITH`

```googlesql
STARTS_WITH(value: STRING, prefix: STRING) -> BOOL
```

Returns whether a string starts with another.

**Parameters**

+ `value`: The string.
+ `prefix`: The string to look for at the start.

A call can write `SAFE.` before the name.

**Examples**

```googlesql
STARTS_WITH('apple', 'ap')
```

Result: `TRUE`


## `ENDS_WITH`

```googlesql
ENDS_WITH(value: STRING, suffix: STRING) -> BOOL
```

Returns whether a string ends with another.

**Parameters**

+ `value`: The string.
+ `suffix`: The string to look for at the end.

A call can write `SAFE.` before the name.

**Examples**

```googlesql
ENDS_WITH('apple', 'le')
```

Result: `TRUE`


## `STRPOS`

```googlesql
STRPOS(value: STRING, search: STRING) -> INT64
```

Returns the position of the first occurrence of a string in another.

Positions count from 1, and the result is 0 when the string doesn't occur.

**Parameters**

+ `value`: The string to search.
+ `search`: The string to find.

A call can write `SAFE.` before the name.

**Examples**

```googlesql
STRPOS('banana', 'an')
```

Result: `2`

```googlesql
STRPOS('apple', 'z')
```

Result: `0`


## `SPLIT`

```googlesql
SPLIT(value: STRING[, delimiter: STRING]) -> ARRAY<STRING>
```

Splits a string into an array at a delimiter.

**Parameters**

+ `value`: The string.
+ `delimiter`: The string between the parts. When omitted, `','`.

A call can write `SAFE.` before the name.

**Examples**

```googlesql
SPLIT('a,b,c')
```

Result: `['a', 'b', 'c']`

```googlesql
SPLIT('a b', ' ')
```

Result: `['a', 'b']`


## `REGEXP_CONTAINS`

```googlesql
REGEXP_CONTAINS(value: STRING, regexp: STRING) -> BOOL
```

Returns whether a string has a match of a regular expression.

**Parameters**

+ `value`: The string.
+ `regexp`: The regular expression, in RE2's syntax. A match anywhere in the string counts.

**Examples**

```googlesql
REGEXP_CONTAINS('abc123', r'[0-9]')
```

Result: `TRUE`


## `REGEXP_EXTRACT`

```googlesql
REGEXP_EXTRACT(value: STRING, regexp: STRING) -> STRING
```

Returns the first match of a regular expression in a string.

Returns `NULL` when the string has no match.

**Parameters**

+ `value`: The string.
+ `regexp`: The regular expression, in RE2's syntax, with at most one capturing group. With a group, the result is the group's match, and without one, the whole match.

**Examples**

```googlesql
REGEXP_EXTRACT('abc123', r'[0-9]+')
```

Result: `'123'`

```googlesql
REGEXP_EXTRACT('user@example.com', r'@(.+)')
```

Result: `'example.com'`


## `REGEXP_REPLACE`

```googlesql
REGEXP_REPLACE(value: STRING, regexp: STRING, replacement: STRING) -> STRING
```

Replaces every match of a regular expression in a string.

**Parameters**

+ `value`: The string.
+ `regexp`: The regular expression, in RE2's syntax.
+ `replacement`: The string that takes the place of each match, where `\1` to `\9` stand for the matches of the capturing groups.

**Examples**

```googlesql
REGEXP_REPLACE('a1b2', r'[0-9]', '#')
```

Result: `'a#b#'`

```googlesql
REGEXP_REPLACE('abc', r'(b)', r'<\1>')
```

Result: `'a<b>c'`


## `symbol`

```googlesql
symbol([x: ANY][, y: ANY][, color: ANY][, fill: ANY][, stroke: ANY][, opacity: ANY][, fill_opacity: ANY][, stroke_opacity: ANY][, stroke_width: ANY][, size: ANY][, shape: ANY][, angle: ANY][, pattern: ANY][, filled: BOOL][, coord: ANY]) -> BYTES
```

A symbol at each row.

**Parameters**

+ `x`: The horizontal position of a Cartesian mark. Values: values of its scale's domain, and a direct value is a fraction of the plot's width, from 0 at its left edge to 1 at its right edge, and past them outside the plot, so it stands at one place of the plot at any size.
+ `y`: The vertical position of a Cartesian mark. Values: values of its scale's domain, and a direct value is a fraction of the plot's height, from 0 at its bottom edge to 1 at its top edge, and past them outside the plot.
+ `color`: The fill of a mark with a fill, and the stroke of a mark without one: a line, a trail, a rule, or a symbol that is not filled. Values: CSS color strings.
+ `fill`: Values: CSS color strings.
+ `stroke`: Values: CSS color strings.
+ `opacity`: A factor of the alpha of the fill and the stroke. Values: numbers from 0 to 1.
+ `fill_opacity`: A factor of the alpha of the fill. Values: numbers from 0 to 1.
+ `stroke_opacity`: A factor of the alpha of the stroke. Values: numbers from 0 to 1.
+ `stroke_width`: The width of a stroke. Values: numbers, in pixels.
+ `size`: The area of each symbol, in square pixels.
+ `shape`: The shape of a symbol, or of the symbol of a point geometry. Values: the shape names `circle`, `square`, `cross`, `diamond`, `triangle-up`, `triangle-down`, `triangle-right`, `triangle-left`, `triangle`, `arrow`, `wedge`, `star`, `wye`, `pentagon`, and `star-4`, in any case, as Vega draws them, or SVG path data in a box from -1 to 1, which the symbol scales to the square of its size.
+ `angle`: The rotation of a symbol, a text, or a path, in degrees clockwise. Values: numbers.
+ `pattern`: A pattern over the fill. Values: pattern text, such as a preset with an optional scale, `crosshatch` or `dots 2`, layers, such as `stripes 30 8 2 + circle 12 on hex 10`, or `none` or null for no pattern, in this grammar:

  ```text pattern  = ( "none" | preset | layers ) { ";" option } preset   = PRESET [ SCALE ] layers   = [ op ] layer { op layer } op       = "+" | "-" | "^" layer    = stripes | symbols stripes  = "stripes" ANGLE SPACING WIDTH [ "dash" LENGTH GAP [ PHASE ] ] [ "offset" PHASE ] symbols  = SHAPE SIZE [ "rotate" ANGLE ] [ "open" WIDTH ] "on" lattice lattice  = ( "grid" SPACING [ SPACING ] | "hex" SPACING | "lattice" SPACING ANGLE SPACING ANGLE ) [ "at" ANGLE ] [ "offset" PHASE PHASE ] option   = "ink" ( "auto" | COLOR ) [ OPACITY ] | "anchor" ( "group" | "mark" | "scene" ) ```

  Angles are in degrees and lengths in pixels. A symbol's `SIZE` is the area of its bounding square in square pixels, as for symbol marks, and its `SHAPE` is one of the shape names of `CHANNEL_SHAPE`. `COLOR` is a CSS color. Spaces separate the tokens, so an operator has a space on each side and `-45` is a number, and the spaces inside parentheses, as in `rgba(26, 51, 153, 0.5)`, belong to one token.

  The operators combine each layer with the coverage of the layers before it: `+` adds, `-` subtracts, and `^` takes the exclusive or. The clauses of a layer and the options can come in any order, each at most once. `grid` and `hex` give a lattice with steps at 0 and 90 degrees, and with equal steps at 0 and 60 degrees, and `at` turns both steps. `lattice` gives the spacing and the angle of each step. A pattern without an option takes its default: `ink auto 0.18` and `anchor group`. `none` is a pattern without layers.

  `PRESET` names a whole pattern, and its `SCALE`, 1 when it is left out, multiplies each length of the preset, and each symbol size, which is an area, by its square, so `dots 2` has dots twice as wide and twice as far apart:

  - `diagonal-down`: `stripes 45 8 2` - `diagonal-up`: `stripes -45 8 2` - `horizontal`: `stripes 0 8 2` - `vertical`: `stripes 90 8 2` - `crosshatch`: `stripes 45 8 1.5 + stripes -45 8 1.5` - `gridlines`: `stripes 0 8 1.5 + stripes 90 8 1.5` - `dashes`: `stripes 0 6 2 dash 6 3` - `dots`: `circle 16 on hex 8` - `rings`: `circle 36 open 1 on hex 10` - `triangles`: `triangle-up 30 on hex 10` - `checkerboard`: `stripes 0 8 4 ^ stripes 90 8 4` - `bricks`: `stripes 0 8 1.5 + stripes 90 16 1.5 dash 8 8 + stripes 90 16 1.5 dash 8 8 8 offset 8`
+ `filled`: Whether the symbol has a fill. Without one, `color` colors the stroke of the symbol.
+ `coord`: The options of the plot's coordinate system, such as `polar(inner_radius => 0.5)`.

**Clauses**: `ORDER BY`

## `line`

```googlesql
line([x: ANY][, y: ANY][, color: ANY][, stroke: ANY][, opacity: ANY][, stroke_opacity: ANY][, stroke_width: ANY][, stroke_dash: ANY][, detail: ANY][, coord: ANY]) -> BYTES
```

A line through the rows of each series, in the order of the rows.

A line through the rows of each series, in the order of the rows. The `detail` and the color channels split the rows into series. The channels other than the positions take one value for each series.

**Parameters**

+ `x`: The horizontal position of a Cartesian mark. Values: values of its scale's domain, and a direct value is a fraction of the plot's width, from 0 at its left edge to 1 at its right edge, and past them outside the plot, so it stands at one place of the plot at any size.
+ `y`: The vertical position of a Cartesian mark. Values: values of its scale's domain, and a direct value is a fraction of the plot's height, from 0 at its bottom edge to 1 at its top edge, and past them outside the plot.
+ `color`: The fill of a mark with a fill, and the stroke of a mark without one: a line, a trail, a rule, or a symbol that is not filled. Values: CSS color strings.
+ `stroke`: Values: CSS color strings.
+ `opacity`: A factor of the alpha of the fill and the stroke. Values: numbers from 0 to 1.
+ `stroke_opacity`: A factor of the alpha of the stroke. Values: numbers from 0 to 1.
+ `stroke_width`: The width of a stroke. Values: numbers, in pixels.
+ `stroke_dash`: The dash of a stroke, as alternating dash and gap lengths in pixels. Values: lists of numbers, or strings of numbers such as `4 2`. No lengths draws a solid stroke.
+ `detail`: Splits the rows of a line, a trail, or an area into series, without a scale.
+ `coord`: The options of the plot's coordinate system, such as `polar(inner_radius => 0.5)`.

**Clauses**: `ORDER BY`

## `trail`

```googlesql
trail([x: ANY][, y: ANY][, color: ANY][, stroke: ANY][, opacity: ANY][, size: ANY][, detail: ANY][, coord: ANY]) -> BYTES
```

A line through the rows of each series, as for `LineMark`, whose width at each row is its `size`, in pixels.

A line through the rows of each series, as for `LineMark`, whose width at each row is its `size`, in pixels. `color`, `stroke`, and `opacity` take one value for each series.

**Parameters**

+ `x`: The horizontal position of a Cartesian mark. Values: values of its scale's domain, and a direct value is a fraction of the plot's width, from 0 at its left edge to 1 at its right edge, and past them outside the plot, so it stands at one place of the plot at any size.
+ `y`: The vertical position of a Cartesian mark. Values: values of its scale's domain, and a direct value is a fraction of the plot's height, from 0 at its bottom edge to 1 at its top edge, and past them outside the plot.
+ `color`: The fill of a mark with a fill, and the stroke of a mark without one: a line, a trail, a rule, or a symbol that is not filled. Values: CSS color strings.
+ `stroke`: Values: CSS color strings.
+ `opacity`: A factor of the alpha of the fill and the stroke. Values: numbers from 0 to 1.
+ `size`: The width of the trail at each row, in pixels.
+ `detail`: Splits the rows of a line, a trail, or an area into series, without a scale.
+ `coord`: The options of the plot's coordinate system, such as `polar(inner_radius => 0.5)`.

**Clauses**: `ORDER BY`

## `area`

```googlesql
area([x: ANY][, y: ANY][, x2: ANY][, y2: ANY][, color: ANY][, fill: ANY][, stroke: ANY][, opacity: ANY][, fill_opacity: ANY][, stroke_opacity: ANY][, stroke_width: ANY][, stroke_dash: ANY][, pattern: ANY][, detail: ANY][, orient: 'vertical' | 'horizontal'][, coord: ANY]) -> BYTES
```

The region of each series between a line and a baseline: `y2`, or zero, along `x` for a vertical area, and `x2`, or zero, along `y` for a horizontal area.

The region of each series between a line and a baseline: `y2`, or zero, along `x` for a vertical area, and `x2`, or zero, along `y` for a horizontal area. The channels other than the positions take one value for each series.

**Parameters**

+ `x`: The horizontal position of a Cartesian mark. Values: values of its scale's domain, and a direct value is a fraction of the plot's width, from 0 at its left edge to 1 at its right edge, and past them outside the plot, so it stands at one place of the plot at any size.
+ `y`: The vertical position of a Cartesian mark. Values: values of its scale's domain, and a direct value is a fraction of the plot's height, from 0 at its bottom edge to 1 at its top edge, and past them outside the plot.
+ `x2`: The second x position, through the scale of `x`, or a direct fraction of the plot's width, as for `x`.
+ `y2`: The second y position, through the scale of `y`, or a direct fraction of the plot's height, as for `y`.
+ `color`: The fill of a mark with a fill, and the stroke of a mark without one: a line, a trail, a rule, or a symbol that is not filled. Values: CSS color strings.
+ `fill`: Values: CSS color strings.
+ `stroke`: Values: CSS color strings.
+ `opacity`: A factor of the alpha of the fill and the stroke. Values: numbers from 0 to 1.
+ `fill_opacity`: A factor of the alpha of the fill. Values: numbers from 0 to 1.
+ `stroke_opacity`: A factor of the alpha of the stroke. Values: numbers from 0 to 1.
+ `stroke_width`: The width of a stroke. Values: numbers, in pixels.
+ `stroke_dash`: The dash of a stroke, as alternating dash and gap lengths in pixels. Values: lists of numbers, or strings of numbers such as `4 2`. No lengths draws a solid stroke.
+ `pattern`: A pattern over the fill. Values: pattern text, such as a preset with an optional scale, `crosshatch` or `dots 2`, layers, such as `stripes 30 8 2 + circle 12 on hex 10`, or `none` or null for no pattern, in this grammar:

  ```text pattern  = ( "none" | preset | layers ) { ";" option } preset   = PRESET [ SCALE ] layers   = [ op ] layer { op layer } op       = "+" | "-" | "^" layer    = stripes | symbols stripes  = "stripes" ANGLE SPACING WIDTH [ "dash" LENGTH GAP [ PHASE ] ] [ "offset" PHASE ] symbols  = SHAPE SIZE [ "rotate" ANGLE ] [ "open" WIDTH ] "on" lattice lattice  = ( "grid" SPACING [ SPACING ] | "hex" SPACING | "lattice" SPACING ANGLE SPACING ANGLE ) [ "at" ANGLE ] [ "offset" PHASE PHASE ] option   = "ink" ( "auto" | COLOR ) [ OPACITY ] | "anchor" ( "group" | "mark" | "scene" ) ```

  Angles are in degrees and lengths in pixels. A symbol's `SIZE` is the area of its bounding square in square pixels, as for symbol marks, and its `SHAPE` is one of the shape names of `CHANNEL_SHAPE`. `COLOR` is a CSS color. Spaces separate the tokens, so an operator has a space on each side and `-45` is a number, and the spaces inside parentheses, as in `rgba(26, 51, 153, 0.5)`, belong to one token.

  The operators combine each layer with the coverage of the layers before it: `+` adds, `-` subtracts, and `^` takes the exclusive or. The clauses of a layer and the options can come in any order, each at most once. `grid` and `hex` give a lattice with steps at 0 and 90 degrees, and with equal steps at 0 and 60 degrees, and `at` turns both steps. `lattice` gives the spacing and the angle of each step. A pattern without an option takes its default: `ink auto 0.18` and `anchor group`. `none` is a pattern without layers.

  `PRESET` names a whole pattern, and its `SCALE`, 1 when it is left out, multiplies each length of the preset, and each symbol size, which is an area, by its square, so `dots 2` has dots twice as wide and twice as far apart:

  - `diagonal-down`: `stripes 45 8 2` - `diagonal-up`: `stripes -45 8 2` - `horizontal`: `stripes 0 8 2` - `vertical`: `stripes 90 8 2` - `crosshatch`: `stripes 45 8 1.5 + stripes -45 8 1.5` - `gridlines`: `stripes 0 8 1.5 + stripes 90 8 1.5` - `dashes`: `stripes 0 6 2 dash 6 3` - `dots`: `circle 16 on hex 8` - `rings`: `circle 36 open 1 on hex 10` - `triangles`: `triangle-up 30 on hex 10` - `checkerboard`: `stripes 0 8 4 ^ stripes 90 8 4` - `bricks`: `stripes 0 8 1.5 + stripes 90 16 1.5 dash 8 8 + stripes 90 16 1.5 dash 8 8 8 offset 8`
+ `detail`: Splits the rows of a line, a trail, or an area into series, without a scale.
+ `orient`: When unset, the area is vertical unless only its `x` scale is continuous.
+ `coord`: The options of the plot's coordinate system, such as `polar(inner_radius => 0.5)`.

**Clauses**: `ORDER BY`

## `bar`

```googlesql
bar([x: ANY][, y: ANY][, x2: ANY][, y2: ANY][, color: ANY][, fill: ANY][, stroke: ANY][, opacity: ANY][, fill_opacity: ANY][, stroke_opacity: ANY][, stroke_width: ANY][, pattern: ANY][, orient: 'vertical' | 'horizontal'][, corner_radius: ANY][, coord: ANY]) -> BYTES
```

A rectangle from a baseline to a value, along a band, or at a continuous key with the thickness that the styles give.

**Parameters**

+ `x`: The horizontal position of a Cartesian mark. Values: values of its scale's domain, and a direct value is a fraction of the plot's width, from 0 at its left edge to 1 at its right edge, and past them outside the plot, so it stands at one place of the plot at any size.
+ `y`: The vertical position of a Cartesian mark. Values: values of its scale's domain, and a direct value is a fraction of the plot's height, from 0 at its bottom edge to 1 at its top edge, and past them outside the plot.
+ `x2`: The second x position, through the scale of `x`, or a direct fraction of the plot's width, as for `x`.
+ `y2`: The second y position, through the scale of `y`, or a direct fraction of the plot's height, as for `y`.
+ `color`: The fill of a mark with a fill, and the stroke of a mark without one: a line, a trail, a rule, or a symbol that is not filled. Values: CSS color strings.
+ `fill`: Values: CSS color strings.
+ `stroke`: Values: CSS color strings.
+ `opacity`: A factor of the alpha of the fill and the stroke. Values: numbers from 0 to 1.
+ `fill_opacity`: A factor of the alpha of the fill. Values: numbers from 0 to 1.
+ `stroke_opacity`: A factor of the alpha of the stroke. Values: numbers from 0 to 1.
+ `stroke_width`: The width of a stroke. Values: numbers, in pixels.
+ `pattern`: A pattern over the fill. Values: pattern text, such as a preset with an optional scale, `crosshatch` or `dots 2`, layers, such as `stripes 30 8 2 + circle 12 on hex 10`, or `none` or null for no pattern, in this grammar:

  ```text pattern  = ( "none" | preset | layers ) { ";" option } preset   = PRESET [ SCALE ] layers   = [ op ] layer { op layer } op       = "+" | "-" | "^" layer    = stripes | symbols stripes  = "stripes" ANGLE SPACING WIDTH [ "dash" LENGTH GAP [ PHASE ] ] [ "offset" PHASE ] symbols  = SHAPE SIZE [ "rotate" ANGLE ] [ "open" WIDTH ] "on" lattice lattice  = ( "grid" SPACING [ SPACING ] | "hex" SPACING | "lattice" SPACING ANGLE SPACING ANGLE ) [ "at" ANGLE ] [ "offset" PHASE PHASE ] option   = "ink" ( "auto" | COLOR ) [ OPACITY ] | "anchor" ( "group" | "mark" | "scene" ) ```

  Angles are in degrees and lengths in pixels. A symbol's `SIZE` is the area of its bounding square in square pixels, as for symbol marks, and its `SHAPE` is one of the shape names of `CHANNEL_SHAPE`. `COLOR` is a CSS color. Spaces separate the tokens, so an operator has a space on each side and `-45` is a number, and the spaces inside parentheses, as in `rgba(26, 51, 153, 0.5)`, belong to one token.

  The operators combine each layer with the coverage of the layers before it: `+` adds, `-` subtracts, and `^` takes the exclusive or. The clauses of a layer and the options can come in any order, each at most once. `grid` and `hex` give a lattice with steps at 0 and 90 degrees, and with equal steps at 0 and 60 degrees, and `at` turns both steps. `lattice` gives the spacing and the angle of each step. A pattern without an option takes its default: `ink auto 0.18` and `anchor group`. `none` is a pattern without layers.

  `PRESET` names a whole pattern, and its `SCALE`, 1 when it is left out, multiplies each length of the preset, and each symbol size, which is an area, by its square, so `dots 2` has dots twice as wide and twice as far apart:

  - `diagonal-down`: `stripes 45 8 2` - `diagonal-up`: `stripes -45 8 2` - `horizontal`: `stripes 0 8 2` - `vertical`: `stripes 90 8 2` - `crosshatch`: `stripes 45 8 1.5 + stripes -45 8 1.5` - `gridlines`: `stripes 0 8 1.5 + stripes 90 8 1.5` - `dashes`: `stripes 0 6 2 dash 6 3` - `dots`: `circle 16 on hex 8` - `rings`: `circle 36 open 1 on hex 10` - `triangles`: `triangle-up 30 on hex 10` - `checkerboard`: `stripes 0 8 4 ^ stripes 90 8 4` - `bricks`: `stripes 0 8 1.5 + stripes 90 16 1.5 dash 8 8 + stripes 90 16 1.5 dash 8 8 8 offset 8`
+ `orient`: When unset, the bar is vertical unless only its `x` scale is continuous.
+ `corner_radius`: The radius of the corners of each bar, in pixels. When unset, the styles give it.
+ `coord`: The options of the plot's coordinate system, such as `polar(inner_radius => 0.5)`.

**Clauses**: `ORDER BY`

## `rect`

```googlesql
rect([x: ANY][, y: ANY][, x2: ANY][, y2: ANY][, color: ANY][, fill: ANY][, stroke: ANY][, opacity: ANY][, fill_opacity: ANY][, stroke_opacity: ANY][, stroke_width: ANY][, pattern: ANY][, corner_radius: ANY][, inset: ANY][, coord: ANY]) -> BYTES
```

A rectangle from `x` to `x2` and from `y` to `y2`, or across the band of `x` or `y` on a band scale without its second channel.

A rectangle from `x` to `x2` and from `y` to `y2`, or across the band of `x` or `y` on a band scale without its second channel. A rect without `x` spans the width of its plot, and a rect without `y` spans its height.

**Parameters**

+ `x`: The horizontal position of a Cartesian mark. Values: values of its scale's domain, and a direct value is a fraction of the plot's width, from 0 at its left edge to 1 at its right edge, and past them outside the plot, so it stands at one place of the plot at any size.
+ `y`: The vertical position of a Cartesian mark. Values: values of its scale's domain, and a direct value is a fraction of the plot's height, from 0 at its bottom edge to 1 at its top edge, and past them outside the plot.
+ `x2`: The second x position, through the scale of `x`, or a direct fraction of the plot's width, as for `x`.
+ `y2`: The second y position, through the scale of `y`, or a direct fraction of the plot's height, as for `y`.
+ `color`: The fill of a mark with a fill, and the stroke of a mark without one: a line, a trail, a rule, or a symbol that is not filled. Values: CSS color strings.
+ `fill`: Values: CSS color strings.
+ `stroke`: Values: CSS color strings.
+ `opacity`: A factor of the alpha of the fill and the stroke. Values: numbers from 0 to 1.
+ `fill_opacity`: A factor of the alpha of the fill. Values: numbers from 0 to 1.
+ `stroke_opacity`: A factor of the alpha of the stroke. Values: numbers from 0 to 1.
+ `stroke_width`: The width of a stroke. Values: numbers, in pixels.
+ `pattern`: A pattern over the fill. Values: pattern text, such as a preset with an optional scale, `crosshatch` or `dots 2`, layers, such as `stripes 30 8 2 + circle 12 on hex 10`, or `none` or null for no pattern, in this grammar:

  ```text pattern  = ( "none" | preset | layers ) { ";" option } preset   = PRESET [ SCALE ] layers   = [ op ] layer { op layer } op       = "+" | "-" | "^" layer    = stripes | symbols stripes  = "stripes" ANGLE SPACING WIDTH [ "dash" LENGTH GAP [ PHASE ] ] [ "offset" PHASE ] symbols  = SHAPE SIZE [ "rotate" ANGLE ] [ "open" WIDTH ] "on" lattice lattice  = ( "grid" SPACING [ SPACING ] | "hex" SPACING | "lattice" SPACING ANGLE SPACING ANGLE ) [ "at" ANGLE ] [ "offset" PHASE PHASE ] option   = "ink" ( "auto" | COLOR ) [ OPACITY ] | "anchor" ( "group" | "mark" | "scene" ) ```

  Angles are in degrees and lengths in pixels. A symbol's `SIZE` is the area of its bounding square in square pixels, as for symbol marks, and its `SHAPE` is one of the shape names of `CHANNEL_SHAPE`. `COLOR` is a CSS color. Spaces separate the tokens, so an operator has a space on each side and `-45` is a number, and the spaces inside parentheses, as in `rgba(26, 51, 153, 0.5)`, belong to one token.

  The operators combine each layer with the coverage of the layers before it: `+` adds, `-` subtracts, and `^` takes the exclusive or. The clauses of a layer and the options can come in any order, each at most once. `grid` and `hex` give a lattice with steps at 0 and 90 degrees, and with equal steps at 0 and 60 degrees, and `at` turns both steps. `lattice` gives the spacing and the angle of each step. A pattern without an option takes its default: `ink auto 0.18` and `anchor group`. `none` is a pattern without layers.

  `PRESET` names a whole pattern, and its `SCALE`, 1 when it is left out, multiplies each length of the preset, and each symbol size, which is an area, by its square, so `dots 2` has dots twice as wide and twice as far apart:

  - `diagonal-down`: `stripes 45 8 2` - `diagonal-up`: `stripes -45 8 2` - `horizontal`: `stripes 0 8 2` - `vertical`: `stripes 90 8 2` - `crosshatch`: `stripes 45 8 1.5 + stripes -45 8 1.5` - `gridlines`: `stripes 0 8 1.5 + stripes 90 8 1.5` - `dashes`: `stripes 0 6 2 dash 6 3` - `dots`: `circle 16 on hex 8` - `rings`: `circle 36 open 1 on hex 10` - `triangles`: `triangle-up 30 on hex 10` - `checkerboard`: `stripes 0 8 4 ^ stripes 90 8 4` - `bricks`: `stripes 0 8 1.5 + stripes 90 16 1.5 dash 8 8 + stripes 90 16 1.5 dash 8 8 8 offset 8`
+ `corner_radius`: The radius of the corners of each rect, in pixels. When unset, the styles give it.
+ `inset`: The space that the rect gives up at each side, in pixels, so neighboring rects stand twice as far apart, as the bins of a histogram or the slices of a pie do.
+ `coord`: The options of the plot's coordinate system, such as `polar(inner_radius => 0.5)`.

**Clauses**: `ORDER BY`

## `rule`

```googlesql
rule([x: ANY][, y: ANY][, x2: ANY][, y2: ANY][, color: ANY][, stroke: ANY][, opacity: ANY][, stroke_width: ANY][, stroke_dash: ANY][, coord: ANY]) -> BYTES
```

A line segment from (`x`, `y`) to (`x2`, `y2`), where a missing `x2` takes the value of `x` and a missing `y2` takes the value of `y`.

A line segment from (`x`, `y`) to (`x2`, `y2`), where a missing `x2` takes the value of `x` and a missing `y2` takes the value of `y`. A rule without `x` spans the width of its plot, and a rule without `y` spans its height.

**Parameters**

+ `x`: The horizontal position of a Cartesian mark. Values: values of its scale's domain, and a direct value is a fraction of the plot's width, from 0 at its left edge to 1 at its right edge, and past them outside the plot, so it stands at one place of the plot at any size.
+ `y`: The vertical position of a Cartesian mark. Values: values of its scale's domain, and a direct value is a fraction of the plot's height, from 0 at its bottom edge to 1 at its top edge, and past them outside the plot.
+ `x2`: The second x position, through the scale of `x`, or a direct fraction of the plot's width, as for `x`.
+ `y2`: The second y position, through the scale of `y`, or a direct fraction of the plot's height, as for `y`.
+ `color`: The fill of a mark with a fill, and the stroke of a mark without one: a line, a trail, a rule, or a symbol that is not filled. Values: CSS color strings.
+ `stroke`: Values: CSS color strings.
+ `opacity`: A factor of the alpha of the fill and the stroke. Values: numbers from 0 to 1.
+ `stroke_width`: The width of a stroke. Values: numbers, in pixels.
+ `stroke_dash`: The dash of a stroke, as alternating dash and gap lengths in pixels. Values: lists of numbers, or strings of numbers such as `4 2`. No lengths draws a solid stroke.
+ `coord`: The options of the plot's coordinate system, such as `polar(inner_radius => 0.5)`.

**Clauses**: `ORDER BY`

## `text`

```googlesql
text(text: ANY[, x: ANY][, y: ANY][, color: ANY][, fill: ANY][, opacity: ANY][, size: ANY][, angle: ANY][, font_weight: ANY][, font_style: ANY][, width: ANY][, format: STRING][, align: 'left' | 'center' | 'right'][, baseline: 'top' | 'middle' | 'alphabetic' | 'bottom'][, font_family: STRING][, syntax: 'plain' | 'plain_lines' | 'typst'][, width_mode: 'max' | 'fixed'][, wrap: BOOL][, max_lines: INT64][, ellipsis: BOOL][, coord: ANY]) -> BYTES
```

The `text` value of each row.

A text aligns to its position as its `align` and its `baseline` say, centered by default.

**Parameters**

+ `text`: The text of a row, which the text mark formats, without a scale.
+ `x`: The horizontal position of a Cartesian mark. Values: values of its scale's domain, and a direct value is a fraction of the plot's width, from 0 at its left edge to 1 at its right edge, and past them outside the plot, so it stands at one place of the plot at any size.
+ `y`: The vertical position of a Cartesian mark. Values: values of its scale's domain, and a direct value is a fraction of the plot's height, from 0 at its bottom edge to 1 at its top edge, and past them outside the plot.
+ `color`: The fill of a mark with a fill, and the stroke of a mark without one: a line, a trail, a rule, or a symbol that is not filled. Values: CSS color strings.
+ `fill`: Values: CSS color strings.
+ `opacity`: A factor of the alpha of the fill and the stroke. Values: numbers from 0 to 1.
+ `size`: The font size of each text, in pixels.
+ `angle`: The rotation of a symbol, a text, or a path, in degrees clockwise. Values: numbers.
+ `font_weight`: The weight of the font of a text. Values: numbers from 100 to 900, or `normal`, which is 400, and `bold`, which is 700, as in CSS, without a scale.
+ `font_style`: The style of the font of a text. Values: `normal`, `italic`, or `oblique`, without a scale.
+ `width`: The width of the box of each text, in pixels, at which its lines wrap. When it is not encoded, a text is as wide as its widest line and ends lines only at its line breaks.
+ `format`: The pattern of a numeric or temporal `text` value, as for `Labels.format`.
+ `align`: How each text aligns to its position across its lines. When unset, centered.
+ `baseline`: How each text aligns to its position along its lines. When unset, at its middle.
+ `font_family`: The font family of the texts. When it is empty, the styles give it.
+ `syntax`: How the text engine reads each text.
+ `width_mode`: Whether `width` is the most that the box of each text takes, or exactly its width. When unset, the width is a maximum.
+ `wrap`: Whether the lines of each text wrap at its `width`. When unset, a text with a `width` wraps.
+ `max_lines`: The most lines that each text keeps. When unset, it keeps all of them.
+ `ellipsis`: Whether an ellipsis marks a text that its `width` or `max_lines` cuts. When unset, none does.
+ `coord`: The options of the plot's coordinate system, such as `polar(inner_radius => 0.5)`.

**Clauses**: `ORDER BY`

## `image`

```googlesql
image(image: ANY[, x: ANY][, y: ANY][, x2: ANY][, y2: ANY][, width: ANY][, height: ANY][, align: 'left' | 'center' | 'right'][, baseline: 'top' | 'middle' | 'bottom'][, aspect: BOOL][, smooth: BOOL][, coord: ANY]) -> BYTES
```

The `image` value of each row.

The `image` value of each row. An image with `x2` and `y2` spans from (`x`, `y`) to (`x2`, `y2`), and its `width`, its `height`, and its alignment do not apply.

**Parameters**

+ `image`: The image of a row, without a scale: a URL, or a struct of `width` (UInt32), `height` (UInt32), and `data` (a list of the UInt8 RGBA bytes of its pixels).
+ `x`: The horizontal position of a Cartesian mark. Values: values of its scale's domain, and a direct value is a fraction of the plot's width, from 0 at its left edge to 1 at its right edge, and past them outside the plot, so it stands at one place of the plot at any size.
+ `y`: The vertical position of a Cartesian mark. Values: values of its scale's domain, and a direct value is a fraction of the plot's height, from 0 at its bottom edge to 1 at its top edge, and past them outside the plot.
+ `x2`: The second x position, through the scale of `x`, or a direct fraction of the plot's width, as for `x`.
+ `y2`: The second y position, through the scale of `y`, or a direct fraction of the plot's height, as for `y`.
+ `width`: The width of each image, in pixels. When it is not encoded, the styles give it.
+ `height`: The height of each image, in pixels. When it is not encoded, the styles give it.
+ `align`: The side of each image at its `x`.
+ `baseline`: The side of each image at its `y`.
+ `aspect`: Whether each image keeps its own aspect ratio inside its box. When unset, it keeps it.
+ `smooth`: Whether each image is filtered smoothly when it is scaled. When unset, it is.
+ `coord`: The options of the plot's coordinate system, such as `polar(inner_radius => 0.5)`.

**Clauses**: `ORDER BY`

## `path`

```googlesql
path(path: ANY[, x: ANY][, y: ANY][, color: ANY][, fill: ANY][, stroke: ANY][, opacity: ANY][, fill_opacity: ANY][, stroke_opacity: ANY][, stroke_width: ANY][, angle: ANY][, pattern: ANY][, coord: ANY]) -> BYTES
```

The SVG path of each row, in pixels, with its origin at the position of the row and turned by its `angle` about that origin.

The SVG path of each row, in pixels, with its origin at the position of the row and turned by its `angle` about that origin. A path without `x` or `y` has its origin at the left or the top edge of its plot.

**Parameters**

+ `path`: The SVG path of a row, in pixels from its position, without a scale.
+ `x`: The horizontal position of a Cartesian mark. Values: values of its scale's domain, and a direct value is a fraction of the plot's width, from 0 at its left edge to 1 at its right edge, and past them outside the plot, so it stands at one place of the plot at any size.
+ `y`: The vertical position of a Cartesian mark. Values: values of its scale's domain, and a direct value is a fraction of the plot's height, from 0 at its bottom edge to 1 at its top edge, and past them outside the plot.
+ `color`: The fill of a mark with a fill, and the stroke of a mark without one: a line, a trail, a rule, or a symbol that is not filled. Values: CSS color strings.
+ `fill`: Values: CSS color strings.
+ `stroke`: Values: CSS color strings.
+ `opacity`: A factor of the alpha of the fill and the stroke. Values: numbers from 0 to 1.
+ `fill_opacity`: A factor of the alpha of the fill. Values: numbers from 0 to 1.
+ `stroke_opacity`: A factor of the alpha of the stroke. Values: numbers from 0 to 1.
+ `stroke_width`: The width of a stroke. Values: numbers, in pixels.
+ `angle`: The rotation of a symbol, a text, or a path, in degrees clockwise. Values: numbers.
+ `pattern`: A pattern over the fill. Values: pattern text, such as a preset with an optional scale, `crosshatch` or `dots 2`, layers, such as `stripes 30 8 2 + circle 12 on hex 10`, or `none` or null for no pattern, in this grammar:

  ```text pattern  = ( "none" | preset | layers ) { ";" option } preset   = PRESET [ SCALE ] layers   = [ op ] layer { op layer } op       = "+" | "-" | "^" layer    = stripes | symbols stripes  = "stripes" ANGLE SPACING WIDTH [ "dash" LENGTH GAP [ PHASE ] ] [ "offset" PHASE ] symbols  = SHAPE SIZE [ "rotate" ANGLE ] [ "open" WIDTH ] "on" lattice lattice  = ( "grid" SPACING [ SPACING ] | "hex" SPACING | "lattice" SPACING ANGLE SPACING ANGLE ) [ "at" ANGLE ] [ "offset" PHASE PHASE ] option   = "ink" ( "auto" | COLOR ) [ OPACITY ] | "anchor" ( "group" | "mark" | "scene" ) ```

  Angles are in degrees and lengths in pixels. A symbol's `SIZE` is the area of its bounding square in square pixels, as for symbol marks, and its `SHAPE` is one of the shape names of `CHANNEL_SHAPE`. `COLOR` is a CSS color. Spaces separate the tokens, so an operator has a space on each side and `-45` is a number, and the spaces inside parentheses, as in `rgba(26, 51, 153, 0.5)`, belong to one token.

  The operators combine each layer with the coverage of the layers before it: `+` adds, `-` subtracts, and `^` takes the exclusive or. The clauses of a layer and the options can come in any order, each at most once. `grid` and `hex` give a lattice with steps at 0 and 90 degrees, and with equal steps at 0 and 60 degrees, and `at` turns both steps. `lattice` gives the spacing and the angle of each step. A pattern without an option takes its default: `ink auto 0.18` and `anchor group`. `none` is a pattern without layers.

  `PRESET` names a whole pattern, and its `SCALE`, 1 when it is left out, multiplies each length of the preset, and each symbol size, which is an area, by its square, so `dots 2` has dots twice as wide and twice as far apart:

  - `diagonal-down`: `stripes 45 8 2` - `diagonal-up`: `stripes -45 8 2` - `horizontal`: `stripes 0 8 2` - `vertical`: `stripes 90 8 2` - `crosshatch`: `stripes 45 8 1.5 + stripes -45 8 1.5` - `gridlines`: `stripes 0 8 1.5 + stripes 90 8 1.5` - `dashes`: `stripes 0 6 2 dash 6 3` - `dots`: `circle 16 on hex 8` - `rings`: `circle 36 open 1 on hex 10` - `triangles`: `triangle-up 30 on hex 10` - `checkerboard`: `stripes 0 8 4 ^ stripes 90 8 4` - `bricks`: `stripes 0 8 1.5 + stripes 90 16 1.5 dash 8 8 + stripes 90 16 1.5 dash 8 8 8 offset 8`
+ `coord`: The options of the plot's coordinate system, such as `polar(inner_radius => 0.5)`.

**Clauses**: `ORDER BY`

## `polar_symbol`

```googlesql
polar_symbol([theta: ANY][, radius: ANY][, color: ANY][, fill: ANY][, stroke: ANY][, opacity: ANY][, fill_opacity: ANY][, stroke_opacity: ANY][, stroke_width: ANY][, size: ANY][, shape: ANY][, angle: ANY][, pattern: ANY][, filled: BOOL][, coord: ANY]) -> BYTES
```

A symbol at each row of a polar plot.

**Parameters**

+ `theta`: The angle of a row in a polar plot, through its scale. Values: degrees clockwise from the top.
+ `radius`: The distance of a row from the center of a polar plot, through its scale. Values: fractions of the outer radius, from 0 at the center to 1 at the outer edge, and past 1 outside it, as the labels outside a pie are.
+ `color`: The fill of a mark with a fill, and the stroke of a mark without one: a line, a trail, a rule, or a symbol that is not filled. Values: CSS color strings.
+ `fill`: Values: CSS color strings.
+ `stroke`: Values: CSS color strings.
+ `opacity`: A factor of the alpha of the fill and the stroke. Values: numbers from 0 to 1.
+ `fill_opacity`: A factor of the alpha of the fill. Values: numbers from 0 to 1.
+ `stroke_opacity`: A factor of the alpha of the stroke. Values: numbers from 0 to 1.
+ `stroke_width`: The width of a stroke. Values: numbers, in pixels.
+ `size`: The area of each symbol, in square pixels.
+ `shape`: The shape of a symbol, or of the symbol of a point geometry. Values: the shape names `circle`, `square`, `cross`, `diamond`, `triangle-up`, `triangle-down`, `triangle-right`, `triangle-left`, `triangle`, `arrow`, `wedge`, `star`, `wye`, `pentagon`, and `star-4`, in any case, as Vega draws them, or SVG path data in a box from -1 to 1, which the symbol scales to the square of its size.
+ `angle`: The rotation of a symbol, a text, or a path, in degrees clockwise. Values: numbers.
+ `pattern`: A pattern over the fill. Values: pattern text, such as a preset with an optional scale, `crosshatch` or `dots 2`, layers, such as `stripes 30 8 2 + circle 12 on hex 10`, or `none` or null for no pattern, in this grammar:

  ```text pattern  = ( "none" | preset | layers ) { ";" option } preset   = PRESET [ SCALE ] layers   = [ op ] layer { op layer } op       = "+" | "-" | "^" layer    = stripes | symbols stripes  = "stripes" ANGLE SPACING WIDTH [ "dash" LENGTH GAP [ PHASE ] ] [ "offset" PHASE ] symbols  = SHAPE SIZE [ "rotate" ANGLE ] [ "open" WIDTH ] "on" lattice lattice  = ( "grid" SPACING [ SPACING ] | "hex" SPACING | "lattice" SPACING ANGLE SPACING ANGLE ) [ "at" ANGLE ] [ "offset" PHASE PHASE ] option   = "ink" ( "auto" | COLOR ) [ OPACITY ] | "anchor" ( "group" | "mark" | "scene" ) ```

  Angles are in degrees and lengths in pixels. A symbol's `SIZE` is the area of its bounding square in square pixels, as for symbol marks, and its `SHAPE` is one of the shape names of `CHANNEL_SHAPE`. `COLOR` is a CSS color. Spaces separate the tokens, so an operator has a space on each side and `-45` is a number, and the spaces inside parentheses, as in `rgba(26, 51, 153, 0.5)`, belong to one token.

  The operators combine each layer with the coverage of the layers before it: `+` adds, `-` subtracts, and `^` takes the exclusive or. The clauses of a layer and the options can come in any order, each at most once. `grid` and `hex` give a lattice with steps at 0 and 90 degrees, and with equal steps at 0 and 60 degrees, and `at` turns both steps. `lattice` gives the spacing and the angle of each step. A pattern without an option takes its default: `ink auto 0.18` and `anchor group`. `none` is a pattern without layers.

  `PRESET` names a whole pattern, and its `SCALE`, 1 when it is left out, multiplies each length of the preset, and each symbol size, which is an area, by its square, so `dots 2` has dots twice as wide and twice as far apart:

  - `diagonal-down`: `stripes 45 8 2` - `diagonal-up`: `stripes -45 8 2` - `horizontal`: `stripes 0 8 2` - `vertical`: `stripes 90 8 2` - `crosshatch`: `stripes 45 8 1.5 + stripes -45 8 1.5` - `gridlines`: `stripes 0 8 1.5 + stripes 90 8 1.5` - `dashes`: `stripes 0 6 2 dash 6 3` - `dots`: `circle 16 on hex 8` - `rings`: `circle 36 open 1 on hex 10` - `triangles`: `triangle-up 30 on hex 10` - `checkerboard`: `stripes 0 8 4 ^ stripes 90 8 4` - `bricks`: `stripes 0 8 1.5 + stripes 90 16 1.5 dash 8 8 + stripes 90 16 1.5 dash 8 8 8 offset 8`
+ `filled`: Whether the symbol has a fill. Without one, `color` colors the stroke of the symbol.
+ `coord`: The options of the plot's coordinate system, such as `polar(inner_radius => 0.5)`.

**Clauses**: `ORDER BY`

## `polar_line`

```googlesql
polar_line([theta: ANY][, radius: ANY][, color: ANY][, stroke: ANY][, opacity: ANY][, stroke_opacity: ANY][, stroke_width: ANY][, stroke_dash: ANY][, detail: ANY][, route: 'coordinate' | 'canvas'][, coord: ANY]) -> BYTES
```

A line through the rows of each series of a polar plot, in the order of the rows, as for `LineMark`.

A line through the rows of each series of a polar plot, in the order of the rows, as for `LineMark`. Each segment between two rows takes the line's route.

**Parameters**

+ `theta`: The angle of a row in a polar plot, through its scale. Values: degrees clockwise from the top.
+ `radius`: The distance of a row from the center of a polar plot, through its scale. Values: fractions of the outer radius, from 0 at the center to 1 at the outer edge, and past 1 outside it, as the labels outside a pie are.
+ `color`: The fill of a mark with a fill, and the stroke of a mark without one: a line, a trail, a rule, or a symbol that is not filled. Values: CSS color strings.
+ `stroke`: Values: CSS color strings.
+ `opacity`: A factor of the alpha of the fill and the stroke. Values: numbers from 0 to 1.
+ `stroke_opacity`: A factor of the alpha of the stroke. Values: numbers from 0 to 1.
+ `stroke_width`: The width of a stroke. Values: numbers, in pixels.
+ `stroke_dash`: The dash of a stroke, as alternating dash and gap lengths in pixels. Values: lists of numbers, or strings of numbers such as `4 2`. No lengths draws a solid stroke.
+ `detail`: Splits the rows of a line, a trail, or an area into series, without a scale.
+ `route`: How each segment between two points runs, as `Route` describes.
+ `coord`: The options of the plot's coordinate system, such as `polar(inner_radius => 0.5)`.

**Clauses**: `ORDER BY`

## `polar_trail`

```googlesql
polar_trail([theta: ANY][, radius: ANY][, color: ANY][, stroke: ANY][, opacity: ANY][, size: ANY][, detail: ANY][, route: 'coordinate' | 'canvas'][, coord: ANY]) -> BYTES
```

A trail through the rows of each series of a polar plot, as for `TrailMark`.

A trail through the rows of each series of a polar plot, as for `TrailMark`. Each segment between two rows takes the trail's route.

**Parameters**

+ `theta`: The angle of a row in a polar plot, through its scale. Values: degrees clockwise from the top.
+ `radius`: The distance of a row from the center of a polar plot, through its scale. Values: fractions of the outer radius, from 0 at the center to 1 at the outer edge, and past 1 outside it, as the labels outside a pie are.
+ `color`: The fill of a mark with a fill, and the stroke of a mark without one: a line, a trail, a rule, or a symbol that is not filled. Values: CSS color strings.
+ `stroke`: Values: CSS color strings.
+ `opacity`: A factor of the alpha of the fill and the stroke. Values: numbers from 0 to 1.
+ `size`: The width of the trail at each row, in pixels.
+ `detail`: Splits the rows of a line, a trail, or an area into series, without a scale.
+ `route`: How each segment between two points runs, as `Route` describes.
+ `coord`: The options of the plot's coordinate system, such as `polar(inner_radius => 0.5)`.

**Clauses**: `ORDER BY`

## `polar_area`

```googlesql
polar_area([theta: ANY][, radius: ANY][, theta2: ANY][, radius2: ANY][, color: ANY][, fill: ANY][, stroke: ANY][, opacity: ANY][, fill_opacity: ANY][, stroke_opacity: ANY][, stroke_width: ANY][, stroke_dash: ANY][, pattern: ANY][, detail: ANY][, orient: 'vertical' | 'horizontal'][, route: 'coordinate' | 'canvas'][, coord: ANY]) -> BYTES
```

The region of each series of a polar plot between a line and a baseline, as for `AreaMark`, where `theta` takes the place of `x` and `radius` the place of `y`, so a vertical area fills from `radius2`, or zero, to `radius` along `theta`, as the fill of a radar chart does.

The region of each series of a polar plot between a line and a baseline, as for `AreaMark`, where `theta` takes the place of `x` and `radius` the place of `y`, so a vertical area fills from `radius2`, or zero, to `radius` along `theta`, as the fill of a radar chart does. Each segment of its edges takes the area's route. The channels other than the positions take one value for each series.

**Parameters**

+ `theta`: The angle of a row in a polar plot, through its scale. Values: degrees clockwise from the top.
+ `radius`: The distance of a row from the center of a polar plot, through its scale. Values: fractions of the outer radius, from 0 at the center to 1 at the outer edge, and past 1 outside it, as the labels outside a pie are.
+ `theta2`: The second angle, through the scale of `theta`.
+ `radius2`: The second radius, through the scale of `radius`.
+ `color`: The fill of a mark with a fill, and the stroke of a mark without one: a line, a trail, a rule, or a symbol that is not filled. Values: CSS color strings.
+ `fill`: Values: CSS color strings.
+ `stroke`: Values: CSS color strings.
+ `opacity`: A factor of the alpha of the fill and the stroke. Values: numbers from 0 to 1.
+ `fill_opacity`: A factor of the alpha of the fill. Values: numbers from 0 to 1.
+ `stroke_opacity`: A factor of the alpha of the stroke. Values: numbers from 0 to 1.
+ `stroke_width`: The width of a stroke. Values: numbers, in pixels.
+ `stroke_dash`: The dash of a stroke, as alternating dash and gap lengths in pixels. Values: lists of numbers, or strings of numbers such as `4 2`. No lengths draws a solid stroke.
+ `pattern`: A pattern over the fill. Values: pattern text, such as a preset with an optional scale, `crosshatch` or `dots 2`, layers, such as `stripes 30 8 2 + circle 12 on hex 10`, or `none` or null for no pattern, in this grammar:

  ```text pattern  = ( "none" | preset | layers ) { ";" option } preset   = PRESET [ SCALE ] layers   = [ op ] layer { op layer } op       = "+" | "-" | "^" layer    = stripes | symbols stripes  = "stripes" ANGLE SPACING WIDTH [ "dash" LENGTH GAP [ PHASE ] ] [ "offset" PHASE ] symbols  = SHAPE SIZE [ "rotate" ANGLE ] [ "open" WIDTH ] "on" lattice lattice  = ( "grid" SPACING [ SPACING ] | "hex" SPACING | "lattice" SPACING ANGLE SPACING ANGLE ) [ "at" ANGLE ] [ "offset" PHASE PHASE ] option   = "ink" ( "auto" | COLOR ) [ OPACITY ] | "anchor" ( "group" | "mark" | "scene" ) ```

  Angles are in degrees and lengths in pixels. A symbol's `SIZE` is the area of its bounding square in square pixels, as for symbol marks, and its `SHAPE` is one of the shape names of `CHANNEL_SHAPE`. `COLOR` is a CSS color. Spaces separate the tokens, so an operator has a space on each side and `-45` is a number, and the spaces inside parentheses, as in `rgba(26, 51, 153, 0.5)`, belong to one token.

  The operators combine each layer with the coverage of the layers before it: `+` adds, `-` subtracts, and `^` takes the exclusive or. The clauses of a layer and the options can come in any order, each at most once. `grid` and `hex` give a lattice with steps at 0 and 90 degrees, and with equal steps at 0 and 60 degrees, and `at` turns both steps. `lattice` gives the spacing and the angle of each step. A pattern without an option takes its default: `ink auto 0.18` and `anchor group`. `none` is a pattern without layers.

  `PRESET` names a whole pattern, and its `SCALE`, 1 when it is left out, multiplies each length of the preset, and each symbol size, which is an area, by its square, so `dots 2` has dots twice as wide and twice as far apart:

  - `diagonal-down`: `stripes 45 8 2` - `diagonal-up`: `stripes -45 8 2` - `horizontal`: `stripes 0 8 2` - `vertical`: `stripes 90 8 2` - `crosshatch`: `stripes 45 8 1.5 + stripes -45 8 1.5` - `gridlines`: `stripes 0 8 1.5 + stripes 90 8 1.5` - `dashes`: `stripes 0 6 2 dash 6 3` - `dots`: `circle 16 on hex 8` - `rings`: `circle 36 open 1 on hex 10` - `triangles`: `triangle-up 30 on hex 10` - `checkerboard`: `stripes 0 8 4 ^ stripes 90 8 4` - `bricks`: `stripes 0 8 1.5 + stripes 90 16 1.5 dash 8 8 + stripes 90 16 1.5 dash 8 8 8 offset 8`
+ `detail`: Splits the rows of a line, a trail, or an area into series, without a scale.
+ `orient`: When unset, the area is vertical unless only its `theta` scale is continuous.
+ `route`: How each segment between two points runs, as `Route` describes.
+ `coord`: The options of the plot's coordinate system, such as `polar(inner_radius => 0.5)`.

**Clauses**: `ORDER BY`

## `polar_bar`

```googlesql
polar_bar([theta: ANY][, radius: ANY][, theta2: ANY][, radius2: ANY][, color: ANY][, fill: ANY][, stroke: ANY][, opacity: ANY][, fill_opacity: ANY][, stroke_opacity: ANY][, stroke_width: ANY][, pattern: ANY][, orient: 'vertical' | 'horizontal'][, corner_radius: ANY][, route: 'coordinate' | 'canvas'][, coord: ANY]) -> BYTES
```

A bar of a polar plot, as for `BarMark`, where `theta` takes the place of `x` and `radius` the place of `y`: a vertical bar along a band of `theta` runs out from its baseline to its `radius`, as a wedge of a rose chart does, and a horizontal bar along a band of `radius` runs from its baseline to its `theta`, as a bar of a radial bar chart does.

A bar of a polar plot, as for `BarMark`, where `theta` takes the place of `x` and `radius` the place of `y`: a vertical bar along a band of `theta` runs out from its baseline to its `radius`, as a wedge of a rose chart does, and a horizontal bar along a band of `radius` runs from its baseline to its `theta`, as a bar of a radial bar chart does. Its edges at one radius take its route.

**Parameters**

+ `theta`: The angle of a row in a polar plot, through its scale. Values: degrees clockwise from the top.
+ `radius`: The distance of a row from the center of a polar plot, through its scale. Values: fractions of the outer radius, from 0 at the center to 1 at the outer edge, and past 1 outside it, as the labels outside a pie are.
+ `theta2`: The second angle, through the scale of `theta`.
+ `radius2`: The second radius, through the scale of `radius`.
+ `color`: The fill of a mark with a fill, and the stroke of a mark without one: a line, a trail, a rule, or a symbol that is not filled. Values: CSS color strings.
+ `fill`: Values: CSS color strings.
+ `stroke`: Values: CSS color strings.
+ `opacity`: A factor of the alpha of the fill and the stroke. Values: numbers from 0 to 1.
+ `fill_opacity`: A factor of the alpha of the fill. Values: numbers from 0 to 1.
+ `stroke_opacity`: A factor of the alpha of the stroke. Values: numbers from 0 to 1.
+ `stroke_width`: The width of a stroke. Values: numbers, in pixels.
+ `pattern`: A pattern over the fill. Values: pattern text, such as a preset with an optional scale, `crosshatch` or `dots 2`, layers, such as `stripes 30 8 2 + circle 12 on hex 10`, or `none` or null for no pattern, in this grammar:

  ```text pattern  = ( "none" | preset | layers ) { ";" option } preset   = PRESET [ SCALE ] layers   = [ op ] layer { op layer } op       = "+" | "-" | "^" layer    = stripes | symbols stripes  = "stripes" ANGLE SPACING WIDTH [ "dash" LENGTH GAP [ PHASE ] ] [ "offset" PHASE ] symbols  = SHAPE SIZE [ "rotate" ANGLE ] [ "open" WIDTH ] "on" lattice lattice  = ( "grid" SPACING [ SPACING ] | "hex" SPACING | "lattice" SPACING ANGLE SPACING ANGLE ) [ "at" ANGLE ] [ "offset" PHASE PHASE ] option   = "ink" ( "auto" | COLOR ) [ OPACITY ] | "anchor" ( "group" | "mark" | "scene" ) ```

  Angles are in degrees and lengths in pixels. A symbol's `SIZE` is the area of its bounding square in square pixels, as for symbol marks, and its `SHAPE` is one of the shape names of `CHANNEL_SHAPE`. `COLOR` is a CSS color. Spaces separate the tokens, so an operator has a space on each side and `-45` is a number, and the spaces inside parentheses, as in `rgba(26, 51, 153, 0.5)`, belong to one token.

  The operators combine each layer with the coverage of the layers before it: `+` adds, `-` subtracts, and `^` takes the exclusive or. The clauses of a layer and the options can come in any order, each at most once. `grid` and `hex` give a lattice with steps at 0 and 90 degrees, and with equal steps at 0 and 60 degrees, and `at` turns both steps. `lattice` gives the spacing and the angle of each step. A pattern without an option takes its default: `ink auto 0.18` and `anchor group`. `none` is a pattern without layers.

  `PRESET` names a whole pattern, and its `SCALE`, 1 when it is left out, multiplies each length of the preset, and each symbol size, which is an area, by its square, so `dots 2` has dots twice as wide and twice as far apart:

  - `diagonal-down`: `stripes 45 8 2` - `diagonal-up`: `stripes -45 8 2` - `horizontal`: `stripes 0 8 2` - `vertical`: `stripes 90 8 2` - `crosshatch`: `stripes 45 8 1.5 + stripes -45 8 1.5` - `gridlines`: `stripes 0 8 1.5 + stripes 90 8 1.5` - `dashes`: `stripes 0 6 2 dash 6 3` - `dots`: `circle 16 on hex 8` - `rings`: `circle 36 open 1 on hex 10` - `triangles`: `triangle-up 30 on hex 10` - `checkerboard`: `stripes 0 8 4 ^ stripes 90 8 4` - `bricks`: `stripes 0 8 1.5 + stripes 90 16 1.5 dash 8 8 + stripes 90 16 1.5 dash 8 8 8 offset 8`
+ `orient`: When unset, the bar is vertical unless only its `theta` scale is continuous.
+ `corner_radius`: The radius of the corners of each bar, in pixels. When unset, the styles give it.
+ `route`: How each segment between two points runs, as `Route` describes.
+ `coord`: The options of the plot's coordinate system, such as `polar(inner_radius => 0.5)`.

**Clauses**: `ORDER BY`

## `polar_rect`

```googlesql
polar_rect([theta: ANY][, radius: ANY][, theta2: ANY][, radius2: ANY][, color: ANY][, fill: ANY][, stroke: ANY][, opacity: ANY][, fill_opacity: ANY][, stroke_opacity: ANY][, stroke_width: ANY][, pattern: ANY][, corner_radius: ANY][, route: 'coordinate' | 'canvas'][, inset: ANY][, coord: ANY]) -> BYTES
```

An annular sector from `theta` to `theta2` and from `radius` to `radius2`, such as a node of a sunburst, or across the band of `theta` or `radius` on a band scale without its second channel.

An annular sector from `theta` to `theta2` and from `radius` to `radius2`, such as a node of a sunburst, or across the band of `theta` or `radius` on a band scale without its second channel. A rect without `radius` spans the range of `radius`, as a slice of a pie or a donut does. Its edges at one radius take its route: arcs of rings by default, and straight lines with `ROUTE_CANVAS`, as the cells of the web behind a radar chart are.

**Parameters**

+ `theta`: The angle of a row in a polar plot, through its scale. Values: degrees clockwise from the top.
+ `radius`: The distance of a row from the center of a polar plot, through its scale. Values: fractions of the outer radius, from 0 at the center to 1 at the outer edge, and past 1 outside it, as the labels outside a pie are.
+ `theta2`: The second angle, through the scale of `theta`.
+ `radius2`: The second radius, through the scale of `radius`.
+ `color`: The fill of a mark with a fill, and the stroke of a mark without one: a line, a trail, a rule, or a symbol that is not filled. Values: CSS color strings.
+ `fill`: Values: CSS color strings.
+ `stroke`: Values: CSS color strings.
+ `opacity`: A factor of the alpha of the fill and the stroke. Values: numbers from 0 to 1.
+ `fill_opacity`: A factor of the alpha of the fill. Values: numbers from 0 to 1.
+ `stroke_opacity`: A factor of the alpha of the stroke. Values: numbers from 0 to 1.
+ `stroke_width`: The width of a stroke. Values: numbers, in pixels.
+ `pattern`: A pattern over the fill. Values: pattern text, such as a preset with an optional scale, `crosshatch` or `dots 2`, layers, such as `stripes 30 8 2 + circle 12 on hex 10`, or `none` or null for no pattern, in this grammar:

  ```text pattern  = ( "none" | preset | layers ) { ";" option } preset   = PRESET [ SCALE ] layers   = [ op ] layer { op layer } op       = "+" | "-" | "^" layer    = stripes | symbols stripes  = "stripes" ANGLE SPACING WIDTH [ "dash" LENGTH GAP [ PHASE ] ] [ "offset" PHASE ] symbols  = SHAPE SIZE [ "rotate" ANGLE ] [ "open" WIDTH ] "on" lattice lattice  = ( "grid" SPACING [ SPACING ] | "hex" SPACING | "lattice" SPACING ANGLE SPACING ANGLE ) [ "at" ANGLE ] [ "offset" PHASE PHASE ] option   = "ink" ( "auto" | COLOR ) [ OPACITY ] | "anchor" ( "group" | "mark" | "scene" ) ```

  Angles are in degrees and lengths in pixels. A symbol's `SIZE` is the area of its bounding square in square pixels, as for symbol marks, and its `SHAPE` is one of the shape names of `CHANNEL_SHAPE`. `COLOR` is a CSS color. Spaces separate the tokens, so an operator has a space on each side and `-45` is a number, and the spaces inside parentheses, as in `rgba(26, 51, 153, 0.5)`, belong to one token.

  The operators combine each layer with the coverage of the layers before it: `+` adds, `-` subtracts, and `^` takes the exclusive or. The clauses of a layer and the options can come in any order, each at most once. `grid` and `hex` give a lattice with steps at 0 and 90 degrees, and with equal steps at 0 and 60 degrees, and `at` turns both steps. `lattice` gives the spacing and the angle of each step. A pattern without an option takes its default: `ink auto 0.18` and `anchor group`. `none` is a pattern without layers.

  `PRESET` names a whole pattern, and its `SCALE`, 1 when it is left out, multiplies each length of the preset, and each symbol size, which is an area, by its square, so `dots 2` has dots twice as wide and twice as far apart:

  - `diagonal-down`: `stripes 45 8 2` - `diagonal-up`: `stripes -45 8 2` - `horizontal`: `stripes 0 8 2` - `vertical`: `stripes 90 8 2` - `crosshatch`: `stripes 45 8 1.5 + stripes -45 8 1.5` - `gridlines`: `stripes 0 8 1.5 + stripes 90 8 1.5` - `dashes`: `stripes 0 6 2 dash 6 3` - `dots`: `circle 16 on hex 8` - `rings`: `circle 36 open 1 on hex 10` - `triangles`: `triangle-up 30 on hex 10` - `checkerboard`: `stripes 0 8 4 ^ stripes 90 8 4` - `bricks`: `stripes 0 8 1.5 + stripes 90 16 1.5 dash 8 8 + stripes 90 16 1.5 dash 8 8 8 offset 8`
+ `corner_radius`: The radius of the corners of each rect, in pixels. When unset, the styles give it.
+ `route`: How each segment between two points runs, as `Route` describes.
+ `inset`: The space that the rect gives up at each side, in pixels, so neighboring rects stand twice as far apart, as the bins of a histogram or the slices of a pie do. The gap between two slices is as wide at the center as at the edge.
+ `coord`: The options of the plot's coordinate system, such as `polar(inner_radius => 0.5)`.

**Clauses**: `ORDER BY`

## `polar_rule`

```googlesql
polar_rule([theta: ANY][, radius: ANY][, theta2: ANY][, radius2: ANY][, color: ANY][, stroke: ANY][, opacity: ANY][, stroke_width: ANY][, stroke_dash: ANY][, route: 'coordinate' | 'canvas'][, coord: ANY]) -> BYTES
```

A line segment from (`theta`, `radius`) to (`theta2`, `radius2`), where a missing `theta2` takes the value of `theta` and a missing `radius2` takes the value of `radius`.

A line segment from (`theta`, `radius`) to (`theta2`, `radius2`), where a missing `theta2` takes the value of `theta` and a missing `radius2` takes the value of `radius`. A rule without `theta` is a ring at its radius around the whole range of `theta`, and one without `radius` is a spoke across the range of `radius`, from the inner radius to the outer edge.

A rule whose second value of a coordinate is missing holds that coordinate and runs along its ring or its spoke, whatever its route. A rule with both second values takes its route between its ends.

**Parameters**

+ `theta`: The angle of a row in a polar plot, through its scale. Values: degrees clockwise from the top.
+ `radius`: The distance of a row from the center of a polar plot, through its scale. Values: fractions of the outer radius, from 0 at the center to 1 at the outer edge, and past 1 outside it, as the labels outside a pie are.
+ `theta2`: The second angle, through the scale of `theta`.
+ `radius2`: The second radius, through the scale of `radius`.
+ `color`: The fill of a mark with a fill, and the stroke of a mark without one: a line, a trail, a rule, or a symbol that is not filled. Values: CSS color strings.
+ `stroke`: Values: CSS color strings.
+ `opacity`: A factor of the alpha of the fill and the stroke. Values: numbers from 0 to 1.
+ `stroke_width`: The width of a stroke. Values: numbers, in pixels.
+ `stroke_dash`: The dash of a stroke, as alternating dash and gap lengths in pixels. Values: lists of numbers, or strings of numbers such as `4 2`. No lengths draws a solid stroke.
+ `route`: How each segment between two points runs, as `Route` describes.
+ `coord`: The options of the plot's coordinate system, such as `polar(inner_radius => 0.5)`.

**Clauses**: `ORDER BY`

## `polar_text`

```googlesql
polar_text(text: ANY[, theta: ANY][, radius: ANY][, color: ANY][, fill: ANY][, opacity: ANY][, size: ANY][, font_weight: ANY][, font_style: ANY][, width: ANY][, format: STRING][, font_family: STRING][, syntax: 'plain' | 'plain_lines' | 'typst'][, width_mode: 'max' | 'fixed'][, wrap: BOOL][, max_lines: INT64][, ellipsis: BOOL][, turn: 'radial' | 'tangential'][, radius_side: 'outward' | 'middle' | 'inward'][, theta_side: 'clockwise' | 'middle' | 'counterclockwise'][, coord: ANY]) -> BYTES
```

The `text` value of each row of a polar plot.

A text lies beside its point as `radius_side` and `theta_side` say, which refer to the circle, so a text lies alike on every side of it.

The `text` value of each row of a polar plot.

A text lies beside its point as `radius_side` and `theta_side` say, which refer to the circle, so a text lies alike on every side of it. By default, a text stands outward from its point: centered on the ray from the center through the point, with its edge nearest the center at the point, so a label outside a pie reads clear of the pie at every angle, beside it at the sides and above or below it at the top and the bottom. A tangential text runs along the circle through its point and keeps to its sides of the circle and of the angle on both halves. A text at the center centers on it.

**Parameters**

+ `text`: The text of a row, which the text mark formats, without a scale.
+ `theta`: The angle of a row in a polar plot, through its scale. Values: degrees clockwise from the top.
+ `radius`: The distance of a row from the center of a polar plot, through its scale. Values: fractions of the outer radius, from 0 at the center to 1 at the outer edge, and past 1 outside it, as the labels outside a pie are.
+ `color`: The fill of a mark with a fill, and the stroke of a mark without one: a line, a trail, a rule, or a symbol that is not filled. Values: CSS color strings.
+ `fill`: Values: CSS color strings.
+ `opacity`: A factor of the alpha of the fill and the stroke. Values: numbers from 0 to 1.
+ `size`: The font size of each text, in pixels.
+ `font_weight`: The weight of the font of a text. Values: numbers from 100 to 900, or `normal`, which is 400, and `bold`, which is 700, as in CSS, without a scale.
+ `font_style`: The style of the font of a text. Values: `normal`, `italic`, or `oblique`, without a scale.
+ `width`: The width of the box of each text, in pixels, at which its lines wrap. When it is not encoded, a text is as wide as its widest line and ends lines only at its line breaks.
+ `format`: The pattern of a numeric or temporal `text` value, as for `Labels.format`.
+ `font_family`: The font family of the texts. When it is empty, the styles give it.
+ `syntax`: How the text engine reads each text.
+ `width_mode`: Whether `width` is the most that the box of each text takes, or exactly its width. When unset, the width is a maximum.
+ `wrap`: Whether the lines of each text wrap at its `width`. When unset, a text with a `width` wraps.
+ `max_lines`: The most lines that each text keeps. When unset, it keeps all of them.
+ `ellipsis`: Whether an ellipsis marks a text that its `width` or `max_lines` cuts. When unset, none does.
+ `turn`: How each text turns: along the circle or a spoke, as an orient says, or by the angle of each row. When both are unset, each text is horizontal.
+ `radius_side`: Where each text lies from the radius of its point.
+ `theta_side`: Where each text lies from the angle of its point.
+ `coord`: The options of the plot's coordinate system, such as `polar(inner_radius => 0.5)`.

**Clauses**: `ORDER BY`

## `geo_symbol`

```googlesql
geo_symbol([longitude: ANY][, latitude: ANY][, color: ANY][, fill: ANY][, stroke: ANY][, opacity: ANY][, fill_opacity: ANY][, stroke_opacity: ANY][, stroke_width: ANY][, size: ANY][, shape: ANY][, angle: ANY][, pattern: ANY][, filled: BOOL][, coord: ANY]) -> BYTES
```

A symbol at each row of a geo plot.

**Parameters**

+ `longitude`: The longitude of a row in degrees, or its first coordinate in a planar geo plot, without a scale.
+ `latitude`: The latitude of a row in degrees, or its second coordinate in a planar geo plot, without a scale.
+ `color`: The fill of a mark with a fill, and the stroke of a mark without one: a line, a trail, a rule, or a symbol that is not filled. Values: CSS color strings.
+ `fill`: Values: CSS color strings.
+ `stroke`: Values: CSS color strings.
+ `opacity`: A factor of the alpha of the fill and the stroke. Values: numbers from 0 to 1.
+ `fill_opacity`: A factor of the alpha of the fill. Values: numbers from 0 to 1.
+ `stroke_opacity`: A factor of the alpha of the stroke. Values: numbers from 0 to 1.
+ `stroke_width`: The width of a stroke. Values: numbers, in pixels.
+ `size`: The area of each symbol, in square pixels.
+ `shape`: The shape of a symbol, or of the symbol of a point geometry. Values: the shape names `circle`, `square`, `cross`, `diamond`, `triangle-up`, `triangle-down`, `triangle-right`, `triangle-left`, `triangle`, `arrow`, `wedge`, `star`, `wye`, `pentagon`, and `star-4`, in any case, as Vega draws them, or SVG path data in a box from -1 to 1, which the symbol scales to the square of its size.
+ `angle`: The rotation of a symbol, a text, or a path, in degrees clockwise. Values: numbers.
+ `pattern`: A pattern over the fill. Values: pattern text, such as a preset with an optional scale, `crosshatch` or `dots 2`, layers, such as `stripes 30 8 2 + circle 12 on hex 10`, or `none` or null for no pattern, in this grammar:

  ```text pattern  = ( "none" | preset | layers ) { ";" option } preset   = PRESET [ SCALE ] layers   = [ op ] layer { op layer } op       = "+" | "-" | "^" layer    = stripes | symbols stripes  = "stripes" ANGLE SPACING WIDTH [ "dash" LENGTH GAP [ PHASE ] ] [ "offset" PHASE ] symbols  = SHAPE SIZE [ "rotate" ANGLE ] [ "open" WIDTH ] "on" lattice lattice  = ( "grid" SPACING [ SPACING ] | "hex" SPACING | "lattice" SPACING ANGLE SPACING ANGLE ) [ "at" ANGLE ] [ "offset" PHASE PHASE ] option   = "ink" ( "auto" | COLOR ) [ OPACITY ] | "anchor" ( "group" | "mark" | "scene" ) ```

  Angles are in degrees and lengths in pixels. A symbol's `SIZE` is the area of its bounding square in square pixels, as for symbol marks, and its `SHAPE` is one of the shape names of `CHANNEL_SHAPE`. `COLOR` is a CSS color. Spaces separate the tokens, so an operator has a space on each side and `-45` is a number, and the spaces inside parentheses, as in `rgba(26, 51, 153, 0.5)`, belong to one token.

  The operators combine each layer with the coverage of the layers before it: `+` adds, `-` subtracts, and `^` takes the exclusive or. The clauses of a layer and the options can come in any order, each at most once. `grid` and `hex` give a lattice with steps at 0 and 90 degrees, and with equal steps at 0 and 60 degrees, and `at` turns both steps. `lattice` gives the spacing and the angle of each step. A pattern without an option takes its default: `ink auto 0.18` and `anchor group`. `none` is a pattern without layers.

  `PRESET` names a whole pattern, and its `SCALE`, 1 when it is left out, multiplies each length of the preset, and each symbol size, which is an area, by its square, so `dots 2` has dots twice as wide and twice as far apart:

  - `diagonal-down`: `stripes 45 8 2` - `diagonal-up`: `stripes -45 8 2` - `horizontal`: `stripes 0 8 2` - `vertical`: `stripes 90 8 2` - `crosshatch`: `stripes 45 8 1.5 + stripes -45 8 1.5` - `gridlines`: `stripes 0 8 1.5 + stripes 90 8 1.5` - `dashes`: `stripes 0 6 2 dash 6 3` - `dots`: `circle 16 on hex 8` - `rings`: `circle 36 open 1 on hex 10` - `triangles`: `triangle-up 30 on hex 10` - `checkerboard`: `stripes 0 8 4 ^ stripes 90 8 4` - `bricks`: `stripes 0 8 1.5 + stripes 90 16 1.5 dash 8 8 + stripes 90 16 1.5 dash 8 8 8 offset 8`
+ `filled`: Whether the symbol has a fill. Without one, `color` colors the stroke of the symbol.
+ `coord`: The options of the plot's coordinate system, such as `polar(inner_radius => 0.5)`.

**Clauses**: `ORDER BY`

## `geo_line`

```googlesql
geo_line([longitude: ANY][, latitude: ANY][, color: ANY][, stroke: ANY][, opacity: ANY][, stroke_opacity: ANY][, stroke_width: ANY][, stroke_dash: ANY][, detail: ANY][, route: 'coordinate' | 'canvas'][, coord: ANY]) -> BYTES
```

A line through the rows of each series of a geo plot, in the order of the rows, as for `LineMark`.

A line through the rows of each series of a geo plot, in the order of the rows, as for `LineMark`. Each segment between two rows takes the line's route, so by default it follows the great circle between them.

**Parameters**

+ `longitude`: The longitude of a row in degrees, or its first coordinate in a planar geo plot, without a scale.
+ `latitude`: The latitude of a row in degrees, or its second coordinate in a planar geo plot, without a scale.
+ `color`: The fill of a mark with a fill, and the stroke of a mark without one: a line, a trail, a rule, or a symbol that is not filled. Values: CSS color strings.
+ `stroke`: Values: CSS color strings.
+ `opacity`: A factor of the alpha of the fill and the stroke. Values: numbers from 0 to 1.
+ `stroke_opacity`: A factor of the alpha of the stroke. Values: numbers from 0 to 1.
+ `stroke_width`: The width of a stroke. Values: numbers, in pixels.
+ `stroke_dash`: The dash of a stroke, as alternating dash and gap lengths in pixels. Values: lists of numbers, or strings of numbers such as `4 2`. No lengths draws a solid stroke.
+ `detail`: Splits the rows of a line, a trail, or an area into series, without a scale.
+ `route`: How each segment between two points runs, as `Route` describes.
+ `coord`: The options of the plot's coordinate system, such as `polar(inner_radius => 0.5)`.

**Clauses**: `ORDER BY`

## `geo_trail`

```googlesql
geo_trail([longitude: ANY][, latitude: ANY][, color: ANY][, stroke: ANY][, opacity: ANY][, size: ANY][, detail: ANY][, route: 'coordinate' | 'canvas'][, coord: ANY]) -> BYTES
```

A trail through the rows of each series of a geo plot, as for `TrailMark`.

A trail through the rows of each series of a geo plot, as for `TrailMark`. Each segment between two rows takes the trail's route, so by default it follows the great circle between them.

**Parameters**

+ `longitude`: The longitude of a row in degrees, or its first coordinate in a planar geo plot, without a scale.
+ `latitude`: The latitude of a row in degrees, or its second coordinate in a planar geo plot, without a scale.
+ `color`: The fill of a mark with a fill, and the stroke of a mark without one: a line, a trail, a rule, or a symbol that is not filled. Values: CSS color strings.
+ `stroke`: Values: CSS color strings.
+ `opacity`: A factor of the alpha of the fill and the stroke. Values: numbers from 0 to 1.
+ `size`: The width of the trail at each row, in pixels.
+ `detail`: Splits the rows of a line, a trail, or an area into series, without a scale.
+ `route`: How each segment between two points runs, as `Route` describes.
+ `coord`: The options of the plot's coordinate system, such as `polar(inner_radius => 0.5)`.

**Clauses**: `ORDER BY`

## `geo_rule`

```googlesql
geo_rule(longitude: ANY, latitude: ANY[, longitude2: ANY][, latitude2: ANY][, color: ANY][, stroke: ANY][, opacity: ANY][, stroke_width: ANY][, stroke_dash: ANY][, route: 'coordinate' | 'canvas'][, coord: ANY]) -> BYTES
```

A line segment from (`longitude`, `latitude`) to (`longitude2`, `latitude2`), where a missing `longitude2` takes the value of `longitude` and a missing `latitude2` takes the value of `latitude`.

A line segment from (`longitude`, `latitude`) to (`longitude2`, `latitude2`), where a missing `longitude2` takes the value of `longitude` and a missing `latitude2` takes the value of `latitude`. A geo rule needs `longitude` and `latitude`.

A rule whose second value of a coordinate is missing holds that coordinate and runs along its parallel or its meridian, whatever its route, so a rule with `longitude2` and without `latitude2` follows the parallel between its two longitudes. A rule with both second values takes its route between its ends.

**Parameters**

+ `longitude`: The longitude of a row in degrees, or its first coordinate in a planar geo plot, without a scale.
+ `latitude`: The latitude of a row in degrees, or its second coordinate in a planar geo plot, without a scale.
+ `longitude2`: The second longitude, without a scale.
+ `latitude2`: The second latitude, without a scale.
+ `color`: The fill of a mark with a fill, and the stroke of a mark without one: a line, a trail, a rule, or a symbol that is not filled. Values: CSS color strings.
+ `stroke`: Values: CSS color strings.
+ `opacity`: A factor of the alpha of the fill and the stroke. Values: numbers from 0 to 1.
+ `stroke_width`: The width of a stroke. Values: numbers, in pixels.
+ `stroke_dash`: The dash of a stroke, as alternating dash and gap lengths in pixels. Values: lists of numbers, or strings of numbers such as `4 2`. No lengths draws a solid stroke.
+ `route`: How each segment between two points runs, as `Route` describes.
+ `coord`: The options of the plot's coordinate system, such as `polar(inner_radius => 0.5)`.

**Clauses**: `ORDER BY`

## `geo_text`

```googlesql
geo_text(text: ANY[, longitude: ANY][, latitude: ANY][, color: ANY][, fill: ANY][, opacity: ANY][, size: ANY][, angle: ANY][, font_weight: ANY][, font_style: ANY][, width: ANY][, format: STRING][, align: 'left' | 'center' | 'right'][, baseline: 'top' | 'middle' | 'alphabetic' | 'bottom'][, font_family: STRING][, syntax: 'plain' | 'plain_lines' | 'typst'][, width_mode: 'max' | 'fixed'][, wrap: BOOL][, max_lines: INT64][, ellipsis: BOOL][, coord: ANY]) -> BYTES
```

The `text` value of each row of a geo plot.

A text aligns to its position as its `align` and its `baseline` say, centered by default.

**Parameters**

+ `text`: The text of a row, which the text mark formats, without a scale.
+ `longitude`: The longitude of a row in degrees, or its first coordinate in a planar geo plot, without a scale.
+ `latitude`: The latitude of a row in degrees, or its second coordinate in a planar geo plot, without a scale.
+ `color`: The fill of a mark with a fill, and the stroke of a mark without one: a line, a trail, a rule, or a symbol that is not filled. Values: CSS color strings.
+ `fill`: Values: CSS color strings.
+ `opacity`: A factor of the alpha of the fill and the stroke. Values: numbers from 0 to 1.
+ `size`: The font size of each text, in pixels.
+ `angle`: The rotation of a symbol, a text, or a path, in degrees clockwise. Values: numbers.
+ `font_weight`: The weight of the font of a text. Values: numbers from 100 to 900, or `normal`, which is 400, and `bold`, which is 700, as in CSS, without a scale.
+ `font_style`: The style of the font of a text. Values: `normal`, `italic`, or `oblique`, without a scale.
+ `width`: The width of the box of each text, in pixels, at which its lines wrap. When it is not encoded, a text is as wide as its widest line and ends lines only at its line breaks.
+ `format`: The pattern of a numeric or temporal `text` value, as for `Labels.format`.
+ `align`: How each text aligns to its position across its lines. When unset, centered.
+ `baseline`: How each text aligns to its position along its lines. When unset, at its middle.
+ `font_family`: The font family of the texts. When it is empty, the styles give it.
+ `syntax`: How the text engine reads each text.
+ `width_mode`: Whether `width` is the most that the box of each text takes, or exactly its width. When unset, the width is a maximum.
+ `wrap`: Whether the lines of each text wrap at its `width`. When unset, a text with a `width` wraps.
+ `max_lines`: The most lines that each text keeps. When unset, it keeps all of them.
+ `ellipsis`: Whether an ellipsis marks a text that its `width` or `max_lines` cuts. When unset, none does.
+ `coord`: The options of the plot's coordinate system, such as `polar(inner_radius => 0.5)`.

**Clauses**: `ORDER BY`

## `geo_image`

```googlesql
geo_image(image: ANY[, longitude: ANY][, latitude: ANY][, width: ANY][, height: ANY][, align: 'left' | 'center' | 'right'][, baseline: 'top' | 'middle' | 'bottom'][, aspect: BOOL][, smooth: BOOL][, coord: ANY]) -> BYTES
```

The `image` value of each row of a geo plot, at the projected point of its `longitude` and `latitude`.

**Parameters**

+ `image`: The image of a row, without a scale: a URL, or a struct of `width` (UInt32), `height` (UInt32), and `data` (a list of the UInt8 RGBA bytes of its pixels).
+ `longitude`: The longitude of a row in degrees, or its first coordinate in a planar geo plot, without a scale.
+ `latitude`: The latitude of a row in degrees, or its second coordinate in a planar geo plot, without a scale.
+ `width`: The width of each image, in pixels. When it is not encoded, the styles give it.
+ `height`: The height of each image, in pixels. When it is not encoded, the styles give it.
+ `align`: The side of each image at its `longitude`.
+ `baseline`: The side of each image at its `latitude`.
+ `aspect`: Whether each image keeps its own aspect ratio inside its box. When unset, it keeps it.
+ `smooth`: Whether each image is filtered smoothly when it is scaled. When unset, it is.
+ `coord`: The options of the plot's coordinate system, such as `polar(inner_radius => 0.5)`.

**Clauses**: `ORDER BY`

## `geo_path`

```googlesql
geo_path(path: ANY[, longitude: ANY][, latitude: ANY][, color: ANY][, fill: ANY][, stroke: ANY][, opacity: ANY][, fill_opacity: ANY][, stroke_opacity: ANY][, stroke_width: ANY][, angle: ANY][, pattern: ANY][, coord: ANY]) -> BYTES
```

The SVG path of each row of a geo plot, in pixels, with its origin at the projected point of the row and turned by its `angle` about that origin.

**Parameters**

+ `path`: The SVG path of a row, in pixels from its position, without a scale.
+ `longitude`: The longitude of a row in degrees, or its first coordinate in a planar geo plot, without a scale.
+ `latitude`: The latitude of a row in degrees, or its second coordinate in a planar geo plot, without a scale.
+ `color`: The fill of a mark with a fill, and the stroke of a mark without one: a line, a trail, a rule, or a symbol that is not filled. Values: CSS color strings.
+ `fill`: Values: CSS color strings.
+ `stroke`: Values: CSS color strings.
+ `opacity`: A factor of the alpha of the fill and the stroke. Values: numbers from 0 to 1.
+ `fill_opacity`: A factor of the alpha of the fill. Values: numbers from 0 to 1.
+ `stroke_opacity`: A factor of the alpha of the stroke. Values: numbers from 0 to 1.
+ `stroke_width`: The width of a stroke. Values: numbers, in pixels.
+ `angle`: The rotation of a symbol, a text, or a path, in degrees clockwise. Values: numbers.
+ `pattern`: A pattern over the fill. Values: pattern text, such as a preset with an optional scale, `crosshatch` or `dots 2`, layers, such as `stripes 30 8 2 + circle 12 on hex 10`, or `none` or null for no pattern, in this grammar:

  ```text pattern  = ( "none" | preset | layers ) { ";" option } preset   = PRESET [ SCALE ] layers   = [ op ] layer { op layer } op       = "+" | "-" | "^" layer    = stripes | symbols stripes  = "stripes" ANGLE SPACING WIDTH [ "dash" LENGTH GAP [ PHASE ] ] [ "offset" PHASE ] symbols  = SHAPE SIZE [ "rotate" ANGLE ] [ "open" WIDTH ] "on" lattice lattice  = ( "grid" SPACING [ SPACING ] | "hex" SPACING | "lattice" SPACING ANGLE SPACING ANGLE ) [ "at" ANGLE ] [ "offset" PHASE PHASE ] option   = "ink" ( "auto" | COLOR ) [ OPACITY ] | "anchor" ( "group" | "mark" | "scene" ) ```

  Angles are in degrees and lengths in pixels. A symbol's `SIZE` is the area of its bounding square in square pixels, as for symbol marks, and its `SHAPE` is one of the shape names of `CHANNEL_SHAPE`. `COLOR` is a CSS color. Spaces separate the tokens, so an operator has a space on each side and `-45` is a number, and the spaces inside parentheses, as in `rgba(26, 51, 153, 0.5)`, belong to one token.

  The operators combine each layer with the coverage of the layers before it: `+` adds, `-` subtracts, and `^` takes the exclusive or. The clauses of a layer and the options can come in any order, each at most once. `grid` and `hex` give a lattice with steps at 0 and 90 degrees, and with equal steps at 0 and 60 degrees, and `at` turns both steps. `lattice` gives the spacing and the angle of each step. A pattern without an option takes its default: `ink auto 0.18` and `anchor group`. `none` is a pattern without layers.

  `PRESET` names a whole pattern, and its `SCALE`, 1 when it is left out, multiplies each length of the preset, and each symbol size, which is an area, by its square, so `dots 2` has dots twice as wide and twice as far apart:

  - `diagonal-down`: `stripes 45 8 2` - `diagonal-up`: `stripes -45 8 2` - `horizontal`: `stripes 0 8 2` - `vertical`: `stripes 90 8 2` - `crosshatch`: `stripes 45 8 1.5 + stripes -45 8 1.5` - `gridlines`: `stripes 0 8 1.5 + stripes 90 8 1.5` - `dashes`: `stripes 0 6 2 dash 6 3` - `dots`: `circle 16 on hex 8` - `rings`: `circle 36 open 1 on hex 10` - `triangles`: `triangle-up 30 on hex 10` - `checkerboard`: `stripes 0 8 4 ^ stripes 90 8 4` - `bricks`: `stripes 0 8 1.5 + stripes 90 16 1.5 dash 8 8 + stripes 90 16 1.5 dash 8 8 8 offset 8`
+ `coord`: The options of the plot's coordinate system, such as `polar(inner_radius => 0.5)`.

**Clauses**: `ORDER BY`

## `geo_shape`

```googlesql
geo_shape(geometry: ANY[, color: ANY][, fill: ANY][, stroke: ANY][, opacity: ANY][, fill_opacity: ANY][, stroke_opacity: ANY][, stroke_width: ANY][, size: ANY][, shape: ANY][, pattern: ANY][, coord: ANY]) -> BYTES
```

The geometry of each row: its polygons filled and stroked, its lines stroked, and its points drawn as symbols.

The geometry of each row: its polygons filled and stroked, its lines stroked, and its points drawn as symbols. `color`, `fill`, `fill_opacity`, and `pattern` apply to polygons and points, and `size` and `shape` apply to points.

**Parameters**

+ `geometry`: The geometry of a row, from a GeoArrow column in any encoding, without a scale.
+ `color`: The fill of a mark with a fill, and the stroke of a mark without one: a line, a trail, a rule, or a symbol that is not filled. Values: CSS color strings.
+ `fill`: Values: CSS color strings.
+ `stroke`: Values: CSS color strings.
+ `opacity`: A factor of the alpha of the fill and the stroke. Values: numbers from 0 to 1.
+ `fill_opacity`: A factor of the alpha of the fill. Values: numbers from 0 to 1.
+ `stroke_opacity`: A factor of the alpha of the stroke. Values: numbers from 0 to 1.
+ `stroke_width`: The width of a stroke. Values: numbers, in pixels.
+ `size`: The area of the symbol of each point geometry, in square pixels.
+ `shape`: The shape of a symbol, or of the symbol of a point geometry. Values: the shape names `circle`, `square`, `cross`, `diamond`, `triangle-up`, `triangle-down`, `triangle-right`, `triangle-left`, `triangle`, `arrow`, `wedge`, `star`, `wye`, `pentagon`, and `star-4`, in any case, as Vega draws them, or SVG path data in a box from -1 to 1, which the symbol scales to the square of its size.
+ `pattern`: A pattern over the fill. Values: pattern text, such as a preset with an optional scale, `crosshatch` or `dots 2`, layers, such as `stripes 30 8 2 + circle 12 on hex 10`, or `none` or null for no pattern, in this grammar:

  ```text pattern  = ( "none" | preset | layers ) { ";" option } preset   = PRESET [ SCALE ] layers   = [ op ] layer { op layer } op       = "+" | "-" | "^" layer    = stripes | symbols stripes  = "stripes" ANGLE SPACING WIDTH [ "dash" LENGTH GAP [ PHASE ] ] [ "offset" PHASE ] symbols  = SHAPE SIZE [ "rotate" ANGLE ] [ "open" WIDTH ] "on" lattice lattice  = ( "grid" SPACING [ SPACING ] | "hex" SPACING | "lattice" SPACING ANGLE SPACING ANGLE ) [ "at" ANGLE ] [ "offset" PHASE PHASE ] option   = "ink" ( "auto" | COLOR ) [ OPACITY ] | "anchor" ( "group" | "mark" | "scene" ) ```

  Angles are in degrees and lengths in pixels. A symbol's `SIZE` is the area of its bounding square in square pixels, as for symbol marks, and its `SHAPE` is one of the shape names of `CHANNEL_SHAPE`. `COLOR` is a CSS color. Spaces separate the tokens, so an operator has a space on each side and `-45` is a number, and the spaces inside parentheses, as in `rgba(26, 51, 153, 0.5)`, belong to one token.

  The operators combine each layer with the coverage of the layers before it: `+` adds, `-` subtracts, and `^` takes the exclusive or. The clauses of a layer and the options can come in any order, each at most once. `grid` and `hex` give a lattice with steps at 0 and 90 degrees, and with equal steps at 0 and 60 degrees, and `at` turns both steps. `lattice` gives the spacing and the angle of each step. A pattern without an option takes its default: `ink auto 0.18` and `anchor group`. `none` is a pattern without layers.

  `PRESET` names a whole pattern, and its `SCALE`, 1 when it is left out, multiplies each length of the preset, and each symbol size, which is an area, by its square, so `dots 2` has dots twice as wide and twice as far apart:

  - `diagonal-down`: `stripes 45 8 2` - `diagonal-up`: `stripes -45 8 2` - `horizontal`: `stripes 0 8 2` - `vertical`: `stripes 90 8 2` - `crosshatch`: `stripes 45 8 1.5 + stripes -45 8 1.5` - `gridlines`: `stripes 0 8 1.5 + stripes 90 8 1.5` - `dashes`: `stripes 0 6 2 dash 6 3` - `dots`: `circle 16 on hex 8` - `rings`: `circle 36 open 1 on hex 10` - `triangles`: `triangle-up 30 on hex 10` - `checkerboard`: `stripes 0 8 4 ^ stripes 90 8 4` - `bricks`: `stripes 0 8 1.5 + stripes 90 16 1.5 dash 8 8 + stripes 90 16 1.5 dash 8 8 8 offset 8`
+ `coord`: The options of the plot's coordinate system, such as `polar(inner_radius => 0.5)`.

**Clauses**: `ORDER BY`

## `geo`

```googlesql
geo(surface: ANY[, family: STRING][, aspect: ANY][, navigation: BYTES]) -> BYTES
```

A geographic coordinate system: a map projection of `longitude`, `latitude`, `longitude2`, `latitude2`, and `geometry`, for the `geo_` marks.

A geographic coordinate system: a map projection of `longitude`, `latitude`, `longitude2`, `latitude2`, and `geometry`, for the `geo_` marks. A plot in this coordinate system is a geo plot. The chart stores the spec, and the renderer builds the projection from it and fits its view at the size of each plot.

The geo plots that share a projection merge their data and show one view, so their surfaces, their fits, their aspects, and their navigation must be equal.

**Parameters**

+ `surface`: The coordinates that the projection takes.
+ `family`: The share family. Geo plots whose specs name one family share one projection, in addition to the projections that resolve rules share.
+ `aspect`: The shape of the plot. When unset, the plot keeps the aspect ratio of what its view fits, inside the space that the layout gives it, so the projection fills the plot.
+ `navigation`: How a navigation moves the projection. When unset, a navigation moves the projection in its own pan mode, and its zoom has no limits.

## `geo_navigation`

```googlesql
geo_navigation([pan_mode: 'translate' | 'longitude' | 'sphere'][, min_zoom: ANY][, max_zoom: ANY]) -> BYTES
```

How a navigation moves the projection of a geo plot, which every geo plot that shares the projection moves alike.

**Parameters**

+ `pan_mode`: How a drag moves the projection. Without a mode, the projection's own mode: `PAN_MODE_TRANSLATE` for a planar geo plot, `PAN_MODE_SPHERE` for a projection whose default clip is a circle, as an azimuthal projection's is, and `PAN_MODE_LONGITUDE` otherwise.
+ `min_zoom`: How far the wheel zooms out and in, as factors of the scale of the fitted projection at the plot's size, so a `min_zoom` of 1 stops a zoom out at the fitted view. Without a limit, the zoom has none in that direction.
+ `max_zoom`: As for `min_zoom`, the limit of a zoom in.

## `planar`

```googlesql
planar([reflect_y: BOOL]) -> BYTES
```

Coordinates that are already projected, as d3-geo's `geoIdentity` takes them.

Coordinates that are already projected, as d3-geo's `geoIdentity` takes them. They do not rotate, clip to the sphere, or resample.

**Parameters**

+ `reflect_y`: Whether the y axis flips, for coordinates whose y grows upward.

## `spherical`

```googlesql
spherical(projection: ANY[, rotate: BYTES][, center: BYTES][, clip: ANY][, precision: ANY][, fit: ANY][, graticule: BYTES][, sphere: BYTES][, tiles: ANY]) -> BYTES
```

Longitudes and latitudes in degrees, through a projection.

**Parameters**

+ `projection`: The projection, with its parameters.
+ `rotate`: The rotation of the sphere before it projects. When unset, the sphere does not rotate.
+ `center`: The place at the center of the view. When unset, longitude 0 and latitude 0.
+ `clip`: The part of the sphere that the projection keeps. When unset, the projection keeps its default part, such as the near hemisphere of `orthographic`.
+ `precision`: The resampling precision, in pixels, or 0 for no resampling. When unset, the square root of 0.5, as in d3-geo.
+ `fit`: What the view fits. When unset, the view fits the data of the plot's layers.
+ `graticule`: The lines of longitude and latitude. When unset, the plot has no graticule.
+ `sphere`: The sphere, the whole world of the projection, filled behind the data and outlined. When unset, the plot does not draw the sphere.
+ `tiles`: Raster tiles behind the data, such as a street map. When unset, the map has no tiles.

## `tile_layer`

```googlesql
tile_layer(url_template: STRING[, min_zoom: INT64][, max_zoom: INT64][, tile_size: INT64]) -> BYTES
```

Raster tiles in Web Mercator, which the renderer loads by URL through the image resolver of its host and draws on the projection, clipped to its outline.

Raster tiles in Web Mercator, which the renderer loads by URL through the image resolver of its host and draws on the projection, clipped to its outline. The attribution of the plot credits the tile source.

**Parameters**

+ `url_template`: The URL of each tile, with `{z}`, `{x}`, and `{y}` for its zoom, column, and row, such as `https://tile.openstreetmap.org/{z}/{x}/{y}.png`.
+ `min_zoom`: The lowest zoom that the server has. When unset, 0.
+ `max_zoom`: The highest zoom that the server has. When unset, 19.
+ `tile_size`: The size of each tile, in pixels. When unset, 256.

## `sphere_style`

```googlesql
sphere_style([fill: STRING][, stroke: STRING][, stroke_width: ANY]) -> BYTES
```

The look of the sphere.

The look of the sphere. A field that is unset takes its value from the styles.

**Parameters**

+ `fill`: The CSS color behind the data.
+ `stroke`: The CSS color of the outline.
+ `stroke_width`: The width of the outline, in pixels.

## `graticule`

```googlesql
graticule([longitude_step: ANY][, latitude_step: ANY][, extent: BYTES][, major_longitude_step: ANY][, major_latitude_step: ANY][, major_extent: BYTES][, precision: ANY][, frame: 'outline' | 'rectangle'][, label_sides: BYTES][, format: STRING][, label_style: BYTES][, label_padding: ANY][, color: STRING][, width: ANY]) -> BYTES
```

The meridians and parallels of a spherical geo plot.

The meridians and parallels of a spherical geo plot. Their labels sit on the outline of the projection, or on the edges of the plot for a plot that fits a box.

**Parameters**

+ `longitude_step`: The degrees between meridians. When unset, 10.
+ `latitude_step`: The degrees between parallels. When unset, 10.
+ `extent`: The box that the lines cover. When unset, longitudes from -180 to 180 and latitudes from -80 to 80, as in d3-geo.
+ `major_longitude_step`: The degrees between major meridians, which run past `extent` to the box of `major_extent`. When unset, 90.
+ `major_latitude_step`: The degrees between major parallels. When unset, 360, so the major lines are the meridians alone.
+ `major_extent`: The box that the major lines cover. When unset, longitudes from -180 to 180 and latitudes from -90 to 90.
+ `precision`: The step between the points of each line, in degrees, which the projection resamples. When unset, 2.5.
+ `frame`: Where the labels sit. When unset, the labels sit on the edges of the plot for a plot that fits a box or its data, and on the outline of the projection for a plot that fits the whole sphere.
+ `label_sides`: The sides that label the lines that reach them: meridians on the top and the bottom, and parallels on the left and the right. When unset, the left and the bottom label them.
+ `format`: The pattern of the degrees of each label, in the syntax of `Chart.number_format_syntax`. An empty pattern takes the formatter's default.
+ `label_style`: The style of the labels.
+ `label_padding`: The distance between the end of a line and its label, in pixels.
+ `color`: The CSS color of the lines.
+ `width`: The width of the lines, in pixels.

## `text_style`

```googlesql
text_style([font_family: STRING][, font_size: ANY][, font_weight: ANY][, color: STRING]) -> BYTES
```

The font and the color of a text.

**Parameters**

+ `font_family`: A font family, such as `Lato`.
+ `font_size`: The font size, in pixels.
+ `font_weight`: A weight from 100 to 900.
+ `color`: A CSS color.

## `label_sides`

```googlesql
label_sides([sides: ARRAY<STRING>]) -> BYTES
```

The sides of a plot that carry labels, in any order, where a side that repeats counts once.

The sides of a plot that carry labels, in any order, where a side that repeats counts once. No sides draws no labels.

**Parameters**

+ `sides`: The sides, such as `left` and `bottom`.

## `lon_lat_box`

```googlesql
lon_lat_box([west: ANY][, south: ANY][, east: ANY][, north: ANY]) -> BYTES
```

A box from `west` to `east` and from `south` to `north`, in degrees.

A box from `west` to `east` and from `south` to `north`, in degrees. It crosses the antimeridian when `east` is less than `west`.

**Parameters**

+ `west`: The western edge, in degrees east.
+ `south`: The southern edge, in degrees north.
+ `east`: The eastern edge, in degrees east.
+ `north`: The northern edge, in degrees north.

## `spherical_clip`

```googlesql
spherical_clip([clip: ANY]) -> BYTES
```

A fragment of the message `avenger.chart.v1.SphericalClip`.

**Parameters**

+ `clip`: One of these:

  `antimeridian`: A cut at the antimeridian.

  `circle_angle`: The cap within this many degrees of the center of the view, strictly between 0 and 180.

  `region`: The inside of a spherical polygon.

## `region_clip`

```googlesql
region_clip([polygon: BYTES][, frame: 'earth' | 'view']) -> BYTES
```

A fragment of the message `avenger.chart.v1.RegionClip`.

**Parameters**

+ `polygon`: A MultiPolygon in longitude and latitude degrees, as WKB. Its edges are great circles, its rings follow the winding of RFC 7946, and its polygons have no holes.
+ `frame`: Whether the region turns with the rotation of the sphere.

## `lon_lat`

```googlesql
lon_lat([longitude: ANY][, latitude: ANY]) -> BYTES
```

A fragment of the message `avenger.chart.v1.LonLat`.

**Parameters**

+ `longitude`: In degrees east.
+ `latitude`: In degrees north.

## `rotation`

```googlesql
rotation([lambda: ANY][, phi: ANY][, gamma: ANY]) -> BYTES
```

The yaw, the pitch, and the roll of a rotation, in degrees, as d3-geo's `rotate` takes them.

**Parameters**

+ `lambda`: The yaw: the turn about the polar axis, in degrees.
+ `phi`: The pitch, in degrees.
+ `gamma`: The roll, in degrees.

## `custom_projection`

```googlesql
custom_projection(name: STRING[, parameters: ARRAY<BYTES>]) -> BYTES
```

A projection that an app adds to its renderer.

A projection that an app adds to its renderer. A renderer without a projection of the name reports an error that names it.

**Parameters**

+ `name`: The name of the projection.
+ `parameters`: The parameters of the projection, keyed by name. The projection checks their names and their values.

## `named_option`

```googlesql
named_option([name: STRING][, value: ANY]) -> BYTES
```

One option of a custom scale or a custom projection.

**Parameters**

+ `name`: The name of the option.
+ `value`: The value of the option.

## `option_value`

```googlesql
option_value([value: ANY]) -> BYTES
```

The value of an option: a Boolean, a number, an integer, a string, or a list of numbers or of strings.

**Parameters**

+ `value`: The value: a Boolean, a number, an integer, a string, or a list of numbers or of strings.

## `string_list`

```googlesql
string_list([values: ARRAY<STRING>]) -> BYTES
```

A list of strings.

**Parameters**

+ `values`: The strings.

## `float_list`

```googlesql
float_list([values: ARRAY<FLOAT64>]) -> BYTES
```

A list of numbers.

**Parameters**

+ `values`: The numbers.

## `hammer_projection`

```googlesql
hammer_projection([coefficient: ANY]) -> BYTES
```

A Hammer projection.

**Parameters**

+ `coefficient`: The coefficient, at least 1: 1 gives the azimuthal equal-area projection, and 2 the Hammer projection. When unset, 2.

## `cylindrical_equal_area_projection`

```googlesql
cylindrical_equal_area_projection([parallel: ANY]) -> BYTES
```

A cylindrical equal-area projection, which is true to scale along its standard parallel.

**Parameters**

+ `parallel`: The standard parallel, in degrees from -90 to 90. When unset, 38.58, the parallel of the Hobo-Dyer projection.

## `conic_equidistant`

```googlesql
conic_equidistant([parallel1: ANY][, parallel2: ANY]) -> BYTES
```

The equidistant conic projection, which is true to scale along its two standard parallels.

**Parameters**

+ `parallel1`: The first standard parallel, in degrees from -90 to 90.
+ `parallel2`: The second standard parallel, in degrees from -90 to 90. Equal parallels make a conic with one standard parallel.

## `conic_equal_area`

```googlesql
conic_equal_area([parallel1: ANY][, parallel2: ANY]) -> BYTES
```

The equal-area conic projection, which is true to scale along its two standard parallels.

**Parameters**

+ `parallel1`: The first standard parallel, in degrees from -90 to 90.
+ `parallel2`: The second standard parallel, in degrees from -90 to 90. Equal parallels make a conic with one standard parallel.

## `conic_conformal`

```googlesql
conic_conformal([parallel1: ANY][, parallel2: ANY]) -> BYTES
```

The conformal conic projection, which is true to scale along its two standard parallels.

**Parameters**

+ `parallel1`: The first standard parallel, in degrees from -90 to 90.
+ `parallel2`: The second standard parallel, in degrees from -90 to 90. Equal parallels make a conic with one standard parallel.

## `polar`

```googlesql
polar([start_angle: ANY][, end_angle: ANY][, inner_radius: ANY]) -> BYTES
```

Angles and radii about a center, for the `polar_` marks.

Angles and radii about a center, for the `polar_` marks. They take `theta`, `radius`, `theta2`, and `radius2` in place of `x`, `y`, `x2`, and `y2`, so `theta` plays the part of `x` and `radius` the part of `y` in the orientations and the spans of marks:

- A pie or a donut is a rect for each slice, from its start angle to its end angle, without `radius`, so each slice spans the range of `radius`. - A sunburst is a rect for each node of a hierarchy, with the node's share of its parent as its angles and its depth as its radii. - A rose chart is a bar for each band of `theta`, and a radial bar chart is a bar for each band of `radius`. - A radar chart is a line or an area over the bands of `theta` with `ROUTE_CANVAS`, which repeats its first row at its end to close its shape.

The producer computes the angles of pies and sunbursts, as it computes the stacks of bars. A pie's `theta` that holds shares of its total, from 0 to 1, keeps every pie of a facet grid whole when the facets share `theta`.

A band or a point scale of `theta` over a whole circle takes one equal step for each category, with the first category centered at the start angle, because the end of the circle is its start: the points of a radar chart lie on their spokes, the wedges of a rose chart center on theirs, and the first category is at the top. Over part of a circle, the scale spreads its categories as it does along a line.

The range of `theta` runs clockwise from `start_angle` to `end_angle`, and the range of `radius` runs from `inner_radius` to the outer radius. The plot keeps the shape of the box around the circle's part between the two angles: a square for a whole circle, and twice as wide as it is tall for a half circle from -90 to 90. The outer radius is the largest that fits the plot, less the farthest that the marks and the labels of its axes reach past it, so labels outside a pie stay inside the plot.

`theta` and `radius` have no axis unless their specs have an `Axis`, because a pie has none. Each polar plot draws its own axes inside its box, so the part rules of concats do not apply to them.

**Parameters**

+ `start_angle`: Where the range of `theta` starts, in degrees clockwise from the top. When unset, 0.
+ `end_angle`: Where the range of `theta` ends, in degrees clockwise from the top, more than `start_angle` and at most 360 degrees past it. When unset, 360 degrees past `start_angle`.
+ `inner_radius`: Where the range of `radius` starts, as a fraction of the outer radius: 0 for a pie, and 0.5 for a donut whose hole is half its width. When unset, 0.

## `cartesian`

```googlesql
cartesian([aspect_ratio: ANY]) -> BYTES
```

Position scales on `x`, `y`, `x2`, and `y2`, for the marks without a prefix.

**Parameters**

+ `aspect_ratio`: The width of the plot over its height, which the plot keeps inside the space that the layout gives it, or with the plot's `width` or `height`, as `Plot.width` describes. When unset, the plot takes all of that space.

## `scale`

```googlesql
scale([type: ANY][, domain: ANY][, range_: ANY][, family: STRING][, axis: BYTES][, navigation_limits: ANY][, legend: BYTES]) -> BYTES
```

The scale of a channel.

The scale of a channel. The chart stores the spec, and the renderer builds the configured scale from the spec, the styles, and a fit to the domain's summaries at the size of each plot.

The specs of the layers that share a scale must agree. They have one type, the custom scales one type name, except that a band and a point scale share a band scale, whose bands hold the points at their middles: the point scale's `align`, `round`, and clip paddings are the band scale's, and a point scale with `padding` and a band scale are a conflict. The range, each field of the fits of their domains, and each option of the type that both specs set must be equal. A range, a field of a fit, or an option that one spec sets and another leaves unset takes the value that is set, so bars, whose measure's fit contains zero, share a scale with a rule of their mean, and the domain contains zero. The axis and the legend follow the rule of the options: an axis that one spec sets and another leaves unset is the axis of both, and each setting that both set, in the axis and in its parts, must be equal, except that the classes of the axes join, and the same for the legend. Every axis of the shared scale draws the merged axis, and every legend the merged legend. The domains of the layers are of one kind: summary and data domains merge, explicit domains must be equal, and selection domains must be equal. A layer with an explicit or a selection domain and a layer with a domain of another kind are a conflict, so a chart gives them independent scales, through a resolve rule or different families.

The plots whose layers share `x` or `y` get equal lengths along it, so a value lies as far from the start of each plot. Plots of a shared `x` in different columns of a table each stand in one column, and the columns have one size, and the rows of a shared `y` likewise. Plots that share `x` and fix different widths, or share `y` and fix different heights, as `Plot.width` describes, are a conflict, and a plot of the shared scale that fixes no length takes the fixed length of the others. The polar plots whose layers share `radius` draw it at one outer radius, the smallest of theirs, so a value lies as far from the center of each.

**Parameters**

+ `type`: The type of the scale and its options. An option that is unset takes the default that its comment gives. When the type is unset, it follows the type of the values, the channel, and for categories on a position, the mark:

  - numbers: a linear scale, and an ordinal scale on `shape`, `pattern`, and `stroke_dash` - dates and timestamps: a time scale, and an ordinal scale on `shape`, `pattern`, and `stroke_dash` - strings and booleans: on a position, a band scale for bars and rects and a point scale for other marks, a point scale on `size`, `opacity`, `fill_opacity`, `stroke_opacity`, and `stroke_width`, which spreads the categories over the range, and elsewhere an ordinal scale - structs of the levels of nested categories: a nested band scale on a position

  The values are those of the encoding when they are scaled, and otherwise of its first scaled conditional value. Values of another type, such as durations, have no inferred type, and a chart that leaves their type unset is invalid.
+ `domain`: Where the domain comes from. When unset, and the scale has a type, the domain is the data domain of the layer, with no fit.
+ `range_`: The range. When unset, the styles give the range of a non-position channel, and the plot gives the range of a position channel, whose `Range` holds only `reverse`.
+ `family`: The share family. Layers whose specs name one family for a channel share one scale, in addition to the scales that resolve rules share.
+ `axis`: The axis of a position scale. When unset, `x` and `y` have an axis and `theta` and `radius` have none. Invalid on the scale of another channel.
+ `navigation_limits`: The limits of how a navigation moves a view of the scale, which every plot of the scale keeps. Valid only on a continuous scale of `x` or `y`: a linear, log, pow, symlog, or time scale, or a custom scale. When unset, a view moves freely.
+ `legend`: The legend of a non-position scale. When unset, a discrete scale has a symbol legend, a quantize, quantile, or threshold scale has a symbol legend of its bins, a continuous color scale has a colorbar, another continuous scale, such as `size`, has a symbol legend of its ticks, and an `angle` scale has none. Invalid on the scale of a position channel.

## `navigation_limits`

```googlesql
navigation_limits([bounds: ANY][, min_span: ANY]) -> BYTES
```

The limits of how a navigation moves the domain of a scale, in the numeric data values of the scale: numbers, or epoch milliseconds for a time scale.

The limits of how a navigation moves the domain of a scale, in the numeric data values of the scale: numbers, or epoch milliseconds for a time scale. The bounds and the span of a log, pow, or symlog scale are in data values, not in the coordinate of the scale.

**Parameters**

+ `bounds`: The bounds that the domain stays inside. A pan stops at a bound, and a zoom out stops at the width of the bounds. Without bounds, the domain moves freely.
+ `min_span`: The smallest width of the domain, at which a zoom in stops. Without it, a zoom in has no limit.

## `navigation_interval`

```googlesql
navigation_interval([low: ANY][, high: ANY]) -> BYTES
```

An interval from `low` to `high`.

**Parameters**

+ `low`: The low end.
+ `high`: The high end, more than `low`.

## `legend`

```googlesql
legend([title: ANY][, labels: BYTES][, side: 'bottom' | 'top' | 'left' | 'right'][, wrap: ANY][, label_padding: ANY][, column_gap: ANY][, row_gap: ANY][, background: BYTES][, entries: ANY][, classes: ARRAY<STRING>][, id: STRING][, form: 'symbol' | 'colorbar'][, thickness: ANY][, axis_gap: ANY][, max_length: ANY][, show_labels: BOOL][, label_angle: ANY][, label_align: 'left' | 'center' | 'right'][, label_baseline: 'top' | 'middle' | 'alphabetic' | 'bottom'][, ticks: ANY][, domain_color: STRING][, show: BOOL][, show_title: BOOL][, show_swatches: BOOL][, show_ticks: BOOL][, show_domain: BOOL]) -> BYTES
```

The legend of a non-position scale, in one of two forms.

The legend of a non-position scale, in one of two forms. A symbol legend has an entry for each category of a discrete scale, for each bin of a quantize, quantile, or threshold scale, labeled with the range of the bin, or for each tick of another continuous scale, such as the sizes of a size scale, with a swatch of the mark and a label. A colorbar draws the colors of a continuous color scale as a gradient, with an axis beside it. A style field that is unset takes its value from the styles. The settings of one form are invalid on a legend of the other: `wrap`, `label_padding`, `column_gap`, `row_gap`, `entries`, and `show_swatches` on a colorbar, and `thickness`, `axis_gap`, `max_length`, `label_align`, `label_baseline`, `ticks`, `show_ticks`, `domain_color`, and `show_domain` on a symbol legend.

The symbol legends of discrete scales with one owner merge into one legend when the scales encode the same columns, hold the same categories in the same order, and the legends are equal, so the `color` and the `shape` of one column show one legend, whose swatches take the value of each scale, and two columns whose categories happen to be equal show two. A legend of a continuous scale does not merge.

**Parameters**

+ `title`: The title. When unset, the legend takes the titles of the encodings of its scale, as `Encoding.title` describes.
+ `labels`: The labels of the entries, where `#value` is the formatted entry, or the formatted range of a bin, or the labels of the axis of a colorbar.
+ `side`: The side of the plot that the legend stands on. When unset, the styles give it.
+ `wrap`: How the entries of a symbol legend fill the lines along the side.
+ `label_padding`: The space between a swatch and its label, in pixels.
+ `column_gap`: The space between columns of entries, in pixels.
+ `row_gap`: The space between rows of entries, in pixels.
+ `background`: The box behind the legend. When unset, the legend has no background.
+ `entries`: The entries of the symbol legend of a continuous scale. When unset, at the ticks of the scale, with a count that the styles give.
+ `classes`: The class names of the legend, which a stylesheet selects with `.name`, as the HTML `class` attribute lists them. Each is not empty and holds no whitespace.
+ `id`: An id, or empty, which a stylesheet selects with `#id`, with CSS escapes for an id that is not a CSS identifier. With its partition, it identifies the legend, as `Node` describes.
+ `form`: The form of the legend. When unset, a continuous scale of `color`, `fill`, or `stroke` has a colorbar, and another scale has a symbol legend.
+ `thickness`: The thickness of the gradient of a colorbar, across the colorbar, in pixels.
+ `axis_gap`: The space between the gradient and its axis, in pixels.
+ `max_length`: The longest that a colorbar runs along its side, with its background, in pixels. When unset, it runs the whole length that the layout gives it.
+ `show_labels`: Whether the legend shows its labels. When unset, the styles decide.
+ `label_angle`: The angle of the labels, in degrees clockwise.
+ `label_align`: When unset, each label of a colorbar's axis aligns to its end nearest its tick, for the side of the colorbar and the angle of the labels.
+ `label_baseline`: When unset, each label aligns to its end nearest its tick, as for `label_align`.
+ `ticks`: The ticks of a colorbar's axis.
+ `domain_color`: The CSS color of the line along a colorbar's axis.
+ `show`: Whether the scale draws this legend. When unset, it does. `false` draws no legend.
+ `show_title`: Whether the legend shows its title. When unset, the styles decide.
+ `show_swatches`: Whether a symbol legend shows the swatches of its entries. When unset, the styles decide.
+ `show_ticks`: Whether a colorbar's axis shows its ticks. When unset, the styles decide.
+ `show_domain`: Whether a colorbar's axis shows its line. When unset, the styles decide.

## `ticks`

```googlesql
ticks([at_: ANY][, length: ANY][, color: STRING]) -> BYTES
```

The ticks of an axis.

The ticks of an axis. A style field that is unset takes its value from the styles.

**Parameters**

+ `at_`: Where the ticks stand. When unset, at the ticks of the scale, with a count that the axis fits to its length. On the axis of a band or a point scale, only `values` is valid, and on the axis of a nested band scale, which ticks at the bands of its leaf, none is.
+ `length`: The length of each tick, in pixels.
+ `color`: A CSS color.

## `tick_interval`

```googlesql
tick_interval(interval_: ANY) -> BYTES
```

Ticks at a start and each whole multiple of a step before and after it.

**Parameters**

+ `interval_`: One of these:

  `numeric`: Ticks of a numeric scale: a first tick and the step between two ticks.

  `calendar`: Ticks of a time scale.

## `calendar_ticks`

```googlesql
calendar_ticks(unit: 'month' | 'day' | 'millisecond'[, start: INT64][, count: INT64]) -> BYTES
```

Ticks of a time scale.

Ticks of a time scale. Months and days step in the civil time of the scale.

**Parameters**

+ `unit`: The unit of the step between two ticks.
+ `start`: The first tick, in milliseconds since the Unix epoch.
+ `count`: The number of units between two ticks, more than 0.

## `numeric_ticks`

```googlesql
numeric_ticks([start: ANY][, step: ANY]) -> BYTES
```

Ticks of a numeric scale: a first tick and the step between two ticks.

**Parameters**

+ `start`: A tick, which the other ticks step from.
+ `step`: The distance between two ticks, more than 0.

## `legend_wrap`

```googlesql
legend_wrap([wrap: ANY]) -> BYTES
```

A fragment of the message `avenger.chart.v1.LegendWrap`.

**Parameters**

+ `wrap`: One of these:

  `flow`: Each entry at its own size, with a line breaking before an entry that would pass the length of the side.

  `grid`: Equal cells, as wide as the widest entry and as tall as the tallest, filled along the side first.

## `legend_grid`

```googlesql
legend_grid([columns: INT64]) -> BYTES
```

A fragment of the message `avenger.chart.v1.LegendGrid`.

**Parameters**

+ `columns`: The count of columns at every length. When unset, the legend takes as many cells along the side as fit.

## `background`

```googlesql
background([fill: STRING][, stroke: STRING][, corner_radius: ANY][, padding: ANY]) -> BYTES
```

A box behind an axis, a legend, or a widget.

**Parameters**

+ `fill`: A CSS color, such as `transparent`.
+ `stroke`: A CSS color for the outline.
+ `corner_radius`: The radius of the corners, in pixels.
+ `padding`: The space between the edges of the box and its content, in pixels.

## `labels`

```googlesql
labels([format: STRING][, markup: STRING][, style: BYTES]) -> BYTES
```

The labels of the ticks of an axis, the entries of a legend, the values of a facet, or the values of the fields of a tooltip.

The labels of the ticks of an axis, the entries of a legend, the values of a facet, or the values of the fields of a tooltip. A style field that is unset takes its value from the styles.

**Parameters**

+ `format`: The pattern of numeric and temporal values, such as `,.0f`. A pattern of numbers is in the syntax of `Chart.number_format_syntax`, and a pattern of dates or datetimes is in the syntax of `Chart.datetime_format_syntax`. An empty pattern takes the default of the text engine's formatter, which needs no syntax. Other values, such as strings, show as text.
+ `markup`: Typst markup for each label, as `TEXT_SYNTAX_TYPST` reads it, where `#value` is the formatted value, such as `#value m/s`. An empty markup shows the formatted value as plain text. The label of a category of a band, a point, or an ordinal scale also binds `null`, whether it is the NULL category, whose `value` is `null`, such as `#if null [Unknown] else [#value]`. The label of a column of lumps also binds `other`, whether it is the Other group, `count`, its number of categories, and `weight`, its weight, such as `#if other [#value (#count)] else [#value]`. `count` and `weight` are `?` when the rows that the label stands for disagree.
+ `style`: The style of the labels.

## `title`

```googlesql
title(text: STRING[, side: 'top' | 'left' | 'right' | 'bottom'][, syntax: 'plain' | 'plain_lines' | 'typst'][, style: BYTES][, subtitle: STRING][, subtitle_style: BYTES][, align: 'start' | 'center' | 'end']) -> BYTES
```

The title of a plot, a concat, a wrap, an axis, a legend, a facet, or a widget.

The title of a plot, a concat, a wrap, an axis, a legend, a facet, or a widget. A style field that is unset takes its value from the styles. The title of a node, which is a plot, a concat, or a wrap, stands on one side of the node with an optional subtitle after it. It runs along the whole side of the node's box, over the slabs of its axes and legends, and a title at the bottom serves as a caption. The settings that place the title of a node and give its subtitle, `side`, `align`, `subtitle`, and `subtitle_style`, are invalid on the title of another element, which stands where its axis, legend, facet, or widget puts it.

**Parameters**

+ `text`: The text, in the syntax of `syntax`.
+ `side`: When unset, the styles give it.
+ `syntax`: How the text engine reads the text and the subtitle.
+ `style`: The style of the text.
+ `subtitle`: A line of text after the text, such as the units or the period of the data, or empty for no subtitle.
+ `subtitle_style`: The style of the subtitle.
+ `align`: Where the title stands along its side. When unset, the styles give it.

## `axis`

```googlesql
axis([title: ANY][, side: 'bottom' | 'top' | 'left' | 'right'][, labels: BYTES][, show_labels: BOOL][, label_rotation: ANY][, label_align: 'left' | 'center' | 'right'][, label_baseline: 'top' | 'middle' | 'alphabetic' | 'bottom'][, ticks: ANY][, domain_color: STRING][, grid: BYTES][, levels: ARRAY<BYTES>][, classes: ARRAY<STRING>][, id: STRING][, angle: ANY][, show: BOOL][, show_title: BOOL][, show_ticks: BOOL][, show_domain: BOOL][, show_grid: BOOL][, route: 'coordinate' | 'canvas']) -> BYTES
```

The axis of a position scale: its title, its line, its ticks, their labels, and the grid lines of the plot at its ticks.

The axis of a position scale: its title, its line, its ticks, their labels, and the grid lines of the plot at its ticks. A style field that is unset takes its value from the styles.

In a Cartesian plot, the axis of `x` or `y` stands on a side of the plot. In a polar plot, the axis of `theta` runs around the outer edge, with its labels outside the circle and spokes as its grid lines, and the axis of `radius` runs along the spoke at its `angle`, with rings as its grid lines. A polar axis draws no title. A band or point scale ticks at each category, so a radar chart has a spoke and a label for each of its categories. The settings that only one kind of plot draws are invalid on the axis of the other: `side`, `levels`, and the alignment of the labels on a polar axis, and `angle`, `route`, and `label_orient` on a Cartesian axis.

**Parameters**

+ `title`: The title. When unset, the axis takes the titles of the encodings of its scale, as `Encoding.title` describes.
+ `side`: The side of a Cartesian plot. When unset, `x` has its axis at the bottom and `y` at the left, and the second scale of a plot has its axis on the other side.
+ `labels`: The labels of the ticks of an axis, the entries of a legend, the values of a facet, or the values of the fields of a tooltip. A style field that is unset takes its value from the styles.
+ `show_labels`: Whether the axis shows its labels. When unset, the styles decide.
+ `label_rotation`: How the labels turn. When both are unset, the styles give the angle.
+ `label_align`: When unset, each label aligns to its end nearest its tick, for the side of the axis and the angle of the labels.
+ `label_baseline`: When unset, each label aligns to its end nearest its tick, as for `label_align`.
+ `ticks`: The ticks of an axis. A style field that is unset takes its value from the styles.
+ `domain_color`: The CSS color of the line along the axis: the outer edge of the plot for an axis of `theta`.
+ `grid`: The grid lines of a plot at the ticks of an axis. A style field that is unset takes its value from the styles.
+ `levels`: The options of each level of the axis of a nested band scale, outermost first, which stacks a row of labels for each outer level outward from the labels of the leaf, with separators between the bands of each outer level. A level past the end of the list takes the defaults. Valid only on the axis of a nested band scale, with at most one entry for each level.
+ `classes`: The class names of the axis, which a stylesheet selects with `.name`, as the HTML `class` attribute lists them. Each is not empty and holds no whitespace.
+ `id`: An id, or empty, which a stylesheet selects with `#id`, with CSS escapes for an id that is not a CSS identifier. With its partition, it identifies the axis, as `Node` describes.
+ `angle`: The spoke that the axis of `radius` runs along, in degrees clockwise from the top. When unset, the start angle of the plot. Invalid on the axis of `theta`.
+ `show`: Whether the scale draws this axis. When unset, it does. `false` draws no axis, as an unset axis of `theta` or `radius` draws none, and an unset axis of `x` or `y` draws the default axis.
+ `show_title`: Whether the axis shows its title. When unset, the styles decide.
+ `show_ticks`: Whether the axis shows its ticks. When unset, the styles decide.
+ `show_domain`: Whether the axis shows its line. When unset, the styles decide.
+ `show_grid`: Whether the plot draws grid lines at the ticks of the axis. When unset, the styles decide.
+ `route`: The route of the axis's lines at one radius: the rings of the grid of an axis of `radius`, and the outer edge of an axis of `theta`. Circles with `ROUTE_COORDINATE`, and with `ROUTE_CANVAS`, polygons whose corners are the ticks of `theta`, as the web and the rim of a radar chart. Spokes are straight with either route.

## `nested_axis_level`

```googlesql
nested_axis_level([show_labels: BOOL][, title: STRING][, label_angle: ANY][, format: STRING]) -> BYTES
```

The options of one level of the axis of a nested band scale.

**Parameters**

+ `show_labels`: Whether the axis shows the labels of the level. A leaf without labels has no ticks either. When unset, it shows them.
+ `title`: The part of the level in the title of an encoding of the scale whose own title is unset, which joins the parts of the levels, outermost first, with ` / ` in place of the name of the encoding's column. When unset, the name of the level's field. Empty leaves the level out.
+ `label_angle`: The angle of the labels of the level, in degrees clockwise. When unset, the leaf takes the `label_angle` of the axis, and the labels of an outer level are level.
+ `format`: The pattern of the numeric and temporal values of the level, as for `Labels.format`. When unset, the pattern of the axis.

## `grid`

```googlesql
grid([color: STRING][, width: ANY]) -> BYTES
```

The grid lines of a plot at the ticks of an axis.

The grid lines of a plot at the ticks of an axis. A style field that is unset takes its value from the styles.

**Parameters**

+ `color`: A CSS color.
+ `width`: The width of each grid line, in pixels.

## `scale_range`

```googlesql
scale_range([range_: ANY][, reverse: BOOL]) -> BYTES
```

A fragment of the message `avenger.chart.v1.Range`.

**Parameters**

+ `range_`: One of these:

  `values`: A list of the values of the range, such as CSS colors, sizes, or shape names.

  `palette`: The name of a palette, such as `tableau10`: a built-in palette of the renderer, or one that a stylesheet defines with `@palette`.
+ `reverse`: Whether the range runs from its end to its start: the values or the palette in reverse, or the plot's range of a position channel from its far end.

## `scale_domain`

```googlesql
scale_domain([domain: ANY]) -> BYTES
```

Where a scale's domain comes from.

**Parameters**

+ `domain`: One of these:

  `explicit`: An explicit domain: a list of the values that the scale maps.

  `summary`: A summary of the data that the domain covers, which can be more than the rows of the layer's table, such as a summary that a query computed over all of the data of a facet grid.

  `data`: The renderer summarizes this layer's own data: the column of this channel and of its pair, `x2` for `x`, `y2` for `y`, `theta2` for `theta`, and `radius2` for `radius`, or its scaled constant, with the reach of each mark that the rows draw.

  `selection`: The values of a selection for one channel, such as the brush of an overview chart for the `x` of its detail chart.

## `selection_domain`

```googlesql
selection_domain(selection: STRING, channel: 'x' | 'y' | 'x2' | 'y2' | 'theta' | 'radius' | 'theta2' | 'radius2' | 'longitude' | 'latitude' | 'longitude2' | 'latitude2' | 'geometry' | 'text' | 'image' | 'path' | 'color' | 'fill' | 'stroke' | 'opacity' | 'fill_opacity' | 'stroke_opacity' | 'stroke_width' | 'stroke_dash' | 'size' | 'shape' | 'angle' | 'pattern' | 'font_weight' | 'font_style' | 'width' | 'height' | 'detail', fallback: ANY) -> BYTES
```

A domain from the values of a selection.

**Parameters**

+ `selection`: The name of the selection.
+ `channel`: The channel whose conditions give the domain: the ranges that projected through this channel, and the values of the set columns that did.
+ `fallback`: The domain when the selection gives no values.

## `data_domain`

```googlesql
data_domain([fit: ANY]) -> BYTES
```

A domain that the renderer summarizes from the layer's own data, as `Domain.data` describes.

**Parameters**

+ `fit`: How the domain fits the data.

## `domain_fit`

```googlesql
domain_fit([fit: ANY]) -> BYTES
```

How a summary or a data domain fits the data, by the kind of the scale.

How a summary or a data domain fits the data, by the kind of the scale. Its kind is the kind of the scale's options: `interval` for linear, log, pow, symlog, time, and quantize scales, `categories` for band, point, nested band, and ordinal scales, and the kind of its summary for a custom scale. A fit is invalid on a quantile scale. When the fit of a domain is unset, the fit of the scale's kind applies with its defaults.

**Parameters**

+ `fit`: One of these:

  `interval`: How a domain of a continuous scale fits the data: whether it holds zero, whether its ends are round, its domain without data, and a value that it pins.

  `categories`: The order of the categories of a summary or data domain.

## `category_fit`

```googlesql
category_fit([order_: ANY]) -> BYTES
```

The order of the categories of a summary or data domain.

The order of the categories of a summary or data domain. The data decides which categories the domain holds, and a NULL value is the NULL category. An explicit domain gives exactly its categories in its order instead, with the NULL category where it names NULL.

**Parameters**

+ `order_`: When unset, the categories of the data, ascending, or for a column of lumps, the kept categories by descending weight. The Other group of a column of lumps comes after the other categories in every order, and the NULL category is last.

## `interval_fit`

```googlesql
interval_fit([zero: BOOL][, nice: ANY][, empty: ANY][, pin: BYTES]) -> BYTES
```

How a domain of a continuous scale fits the data: whether it holds zero, whether its ends are round, its domain without data, and a value that it pins.

**Parameters**

+ `zero`: Whether the domain contains zero. When unset, a linear, a pow, or a symlog scale of `x`, `y`, or `size` contains zero when one of its encodings asks for it: every encoding except the key of a bar, an area, or a line, along which the marks stand at their values. A log or a time scale, a selection domain, and a pinned fit contain no zero by default. A pinned end past zero holds, so the domain does not contain zero. `false` turns the default off, such as for a scatter plot whose data lies far from zero.
+ `nice`: Whether the domain has nice ends, which are round values, and the tick count that rounds them. When unset, a linear, a log, a pow, or a symlog scale of `x` or `y` has nice ends at a count of 10, and another scale has none. A log scale's nice domain ends on powers of its base, whatever the count. With a pin, the free end is round, or the radius about a pinned middle.
+ `empty`: The domain when there is no data, as an explicit domain holds it. When unset, the scale type gives the domain.
+ `pin`: A value that the domain holds exactly, and the fit finds the rest of the domain. Two pinned values would determine the domain, which an explicit domain gives, so a fit pins one. Marks past a pinned end lie outside the domain, and the plot clips them. When unset, the fit finds both ends.

## `domain_pin`

```googlesql
domain_pin(value: ANY, position: 'low' | 'mid' | 'high') -> BYTES
```

A value that the domain of a fit holds exactly, and where the domain holds it.

**Parameters**

+ `value`: The pinned value, a scalar in the type of the domain.
+ `position`: Where the domain holds the value.

## `summary_domain`

```googlesql
summary_domain(value: ANY[, fit: ANY]) -> BYTES
```

A summary domain: a summary of the data that the domain covers, and how the domain fits it.

**Parameters**

+ `value`: The summary is a struct with one row, whose fields are lists, in one of three layouts by the kind of the scale:

  - for linear, log, pow, symlog, time, and quantize scales: `lower_values`, `lower_extents`, `upper_values`, and `upper_extents`, lists of `Float64` values. Each value with its extent is a marker: a value where a mark stands and the pixels that the mark reaches past it. The lower markers reach toward smaller values, and the upper markers toward larger values. A value is in the coordinate of the scale: the logarithm of the data in the scale's base for a log scale, `sign(x) * |x|^exponent` for a pow scale, `sign(x) * ln(1 + |x| / constant)` for a symlog scale, epoch milliseconds for a time scale, and the data for linear and quantize scales. A date or a timestamp without a timezone counts its calendar fields as UTC. - for band, point, nested band, and ordinal scales: `categories`, a list of the categories in the type of the domain, and `lower_extents` and `upper_extents`, lists of `Float64` values, with the pixels that the marks of each category reach before and after its position. - for a quantile scale: `values`, a list of `Float64` values, a sample of the data.

  A summary domain is invalid on a threshold scale. The summary of a custom scale has the layout of its kind. Extents are finite and at least 0, and an extent is 0 for marks that reach no farther than their value. The markers and the categories come in any order, and they can repeat: the renderer keeps the markers that decide the fit, and the largest extents of each category. So a summary can hold a marker for every row, or, for marks without extents, only the smallest value as a lower marker and the largest as an upper marker.

  The extents hold for the non-position scales and the styles that the producer used, such as the size scale of symbols. When a combination shares a non-position scale that the extents depend on, the shared scale can differ from the producer's, and the layer's marks can reach past the edges of its plot. The renderer then reports that the marks do not fit.
+ `fit`: How the domain fits the summary.

## `custom_scale`

```googlesql
custom_scale(type: STRING[, options: ARRAY<BYTES>]) -> BYTES
```

A scale that an app adds to its renderer.

A scale that an app adds to its renderer. A renderer without a scale of the type reports an error that names it.

**Parameters**

+ `type`: The name of the type.
+ `options`: The options of the scale, keyed by name. The scale checks their names and their values.

## `ordinal_scale`

```googlesql
ordinal_scale([kind: 'categorical' | 'ordinal']) -> BYTES
```

A discrete scale that maps each category to one value of its range, in order, and starts over when the range runs out, such as the colors of a palette.

**Parameters**

+ `kind`: The kind of the scale, which stylesheets select and which chooses the default palette of a color scale. When unset, the kind follows from the type of the scale's values.

## `nested_band_scale`

```googlesql
nested_band_scale([align: ANY][, nest_scopes: ARRAY<STRING>][, padding_inner: ARRAY<FLOAT64>][, padding_outer: ARRAY<FLOAT64>][, round: BOOL][, clip_padding_lower: ANY][, clip_padding_upper: ANY][, kind: 'categorical' | 'ordinal']) -> BYTES
```

A band scale of nested categories: structs with one field for each level, outermost first, whose last field is the leaf.

A band scale of nested categories: structs with one field for each level, outermost first, whose last field is the leaf. The bands of each level nest in the bands of its parent, as the groups and the bars of a grouped bar chart. Every gap is in leaf steps, so a nested band scale of one level places its categories as a band scale with the same options does. Its paddings are style fields, as those of a band scale are. A mark stands in the band of the leaf, or of the level that the `band_level` of its encoding names, as for a band scale.

**Parameters**

+ `align`: As for `BandScale.align`.
+ `nest_scopes`: How each level nests in its parent. One entry applies to every level, and more entries apply to one level each, outermost first. The outermost level is always free. When it is empty, every level is free.
+ `padding_inner`: The space between two bands of a level. At the leaf level, a fraction of the step from 0 to 1, as for `BandScale.padding_inner`, and at an outer level, the gap between its groups in leaf steps, at least 0. One entry applies to every level, and more entries apply to one level each, outermost first. When it is empty, the styles give it, as the `padding-inner` of the scale.
+ `padding_outer`: The space before the first band of a level and after its last, in leaf steps, at least 0: inside each parent for an inner level, and at the ends of the range for the outermost. One entry or one for each level, as for `padding_inner`. When it is empty, the styles give it, as the `padding-outer` of the scale.
+ `round`: Whether the scale rounds the step and the start of the bands to whole pixels, and the leaf band to the nearest pixel. When unset, it does not.
+ `clip_padding_lower`: As for `BandScale.clip_padding_lower`.
+ `clip_padding_upper`: As for `BandScale.clip_padding_upper`.
+ `kind`: The kind of the scale, which stylesheets select and which chooses the default palette of a color scale. When unset, the kind follows from the type of the scale's values.

## `point_scale`

```googlesql
point_scale([align: ANY][, padding: ANY][, round: BOOL][, clip_padding_lower: ANY][, clip_padding_upper: ANY][, kind: 'categorical' | 'ordinal']) -> BYTES
```

A discrete scale that places one point for each category, evenly along its range, as the points of a dot plot take it.

**Parameters**

+ `align`: As for `BandScale.align`.
+ `padding`: The space before the first point and after the last, in steps, at least 0. When unset, the styles give it.
+ `round`: Whether the scale rounds the positions to whole pixels. When unset, it does not.
+ `clip_padding_lower`: As for `BandScale.clip_padding_lower`, before the first point.
+ `clip_padding_upper`: As for `BandScale.clip_padding_upper`, after the last point.
+ `kind`: The kind of the scale, which stylesheets select and which chooses the default palette of a color scale. When unset, the kind follows from the type of the scale's values.

## `band_scale`

```googlesql
band_scale([align: ANY][, padding_inner: ANY][, padding_outer: ANY][, round: BOOL][, clip_padding_lower: ANY][, clip_padding_upper: ANY][, kind: 'categorical' | 'ordinal']) -> BYTES
```

A discrete scale that divides its range into one band for each category, as the bars of a bar chart take it.

A discrete scale that divides its range into one band for each category, as the bars of a bar chart take it. Its options are fractions of the step: the length of one band and the padding after it. A bar or a rect spans its band, and another mark stands at the middle of its band, or where the `band_position` of its encoding puts it.

**Parameters**

+ `align`: Where the bands stand in the range when the outer padding leaves room, from 0 at the start to 1 at the end. When unset, 0.5.
+ `padding_inner`: The space between two bands, as a fraction of the step from 0 to 1. When unset, the styles give it.
+ `padding_outer`: The space before the first band and after the last, in steps, at least 0. When unset, the styles give it.
+ `round`: Whether the scale rounds the step and the start of the bands to whole pixels. When unset, it does not.
+ `clip_padding_lower`: The pixels that the range keeps empty before the first band, so the marks at the outer bands stay inside the plot. A minimum, which the fit raises to the reach of the marks. When unset, 0.
+ `clip_padding_upper`: The pixels that the range keeps empty after the last band, as for `clip_padding_lower`.
+ `kind`: The kind of the scale, which stylesheets select and which chooses the default palette of a color scale. When unset, the kind follows from the type of the scale's values.

## `time_scale`

```googlesql
time_scale([timezone: STRING]) -> BYTES
```

A continuous scale of dates and timestamps, which maps time linearly onto its range and ticks on calendar units.

**Parameters**

+ `timezone`: The timezone of the ticks of a timestamp with a timezone, which decides where days and months start: an IANA name, such as `America/New_York`, or `UTC`. When unset, `UTC`. Dates and timestamps without a timezone keep their calendar fields and ignore it.

## `symlog_scale`

```googlesql
symlog_scale([constant: ANY][, clamp: BOOL][, round: BOOL]) -> BYTES
```

A continuous scale that maps `sign(x) * ln(1 + |x| / constant)` of its domain linearly onto its range, so it is close to linear near zero and close to logarithmic far from it, and it takes zero and negative values.

**Parameters**

+ `constant`: The width of the nearly linear part around zero, positive. When unset, 1.
+ `clamp`: As for `LinearScale.clamp`.
+ `round`: As for `LinearScale.round`.

## `pow_scale`

```googlesql
pow_scale([exponent: ANY][, clamp: BOOL][, round: BOOL]) -> BYTES
```

A continuous scale that maps `sign(x) * |x|^exponent` of its domain linearly onto its range.

**Parameters**

+ `exponent`: The exponent, not 0. When unset, 1, which maps as a linear scale does.
+ `clamp`: As for `LinearScale.clamp`.
+ `round`: As for `LinearScale.round`.

## `log_scale`

```googlesql
log_scale([base: ANY][, clamp: BOOL][, round: BOOL]) -> BYTES
```

A continuous scale that maps the logarithms of its domain linearly onto its range.

A continuous scale that maps the logarithms of its domain linearly onto its range. Its domain and its data are positive.

**Parameters**

+ `base`: The base of the logarithm, positive and not 1. When unset, 10.
+ `clamp`: As for `LinearScale.clamp`.
+ `round`: As for `LinearScale.round`.

## `linear_scale`

```googlesql
linear_scale([clamp: BOOL][, round: BOOL]) -> BYTES
```

A continuous scale that maps its domain linearly onto its range.

**Parameters**

+ `clamp`: Whether a value outside the domain maps to the nearest end of the range. When unset, it does not, and the scale extends the line past the domain.
+ `round`: Whether the scale rounds each position to a whole pixel. When unset, it does not.

## `scaled`

```googlesql
scaled(value: T[, scale => ANY][, selection_field => STRING][, title => STRING][, band_position => ANY][, band_level => INT64]) -> T where T: ANY
```

Gives a channel of a mark a value that passes through the channel's scale.

Use `scaled` in an argument of a mark function, as in `color => scaled('Adelie')`, the color that the color scale gives the category `Adelie`. A channel argument without `scaled` or `direct` passes through the scale when the channel has a scale, such as `color` or `y`, and is drawn as it is otherwise, such as `text`. A constant on a position passes through the scale, as `y => 0` does for a reference line at zero. `scaled` makes the scaled kind explicit, and its options set the channel's encoding.

**Parameters**

+ `value`: The value of the channel.
+ `scale`: The scale of the channel, which maps the `scaled_field` and `scaled_value` values of the encoding and of its conditional values to what the mark draws. An encoding has a scale exactly when its value or one of its conditional values is a `scaled_` value. When it is unset, the encoding has the scale of the defaults of a scale's type and domain, so a scale is needed only to change them.

  `x` and `y` take `scaled_` values only, because the axes and the fit of the plot need a scale. The second channels of pairs, `x2`, `y2`, `theta2`, and `radius2`, never have a scale of their own: their `scaled_` values go through the scale of `x`, `y`, `theta`, or `radius`, so they are `scaled_` values exactly when the first channel has a scale. `longitude`, `latitude`, `longitude2`, `latitude2`, `geometry`, `text`, `image`, `path`, and `detail` take no scale, so their values are direct. An encoding of any other channel without a scale has no axis or legend and joins no shared scale.
+ `selection_field`: The column that a selection through this channel records, when the encoded column derives from another, such as the bounds of the bins of a histogram. With a paired channel, `x` with `x2`, `y` with `y2`, `theta` with `theta2`, or `radius` with `radius2`, the two columns are the ends of a range of this column: a point selection records `[x, x2)` as a range without its end, and an interval selection records its range. When it is empty, a selection records the encoded column. It is unset on the second channels of the pairs.
+ `title`: The title of the encoded values in the axis or the legend of the channel's scale. When unset, the title of a field is the name of its column, and a value has none. An empty title gives the axis or the legend no title from this encoding. An axis or a legend whose own title is unset takes the distinct titles of the encodings of its scale, in tree order, joined by `, `, so a plot of `count` bars and a `mean` rule titles its shared `y` axis `count, mean`, and it draws no title when they give none. An encoding without a scale has no axis or legend, so it has no title.
+ `band_position`: Where the mark of a position channel stands in its band, from 0 at the start of the band to 1 at its end, on a band or a nested band scale. When unset, a point mark, such as a symbol, a text, or the vertex of a line, stands at the middle of its band, and a bar or a rect spans its band, from its start to its end. Valid only on `x`, `y`, `theta`, and `radius` through a band or a nested band scale.
+ `band_level`: The level whose band holds the mark of a position channel on a nested band scale, from 0 for the outermost, such as the band of a group behind the bars of its members. A band at an outer level spans the bands of its members, so the bands of a level differ in width. When unset, the leaf. Valid only on `x`, `y`, `theta`, and `radius` through a nested band scale, below the number of its levels.

  At an outer level, only the fields of a path up to the level name its band, and the deeper fields are ignored, so a row of a group, such as `{region: "East", channel: null}`, can hold nulls there. The paths of the encoding then add no path to the scale's domain: the groups and their bands come from the scale's other paths, and a row whose group they do not hold has no band and does not draw.

**Examples**

```googlesql
FROM penguins |> AGGREGATE AVG(body_mass_g) AS mass GROUP BY species |> AGGREGATE bar(x => species, y => scaled(mass, title => 'Body mass (g)')) AS chart
```

Result: `The axis of `y` takes the title `Body mass (g)`.`


## `direct`

```googlesql
direct(value: T) -> T where T: ANY
```

Gives a channel of a mark a value that the mark draws as it is, without the channel's scale.

Use `direct` in an argument of a mark function, as in `fill => direct(color)` for a column of CSS colors. On a position, a direct value is a fraction of the plot: `x => direct(0.5)` is the middle of the plot's width, and `y => direct(0)` its bottom edge, at any size of the plot.

**Parameters**

+ `value`: The value of the channel.

**Examples**

```googlesql
FROM penguins |> AGGREGATE AVG(body_mass_g) AS mass GROUP BY species |> AGGREGATE bar(x => species, y => mass, fill => direct('steelblue')) AS chart
```

Result: `Every bar is steel blue, and the chart has no color legend.`


## `bin`

```googlesql
bin(value: T[, maxbins => FLOAT64][, base => FLOAT64][, divide => ARRAY<FLOAT64>][, extent => ARRAY<FLOAT64>][, span => FLOAT64][, step => FLOAT64][, steps => ARRAY<FLOAT64>][, minstep => FLOAT64][, nice => BOOL][, anchor => FLOAT64]) -> STRUCT<start FLOAT64, end FLOAT64> where T: INT64 | NUMERIC | BIGNUMERIC | FLOAT64
OVER ([PARTITION BY expression[, ...]])
```

Divides the range of numbers into bins of a round step, and returns the bounds of the bin of each row.

`bin` computes over each partition of its `OVER` clause, which takes no `ORDER BY` or frame: the bins of the rows of a partition share one step and one start. `OVER ()` bins all the rows together. Without options, `bin` chooses the step so the range holds at most 20 bins, from steps of a power of 10 divided by 5 or 2. The bounds hold the values from `start` up to, but not including, `end`, and are `NULL` for a value without a bin.

**Parameters**

+ `value`: The number of each row. A `NULL`, `NaN` or infinite value has no bin.
+ `maxbins`: The greatest number of bins. When omitted, 20.
+ `base`: The number base of the steps. When omitted, 10.
+ `divide`: The factors by which a step can be divided to give more bins. When omitted, `[5, 2]`.
+ `extent`: The range to bin, as `[low, high]`. A value outside it has no bin. When omitted, the range of the finite values of the partition.
+ `span`: The length of the range that chooses the step, in place of the length of the extent.
+ `step`: The step of the bins, in place of the step that `bin` chooses.
+ `steps`: The steps that `bin` chooses from: the smallest that gives at most `maxbins` bins.
+ `minstep`: The smallest step. When omitted, 0.
+ `nice`: Whether the first bin starts at a whole multiple of the step. When omitted, `TRUE`. With `FALSE`, the first bin starts at the low end of the range.
+ `anchor`: A value at which a bin starts.

**Examples**

```googlesql
FROM UNNEST([1.0, 7.0, 12.0, 18.0, 25.0]) AS x |> EXTEND bin(x, maxbins => 3) OVER () AS b |> ORDER BY x
```

| x | b |
| --- | --- |
| 1.0 | STRUCT(0.0 AS start, 10.0 AS end) |
| 7.0 | STRUCT(0.0 AS start, 10.0 AS end) |
| 12.0 | STRUCT(10.0 AS start, 20.0 AS end) |
| 18.0 | STRUCT(10.0 AS start, 20.0 AS end) |
| 25.0 | STRUCT(20.0 AS start, 30.0 AS end) |

```googlesql
FROM UNNEST([1.0, 7.0, 12.0]) AS x |> EXTEND bin(x, extent => [0, 10], step => 5) OVER () AS b |> ORDER BY x
```

| x | b |
| --- | --- |
| 1.0 | STRUCT(0.0 AS start, 5.0 AS end) |
| 7.0 | STRUCT(5.0 AS start, 10.0 AS end) |
| 12.0 | NULL |


## `stack`

```googlesql
stack(value: T[, offset => 'zero' | 'normalize' | 'center']) -> STRUCT<start FLOAT64, end FLOAT64> where T: INT64 | NUMERIC | BIGNUMERIC | FLOAT64
OVER ([PARTITION BY expression[, ...]] ORDER BY expression[, ...])
```

Stacks numbers, and returns the bounds of the layer of each row, as for the segments of a stacked bar.

`PARTITION BY` gives the stacks, and `ORDER BY` the order of the layers in each stack, from the bottom. The bounds are `NULL` for a value without a layer.

**Parameters**

+ `value`: The number of each row. A `NULL` or `NaN` value has no layer and adds nothing to its stack.
+ `offset`: Where each stack starts: `'zero'`, where positive values stack up from zero and negative values down from zero, `'normalize'`, where the absolute values stack from 0 to 1 as fractions of their stack's total, or `'center'`, where the absolute values stack centered on zero. When omitted, `'zero'`.

**Examples**

```googlesql
FROM sales |> EXTEND stack(units) OVER (PARTITION BY region ORDER BY product) AS s |> ORDER BY region, product
```

| region | product | units | s |
| --- | --- | --- | --- |
| east | a | 3 | STRUCT(0.0 AS start, 3.0 AS end) |
| east | b | 5 | STRUCT(3.0 AS start, 8.0 AS end) |
| west | a | 2 | STRUCT(0.0 AS start, 2.0 AS end) |
| west | b | 4 | STRUCT(2.0 AS start, 6.0 AS end) |


## `timeunit`

```googlesql
timeunit(value: T[, unit => STRING][, min_unit => STRING][, maxbins => INT64][, week_start => 'monday' | 'sunday'][, fiscal_year_start => INT64]) -> STRUCT<start T, end T, unit STRING> where T: DATE | DATETIME | TIMESTAMP
OVER ([PARTITION BY expression[, ...]])
```

Returns the calendar unit, such as the month or the quarter, that holds each date or instant.

`timeunit` takes no `ORDER BY` or frame in its `OVER` clause. Without a `unit`, it chooses one unit for each partition. With a `unit`, the partitions don't matter, so `OVER ()` serves. A count steps evenly across the longer units, so `'7 days'` are weeks, and a count that divides a longer unit, such as `'15 minutes'` or `'3 months'`, starts again at each start of the longer unit. The result holds the start of the unit, the start of the next unit, and the name of the unit, such as `'month'` or `'3 months'`.

**Parameters**

+ `value`: The `DATE`, `DATETIME` or `TIMESTAMP` of each row. A `TIMESTAMP` takes its units in the civil time of its time zone.
+ `unit`: The unit, such as `'month'`, which holds a span of time and takes a count and a plural, as in `'15 minutes'` or `'3 months'`, or a collapsing unit, such as `'month_of_year'`, which maps each value into one reference year. The fiscal units `'fiscal_year'` and `'fiscal_quarter'` need `fiscal_year_start`. When omitted, `timeunit` chooses one unit for each partition.
+ `min_unit`: The shortest unit that `timeunit` can choose. When omitted, the shortest is the resolution of the data, such as `'month'` for values that are all first days of months.
+ `maxbins`: The greatest number of units that a chosen unit gives the partition. When omitted, 20.
+ `week_start`: The first day of a week. When omitted, `'monday'`.
+ `fiscal_year_start`: The number of the month, from 1 to 12, in which a fiscal year starts, such as 10 for October.

**Examples**

```googlesql
FROM UNNEST([DATE '2024-01-15', DATE '2024-02-03', DATE '2024-02-28']) AS day |> EXTEND timeunit(day, unit => 'month') OVER () AS u |> ORDER BY day
```

| day | u |
| --- | --- |
| 2024-01-15 | STRUCT(DATE '2024-01-01' AS start, DATE '2024-02-01' AS end, 'month' AS unit) |
| 2024-02-03 | STRUCT(DATE '2024-02-01' AS start, DATE '2024-03-01' AS end, 'month' AS unit) |
| 2024-02-28 | STRUCT(DATE '2024-02-01' AS start, DATE '2024-03-01' AS end, 'month' AS unit) |

