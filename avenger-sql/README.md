# avenger-sql

GoogleSQL pipe queries that build avenger charts, planned onto DataFusion. The mark functions of avenger-chart are aggregate functions, so a query builds a chart of its rows, or one chart for each group with `GROUP BY`, in a `BinaryView` column whose field has the extension name `avenger.chart`:

```sql
FROM penguins
|> AGGREGATE count(*) AS n GROUP BY species
|> AGGREGATE mark_bar(x => species, y => n) AS chart
```

`sql(ctx, text)` plans a script over the tables of a DataFusion session and returns the rows of its last query as a DataFrame. avenger-sql-syntax parses the script, and avenger-sql resolves its names and types by GoogleSQL's rules and plans each statement as a DataFusion logical plan. DataFusion's SQL planner takes no part, and the build of avenger-sql holds no sqlparser. `register(ctx)` adds what the plans of the transforms need to run: avenger-sql's query planner, which plans `impute` and the inputs that a plan reads more than once and runs once, and the optimizer rules that compute cheap shared inputs again and group by the fields of a struct, such as a bin.

## The language

[SPEC.md](SPEC.md) is a short guide to the language. [STYLE.md](STYLE.md) is the layout of scripts, which avenger-sql-syntax's formatter writes. A query is GoogleSQL's pipe syntax, the subset that avenger-sql-syntax parses, with a trailing comma allowed in every comma-separated list. It starts with `FROM`, and its pipe operators are `WHERE`, `SELECT`, `EXTEND`, `SET`, `DROP`, `RENAME`, `AS`, `AGGREGATE` with `GROUP BY`, `GROUPING SETS`, `ROLLUP` and `CUBE`, `ORDER BY`, `LIMIT`, `DISTINCT`, the joins, the set operations, `PIVOT`, `UNPIVOT`, `TABLESAMPLE`, `MATCH_RECOGNIZE`, `WITH`, and `CALL`. Expressions include subqueries, `UNNEST`, named windows, and the `WINDOW` clause. A standard `SELECT ... FROM` query isn't in the subset, and its error offers a rewrite to pipe syntax for a simple query. GoogleSQL's other syntax outside the subset, such as DDL and DML statements, gets an error that names it as unsupported.

A script is statements separated by `;`. `DECLARE` and `SET` give script variables, `CREATE TEMP TABLE` and `CREATE TEMP FUNCTION` define names for the statements after them, and `IMPORT MODULE` imports the public functions and constants of a module that the catalog holds. `sql` returns the rows of the last query, after the variables before it.

The functions are those of the `library`: GoogleSQL's functions, avenger-chart's marks and compositions, and the transforms of avenger-datafusion-transforms, with avenger-chart's constructors, which only the arguments of a chart function call. Each function declares its forms, with GoogleSQL's types and coercion, and documents itself and its parameters. `library::reference` renders the reference of every function, which notes where a function differs from GoogleSQL. A parameter keeps its name when it is a reserved word of GoogleSQL, as `range` of `scale` does, because the parser reads a word that `=>` follows as an argument's name.

## Data scripts

A project's data scripts register its tables with GoogleSQL's `CREATE EXTERNAL TABLE`, `LOAD DATA`, `CREATE VIEW`, `CREATE TABLE ... AS`, and `CREATE SCHEMA`, as [SPEC.md](SPEC.md) describes. avenger-sql-syntax's `parse_data` reads a data script, and `analyze_data` analyzes it into a `data::DataAnalysis`: each definition, with its name, its kind, whether it is `TEMP`, its ranges, its description, its schema, and the plan of the query of a view or a table, the reads of tables that leave the script, and the errors. A read of a table that an earlier statement of the script defines plans a scan of a `data::ScriptTable`, a placeholder of its definition and its schema, which a loader replaces with the table that it registers. `analyze` analyzes a chart script's own tables of files, `CREATE TEMP EXTERNAL TABLE` and `LOAD DATA ... TEMP TABLE`, with the same code, into `Analysis::definitions`, and a read of one plans a scan of its placeholder too. `catalog::references` leaves out the reads of a table that an earlier statement of the script defines, so a catalog loads only the tables that the script reads from outside it.

