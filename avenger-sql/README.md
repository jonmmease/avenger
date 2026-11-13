# avenger-sql

GoogleSQL pipe queries that build avenger charts, planned onto DataFusion. The mark functions of avenger-chart are aggregate functions, so a query builds a chart of its rows, or one chart for each group with `GROUP BY`, in a `BinaryView` column whose field has the extension name `avenger.chart`:

```sql
FROM penguins
|> AGGREGATE COUNT(*) AS n GROUP BY species
|> AGGREGATE bar(x => species, y => n) AS chart
```

`sql(ctx, text)` plans a script over the tables of a DataFusion session and returns the rows of its last query as a DataFrame. avenger-sql-syntax parses the script, and avenger-sql resolves its names and types by GoogleSQL's rules and plans each statement as a DataFusion logical plan. DataFusion's SQL planner takes no part, and the build of avenger-sql holds no sqlparser. `register(ctx)` adds what the plans of the transforms need to run: the query planner of `impute`, and the optimizer rules of the window transforms.

## The language

A query is GoogleSQL's pipe syntax, the subset that avenger-sql-syntax parses. It starts with `FROM`, and its pipe operators are `WHERE`, `SELECT`, `EXTEND`, `SET`, `DROP`, `RENAME`, `AS`, `AGGREGATE` with `GROUP BY`, `GROUPING SETS`, `ROLLUP` and `CUBE`, `ORDER BY`, `LIMIT`, `DISTINCT`, the joins, the set operations, `PIVOT`, `UNPIVOT`, `TABLESAMPLE`, `MATCH_RECOGNIZE`, `WITH`, and `CALL`. Expressions include subqueries, `UNNEST`, named windows, and the `WINDOW` clause. A standard `SELECT ... FROM` query isn't in the subset, and its error offers a rewrite to pipe syntax for a simple query. GoogleSQL's other syntax outside the subset, such as DDL and DML statements, gets an error that names it as unsupported.

A script is statements separated by `;`. `DECLARE` and `SET` give script variables, `CREATE TEMP TABLE` and `CREATE TEMP FUNCTION` define names for the statements after them, and `IMPORT MODULE` imports the public functions and constants of a module that the catalog holds. `sql` returns the rows of the last query, after the variables before it.

The functions are those of the `library`: GoogleSQL's functions, avenger-chart's marks and constructors, and the transforms of avenger-datafusion-transforms. Each function declares its forms, with GoogleSQL's types and coercion, and documents itself and its parameters. `library::reference` renders the reference of every function, which notes where a function differs from GoogleSQL. A parameter whose name is a reserved word of GoogleSQL takes the name with a trailing underscore, as `range_` of `scale` does.

## Row pattern recognition

