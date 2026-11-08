# avenger-datafusion-transforms

Vega's data transforms as Apache DataFusion functions: `bin`, `stack`, `timeunit`, `lump`, and `impute`. Each transform is a window function or a table function. Optimizer rules plan the window functions as aggregates and joins.

```sql
SELECT *, bin(x, 'maxbins', 30) OVER (PARTITION BY g) AS binned FROM t
SELECT *, stack(v, 'offset', 'center') OVER (PARTITION BY x ORDER BY series) AS y FROM t
SELECT *, timeunit(t, 'maxbins', 20) OVER (PARTITION BY g) AS u FROM events
SELECT *, lump(port, 'n', 3, 'by', 'sum(tons)') OVER (PARTITION BY region) AS l FROM catches
SELECT * FROM impute((SELECT month, region, units FROM sales), month, [region], units, value => 0)
```

`session` makes a `SessionContext` with the functions and the rules.

| Transform | Result | Notes |
|---|---|---|
| `bin` | `struct<start, end>` | Vega's bin transform and its options. `bin_params(lo, hi, ...)` gives the bins of an extent. |
| `stack` | `struct<start, end>` | Vega's stack transform. The `center` offset centers each stack on zero. |
| `timeunit` | `struct<start, end, unit>` | A calendar unit in the time zone of the value's type, chosen for each window partition or given with `'unit'`. Units include fiscal quarters and years, and collapsing units such as `'month_of_year'`. |
| `lump` | `struct<value, other, weight>` | Keeps the top categories, or the categories that a condition keeps, and lumps the rest into an Other group. The `'by'` and `'keep'` options are SQL. |
| `impute` | the input's rows, then the added rows | Adds the rows that each group lacks, with a constant or a statistic over a frame. |

The options of the window functions are pairs of a name and a constant after the first argument, because DataFusion 55's named arguments can omit only the last parameters. The scalar form of `timeunit` is `__timeunit`: DataFusion plans a scalar function before a window function of the same name and drops its `OVER` clause, so `timeunit::sql` renames calls without `OVER`.

## Tests

The tests check `bin` and `stack` against Vega's code, and `timeunit` against vega-time, with fixtures in `tests/fixtures/` that `tools/vega-transforms-reference` writes. `impute` and `lump` are checked against equivalent SQL.
