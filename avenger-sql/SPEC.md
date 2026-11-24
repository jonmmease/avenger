# avenger-sql in brief

The pipe subset of GoogleSQL, with trailing commas allowed in every comma-separated list, and functions that build avenger charts. This is a short guide for writing avenger-sql. The crate READMEs and `library::reference` hold the details.

## Scripts

A script is statements separated by `;`. The value of a script is the rows of its last query.

```sql
DECLARE min_mass INT64 DEFAULT 3000;

CREATE TEMP FUNCTION kg(g FLOAT64) AS (g / 1000);

FROM penguins
|> WHERE body_mass_g >= min_mass
|> AGGREGATE avg(kg(body_mass_g)) AS kg GROUP BY species
```

- `DECLARE name [type] [DEFAULT value]` declares variables, before any other statement. `SET name = value` and `SET (a, b) = (...)` assign them. A variable stands in the options of the transforms, as in `bin(x, step => width)`, also when its value is the result of a query, since the plan binds it before the query runs.
- `CREATE TEMP TABLE t AS (query)`, or `CREATE TEMP TABLE t (columns)`, names a table for the statements after it.
- `CREATE TEMP EXTERNAL TABLE` and `LOAD DATA {OVERWRITE | INTO} TEMP TABLE` name a table of files for the statements after it, as in a data script below.
- `CREATE TEMP [AGGREGATE | TABLE] FUNCTION` defines a function. A parameter takes a type, `ANY TYPE`, `ANY TABLE`, or `TABLE<...>`. A table function's body is a query.
- `IMPORT MODULE path [AS alias]` imports the public functions and constants of a module that the catalog holds. A module starts with `MODULE path;` and holds `CREATE PUBLIC` or `CREATE PRIVATE` functions and constants.
- Query parameters are `@name`.

## Data scripts

A project's data scripts register the tables that its scripts read, with GoogleSQL's DDL. A data script holds no query, `DECLARE`, or `SET`.

```sql
CREATE SCHEMA vega_datasets OPTIONS (description = 'The datasets of the Vega examples');

CREATE EXTERNAL TABLE vega_datasets.cars
OPTIONS (format = 'JSON', uris = ['data/files/vega_datasets/cars.json']);

LOAD DATA OVERWRITE sales
FROM FILES (format = 'CSV', uris = ['data/files/sales/*.csv']);

CREATE TEMP VIEW recent AS (FROM sales |> WHERE month > 6);

CREATE TABLE totals AS (FROM recent |> AGGREGATE sum(amount) AS total GROUP BY region);
```

- Declared tables read their files or run their query at each query: `CREATE EXTERNAL TABLE` and `CREATE VIEW`. Loaded tables read them once, into memory: `LOAD DATA` and `CREATE TABLE ... AS`.
- `LOAD DATA INTO t` creates `t`, or appends to a `t` that an earlier `LOAD DATA` or `CREATE TABLE ... AS` of the script creates. `LOAD DATA OVERWRITE t` creates `t`, which no earlier statement of the script defines.
- The options of files, in an external table's `OPTIONS (...)` and in `FROM FILES (...)`: `format` (`CSV`, `NEWLINE_DELIMITED_JSON` or `JSON`, `PARQUET`, `AVRO`, `ARROW`) and `uris` are required. CSV files take `skip_leading_rows` (0 or 1), `field_delimiter`, `quote`, `null_marker`, and `allow_quoted_newlines`, CSV and JSON files take `compression` (`GZIP`, `BZIP2`, `XZ`, `ZSTD`), and `hive_partition_uri_prefix`, the path of a directory, goes with `WITH PARTITION COLUMNS`. Without a list, `WITH PARTITION COLUMNS` reads the keys of the paths after the prefix, as `year=2025/month=01/`, each of the type that all its values have, as BigQuery detects them: `INT64`, else `DATE`, as `2025-01-31`, else `TIMESTAMP`, as `2025-01-31 16:00:00+00`, else `STRING`, so `month=01` is the `INT64` 1. `description` is an option of the table, in the `OPTIONS (...)` of `LOAD DATA`, a view, a table, or a schema. Other options of BigQuery's are errors.
- A URI is a path or a `file:` URL with one `*` at most, and a URL of another scheme is an error. A path separates its names with `/` or `\`, and `?` and `#` are characters of its names, while a `file:` URL follows the syntax of URLs, so a `?` or a `#` in it starts its query or its fragment. The loader reads a relative path from the project root, and decides whether it reads an absolute path, a path above the root, or a `file:` URL. The `*` matches any text in the name of a file in the directory before it, so no `/` or `\` follows it, and `[` and `?` are characters of the name. A URI that ends with `/` or `\` reads the files of its directory with the format's extension, as `.csv` or `.csv.gz`. Neither reads the files of a subdirectory. A URI that names a directory without a `/` or `\` at its end is an error, as is a URI that matches no file. A table without a column list takes the schema of its files: the first rows of the first CSV or JSON file that is not empty name and type its columns, the first row of a CSV file names them, and a JSON file whose first character is `[` is an array of objects. `null_marker` other than `''` is an error, since DataFusion's CSV reader reads only an empty field as null.
- `TEMP` keeps a table in its script, as a step that the scripts don't see.
- `CREATE SCHEMA s` creates a schema, a dataset in BigQuery, and `s.t` names a table in it. A statement reads only the tables and schemas of the statements before it, of other data scripts, and of the host.