`MATCH_RECOGNIZE` finds the runs of rows that match a pattern, as a pipe operator or after a source in `FROM`, with the syntax of [GoogleSQL's `MATCH_RECOGNIZE`](https://github.com/google/googlesql/blob/2026.11.1/docs/pipe-syntax.md#match_recognize_pipe_operator). A pattern variable's condition reads the row, and `PREV` and `NEXT` read its neighbors in the partition. A measure reads aggregates of the match's rows, such as `COUNT(*)`, or of one variable's rows, such as `MIN(dn.price)`:

```sql
FROM prices
|> MATCH_RECOGNIZE(
     PARTITION BY symbol
     ORDER BY day
     MEASURES FIRST(day) AS start_day, MIN(dn.price) AS low, LAST(up.price) AS high
     PATTERN (st dn+ up+)
     DEFINE
       st AS TRUE,
       dn AS price < PREV(price),
       up AS price > PREV(price))
```

The output has one row for each match: the partition columns, then the measures. The clause supports GoogleSQL's subset of SQL:2016. `ALL ROWS PER MATCH`, `ONE ROW PER MATCH`, `RUNNING` and `FINAL`, `SUBSET`, `PERMUTE`, exclusion, and `AFTER MATCH SKIP TO` a variable are errors. avenger-sql behaves as follows where the syntax leaves a choice:

- `MATCH_ROW_NUMBER()` is the row's position in its partition, from 1, so the rows of a second match can be 4, 5 and 6.
- `FIRST` and `LAST` leave out `NULL` values: `FIRST(v)` is the first value of the match's rows that is not `NULL`.
- Rows with equal `ORDER BY` keys have no order of their own. avenger-sql orders them once, and `PREV`, `NEXT` and the matcher read that one order, so a query gives a consistent result. Another engine can order the ties another way.
- A quantifier bound and the option `use_longest_match` are literals, not query parameters, because avenger-sql compiles the pattern when it plans the query.
- A pattern compiles to at most 100,000 instructions, which bounded quantifiers such as `a{50000}` reach. The matcher's cost for each row is at most the size of the pattern's program, so no pattern takes exponential time.
- The clause holds the input's rows in memory, in one task for each group of partitions.

## Errors

A script with errors returns `ScriptError` as `DataFusionError::External`: the syntax errors of the parse, or else every error of the analysis, each a `Diagnostic` with the range of its text. Its message gives each error with its line and column. `analysis::analyze` returns the same diagnostics with the semantic model of a script, its names and the forms of its calls, for an editor.

## Mark functions

The library binds the functions of avenger-chart's `functions` feature: the README of avenger-chart describes their names, their parameters, and the kinds of their arguments. In a query:

- A channel argument is an expression of each row, such as `body_mass * 2`. Its column is named after the expression, without the qualifiers of its columns, and keeps the type and the field metadata of the expression. Channels with equal expressions share one column.
- `NULL` leaves the channel unset, so an omitted parameter and `NULL` mean the same thing. In `scaled` or `direct`, a `NULL` is a value of GoogleSQL's type of `NULL`, `INT64`, or of a cast, such as `direct(CAST(NULL AS STRING))`.
- The `ORDER BY` of a call is the draw order of its rows, a `WHERE` before the `AGGREGATE` selects them, and `GROUP BY` gives one chart for each group.
- A property, `coord`, and an option of `scaled` are constants, such as the folded call of a constructor.

Each call binds at planning to an aggregate of its own, which takes only the columns of the call, so an invalid call, such as one without a required channel, fails at planning. A channel argument reads each row, so the aggregates of a chart come from an `AGGREGATE` before the mark, as `COUNT(*)` does above.

A constructor is a scalar function, which returns a fragment of its message in a `BinaryView` value whose field has the extension name `avenger.chart.fragment` and the full name of the message as its extension metadata. A call of constants folds to a literal, and its errors, such as an unknown enum name, are planning errors. The constructors set the options of a chart: `scaled(flipper_length_mm, scale => interval_fit(zero => FALSE))` fits a domain to the data without zero, and `polar_bar(..., coord => polar(inner_radius => 0.5))` sets the options of a plot.

## Transforms

`bin`, `stack`, and `timeunit` are window functions, whose options are named arguments, as `bin(body_mass_g, maxbins => 30) OVER ()`.

`lump` keeps the heaviest categories of each partition of its `OVER` clause, or the categories that a condition keeps, and lumps the rest into an Other group:

```sql
FROM catches
|> EXTEND lump(port, weight => SUM(tons), keep => ROW_NUMBER() OVER ranked <= 3)
     OVER (PARTITION BY region) AS l
```

Its arguments read three scopes. The category reads each row. `weight`, `COUNT(*)` by default, is an aggregate of the rows of each category. `keep` is evaluated once for each category: like a `HAVING` clause it compares aggregates of the category's rows, and like `QUALIFY` it can use windows over the categories of the partition, which take no `PARTITION BY`. The named window `ranked` orders the categories by weight, the heaviest first. `n => 3` keeps the first 3 categories of `ranked`, in place of `keep`. `other` labels the Other group, and `null_category` keeps the `NULL` category as its own, lumps it into the Other group, or drops its rows from every lump. The result is `STRUCT<value, other, weight, category_count>`.

`impute` is a table function that adds the rows that each group lacks. A pipe `CALL` imputes the rows before it, and `FROM` the rows of a table or a query:

```sql
FROM sales
|> AGGREGATE SUM(units) AS units GROUP BY month, region
|> CALL impute(month, [region], units, value => 0, flag => 'imputed')

FROM impute(TABLE monthly, month, [region], units, method => 'avg', frame => [-1, 1])
```

Its arguments name the columns of its input: the key, an array of the group columns, and the column to impute, then named arguments such as `fill_gaps` and `partition_by`. The reference of the library documents each argument of each transform.

The example `sql-chart` shows the chart of a query in a chart app, and `chart-lump` the lumped genres of movies.

## Conformance with GoogleSQL

Two kinds of tests compare avenger-sql with GoogleSQL's `execute_query` 2026.11.1. `tests/googlesql_queries.rs` runs pipe queries whose results `execute_query` recorded in `tests/data/googlesql_pipe_*.txt`, and runs in every test run. The ignored test `tests/conformance.rs` runs each query of `tests/conformance/queries.txt` through `sql` and through `execute_query --mode=execute` over the CSV tables of `tests/conformance`, which both read as `STRING` columns and `tables.sql` types, and compares the names of the columns and the rows, or the errors. A difference that `tests/conformance/known.txt` lists, such as a function that the library doesn't map yet, is explained, and any other fails the test. The report is in `tests/output/conformance.md`. The test needs the binary at `scratch/tools/googlesql/execute_query_macos`, and without it skips with a message:

```bash
CARGO_TARGET_DIR=target cargo test --release -p avenger-sql --test conformance -- --ignored
```
