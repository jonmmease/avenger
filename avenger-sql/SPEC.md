# avenger-sql in brief

The pipe subset of GoogleSQL, with trailing commas allowed in every comma-separated list, and functions that build avenger charts. This is a short guide for writing avenger-sql. The crate READMEs and `library::reference` hold the details.

## Scripts

A script is statements separated by `;`. The value of a script is the rows of its last query.

```sql
DECLARE min_mass INT64 DEFAULT 3000;

CREATE TEMP FUNCTION kg(g FLOAT64) AS (g / 1000);

FROM penguins
|> WHERE body_mass_g >= min_mass
|> AGGREGATE AVG(kg(body_mass_g)) AS kg GROUP BY species
```

- `DECLARE name [type] [DEFAULT value]` declares variables, before any other statement. `SET name = value` and `SET (a, b) = (...)` assign them. A variable stands in the options of the transforms, as in `bin(x, step => width)`, also when its value is the result of a query, since the plan binds it before the query runs.
- `CREATE TEMP TABLE t AS (query)`, or `CREATE TEMP TABLE t (columns)`, names a table for the statements after it.
- `CREATE TEMP [AGGREGATE | TABLE] FUNCTION` defines a function. A parameter takes a type, `ANY TYPE`, `ANY TABLE`, or `TABLE<...>`. A table function's body is a query.
- `IMPORT MODULE path [AS alias]` imports the public functions and constants of a module that the catalog holds. A module starts with `MODULE path;` and holds `CREATE PUBLIC` or `CREATE PRIVATE` functions and constants.
- Query parameters are `@name`.

## Queries

A query starts with `FROM` and continues with pipe operators. `SELECT ... FROM` and a standard `WITH` are errors that suggest the pipe form.

```sql
FROM orders AS o
JOIN customers AS c USING (customer_id)
|> WHERE o.total > 0
|> EXTEND o.total * 0.2 AS tax
|> AGGREGATE SUM(total) AS revenue, COUNT(*) AS n GROUP BY region
|> ORDER BY revenue DESC
|> LIMIT 10
```

- Pipe operators: `WHERE`, `SELECT`, `EXTEND`, `SET`, `DROP`, `RENAME`, `AS`, `AGGREGATE ... [GROUP [AND ORDER] BY ...]` (with `GROUPING SETS`, `ROLLUP`, `CUBE`), `ORDER BY`, `LIMIT ... [OFFSET ...]`, `DISTINCT`, `JOIN` and its kinds, `UNION`/`INTERSECT`/`EXCEPT` (with `BY NAME` or `CORRESPONDING`), `PIVOT`, `UNPIVOT`, `TABLESAMPLE`, `MATCH_RECOGNIZE`, `WITH`, and `CALL` of a table function.
- In `FROM`: tables, `(query)`, joins in parentheses, `UNNEST(...)`, table function calls such as `f(TABLE t, 3)`, `LATERAL`, and `WITH OFFSET`.
- Expressions are GoogleSQL's: subqueries, `EXISTS`, `IN`, `CASE`, `CAST` and `SAFE_CAST`, `EXTRACT`, intervals, `ARRAY` and `STRUCT` literals, typed literals, window functions with named windows and a `WINDOW` clause, and `SAFE.` calls.
- Names are case-insensitive. Quote a name that is a reserved word, or has dashes outside a table path, in backticks.
- A call can name its arguments, as in `f(x, step => 2)`. A parameter whose name is a reserved word takes a trailing underscore, as `range_` of `scale` does.
- GoogleSQL's syntax outside the subset, such as DDL and DML statements, is an error that names it as unsupported.

## Trailing commas

Every comma-separated list takes a comma after its last item, whether GoogleSQL takes one or not: call arguments, `IN (...)`, arrays, struct fields and values, column lists, `GROUP BY`, `PARTITION BY`, `ORDER BY`, `OPTIONS`, parameters, and the lists of every pipe operator.

```sql
FROM penguins
|> WHERE species IN ('Adelie', 'Gentoo',)
|> AGGREGATE
     COUNT(*) AS n,
     AVG(body_mass_g) AS mass,
   GROUP BY
     species,
     island,
```

There are two exceptions:

- `(x,)` is an error, since GoogleSQL has no tuple of one value. `(a, b,)` is a tuple of two.
- The names of a `DECLARE` take no trailing comma, since a type can follow them: `DECLARE a, b, INT64` reads `INT64` as a third name, and lacks a type.

## Charts

