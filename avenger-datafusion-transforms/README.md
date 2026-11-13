# avenger-datafusion-transforms

Data transforms for Apache DataFusion: the Vega-style `bin`, `stack`, `timeunit`, and `impute`, and `lump`, which keeps the top categories and lumps the rest into an Other group. `bin`, `stack`, and `timeunit` are window functions, which optimizer rules plan as aggregates and joins:

```sql
SELECT *, bin(x, 'maxbins', 30) OVER (PARTITION BY g) AS binned FROM t
SELECT *, stack(v, 'offset', 'center') OVER (PARTITION BY x ORDER BY series) AS y FROM t
SELECT *, timeunit(t, 'maxbins', 20) OVER (PARTITION BY g) AS u FROM events
```

`impute` and `lump` are plans over an input, which a front end builds from its planned arguments: `impute::plan` takes the input's plan, its key, group, and value columns, and its named arguments, and `lump::plan` the category, the partition, and the expressions of `weight` and `keep`. avenger-sql plans both from GoogleSQL, as `|> CALL impute(...)` and `lump(...) OVER (...)`. `register` adds the window functions, the rules, and the query planner of the table functions to a `SessionContext`.

| Transform | Result | Notes |
|---|---|---|
| `bin` | `struct<start, end>` | Bins of a nice step, for at most `maxbins` bins, over the extent of each window partition or an explicit `extent`. `bin_params(lo, hi, ...)` gives the bins of an extent. |
| `stack` | `struct<start, end>` | The layers of each stack, with the offsets `zero`, `normalize`, and `center`, which centers each stack on zero. |
| `timeunit` | `struct<start, end, unit>` | A calendar unit in the time zone of the value's type, chosen for each window partition or given with `'unit'`. Units include fiscal quarters and years, and collapsing units such as `'month_of_year'`. |
| `lump` | `struct<value, other, weight, category_count>` | Keeps the top categories, or the categories that a condition keeps, and lumps the rest into an Other group, with the number of categories that it holds. `weight` is an aggregate of the rows of each category, and `keep` a condition of aggregates of its rows and windows over the categories, which take no `PARTITION BY`. `null_category` keeps the NULL category apart from the ranking, puts it in the Other group, or gives its rows a NULL lump. |
| `impute` | the input's rows, then the added rows | Adds the rows that each group lacks, with a constant or a statistic over a frame. `fill_gaps` adds the bins, time units, or dates that no row has, and `partition_by` imputes partitions separately. |

A table function such as `impute` reads the plan of an input relation. Every table function takes `partition_by`, which transforms partitions separately and in parallel. The module `table_function` holds this machinery for new table functions, and `ArgError` places the error of an argument at the argument. A front end that plans `lump` as an expression of a projection writes it as the placeholder window of `lump::window`, and `lump::plan_windows` plans each lump of a window node onto its input. The plan of a lump reads its input more than once, and the analyzer rule `lump::LumpAggregateFusion`, which `register` adds, plans an aggregate grouped by a lump, and by its partition keys, in one read of the input, for the aggregates that merge from states, such as `SUM` and `AVG`.

The options of the window functions are pairs of a name and a constant after the first argument, because DataFusion 55's named arguments can omit only the last parameters. A `timeunit` call with a `'unit'` takes `OVER ()`: DataFusion plans a scalar function before a window function of the same name, so the scalar form is internal.

The module `match_recognize` holds the matcher of row pattern recognition and its node. A pattern compiles to a program, which a Pike VM runs over each partition's rows, with the preferences of GoogleSQL's matcher, both skip modes, and `use_longest_match`. `match_recognize::plan` takes a plan whose rows have their position in the partition and a boolean column for each pattern variable, and outputs each row of each match with its match number and its variable, and one row without a variable for each empty match. avenger-sql plans GoogleSQL's `MATCH_RECOGNIZE` onto it, and computes the measures with an aggregate over its output. `tail_matches` matches the end of a partition as rows arrive: it returns the settled frontier, the first row whose match a later row, a condition that reads past the last row, or `$` can still change, so a streaming runtime matches only the rows from there again.

## Tests

The tests state what each transform does: bins, stacks, and time units of chosen values and options, time units that tile time across changes of the UTC offset, the rules against the window evaluators, `impute` and `lump` against equivalent SQL, and the gaps and partitions of `impute`. The tests write the calls of `impute` and `lump` in their queries, which an adapter of the tests plans through `impute::plan` and `lump::plan`, and compare the results with reference queries in DataFusion's SQL, which only the tests enable.