`data::FILE_OPTIONS` is the table of the options of files, with each option's type, formats, meaning, and values, and `data::UNSUPPORTED_OPTIONS` the options of BigQuery that a data script rejects. The options of a statement give its `data::Files`: the format, the URIs, and how the files read. `data::uri_kind` tells what a URI names from its text alone: a path relative to the project's root, one that goes above the root, an absolute path of Unix or Windows, a `file:` URL, or a URL of another scheme. Reading the schema of files is I/O, so a loader collects the files of a script with `data::file_reads` before the analysis, and the catalog gives the table of each with `Catalog::files`. `Catalog::has_schema` tells whether the catalog holds a schema, and `Catalog::definition` gives the place that defines a table and its description. `MemoryCatalog` holds tables of files, schemas, and definitions for tests.

The `project` module holds a project's data scripts together, without I/O. `project::Graph` reads, from the parses of the scripts, the names that each defines without `TEMP` and the names that it reads from the others, including the schema of each table in a schema that it defines, and gives the order that they run in: each after the scripts that it reads, and among those that are ready at once, by path. A path of a table of one name is in the default schema that the host gives, so `orders` and `public.orders` are one name when it is `public`. A name that two scripts define, and a cycle, which blocks its scripts and those downstream of it, are its errors, at the definitions and at the reads. `Graph::downstream` gives the scripts that a change of a script reloads, and `Graph::importers` the scripts that import a module, through their `IMPORT MODULE`s and the imports of those modules in turn, which a change of the module reloads. `project::ProjectCatalog` is the catalog of a project: over a base catalog of the host's tables, it holds the tables, views, and schemas that the scripts' analyses export, each with the place that defines it, the tables of the scripts' files, and the modules, and a lookup finds a script's name before the base's, even when the name has no table after an error.

`data::file_table` builds the table of a `Files` over the object stores of a DataFusion session: a `ListingTable` of the format and options of the files, whose URIs are relative to a base URL that the caller gives, the URL of the project's root at a store that the caller registers, such as `memory://project/`. A URI with a `*` lists the directory before it, up to its last `/` or `\`, with a glob pattern of the names of its files in which `[` and `?` are literal, and a URI that ends with `/` or `\` lists its directory with the format's extension, and neither reads the files of a subdirectory. It reads the first character of the first JSON file that is not empty to tell an array from newline-delimited JSON, and the keys of a Hive layout from the paths under `hive_partition_uri_prefix`. When `WITH PARTITION COLUMNS` has no list, each key takes the type that all its values in the paths have, as BigQuery detects the types of keys: `INT64`, else `DATE`, else `TIMESTAMP`, else `STRING`. The scan of a Hive layout writes an equality of a key and a value as `IN` of the value, since DataFusion spells a prefix of the listing from such equalities, which misses a folder spelled otherwise, such as `month=01` for `month = 1`. It parses no file paths on wasm, where an absolute path is an error. A table without partition columns reads the files that its URIs matched when it was built, each by its URL. The scans of a Hive layout list the files under its prefix, and its listings replace those of the session's cache of listings, which DataFusion keeps by default, so a table built again after its files change lists them as they are. `data::lower` lowers an analyzed data script into a `SessionContext`, in the order of the text: the table of the files of an external table, a `MemTable` of the rows of a `LOAD DATA`, with the rows of an append of `LOAD DATA INTO` added in a new `MemTable`, a `ViewTable` of a view, a `MemTable` of the rows of a `CREATE TABLE ... AS`, and a `MemorySchemaProvider` of a schema. Each scan of a `data::ScriptTable` reads the table of the definition whose rows it sees, so a view over a `LOAD DATA` reads the rows in memory. It registers the definitions that the script exports, each table in place of the tables of the same name without regard to case, and returns them, each with the tables that it replaced, which a loader registers again when it removes the table, or the error of each definition that fails, and then registers nothing. A registration that fails leaves the session as it was. `data::ScriptTables` builds the tables of a chart script's own tables of files in the same way and registers none, and `Analysis::with_tables` gives the chart's plans that read them in place of their placeholders. `sql` and `sql_with_modules` give an error at each of a chart's statements of files, since their URIs are relative to the root of a project, which runs such a chart.

