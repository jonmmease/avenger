# avenger-datafusion-transforms

Data transforms as Apache DataFusion functions: the Vega-style `bin`, `stack`, `timeunit`, and `impute`, and `lump`, which keeps the top categories and lumps the rest into an Other group. Each transform is a window function or a table function. Optimizer rules plan the window functions as aggregates and joins.

```sql
SELECT *, bin(x, 'maxbins', 30) OVER (PARTITION BY g) AS binned FROM t
SELECT *, stack(v, 'offset', 'center') OVER (PARTITION BY x ORDER BY series) AS y FROM t
SELECT *, timeunit(t, 'maxbins', 20) OVER (PARTITION BY g) AS u FROM events
SELECT *, lump(port, 'n', 3, 'weight', 'sum(tons)') OVER (PARTITION BY region) AS l FROM catches
SELECT * FROM impute(sales, month, [region], units, value => 0)
```

`register` adds the functions and the rules to a `SessionContext`.

| Transform | Result | Notes |
|---|---|---|
| `bin` | `struct<start, end>` | Bins of a nice step, for at most `maxbins` bins, over the extent of each window partition or an explicit `extent`. `bin_params(lo, hi, ...)` gives the bins of an extent. |
| `stack` | `struct<start, end>` | The layers of each stack, with the offsets `zero`, `normalize`, and `center`, which centers each stack on zero. |
| `timeunit` | `struct<start, end, unit>` | A calendar unit in the time zone of the value's type, chosen for each window partition or given with `'unit'`. Units include fiscal quarters and years, and collapsing units such as `'month_of_year'`. |
| `lump` | `struct<value, other, weight, category_count>` | Keeps the top categories, or the categories that a condition keeps, and lumps the rest into an Other group, with the number of categories that it holds. The `'weight'` and `'keep'` options are SQL. The `'nulls'` option keeps the NULL category apart from the ranking, puts it in the Other group, or gives its rows a NULL lump. |
| `impute` | the input's rows, then the added rows | Adds the rows that each group lacks, with a constant or a statistic over a frame. `fill_gaps` adds the bins, time units, or dates that no row has, and `partition_by` imputes partitions separately. |

A table function such as `impute` reads an input relation: a subquery, a CTE of the statement, or a table by name. Every table function takes `partition_by`, which transforms partitions separately and in parallel. The module `table_function` holds this machinery for new table functions.

The options of the window functions are pairs of a name and a constant after the first argument, because DataFusion 55's named arguments can omit only the last parameters. A `timeunit` call with a `'unit'` takes `OVER ()`: DataFusion plans a scalar function before a window function of the same name, so the scalar form is internal.

## Tests

The tests state what each transform does: bins, stacks, and time units of chosen values and options, time units that tile time across changes of the UTC offset, the rules against the window evaluators, `impute` and `lump` against equivalent SQL, the inputs of `impute`, and the gaps and partitions of `impute`.