A mark is an aggregate function, so `|> AGGREGATE` builds a chart of the rows, or one chart for each group with `GROUP BY`. A mark or a composition returns a `CHART`, the bytes of the chart in a column that holds charts. Each channel is a named argument that reads each row, or aggregates them.

```sql
FROM penguins
|> AGGREGATE COUNT(*) AS n GROUP BY species
|> AGGREGATE bar(x => species, y => n) AS chart
```

- Marks include `bar`, `line`, `area`, `symbol`, `rule`, `rect`, `text`, `trail`, `path`, `image`, their `polar_` forms, and the `geo_` marks of maps.
- `scaled(value, scale => ...)` and `direct(value)` choose whether a channel goes through a scale.
- A constructor builds the options of a chart, such as `polar(inner_radius => 0.5)` for a mark's `coord`, or `interval_fit(zero => FALSE)` for a scale. Its arguments are constants.
- A mark call takes the options of its plot and its layer as named constants after its channels: `title`, `attribution`, `tools`, `width`, `height`, `id`, and `classes`, as in `symbol(x => a, y => b, title => 'Points', tools => [navigate(), tooltip()])`. A navigation goes on the plot, and a tooltip or a selection on the mark's layer.
- The `ORDER BY` of a mark call is the draw order of its rows, and a `WHERE` before the `AGGREGATE` selects them.
- A channel takes aggregates, as in `bar(x => species, y => COUNT(*))`: each channel without an aggregate is a key, and the call aggregates once for each group and key. A channel's column takes the text of its value, such as `COUNT(*)`, as its title.
- A channel takes windows on its rows too, as in `bar(x => bin(body_mass_g), y => COUNT(*))`. There `bin`, `timeunit`, and `lump` take no `OVER`, and compute over the whole input, or over each group with `OVER panel`.
- A window that reads an aggregate runs on the aggregated rows, as `stack(COUNT(*))` does. `stack` takes no `OVER` there and stacks within `position`, the groups and the position keys. The windows `panel`, `position`, and `series` partition by the groups, and by the position or the series keys, and give an order where GoogleSQL leaves one unspecified.

A composition aggregate composes the charts of the rows of a group, one chart for each row:

```sql
FROM penguins
|> AGGREGATE symbol(x => bill_length_mm, y => bill_depth_mm) AS chart GROUP BY island
|> AGGREGATE facet_col(chart, island) AS chart
```

- `facet_row(chart, key)`, `facet_col(chart, key)`, and `facet_wrap(chart, key)` lay out the charts by the values of a key, and label them. A `facet_row` of `facet_col` charts is a grid.
- `layer_agg(chart ORDER BY k)` overlays the plots of the charts, and `hconcat_agg` and `vconcat_agg` place them in a row or a column, in the order of the `ORDER BY`. Without one, their order is unspecified.
- The options of a composition are named constants, as in `facet_wrap(chart, species, layout => wrap_layout(max_columns => 2))`.
- The scalars `layer(a, b, ...)`, `hconcat(...)`, and `vconcat(...)` compose the charts of one row, in the order of their arguments, such as the marks of one `AGGREGATE` item: `layer(bar(...), symbol(...))`. Their options follow the charts by name.

## Transforms

- `bin`, `stack`, and `timeunit` are window functions with named options, as in `bin(body_mass_g, maxbins => 30) OVER ()`. Each returns a struct, such as the `start` and `end` of a bin.
- `lump(category, n => 3) OVER (PARTITION BY g)` keeps the heaviest categories of each partition and lumps the rest into an Other group. `weight` and `keep` set the weight and the rule.
- `impute(key, [groups], value_column, value => 0)` adds the rows that each group lacks, as `|> CALL impute(...)` or in `FROM`.

## Row patterns

`MATCH_RECOGNIZE` finds runs of rows that match a pattern, with GoogleSQL's syntax and one row for each match:

```sql
FROM prices
|> MATCH_RECOGNIZE(
     PARTITION BY symbol
     ORDER BY day
     MEASURES FIRST(day) AS start_day, LAST(up.price) AS high
     PATTERN (dn+ up+)
     DEFINE dn AS price < PREV(price), up AS price > PREV(price))
```

## Where to look

- `avenger-sql/README.md`: the language, marks, transforms, `MATCH_RECOGNIZE`'s choices, and conformance with GoogleSQL.
- `avenger-sql-syntax/sql.ungram`: the grammar of the subset.
- `library::reference`: every function, its forms, and its parameters.