A chart script holds its own tables of files: `CREATE TEMP EXTERNAL TABLE` and `LOAD DATA {OVERWRITE | INTO} TEMP TABLE`, with the options and the URIs of a data script's. `TEMP` in a chart means what it means in a data script: only the chart's later statements read the table, and no other script sees it. A chart's `CREATE TEMP TABLE ... AS` is a view, whose query runs where a statement reads it, so a chart loads rows once with `LOAD DATA`. These statements without `TEMP`, `CREATE VIEW`, and `CREATE SCHEMA` are errors in a chart, whose help names a data script. Its URIs are relative to the root of its project, so a project runs such a chart.

```sql
CREATE TEMP EXTERNAL TABLE orders
OPTIONS (format = 'CSV', uris = ['data/files/orders/*.csv']);

FROM orders
|> AGGREGATE mark_bar(x => region, y => SUM(amount)) AS chart
```

## Queries

A query starts with `FROM` and continues with pipe operators. `SELECT ... FROM` and a standard `WITH` are errors that suggest the pipe form.

```sql
FROM orders AS o JOIN customers AS c USING (customer_id)
|> WHERE o.total > 0
|> EXTEND o.total * 0.2 AS tax
|> AGGREGATE sum(total) AS revenue, count(*) AS n GROUP BY region
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
|> WHERE species IN (
    'Adelie',
    'Gentoo',
   )
|> AGGREGATE
    count(*) AS n,
    avg(body_mass_g) AS mass,
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
|> AGGREGATE count(*) AS n GROUP BY species
|> AGGREGATE mark_bar(x => species, y => n) AS chart
```

- A mark's name is `mark_` and its kind, as Altair's methods are: `mark_bar`, `mark_line`, `mark_area`, `mark_symbol`, `mark_rule`, `mark_rect`, `mark_text`, `mark_trail`, `mark_path`, and `mark_image`, their `mark_polar_` forms, such as `mark_polar_bar`, and the `mark_geo_` marks of maps, such as `mark_geo_shape`. So `mark_` completes every mark, and `mark_polar_` the marks of polar plots.
- A bare value of the rows goes through the channel's scale, and a bare constant goes through the scale on a position, as `y => 0` does, and is drawn as it is on another channel, as `fill => 'red'` is. `datum(...)` makes a constant go through the scale, as in `color => datum('Adelie')`, and `value(...)` draws a constant or a column as it is, as in `fill => value(css_color)`.
- A channel with a scale takes its channel function, named after the channel, which keeps the kind of its value and takes the options of the channel's encoding by name: `scale`, `axis`, `title`, `band_position`, `band_level`, and `selection_field` on `x`, `y`, `theta`, and `radius`, none on `x2`, `y2`, `theta2`, and `radius2`, which follow the first position, and `scale`, `legend`, `title`, and `selection_field` on the other channels, such as `color` and `size`. So `fill => fill('red')` draws red, as `fill => 'red'` does. The value of a channel function can be `datum(...)` or `value(...)`:

