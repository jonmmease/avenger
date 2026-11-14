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

- `DECLARE name [type] [DEFAULT value]` declares variables, before any other statement. `SET name = value` and `SET (a, b) = (...)` assign them.
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
- Expressions are GoogleSQL's: subqueries, `EXISTS`, `IN`, `CASE`, `CAST ... FORMAT`, `EXTRACT`, intervals, `ARRAY` and `STRUCT` literals, typed literals, window functions with named windows and a `WINDOW` clause, and `SAFE.` calls.
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

A mark is an aggregate function, so `|> AGGREGATE` builds a chart of the rows, or one chart for each group with `GROUP BY`. Each channel is a named argument that reads each row.

```sql
FROM penguins
|> AGGREGATE COUNT(*) AS n GROUP BY species
|> AGGREGATE bar(x => species, y => n) AS chart
```

- Marks include `bar`, `line`, `area`, `symbol`, `rule`, `rect`, `text`, `trail`, `path`, `image`, their `polar_` forms, and the `geo_` marks of maps.
- `scaled(value, scale => ...)` and `direct(value)` choose whether a channel goes through a scale.
- A constructor builds the options of a chart, such as `polar(inner_radius => 0.5)` for a mark's `coord`, or `interval_fit(zero => FALSE)` for a scale. Its arguments are constants.
- The `ORDER BY` of a mark call is the draw order of its rows, and a `WHERE` before the `AGGREGATE` selects them.
- Compute aggregates in an `AGGREGATE` before the mark, since a channel reads each row.

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
