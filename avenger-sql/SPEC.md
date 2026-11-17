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
- A call can name its arguments, as in `f(x, step => 2)`. A parameter keeps its name when it is a reserved word, as `range` of `scale` does, by two rules beyond GoogleSQL. A word that `=>` follows is an argument's name, reserved or not. `SELECT` and `INTERVAL` that `(` follows are calls, as `select(...)` and `interval(...)` are, except that `INTERVAL (x) DAY` is an interval literal, since a date part follows the parentheses.
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
- A bare value of the rows goes through the channel's scale, and a bare constant goes through the scale on a position, as `y => 0` does, and is drawn as it is on another channel, as `fill => 'red'` is. `datum(...)` makes a constant go through the scale, as in `color => datum('Adelie')`, and `value(...)` draws a constant or a column as it is, as in `fill => value(css_color)`.
- A channel with a scale takes its channel function, named after the channel, which keeps the kind of its value and takes the options of the channel's encoding by name: `scale`, `axis`, `title`, `band_position`, `band_level`, and `selection_field` on `x`, `y`, `theta`, and `radius`, none on `x2`, `y2`, `theta2`, and `radius2`, which follow the first position, and `scale`, `legend`, `title`, and `selection_field` on the other channels, such as `color` and `size`. So `fill => fill('red')` draws red, as `fill => 'red'` does. The value of a channel function can be `datum(...)` or `value(...)`:

```sql
FROM penguins
|> AGGREGATE symbol(
     x => x(body_mass_g, axis => axis(grid => FALSE)),
     y => y(AVG(flipper_length_mm), title => 'Flipper length (mm)'),
     color => color(datum('Adelie'), legend => FALSE),
   ) AS chart
   GROUP BY species
```

- The `format` patterns of a chart, on labels, a text mark, or a tooltip field, are GoogleSQL's: a number pattern is a `FORMAT` specifier, such as `%'.2f`, and a date pattern holds the format elements of `FORMAT_DATE`, such as `%Y-%m`. A pattern with a `'` takes a double-quoted string:

```sql
FROM penguins
|> AGGREGATE symbol(
     x => x(body_mass_g, axis => axis(labels => "%'.0f g")),
     y => y(bill_length_mm, axis => labels(format => '%.1f')),
   ) AS chart
```

- A mark call takes the options of its plot and its layer as named constants after its channels: `title`, `attribution`, `tools`, `width`, `height`, `id`, and `classes`, as in `symbol(x => a, y => b, title => 'Points', tools => [navigate(), tooltip()])`. A navigation goes on the plot, and a tooltip or a selection tool on the mark's layer. A selection tool, `select(...)`, writes a selection that the chart declares, which avenger-sql can't declare yet, so a chart with one is an error.
- The `ORDER BY` of a mark call is the draw order of its rows, and a `WHERE` before the `AGGREGATE` selects them. The `order` channel of a line or an area orders the rows of its path, as in `line(x => x, y => y, order => step)`.
- A channel takes aggregates, as in `bar(x => species, y => COUNT(*))`: each channel without an aggregate is a key, and the call aggregates once for each group and key. A channel's column takes the text of its value, such as `COUNT(*)`, as its title.
- A channel takes windows on its rows too, as in `bar(x => bin(body_mass_g), y => COUNT(*))`. There `bin`, `timeunit`, and `lump` take no `OVER`, and compute over the whole input, or over each group with `OVER panel`.
- A window that reads an aggregate runs on the aggregated rows, as `stack(COUNT(*))` does. `stack` takes no `OVER` there and stacks within `position`, the groups and the position keys. The windows `panel`, `position`, and `series` partition by the groups, and by the position or the series keys, and give an order where GoogleSQL leaves one unspecified.

A composition aggregate composes the charts of the rows of a group, one chart for each row:

```sql
FROM penguins
|> AGGREGATE symbol(x => bill_length_mm, y => bill_depth_mm) AS chart GROUP BY island
|> AGGREGATE facet_column(chart, island) AS chart
```

- `facet_row(chart, key)`, `facet_column(chart, key)`, and `facet_wrap(chart, key)` lay out the charts by the values of a key, and label them. A `facet_row` of `facet_column` charts is a grid.
- `layer_agg(chart ORDER BY k)` overlays the plots of the charts, and `hconcat_agg` and `vconcat_agg` place them in a row or a column, in the order of the `ORDER BY`. Without one, their order is unspecified.
- The options of a composition are named constants, as in `facet_wrap(chart, species, max_columns => 2)`. `resolve => resolve(y => 'independent')` gives each child its own scale of `y`, `parts => parts(legend => 'keep')` keeps a legend beside each plot, and `tracks => [200, 400]` sets the length of each child's track. An array that mixes lengths and shares writes a constructor for each track, as in `tracks => [track(200), track(share => 2, max => 600)]`.
- The scalars `layer(a, b, ...)`, `hconcat(...)`, and `vconcat(...)` compose the charts of one row, in the order of their arguments, such as the marks of one `AGGREGATE` item: `layer(bar(...), symbol(...))`. Their options follow the charts by name.

