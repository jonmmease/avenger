# avenger-datafusion-transforms

Data transforms for Apache DataFusion: the Vega-style `bin`, `stack`, `timeunit`, and `impute`, and `lump`, which keeps the top categories and lumps the rest into an Other group. A front end, such as avenger-sql, builds a typed call of a transform from its planned arguments, and the crate plans the call:

```rust
let call = BinCall {
    value: col("x"),
    partition: vec![col("g")],
    options: BinOptions { maxbins: 30.0, ..Default::default() },
};
let plan = bin::plan(input, call, Column::from_name("binned"))?;
```

`bin::plan`, `timeunit::plan`, `lump::plan`, and `impute::plan` add the transform to the plan of an input. `bin` and `timeunit` plan as an aggregate of each window partition, joined back to its rows, and `lump` as a grouped plan of its categories. Among the windows of a projection, a front end writes a call of `bin`, `timeunit`, or `lump` as the placeholder window of `bin::window`, `timeunit::window`, or `lump::window`, and `plan_windows` plans each placeholder onto the input. `stack::window` is the window function of `stack`. The options of a call are Rust values, and their `check` methods return an `OptionError` that names the option, so a front end places the error at its argument.

| Transform | Result | Notes |
|---|---|---|
| `bin` | `struct<start, end>` | Bins of a nice step, for at most `maxbins` bins, over the extent of each window partition or an explicit `extent`. `bin::bin_params` gives the bins of an extent. |
| `stack` | `struct<start, end>` | The layers of each stack, with the offsets `zero`, `normalize`, and `center`, which centers each stack on zero. |
| `timeunit` | `struct<start, end, unit>` | A calendar unit in the time zone of the value's type, chosen for each window partition or given with `unit`. Units include fiscal quarters and years, and collapsing units such as `'month_of_year'`. |
| `lump` | `struct<value, other, weight, category_count>` | Keeps the top categories, or the categories that a condition keeps, and lumps the rest into an Other group, with the number of categories that it holds. `weight` is an aggregate of the rows of each category, and `keep` a condition of aggregates of its rows and windows over the categories, which take no `PARTITION BY`. `null_category` keeps the NULL category apart from the ranking, puts it in the Other group, or gives its rows a NULL lump. |
| `impute` | the input's rows, then the added rows | Adds the rows that each group lacks, with a constant or a statistic over a frame. `fill_gaps` adds the bins, time units, or dates that no row has, and `partition_by` imputes partitions separately. |

A table function such as `impute` reads the plan of an input relation. Every table function takes `partition_by`, which transforms partitions separately and in parallel. The module `table_function` holds this machinery for new table functions, its extension planner, and `ArgError`, which places the error of an argument at the argument.

An aggregate grouped by a lump, and by the lump's partition keys, plans through the analyzer rule `lump::LumpAggregateFusion` without the join of the lump to its rows: the categories hold the states of their rows' aggregates, and each value of the lump merges them, for the aggregates that merge from states, such as `SUM` and `AVG`.

The module `match_recognize` holds the matcher of row pattern recognition and its node. A pattern compiles to a program, which a Pike VM runs over each partition's rows, with the preferences of GoogleSQL's matcher, both skip modes, and `use_longest_match`. `match_recognize::plan` takes a plan whose rows have their position in the partition and a boolean column for each pattern variable, and outputs each row of each match with its match number and its variable, and one row without a variable for each empty match. avenger-sql plans GoogleSQL's `MATCH_RECOGNIZE` onto it, and computes the measures with an aggregate over its output. `tail_matches` matches the end of a partition as rows arrive: it returns the settled frontier, the first row whose match a later row, a condition that reads past the last row, or `$` can still change, so a streaming runtime matches only the rows from there again.

The module `codec` writes the transforms of a plan as bytes and builds them again, for a front end that sends its plans to where they run, such as avenger-sql in the file of a baked chart graph: each function by its name and its state, such as the fields of a struct that `bin` makes, and a table function's node by the transform of each of its partitions, the spec of `impute` or the program of `MATCH_RECOGNIZE`, its partition columns, and its schema, with its input as its child. The node's parameters are bound when it is written, so the node that a reader builds has no arguments.

## Tests

The tests state what each transform does: bins, stacks, and time units of chosen values and options, time units that tile time across changes of the UTC offset, `impute` and `lump` against equivalent SQL, the gaps and partitions of `impute`, and the transforms' state, which their codec writes and reads again. The tests build the calls in Rust, plan the inputs and the reference queries in DataFusion's SQL, which only the tests enable, and compare the results.
