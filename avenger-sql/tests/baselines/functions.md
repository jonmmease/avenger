## `COUNT`

```googlesql
COUNT(value: T) -> INT64 where T: ANY
```

Returns the number of rows, or of the rows whose value is not `NULL`.

`COUNT(*)` counts every row of the group, and `COUNT(value)` the rows whose value is not `NULL`. With `DISTINCT`, it counts the distinct values.

**Parameters**

+ `value`: The value whose rows to count.

**Clauses**: `DISTINCT`, `HAVING MAX` and `HAVING MIN`, `OVER`

**Examples**

```googlesql
FROM UNNEST([1, 2, 2, 3, NULL]) AS x |> AGGREGATE COUNT(*) AS row_count, COUNT(x) AS value_count, COUNT(DISTINCT x) AS distinct_count
```

| row_count | value_count | distinct_count |
| --- | --- | --- |
| 5 | 4 | 3 |


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


