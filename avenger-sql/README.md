# avenger-sql

DataFusion SQL that builds avenger charts. The mark functions of avenger-chart are aggregate functions in avenger-sql, so a query builds a chart of its rows, or one chart for each group with `GROUP BY`, in a `BinaryView` column whose field has the extension name `avenger.chart`:

```sql
FROM penguins
|> AGGREGATE count(*) AS n GROUP BY species
|> AGGREGATE bar(x => species, y => n) AS chart
```

`register` adds the mark functions and the constructors of avenger-chart and the transforms of avenger-datafusion-transforms, such as `bin`, `stack`, `timeunit`, `lump`, and `impute`, to a session, `sql` plans a query in a session, and `expand` returns the SQL that `sql` plans. A query is DataFusion SQL after two passes, which DataFusion 55.2 needs to plan these queries. The token pass rewrites the tokens of the text before parsing, and the syntax pass rewrites the parsed statement:

- A word that starts with `__avenger_` is an error at the word. The passes generate the names with this prefix, so a query cannot collide with them.
- A query that starts with `FROM` selects `*`, because DataFusion plans no columns for it.
- A pipe `SELECT` or `EXTEND` that calls a window function or `unnest` becomes a `SELECT` over a derived table of the operators before it, because DataFusion plans a window of a pipe `EXTEND` and fails to run it. So `FROM t |> EXTEND rank() OVER (ORDER BY x) AS r` becomes `SELECT *, rank() OVER (ORDER BY x) AS r FROM (SELECT * FROM t)`. The derived table keeps the qualifiers of the tables before it, so `x.b` stays valid after a window over `t AS x`, and the CTEs of the query stay readable after it.
- A pipe `CALL f(<args>) [OVER (PARTITION BY ...)] [AS t]` becomes `SELECT * FROM f((<input>), <args>, partition_by => [...]) [AS t]`, the `FROM`-position call of the table function over the operators before it, because DataFusion does not plan a pipe `CALL` of a table function. So `FROM t |> AGGREGATE sum(units) AS units GROUP BY month, region |> CALL impute(month, [region], units, value => 0) OVER (PARTITION BY series)` adds the missing months of each region in each series, and the operators after the call read its rows. The `OVER` clause holds only a `PARTITION BY` list of columns, and a call without it passes no `partition_by`.
- `|> CHART <mark call> [AS name] [GROUP BY ...]` becomes the `FROM`-position call `__avenger_chart((<input>), <mark call>, partition_by => [...], name => <name>)`, which the mark planner plans ([`|> CHART`](#-chart)). The token pass writes the operator as a pipe `AGGREGATE`, so its items and its `GROUP BY` parse as those of an `AGGREGATE`, and a group expression becomes a column of the input. The pass does not lower the mark call, whose channels the mark planner reads as written.
- A named option of `bin`, `timeunit`, `lump`, or `stack` becomes the option pair of the transform, after the positional arguments, so `bin(x, maxbins => 30)` becomes `bin(x, 'maxbins', 30)`. The `weight` and `keep` of `lump` are SQL expressions over the rows of each category, and the pass passes their text, so `weight => sum(units)` becomes `'weight', 'sum(units)'`.
- A call with named arguments passes `NULL` for each parameter before its last named argument that it omits, because DataFusion binds named arguments only as a prefix of the parameters. An omitted parameter of an avenger function and `NULL` mean the same thing, so `symbol(x => a, y => b, fill => c)` becomes `symbol(x => a, y => b, fill => c, color => NULL)`. A call of a function without parameter names, or with a name that no parameter has, stays as it is, and DataFusion reports its error.

The parser reads the tokens of the token pass, not printed text, so the statement keeps the spans of the text. `register` sets `datafusion.sql_parser.collect_spans`, so DataFusion reports an error of a column, such as an unknown column in a lowered `EXTEND`, with a diagnostic at the column's text.

## Mark functions

avenger-sql binds the functions of avenger-chart's `functions` feature: the README of avenger-chart describes their names, their parameters, and the kinds of their arguments. In SQL:

- A channel argument is an expression of each row, such as `body_mass * 2`. Its column is named after the expression, without the qualifiers of its columns, and keeps the type and the field metadata of the expression. Channels with equal expressions share one column.
- `NULL` leaves the channel unset, so an omitted parameter and `NULL` mean the same thing. In `scaled` or `direct`, a `NULL` is a value, which needs a type, such as `direct(CAST(NULL AS VARCHAR))`.
- The `ORDER BY` of a call is the draw order of its rows, `FILTER` selects them, and `GROUP BY` gives one chart for each group.
- A property, `coord`, and an option of `scaled` are constants, such as the folded call of a constructor.

A mark also takes the options of its plot that concern no other layer, `title`, `attribution`, `tools`, `width`, and `height`, and of its layer, `id` and `classes`, except an option whose name a parameter of the mark has, such as the `width` channel of `text`. A mark places each of its `tools` on the narrowest part that holds it: a navigation, such as `navigate()`, on its plot, and a tooltip or a selection on its layer, where it acts on that mark's instances only. So `symbol(x => a, y => b, tools => [navigate(), tooltip()])` pans and zooms its plot and shows a tooltip of each point. A tool needs no id: a plot or a layer without one is identified by its place in the chart and its partition, so an app keeps the tool's view while a new chart keeps its structure. An `id` names the mark's layer, for a stylesheet's `#id` or a view that a change of the structure keeps, and a selection's owner needs one, because its clauses name it. `layer(chart)` merges the options of its charts: its own options win, then each option comes from the one chart that sets it, two different values are a conflict, the tools of the charts join, and two equal navigations are one.

A title is Typst markup, so `title => '*Bills* of the _Pygoscelis_ penguins'` draws bold and italic text, and `title => title(text => 'Length × depth', subtitle => 'in $"mm"$')` a subtitle with math. A title of plain text sets `syntax => 'plain'`, as `title(text => 'Revenue in $', syntax => 'plain')`. The titles that axes and legends take from channels, such as `count(*)` and `body_mass_g`, are plain text.

DataFusion's expression simplifier binds each call to an aggregate of its own, which takes only the columns of the call, so an invalid call, such as one without a required channel, fails at planning. A channel argument cannot aggregate, because DataFusion rejects an aggregate inside an aggregate.

A constructor is a scalar function, which returns a fragment of its message in a `BinaryView` value whose field has the extension name `avenger.chart.fragment` and the full name of the message as its extension metadata. It is immutable, so a call of constants folds to a literal, and its errors, such as an unknown enum name, are planning errors.

The constructors of avenger-chart set the options of a chart: `scaled(flipper_length_mm, scale => interval_fit(zero => FALSE))` fits a domain to the data without zero, and `polar_bar(..., coord => polar(inner_radius => 0.5))` sets the options of a plot. Array literals, such as `scale(domain => [0, 100])`, need DataFusion's `nested_expressions` feature, which avenger-sql enables.

A channel argument of a mark function is an expression of each row, so the aggregates of a chart come from an `AGGREGATE` before the mark function, as `count(*)` does above. A window function, such as a bin of avenger-datafusion-transforms, goes in a pipe `EXTEND` before it, which the syntax pass writes as a derived table.

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

## `|> CHART`

The pipe operator `|> CHART` takes a mark call whose channels compute their statistics: `bin`, `timeunit`, and `lump`, aggregates, `stack`, and window functions, with a draw order, and groups its rows as a pipe `AGGREGATE` does, with a chart for each group:

```sql
FROM penguins
|> CHART bar(x => bin(body_mass_g, maxbins => 20), y => count(*), color => lump(species, n => 2))
   GROUP BY island
```

The call means its lifting, a query of stages that ends in the mark aggregate over column references, and `lift` prints it:

```sql
FROM penguins
|> EXTEND bin(body_mass_g, maxbins => 20) OVER () AS body_mass_g_bin,
          lump(species, n => 2) OVER () AS species_lump
|> AGGREGATE count(*) AS n GROUP BY island, body_mass_g_bin, species_lump
|> AGGREGATE bar(x => scaled(body_mass_g_bin, title => 'body_mass_g'), y => scaled(n, title => 'count(*)'),
     color => scaled(species_lump, title => 'species')) AS chart GROUP BY island
```

- **Stage 1** computes the discretizers, `bin`, `timeunit`, and `lump`, in one `EXTEND` for each depth of nesting, so `lump(timeunit(t, unit => 'month'), n => 3)` truncates in the first and lumps in the second.
- **Stage 2** computes the aggregates of a call that aggregates in a channel or in its draw order, grouped by the group columns and the keys: the channel values without an aggregate, such as `species`, a discretizer, or a key expression, such as `upper(species)`, which takes the name `x_expr`. An expression over aggregates and keys, such as `count(*) / 2`, reads their columns.
- **Stage 3** imputes the rows that the groups lack, with the `impute` option ([Imputation](#imputation)).
- **Stage 4** computes the stacks and the window functions in one `EXTEND` over the columns of stage 2, so `y => stack(count(*))` stacks the counts, and `count(*) / sum(count(*)) OVER panel` is the share of each count in its panel. A stack is a struct of a `start` and an `end`, which draws from its start to its end.
- **Stage 5** is the mark aggregate, with the draw order of the call over the columns of the earlier stages, grouped by the group columns. A `line` or an `area` connects the points of each series along its key channel, or in the order of its `order` channel, such as `order => year` for a connected scatter plot, and the draw order orders its series.

The added columns take short names: `n` for `count(*)`, `<aggregate>_<column>` such as `sum_units`, `<column>_bin`, `<column>_unit`, `<column>_lump`, `<column>_stack`, `<function>_<column>_window` such as `sum_n_window`, and `<channel>_expr`, with the smallest free suffix `_2`, `_3`, and so on for a name that the input already has. A scaled channel whose column the lifting creates takes the title of its source: a discretizer or a stack the title of its argument, so the axis of `bin(body_mass_g)` is `body_mass_g`, and another value its text, such as `count(*)`. A `title` of its `scaled` wins.

Each group is a panel. A discretizer without a window computes over every row of the input, `OVER ()`, so the panels share their bins, their units, and their kept categories, and compare. A window function of stage 4 also computes over every row of its stage, so `sum(count(*)) OVER ()` sums over every panel. The mark names three windows, which a window clause uses as the named windows of SQL, alone or with an `ORDER BY` and a frame:

- `panel`: `PARTITION BY` the group columns, so `OVER panel` computes in each panel.
- `stacks`: `panel` and the column of the key channel, for a stack with an order of its own, such as `stack(count(*)) OVER (stacks ORDER BY count(*) DESC)`.
- `stacked`: `stacks` with the implied order, the default window of `stack`.

A discretizer takes `OVER ()`, `OVER panel`, or a window with only a `PARTITION BY` list. The key channel and the measure channel of a stack come from the orientation of the mark: a channel with an aggregate or a stack is the measure when the other position has neither, then a discrete position, a string, a boolean, a discretizer, or a channel with a band, point, or ordinal scale, is the key when the other is continuous, and then the `orient` property decides, with `x` the key by default. A polar bar reads `theta` and `radius` in place of `x` and `y`, and a polar bar with a stack in `theta` and no `radius` is an arc, which stacks without a key. The implied order follows the legends: the domain order of each series channel, `color`, `fill`, `stroke`, `opacity`, `stroke_dash`, `stroke_width`, `pattern`, and `detail`, in this order, reversed in a vertical stack so that the first category of the legend is the top segment. A domain is ascending with `NULL` last, a `lump` orders its kept categories by weight with the Other group last, and a `domain` or a `category_fit` order of the channel's `scale` orders by its list. The argument of the stack and the other channel columns, ascending, break the ties, and then the flag of an imputation, so that an added row follows the rows of its tie.

The output of the call is the group columns, in the order of the `GROUP BY` list, and the chart, one row for each panel, as a pipe `AGGREGATE` gives them, so a composition aggregate composes the panels ([Compositions](#compositions)):

```sql
FROM penguins
|> CHART bar(x => bin(body_mass_g), y => count(*)) GROUP BY island
|> AGGREGATE facet_col(chart, island) AS chart
```

The chart column is `chart`, or the alias of the mark call, as in `|> CHART bar(...) AS counts GROUP BY island`. A group expression is a column of its alias or of its text, so `GROUP BY upper(island) AS isle` returns `isle` and `chart`. A mark call takes no `OVER` clause, and an item or a group takes no `ASC` or `DESC`.

The mark planner checks the call before it plans the stages, and reports each error at its text: an argument of a discretizer or an aggregate that reads a later stage, such as `bin(count(*))`, a value of a grouping call that is neither a key nor inside an aggregate, a draw order that is not a key, `scaled` and `direct` that are not the complete value of a channel, a window function without `OVER`, a window that the mark does not name, a stack window without an order, a stack without a key channel or a measure channel, an `impute` option that [Imputation](#imputation) rejects, an option that is not a plan-time constant, an `OVER` clause of the call with more than a `PARTITION BY` list of columns, and an argument of a transform of a type that the transform rejects, with the error of the transform.

A stack window that crosses panels, one whose `PARTITION BY` list lacks a group column, has no warning, because the SQL planner of DataFusion 55.2 keeps its warnings to itself. These forms are not supported yet: conditional channels, a `CASE` with `scaled` or `direct` in a result, domain orders by an aggregate, and inline compositions.

The example `sql-chart` shows the chart of a query in a chart app.

### Imputation

The `impute` option of a `bar`, `area`, `line`, `symbol`, or `text` call adds a row for each key value that a group lacks, after the aggregates and before the stacks, so every series of a stacked area has a value at every key:

```sql
FROM sales
|> CHART line(x => month, y => sum(units), color => region, impute => impute(method => 'gap'))
```

An `area` or a `line` whose measure is a `stack` imputes zero by default, so a stacked area draws every series at every key without the option:

```sql
FROM sales
|> CHART area(x => timeunit(t, unit => 'month'), y => stack(sum(units)), color => region)
```

The default takes the key values of `impute => 0`, so it also fills the empty bins and units between the keys. It applies only where the option could: a call whose stacked measure is not one aggregate or a column, or whose bins come from a partition that is not a key, keeps its rows. A call of one group whose key values are the distinct keys of its input lacks no key, so it has no stage 3. `impute => FALSE` turns the default off, and `impute => NULL` means an omitted option, as `NULL` does for every parameter.

`impute => 0` is short for `impute => impute(value => 0)`. `impute(...)` takes exactly one of `value`, a constant, and `method`, `'mean'`, `'median'`, `'min'`, `'max'`, or `'gap'`, which is `NULL` and breaks a line. A method of a statistic reads the measures of the existing rows of the group, or with `frame => [lo, hi]`, of the rows from `lo` to `hi` around the added row in key order, where `NULL` is the start or the end of the group. `keys => [...]` gives the key values, and `extra_keys => [...]` adds to them. The option is a parameter of the mark calls of `|> CHART`, which the mark planner reads as written, so it does not collide with the table function `impute`, and the chart holds no imputation.

The key is the key channel of the mark, and the measure is its measure channel without its `stack`: one aggregate in a call that aggregates, or a column in a call that does not. The groups are the group columns of the call and its other keys, except a key that is a function of the imputed key, such as `text => to_char(month, '%b')` next to `x => month`, which computes after the imputation from the key, so an added row has it. The key values are the `keys` of the option, the list of a `domain` or a `category_fit` order of the key channel's `scale`, every bin or unit between the keys of a `bin` or a `timeunit` key, or the distinct keys of the input. The key values come from every panel, so the panels that share a scale share their keys, except the bins of a `bin` with a partition, such as `bin(v) OVER panel`, which come from the mesh of each partition.

Stage 3 is a pipe `CALL` of the table function, which `lift` prints, with a flag column `imputed` that is true in an added row and that stage 5 does not read:

```sql
|> CALL impute(t_unit, [region], sum_units, value => 0, fill_gaps => true, flag => imputed)
```