```sql
FROM penguins
|> AGGREGATE mark_symbol(
    x => x(body_mass_g, axis => axis(grid => FALSE)),
    y => y(avg(flipper_length_mm), title => 'Flipper length (mm)'),
    color => color(datum('Adelie'), legend => FALSE),
   ) AS chart
   GROUP BY species
```

- The `format` patterns of a chart, on labels, a text mark, or a tooltip field, are GoogleSQL's: a number pattern is a `FORMAT` specifier, such as `%'.2f`, and a date pattern holds the format elements of `FORMAT_DATE`, such as `%Y-%m`. A pattern with a `'` takes a double-quoted string:

```sql
FROM penguins
|> AGGREGATE mark_symbol(
    x => x(body_mass_g, axis => axis(labels => "%'.0f g")),
    y => y(bill_length_mm, axis => labels(format => '%.1f')),
   ) AS chart
```

- A mark call takes the options of its plot and its layer as named constants after its channels: `title`, `attribution`, `tools`, `width`, `height`, `id`, and `classes`, as in `mark_symbol(x => a, y => b, title => 'Points', tools => [navigate(), tooltip()])`. A navigation goes on the plot, and a tooltip or a selection tool on the mark's layer. A selection tool, `select(...)`, writes a selection that the chart declares, which avenger-sql can't declare yet, so a chart with one is an error.
- The `ORDER BY` of a mark call is the draw order of its rows, and a `WHERE` before the `AGGREGATE` selects them. The `order` channel of a line or an area orders the rows of its path, as in `mark_line(x => x, y => y, order => step)`.
- A channel takes aggregates, as in `mark_bar(x => species, y => COUNT(*))`: each channel without an aggregate is a key, and the call aggregates once for each group and key. A channel's column takes the text of its value, such as `COUNT(*)`, as its title.
- A channel takes windows on its rows too, as in `mark_bar(x => bin(body_mass_g), y => COUNT(*))`. There `bin`, `timeunit`, and `lump` take no `OVER`, and compute over the whole input, or over each group with `OVER panel`.
- A window that reads an aggregate runs on the aggregated rows, as `stack(COUNT(*))` does. `stack` takes no `OVER` there and stacks within `position`, the groups and the position keys. The windows `panel`, `position`, and `series` partition by the groups, and by the position or the series keys, and give an order where GoogleSQL leaves one unspecified.

A composition aggregate composes the charts of the rows of a group, one chart for each row:

```sql
FROM penguins
|> AGGREGATE mark_symbol(x => bill_length_mm, y => bill_depth_mm) AS chart GROUP BY island
|> AGGREGATE facet_column(chart, island) AS chart
```

- `facet_row(chart, key)`, `facet_column(chart, key)`, and `facet_wrap(chart, key)` lay out the charts by the values of a key, and label them. A `facet_row` of `facet_column` charts is a grid.
- `layer_agg(chart ORDER BY k)` overlays the plots of the charts, and `hconcat_agg` and `vconcat_agg` place them in a row or a column, in the order of the `ORDER BY`. Without one, their order is unspecified.
- The options of a composition are named constants, as in `facet_wrap(chart, species, max_columns => 2)`. `resolve => resolve(y => 'independent')` gives each child its own scale of `y`, `parts => parts(legend => 'keep')` keeps a legend beside each plot, and `tracks => [200, 400]` sets the length of each child's track. An array that mixes lengths and shares writes a constructor for each track, as in `tracks => [track(200), track(share => 2, max => 600)]`.
- The scalars `layer(a, b, ...)`, `hconcat(...)`, and `vconcat(...)` compose the charts of one row, in the order of their arguments, such as the marks of one `AGGREGATE` item: `layer(mark_bar(...), mark_symbol(...))`. Their options follow the charts by name.

## Options of charts

