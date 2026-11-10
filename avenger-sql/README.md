# avenger-sql

DataFusion SQL that builds avenger charts. The mark functions of avenger-chart are aggregate functions in avenger-sql, so a query builds a chart of its rows, or one chart for each group with `GROUP BY`, in a `BinaryView` column whose field has the extension name `avenger.chart`:

```sql
FROM penguins
|> AGGREGATE count(*) AS n GROUP BY species
|> AGGREGATE bar(x => species, y => n) AS chart
```

`register` adds the mark functions and the constructors of avenger-chart and the transforms of avenger-datafusion-transforms, such as `bin`, `stack`, `timeunit`, `lump`, and `impute`, to a session, `sql` plans a query in a session, and `expand` returns the SQL that `sql` plans. A query is DataFusion SQL after two rewrites, which DataFusion 55.2 needs to plan these queries:

- A query that starts with `FROM` selects `*`, because DataFusion plans no columns for it.
- A call with named arguments passes `NULL` for each parameter before its last named argument that it omits, because DataFusion binds named arguments only as a prefix of the parameters. An omitted parameter of an avenger function and `NULL` mean the same thing, so `symbol(x => a, y => b, fill => c)` becomes `symbol(x => a, y => b, fill => c, color => NULL)`. A call of a function without parameter names, or with a name that no parameter has, stays as it is, and DataFusion reports its error.

## Mark functions

avenger-sql binds the functions of avenger-chart's `functions` feature: the README of avenger-chart describes their names, their parameters, and the kinds of their arguments. In SQL:

- A channel argument is an expression of each row, such as `body_mass * 2`. Its column is named after the expression, without the qualifiers of its columns, and keeps the type and the field metadata of the expression. Channels with equal expressions share one column.
- `NULL` leaves the channel unset, so an omitted parameter and `NULL` mean the same thing. In `scaled` or `direct`, a `NULL` is a value, which needs a type, such as `direct(CAST(NULL AS VARCHAR))`.
- The `ORDER BY` of a call is the draw order of its rows, `FILTER` selects them, and `GROUP BY` gives one chart for each group.
- A property, `coord`, and an option of `scaled` are constants, such as the folded call of a constructor.

DataFusion's expression simplifier binds each call to an aggregate of its own, which takes only the columns of the call, so an invalid call, such as one without a required channel, fails at planning. A channel argument cannot aggregate, because DataFusion rejects an aggregate inside an aggregate.

A constructor is a scalar function, which returns a fragment of its message in a `BinaryView` value whose field has the extension name `avenger.chart.fragment` and the full name of the message as its extension metadata. It is immutable, so a call of constants folds to a literal, and its errors, such as an unknown enum name, are planning errors.

The constructors of avenger-chart set the options of a chart: `scaled(flipper_length_mm, scale => interval_fit(zero => FALSE))` fits a domain to the data without zero, and `polar_bar(..., coord => polar(inner_radius => 0.5))` sets the options of a plot. Array literals, such as `scale(domain => [0, 100])`, need DataFusion's `nested_expressions` feature, which avenger-sql enables.

A channel argument of a mark function is an expression of each row, so the aggregates of a chart come from an `AGGREGATE` before the mark function, as `count(*)` does above. A window function, such as a bin of avenger-datafusion-transforms, goes in a standard `SELECT`, because DataFusion 55.2 fails to execute a window function in pipe `EXTEND`.

## Compositions

The compositions of avenger-chart's `functions` feature are aggregate functions over a column of charts, so a query composes the charts of the rows of each group:

```sql
FROM penguins
|> AGGREGATE symbol(x => bill_length_mm, y => bill_depth_mm) AS chart GROUP BY sex, island
|> AGGREGATE facet_col(chart, island) AS chart GROUP BY sex
|> AGGREGATE facet_row(chart, sex) AS chart
```

- `layer(chart)` overlays the plots of the charts, `hconcat(chart)` places them in a row, and `vconcat(chart)` in a column, in the order of the `ORDER BY` of the call, such as `layer(chart ORDER BY k)`. The branches of a `UNION ALL` that feeds the call come next, in their order, and then the other columns of the call's input that hold no charts and that the query does not group by, ascending with `NULL` last. So `(a) UNION ALL (b) |> AGGREGATE layer(chart) AS chart` draws `b` over `a`. A `UNION` without `ALL` removes duplicate rows, so its branches have no order.
- `facet_row(chart, key)`, `facet_col(chart, key)`, and `facet_wrap(chart, key)` lay out the charts by the values of the key, ascending with `NULL` last, or in the order of their first rows in the `ORDER BY`. A facet labels each chart with its value, and its column is the name of the key's column. Two rows with one key value are an error, and a `NULL` chart is an empty cell.
- A `facet_row` of `facet_col` charts, or a `facet_col` of `facet_row` charts, aligns their cells, with an empty cell for each value that an inner facet lacks, so it is a grid.
- The options are those of the format's `Plot`, `Concat`, and `Wrap`, such as `layout => concat_layout(gap => 8)` and `facet => facet(side => 'bottom')`, and they are plan-time constants.

The example `sql-chart` shows the chart of a query in a chart app.
