# avenger-sql

DataFusion SQL that builds avenger charts. The mark functions of avenger-chart's `datafusion` feature are aggregate functions, so a query builds a chart of its rows, or one chart for each group with `GROUP BY`, in a `BinaryView` column whose field has the extension name `avenger.chart`:

```sql
FROM penguins
|> AGGREGATE count(*) AS n GROUP BY species
|> AGGREGATE bar(x => species, y => n) AS chart
```

`register` adds the mark functions to a session, `sql` plans a query in a session, and `expand` returns the SQL that `sql` plans. A query is DataFusion SQL after two rewrites, which DataFusion 55.2 needs to plan these queries:

- A query that starts with `FROM` selects `*`, because DataFusion plans no columns for it.
- A call with named arguments passes `NULL` for each parameter before its last named argument that it omits, because DataFusion binds named arguments only as a prefix of the parameters. An omitted parameter of an avenger function and `NULL` mean the same thing, so `symbol(x => a, y => b, fill => c)` becomes `symbol(x => a, y => b, fill => c, color => NULL)`. A call of a function without parameter names, or with a name that no parameter has, stays as it is, and DataFusion reports its error.

A channel argument of a mark function is an expression of each row, so the aggregates of a chart come from an `AGGREGATE` before the mark function, as `count(*)` does above. A window function, such as a bin of avenger-datafusion-transforms, goes in a standard `SELECT`, because DataFusion 55.2 fails to execute a window function in pipe `EXTEND`.

The example `sql-chart` shows the chart of a query in a chart app.