The cargo features `parquet`, `avro`, and `compression`, which none of the defaults turns on, turn on DataFusion's features of the same names. A data script that reads a format or a compression whose feature a build lacks has an error that names the feature. `parquet` and `compression` build for wasm, and `avro` does not, since DataFusion's Avro reader reads local files through `object_store`'s `GetResultPayload::File`, which `object_store` has only outside wasm.

## Row pattern recognition

`MATCH_RECOGNIZE` finds the runs of rows that match a pattern, as a pipe operator or after a source in `FROM`, with the syntax of [GoogleSQL's `MATCH_RECOGNIZE`](https://github.com/google/googlesql/blob/2026.11.1/docs/pipe-syntax.md#match_recognize_pipe_operator). A pattern variable's condition reads the row, and `PREV` and `NEXT` read its neighbors in the partition. A measure reads aggregates of the match's rows, such as `COUNT(*)`, or of one variable's rows, such as `MIN(dn.price)`:

```sql
FROM prices
|> MATCH_RECOGNIZE(
     PARTITION BY symbol
     ORDER BY day
     MEASURES first(day) AS start_day, min(dn.price) AS low, last(up.price) AS high
     PATTERN (st dn+ up+)
     DEFINE st AS TRUE, dn AS price < prev(price), up AS price > prev(price)
   )
```

The output has one row for each match: the partition columns, then the measures. The clause supports GoogleSQL's subset of SQL:2016. `ALL ROWS PER MATCH`, `ONE ROW PER MATCH`, `RUNNING` and `FINAL`, `SUBSET`, `PERMUTE`, exclusion, and `AFTER MATCH SKIP TO` a variable are errors. avenger-sql behaves as follows where the syntax leaves a choice:

- `MATCH_ROW_NUMBER()` is the row's position in its partition, from 1, so the rows of a second match can be 4, 5 and 6.
- `FIRST` and `LAST` leave out `NULL` values: `FIRST(v)` is the first value of the match's rows that is not `NULL`.
- Rows with equal `ORDER BY` keys have no order of their own. avenger-sql orders them once, and `PREV`, `NEXT` and the matcher read that one order, so a query gives a consistent result. Another engine can order the ties another way.
- A quantifier bound and the option `use_longest_match` are literals, not query parameters, because avenger-sql compiles the pattern when it plans the query.
- A pattern compiles to at most 100,000 instructions, which bounded quantifiers such as `a{50000}` reach. The matcher's cost for each row is at most the size of the pattern's program, so no pattern takes exponential time.
- The clause holds the input's rows in memory, in one task for each group of partitions.
- A clause in a subquery doesn't read the columns of the query around it: a condition or a measure that does is an error, since DataFusion doesn't plan the correlation through the clause.
- Two aggregates that take `ORDER BY` in other orders are an error of DataFusion, and a measure without `ORDER BY`, such as `STRING_AGG(x)`, reads the rows in their order, so `STRING_AGG(x)` beside `STRING_AGG(y ORDER BY y)` is an error. `ARRAY_AGG` takes its own order beside the order of another aggregate.

## Errors

A script with errors returns `ScriptError` as `DataFusionError::External`: the syntax errors of the parse, or else every error of the analysis, each a `Diagnostic` with the range of its text. Its message gives each error with its line and column. `analysis::analyze` returns the same diagnostics with the semantic model of a script, for an editor: the scope of each operator and of each temporary function's body, what each name refers to, the forms of its calls, the type before each `.` of a field access, and its places. A place is a clause or an argument list where the pass plans expressions, such as the condition of a `WHERE` or the arguments of an aggregate, with what it takes: whether it reads columns, aggregates, and window functions, the clause of a `MATCH_RECOGNIZE` that holds it, and the type of its value where the analysis knows it. `Place::takes` tells whether a function can stand in a place, by the rules that the pass reports as errors. `Model::refine_highlights` refines the ranges of avenger-sql-syntax's `highlight` with that model: a name takes the role of its definition, so a script variable or a range variable is no longer a column, a function is a scalar, an aggregate, a window, or a table function, a mark, a constructor, or a transform, and a string that a parameter takes as one value of a set, as `offset => 'normalize'` of `stack`, is an enum value. An editor can show the syntax's classes at once, and the refined ones when the analysis of the same text finishes. [avenger-sql-language](../avenger-sql-language) gives an editor these classes and the rest of its features of a script from one analysis: diagnostics, completion, hover, go to definition, references, rename, signature help, and the outline.

## Mark functions

The library binds the functions of avenger-chart's `functions` feature: the README of avenger-chart describes their names, their parameters, and the kinds of their arguments. In a query:

- A channel argument is an expression of each row, such as `body_mass * 2`. Its column is named after the expression, without the qualifiers of its columns, and keeps the type and the field metadata of the expression. Channels with equal expressions share one column.
- `NULL` leaves the channel unset, so an omitted parameter and `NULL` mean the same thing. A `NULL` in a channel function leaves the channel unset too. In `datum` or `value`, a `NULL` is a value of GoogleSQL's type of `NULL`, `INT64`, or of a cast, such as `value(CAST(NULL AS STRING))`.
- The `ORDER BY` of a call is the draw order of its rows, a `WHERE` before the `AGGREGATE` selects them, and `GROUP BY` gives one chart for each group.
- A channel with a scale takes its channel function, as `x => x(body_mass_g, axis => axis(grid => FALSE))` and `color => color(datum('Adelie'), legend => FALSE)` do. A channel function keeps the kind of its value and takes only the options of its channel's encoding, so `x2 => x2(a, axis => FALSE)` is an error at `axis`. A channel function is valid only as the value of the channel of its name.
- A property, `coord`, and an option of a channel function are constants, such as the folded call of a constructor.

Each call binds at planning to an aggregate of its own, which takes only the columns of the call, so an invalid call, such as one without a required channel, fails at planning.

## Format patterns

Every chart of avenger-sql, a mark's or a composition's, names `googlesql` in `number_format_syntax` and `datetime_format_syntax`, with no option to change it. So a `format` pattern on labels, a text mark, or a tooltip field is GoogleSQL's: a `FORMAT` specifier for numbers, such as `%'.2f`, and the format elements of `FORMAT_DATE` for dates, such as `%Y-%m`. A host that draws the charts registers the two `googlesql` providers on its label engine, or evaluation fails with `MissingFormatter`, also for a chart without patterns:

```rust
use std::sync::Arc;

use avenger_format_datetime_googlesql::GoogleSqlDateTimeFormatProvider;
use avenger_format_number_googlesql::GoogleSqlNumberFormatProvider;

let engine = avenger_typst_label::bundled_label_engine()
    .with_number_formatting(Arc::new(GoogleSqlNumberFormatProvider::new()))
    .with_datetime_formatting(Arc::new(GoogleSqlDateTimeFormatProvider::new()));
```

The analysis checks a chart's literal pattern with these providers, by the type of the value that it formats where it knows it, so an invalid pattern is an error at its argument rather than when the chart renders. The formats of GoogleSQL's functions, such as `FORMAT_DATE`, take no check, since they print an element that GoogleSQL doesn't document as written.

## Strings in languages of their own

Some strings of a chart function are in a language of their own: Typst markup, such as a title, and format patterns. `string-arguments.json` lists these arguments in the terms of a call's syntax, for `tree-sitter-avenger-sql`, whose sync script generates its injection patterns from it: each call by the name that a script writes, a function's or a constructor's, each argument by its name or as the first by position, its language, `avenger-typst` or `avenger-format`, whether it holds an array of strings, `within`, the enclosing calls and arguments of a name whose constructors disagree on their arguments, such as the plot's `title` and the tooltip's, and `unless`, the argument whose values make markup plain, every value of a title's `syntax` but `'typst'`, such as `'plain'` and `'plain_lines'`, by the rule of `TEXT_SYNTAX`, which the analysis and the file share. The values are listed, since the predicates of Tree-sitter's queries have no lookahead to test for a value that isn't `'typst'`. The test `tests/string_arguments.rs` builds the file from the library and compares it with the committed one, which a change accepts by copying `tests/output/string-arguments.json`.

## Statistics in channels

A mark call in an item of a pipe `AGGREGATE` takes aggregates in its channels, and the analysis plans it as the staged query that a user could type for it:

```sql
FROM penguins
|> AGGREGATE mark_bar(x => species, y => avg(body_mass_g), color => sex) AS chart GROUP BY island
```

plans as

```sql
FROM penguins
|> AGGREGATE avg(body_mass_g) AS `avg(body_mass_g)` GROUP BY island, species, sex
|> AGGREGATE mark_bar(x => species, y => `avg(body_mass_g)`, color => sex) AS chart GROUP BY island
```

- In a call that holds an aggregate, each channel that holds none is a key, and the call aggregates once for each combination of the operator's groups and its keys. Here the keys are `species` and `sex`.
- A channel that holds an aggregate is a measure. Outside its aggregates, a measure reads only keys, groups, and constants, so `y => avg(body_mass_g) / 1000` is a measure, and `y => body_mass_g + count(*)` is an error, since `body_mass_g` is neither a key nor aggregated. A key of the call's `ORDER BY` is a channel, a group, or an aggregate.
- Each channel's column takes the text of its value as written, with its spaces collapsed, such as `avg(body_mass_g)`, which titles its axis or its legend. A column keeps its name.
- Each such call is a branch of the operator over its input, aggregated to one row for each group, and the branches and the other items join on the groups, with `NULL` equal to `NULL`. The input runs once for all of them.
- An aggregate of an aggregate in one call, such as `SUM(COUNT(*))` without `OVER`, is an error, and so is such a call in an `AGGREGATE` grouped by `GROUPING SETS`, `ROLLUP`, or `CUBE`.
- A channel also takes windows. A window that reads no aggregate runs on the rows, before the aggregate, so its value is a key: `x => bin(body_mass_g)` bins the penguins, `x => NTILE(4) OVER (ORDER BY body_mass_g)` puts each one in a quartile, and `AVG(m - AVG(m) OVER ())` averages the deviations from the mean. A window may read another, as `lump(timeunit(t, unit => 'month'), n => 3)` does. A call without an aggregate that holds a window draws one row for each row of its input.
- In a channel, `bin`, `timeunit`, and `lump` take no `OVER`, and then compute over the whole input of the `AGGREGATE`, so the charts of its groups share their bins, units, and kept categories. `OVER panel` partitions by the operator's groups, so `bin(body_mass_g) OVER panel` gives each chart its own bins. A window that names `panel` adds its own `ORDER BY` or frame, by GoogleSQL's rules of a named window, and the name stands only in a mark's channels.
- The column of `bin`, `timeunit`, or `lump` takes the text of its first argument, so `bin(body_mass_g)` is titled `body_mass_g`. When a column of the call takes the same name, as `color => body_mass_g` would, the lifted value takes its complete text, `bin(body_mass_g)`.
- A window that reads an aggregate, or names `position` or `series`, runs on the aggregated rows, and its channel is a measure: `y => stack(COUNT(*))`, `y => COUNT(*) / SUM(COUNT(*)) OVER panel`, and `color => bin(COUNT(*)).start`. An explicit `OVER` keeps GoogleSQL's meaning, so `SUM(COUNT(*)) OVER ()` reads the aggregated rows of every group.
- A mark call has three windows: `panel` partitions by the operator's groups, `position` by the groups and the keys of the position channels (`x`, `y`, `theta`, and `radius`), and `series` by the groups and the keys of the series channels (`color`, `fill`, `stroke`, `stroke_dash`, `pattern`, and `detail`). Their keys leave out each channel that names one of them.
- A window over one of them that reads the order of its rows, as `stack`, `ROW_NUMBER`, `LAG`, and a frame of `ROWS` do, takes its own `ORDER BY` and then the window's default order, which breaks its ties and orders a window without an `ORDER BY`. `panel` orders by the position keys, then the series, `position` by the series, and `series` by the position keys, each then by the other keys and the measures. `position` orders the series as a stack shows them: when the call's measure is in `y`, the series descend, so that the first entry of the legend is on top. A series ascends by its values, with `NULL` last, as a chart orders its categories by default. The order that a scale's own category fit gives is not read yet. A window that reads no order, or only the peers of its order, such as `SUM(COUNT(*)) OVER series` and `RANK`, keeps its order as written.
- `stack` without `OVER` takes `OVER position`, so `mark_bar(x => island, y => stack(COUNT(*)), color => species)` stacks the species of each island. `stack(COUNT(*)) OVER (position ORDER BY COUNT(*) DESC)` puts the largest count at the bottom, and `offset` applies as for `stack` anywhere.
- A stacked `mark_line`, `mark_area`, `mark_trail`, `mark_polar_line`, or `mark_polar_area`, whose measure is a span or a field of one built from windows over `position`, fills each key that a series of a group lacks with zero before the windows, so the series do not cross.

`avenger_sql::lift(ctx, text)`, or `Analysis::lift` of an analysis, writes a script as the script that its lifted calls plan, as pipe SQL that a user could type: each `AGGREGATE` with a lifted call becomes its stages, one operator to a line, and the rest of the text stays as written, with its layout and comments. It returns the text and the ranges that it wrote, and the printout has nothing more to lift. A call beside other items prints as a `LEFT JOIN` on the groups whose query repeats the operator's input, so the printout computes the input once for each branch, where the plan reads one run of it.

A constructor builds an option of a chart, and only an argument of a chart function takes it: a mark, a composition, a channel function, or another constructor in them. A constructor is named after the place that it fills, which the argument's parameter gives, so one name builds different messages in different places, as `type => point(...)` is a point scale in `scale(...)` and a point selection in `select(...)`, and `type => log(base => 2)` is a log scale where `LOG(x)` elsewhere is GoogleSQL's logarithm. An argument takes an ordinary value, the constructor named after the argument, as `scale => scale(zero => FALSE)`, or one level down, the constructor of one of that constructor's arguments, as `scale => pin(1000, position => 'low')` is short for `scale => scale(pin => pin(1000, position => 'low'))`. A constructor takes its shorthand by position, as `title('Mass', font_size => 14)` takes the text of `title => 'Mass'`, and every other argument by name. The function of a constructor returns a fragment of its message in a `BinaryView` value whose field has the extension name `avenger.chart.fragment` and the full name of the message as its extension metadata. A call of constants folds to a literal, and its errors, such as an unknown enum name, are errors at its argument. A map takes its projection by name, as `coord => geo(projection => 'mercator')` does, or with its parameters, as `coord => geo(projection => projection('conic_conformal', parallels => [33, 45]), graticule => TRUE)` does, where an array of numbers, such as `rotate => [96, 0]`, is an ordinary SQL array, and `TRUE` draws a graticule or a sphere with its defaults. The named options of a custom projection or a custom scale are a struct, as in `projection(type => custom('healpix', options => STRUCT(4 AS lobes)))`.

## Compositions

The composition aggregates of avenger-chart's `functions` feature compose the charts of the rows of a group, so the data gives the structure, one chart for each row:

```sql
FROM penguins
|> AGGREGATE mark_symbol(x => bill_length_mm, y => bill_depth_mm, color => species) AS chart
   GROUP BY island
|> AGGREGATE facet_column(chart, island) AS chart
```

- `layer_agg(chart)` overlays the plots of the charts of a group, `hconcat_agg(chart)` places them in a row, and `vconcat_agg(chart)` in a column, in the order of the `ORDER BY` of the call, such as `layer_agg(chart ORDER BY k)`. Without an `ORDER BY`, their order is unspecified, as for `ARRAY_AGG`.
- `facet_row(chart, key)`, `facet_column(chart, key)`, and `facet_wrap(chart, key)` lay out the charts by the values of the key, ascending with `NULL` last, or in the order of their first rows in the `ORDER BY` of the call. A facet labels each chart with its value, and its column is the name of the key's column. Two rows of a group with one key value are an error, and a `NULL` chart is an empty cell. A `facet_row` of `facet_column` charts is a grid, whose rows each get a cell for every column.
- The options of a composition are those of its node, by name, such as `max_columns => 2` of a `facet_wrap`, and they are constants, as the properties of a mark are. A `NULL` chart is no child of a layer or a concat, and a composition without children is the empty chart.
- `resolve => resolve(y => 'independent')` gives each child of a concat or a wrap its own scale of `y`, and `resolve(projection => 'shared')` gives its maps one projection. `parts => parts(legend => 'keep')` draws a legend beside each plot in place of one for the concat. `tracks => [200, 400]` sets the length of each child's track along a concat, and a track takes a share of the free length in place of a length, as in `tracks => [track(200), track(share => 2, max => 600)]`, where an array that mixes the two writes `track(...)` for each element.

The scalar functions `layer`, `hconcat`, and `vconcat` compose the charts of a row, the charts that a query writes as columns, so the query authors the structure:

```sql
FROM penguins
|> AGGREGATE layer(
     mark_symbol(x => bill_length_mm, y => bill_depth_mm),
     mark_symbol(x => flipper_length_mm, y => body_mass_g, size => 20),
   ) AS chart
   GROUP BY island
|> AGGREGATE facet_column(chart, island) AS chart
```

- `layer(chart, ...)` overlays the plots of its charts, `hconcat(chart, ...)` places them in a row, and `vconcat(chart, ...)` in a column, in the order of the arguments, so the first layer draws first and the first child of a concat is the leftmost or the topmost. A call takes one chart or more, and calls nest, as in `vconcat(top, hconcat(main, side))`.
- The options of a scalar are those of its aggregate, by name, after the charts, such as `hconcat(a, b, gap => 24)`. A `NULL` chart is no child, and a scalar over mark aggregates, such as the `layer` above, composes the marks of each group.

The aggregates of a layer and the concats take the `_agg` names, as GoogleSQL names `STRING_AGG` and `ARRAY_CONCAT_AGG`, because an aggregate and a scalar function cannot share a name. The facets keep their names, since they compose only rows.

## Transforms

`bin`, `stack`, and `timeunit` are window functions, whose options are named arguments, as `bin(body_mass_g, maxbins => 30) OVER ()`. An option is a literal or a parameter, such as a script variable or a query parameter, which stays a placeholder of the plan until it is bound, as `sql` binds the variables, so a plan of a document compiles once and binds each run. The analysis checks a literal at its argument, and the transform checks the value of a parameter when the query runs. The `method`, `flag`, and `partition_by` of `impute` decide the schema of its result, so they take literals.

`lump` keeps the heaviest categories of each partition of its `OVER` clause, or the categories that a condition keeps, and lumps the rest into an Other group:

```sql
FROM catches
|> EXTEND lump(port, weight => sum(tons), keep => row_number() OVER ranked <= 3) OVER (
     PARTITION BY region
   ) AS l
```

Its arguments read three scopes. The category reads each row. `weight`, `COUNT(*)` by default, is an aggregate of the rows of each category. `keep` is evaluated once for each category: like a `HAVING` clause it compares aggregates of the category's rows, and like `QUALIFY` it can use windows over the categories of the partition, which take no `PARTITION BY`. The named window `ranked` orders the categories by weight, the heaviest first. `n => 3` keeps the first 3 categories of `ranked`, in place of `keep`. `other` labels the Other group, and `null_category` keeps the `NULL` category as its own, lumps it into the Other group, or drops its rows from every lump. The result is `STRUCT<value, other, weight, category_count>`.

`impute` is a table function that adds the rows that each group lacks. A pipe `CALL` imputes the rows before it, and `FROM` the rows of a table or a query:

```sql
FROM sales
|> AGGREGATE sum(units) AS units GROUP BY month, region
|> CALL impute(month, [region], units, value => 0, flag => 'imputed');

FROM impute(TABLE monthly, month, [region], units, method => 'avg', frame => [-1, 1])
```

Its arguments name the columns of its input: the key, an array of the group columns, and the column to impute, then named arguments such as `fill_gaps` and `partition_by`. The reference of the library documents each argument of each transform.

The example `sql-chart` shows the chart of a query in a chart app, and `chart-lump` the lumped genres of movies.

## Streaming queries over events

`streaming::split` splits the plan of a query over a table of events, such as the pointer events of a chart, for a streaming runtime: for each read of the events, the stateless stages over it and a front that reduces their rows, and the rest of the query over the fronts' outputs, which joins or unions the outputs of several fronts. The fronts are an `AGGREGATE`, grouped or ungrouped, a `DISTINCT`, an `ORDER BY ... LIMIT` whose last key is `seq`, and a `MATCH_RECOGNIZE` ordered by `seq`, whose split holds the plans of its conditions and its measures. A query of another shape, such as a join of the events themselves or a subquery of an expression that reads them, is a rejection that names the operator, and runs as a batch plan, as is a query that reads a function whose value changes on its own, such as `CURRENT_TIMESTAMP()`. avenger-streaming runs the split frame by frame, with the batch plan's result after every frame.

## Regular expressions

`REGEXP_CONTAINS`, `REGEXP_EXTRACT`, and `REGEXP_REPLACE` read their patterns as RE2 does, as GoogleSQL's do, and run them on the `regex` crate, whose patterns share RE2's model and most of its syntax. The Perl classes and the word boundaries are ASCII, as in RE2: `\d` is `[0-9]`, `\w` is `[0-9A-Za-z_]`, `\s` is `[\t\n\f\r ]`, without a vertical tab, and `\b` is a boundary between them. A replacement is RE2's rewrite string: `\1` to `\9` are the matches of the groups, `\0` the whole match, and `\\` a backslash, every other character, `$` included, is itself, and another escape, or a group that the pattern lacks, is an error. RE2's own syntax runs too: `\Q…\E` literal text, octal escapes such as `\101`, `\p{^Greek}`, a `{` that starts no repetition, and in a class, `[`, `&`, `-`, and `~` as characters rather than nested classes and set operations, as RE2 reads them. What RE2 rejects is an error with RE2's message: a backreference, an escape that RE2 lacks, a flag other than `i`, `m`, `s`, and `U`, a property name other than `Any`, a general category, or a script, by its exact name, a repetition right after another, a count over 1000, and nested counts whose product is over 1000. `\C`, any byte, is an error too, since a pattern over strings can't match it alone. A pattern from a column compiles once for each distinct pattern. `tests/re2.rs` compares the functions with RE2's own answers, which `tests/data/re2_oracle.py` records with RE2's Python binding: `uv run avenger-sql/tests/data/re2_oracle.py` rewrites `tests/data/re2.jsonl`.

## Conformance with GoogleSQL

Two kinds of tests compare avenger-sql with GoogleSQL's `execute_query` 2026.11.1. `tests/googlesql_queries.rs` runs pipe queries whose results `execute_query` recorded in `tests/data/googlesql_pipe_*.txt`, and runs in every test run. The ignored test `tests/conformance.rs` runs each query of `tests/conformance/queries.txt` through `sql` and through `execute_query --mode=execute` over the CSV tables of `tests/conformance`, which both read as `STRING` columns and `tables.sql` types, and compares the names of the columns and the rows, or the errors. A difference that `tests/conformance/known.txt` lists, such as a function that the library doesn't map yet, is explained, and any other fails the test. The report is in `tests/output/conformance.md`. The test needs the binary at `scratch/tools/googlesql/execute_query_macos`, and without it skips with a message:

```bash
CARGO_TARGET_DIR=target cargo test --release -p avenger-sql --test conformance -- --ignored
```