## Options of charts

A constructor builds an option of a chart, such as `scale(zero => FALSE)` for a scale or `polar(inner_radius => 0.5)` for a mark's `coord`. Its arguments are constants. Only an argument of a chart function takes a constructor: a mark, a composition, a channel function, or a constructor in them. Elsewhere its name is an error that names where it belongs. The name resolves by the option that the argument expects, so one name builds different options in different places: `type => log(base => 2)` is a log scale in `scale(...)`, where `LOG(x)` elsewhere is GoogleSQL's, and `type => point(...)` is a point scale in `scale(...)` and a point selection in `select(...)`.

A named argument takes one of three forms:

1. An ordinary value: a literal, an array, a struct, a script variable, or a string that names a member of a choice, as `type => 'log'` and `range => 'tableau10'` do.
2. The constructor named after the argument, as `scale => scale(zero => FALSE)`.
3. One level down, a constructor named after an argument of that constructor: `scale => pin(1000, position => 'low')` is short for `scale => scale(pin => pin(1000, position => 'low'))`. No form skips more than one level.

An element of an array takes the constructor that its argument names, as `levels => [level(...)]` and `fields => [field(...)]` do, or a member of a choice, as `tools => [navigate(), tooltip()]` does. `NULL` leaves an argument unset.

A shorthand is an ordinary value that fills a plain field of the argument's own option: `title => 'Mass'` fills the text of a title, `axis => FALSE` hides an axis, `scale => 'log'` picks the type of a scale, and `tracks => [200, 400]` fills the length of each track. A shorthand never fills a field of a child, so `axis => 'Bill length'` is an error, and `axis => title('Bill length')` sets the axis title by form 3.

A value always wraps in its constructor: where `foo => v` is valid, `foo => foo(v)` is valid too and means the same, so named arguments can follow the value, as in `title => title('Mass', font_size => 14)`. A constructor takes its shorthand at position 0, and every other argument by name. A shorthand can be valid at position 0 when it is not valid alone: `pin(1000, position => 'low')` is a pin, and `pin => 1000` is an error, since a pin needs its position.

```sql
FROM penguins
|> AGGREGATE symbol(
     x => x(bill_length_mm, axis => title('Bill length (mm)')),
     y => y(body_mass_g, scale => 'log', axis => axis(grid => FALSE)),
     title => title('Penguins', subtitle => 'Palmer Archipelago'),
   ) AS chart
```

## The layout of nested calls

Every example follows one layout of calls and arrays, which the printer of lifted calls writes too:

1. A call or an array that fits in 100 columns stays on one line, however deep it nests.
2. A call that doesn't fit breaks after its `(`, with one argument on each line, two spaces deeper than the line on which the call starts, and its `)` on its own line at that line's depth. A line's depth is the column of its first word, except that a pipe operator's line has the depth of its keyword, column 3 after `|> `. So the arguments of a mark in `|> AGGREGATE` start at column 5, and its `)` lines up with `AGGREGATE`. Each argument follows rule 1 again.
3. A named argument's value starts on the line of its name: `y => y(` opens the call, and `),` closes it at the depth of `y`.
4. Each line of a broken call ends with a comma, the last one included.
5. An array follows the same rules with `[` and `]`.

Arguments never align under the opening parenthesis.

```sql
FROM penguins
|> AGGREGATE symbol(
     x => x(
       bill_length_mm,
       title => 'Bill length (mm)',
       axis => axis(labels => labels(angle => -45), grid => FALSE),
     ),
     y => y(
       body_mass_g,
       scale => scale(type => log(base => 2), pin => pin(1000, position => 'low')),
     ),
     color => color(
       species,
       scale => scale(range => 'tableau10'),
       legend => legend(orient => 'bottom', columns => 2),
     ),
     stroke => 'black',
     tools => [navigate(), tooltip(nearest => TRUE, max_radius => 20)],
   ) AS chart
   GROUP BY island
```

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
- `avenger-chart/VEGA-LITE.md`: the names of Vega-Lite and Altair in the format and in avenger-sql.