A constructor builds an option of a chart, such as `scale(zero => FALSE)` for a scale or `polar(inner_radius => 0.5)` for a mark's `coord`. Its arguments are constants. Only an argument of a chart function takes a constructor: a mark, a composition, a channel function, or a constructor in them. Elsewhere its name is an error that names where it belongs. The name resolves by the option that the argument expects, so one name builds different options in different places: `type => log(base => 2)` is a log scale in `scale(...)`, where `LOG(x)` elsewhere is GoogleSQL's, and `type => point(...)` is a point scale in `scale(...)` and a point selection in `select(...)`.

A named argument takes one of three forms:

1. An ordinary value: a literal, an array, a struct, a script variable, or a string that names a member of a choice, as `type => 'log'` and `range => 'tableau10'` do.
2. The constructor named after the argument, as `scale => scale(zero => FALSE)`.
3. One level down, a constructor named after an argument of that constructor: `scale => pin(1000, position => 'low')` is short for `scale => scale(pin => pin(1000, position => 'low'))`. No form skips more than one level.

An element of an array takes the constructor that its argument names, as `levels => [level(...)]` and `fields => [field(...)]` do, or a member of a choice, as `tools => [navigate(), tooltip()]` does. `NULL` leaves an argument unset.

A shorthand is an ordinary value that fills a plain field of the argument's own option: `title => 'Mass'` fills the text of a title, `axis => FALSE` hides an axis, `scale => 'log'` picks the type of a scale, and `tracks => [200, 400]` fills the length of each track. A shorthand never fills a field of a child, so `axis => 'Bill length'` is an error, and `axis => title('Bill length')` sets the axis title by form 3.

The options of a chart follow the parts of its stylesheets. A part with text of its own nests: `axis(labels => labels(angle => -45))` sets what `axis > label { angle: -45 }` sets. A part that only draws is a prefix: `axis(tick_stroke => 'gray', grid_stroke_width => 0.5)`. A style option takes the name of its stylesheet property with underscores, and its keywords, as `text_align => 'end'` and `white_space => 'nowrap'`. `value_placement` places the values that a guide marks, its ticks, grid lines, labels, and swatches, as `axis(value_placement => 5)`, and `tick` names only the tick marks.

A value always wraps in its constructor: where `foo => v` is valid, `foo => foo(v)` is valid too and means the same, so named arguments can follow the value, as in `title => title('Mass', font_size => 14)`. A constructor takes its shorthand at position 0, and every other argument by name. A shorthand can be valid at position 0 when it is not valid alone: `pin(1000, position => 'low')` is a pin, and `pin => 1000` is an error, since a pin needs its position.

```sql
FROM penguins
|> AGGREGATE mark_symbol(
    x => x(bill_length_mm, axis => title('Bill length (mm)')),
    y => y(body_mass_g, scale => 'log', axis => axis(grid => FALSE)),
    title => title('Penguins', subtitle => 'Palmer Archipelago'),
   ) AS chart
```

## Layout

Every example follows the style of [STYLE.md](STYLE.md), which avenger-sql-syntax's formatter writes. A call that fits in 100 columns stays on one line. A call that doesn't fit breaks after its `(`, with one argument on each line, 4 spaces deeper than the line on which the call starts, followed by a comma, and its `)` on its own line at that line's depth. A pipe operator's line has the depth of its keyword, column 3 after `|> `, and the arguments of a call that it opens are 4 spaces deeper than its `|>`, so the arguments of a mark in `|> AGGREGATE` start at column 4, and its `)` lines up with `AGGREGATE`. A comma after the last argument keeps a call broken.

```sql
FROM penguins
|> AGGREGATE mark_symbol(
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
    MEASURES first(day) AS start_day, last(up.price) AS high
    PATTERN (dn+ up+)
    DEFINE dn AS price < prev(price), up AS price > prev(price)
   )
```

## Where to look

- `avenger-sql/README.md`: the language, marks, transforms, `MATCH_RECOGNIZE`'s choices, and conformance with GoogleSQL.
- `avenger-sql-syntax/sql.ungram`: the grammar of the subset.
- `library::reference`: every function, its forms, and its parameters.
- `avenger-chart/VEGA-LITE.md`: the names of Vega-Lite and Altair in the format and in avenger-sql.
