# avenger-datafusion-aggregate-state

Aggregate states for Apache DataFusion that a query can return and a later query can merge. An application aggregates its rows once into a table of states, keeps the table, and merges the states that each later query selects. One use is pre-aggregation for cross filtering: a table of cells, by the groups of each view and the cells of a brushed dimension, answers each position of the brush by merging the cells that it covers.

## Operations

Each of the native aggregates `count`, `sum`, `min`, `max`, `avg`, `var_samp`, `var_pop`, `stddev_samp`, and `stddev_pop` is a [`Family`] with three aggregates, named after it:

| Aggregate | Result |
|---|---|
| `avg_state(x)` | The state of the rows' values |
| `avg_merge(s)` | The final value of the states, as `avg` of their rows gives it |
| `avg_merge_state(s)` | One state of the states, for a further rollup |

A state holds what a merge needs: an average's count and sum, so averages of groups of different sizes merge with their weights, and the count, mean, and sum of squared deviations of the variance and standard deviation families. `count_state()` counts rows, and `count_state(x)` counts the values that are not null.

## SQL and Rust

`register_all` registers the aggregates for SQL:

```rust
use datafusion::{common::Result, prelude::SessionContext};
use avenger_datafusion_aggregate_state::register_all;

# async fn example() -> Result<()> {
let mut ctx = SessionContext::new();
register_all(&mut ctx)?;

// Collecting the states ends the first query, as storing them would.
let batches = ctx.sql(
    "SELECT avg_state(x) AS s FROM (VALUES (10.0), (20.0)) AS raw(x)"
).await?.collect().await?;
ctx.register_batch("states", batches[0].clone())?;
let result = ctx.sql("SELECT avg_merge(s) FROM states").await?.collect().await?;
assert_eq!(result[0].num_rows(), 1);
# Ok(())
# }
# tokio::runtime::Runtime::new().unwrap().block_on(example()).unwrap();
```

A table of cells for a cross filter, and the merge for one position of a brush:

```sql
SELECT airline, delay_cell, avg_state(distance) AS distance_state
FROM flights
GROUP BY airline, delay_cell
```

```sql
SELECT airline, avg_merge(distance_state) AS average_distance
FROM flight_states
WHERE delay_cell BETWEEN 10 AND 30
GROUP BY airline
```

In Rust, `state`, `merge`, and `merge_state` build the expressions of a family, and `state_udaf`, `merge_udaf`, and `merge_state_udaf` give its aggregates. The expressions carry their aggregates, so a plan built from them needs no registration:

```rust
use datafusion::logical_expr::col;
use avenger_datafusion_aggregate_state::{merge, state, Family};

let partial = state(Family::Avg, col("distance")).alias("distance_state");
let complete = merge(Family::Avg, col("distance_state")).alias("average_distance");
```

The example `sql_rollup` builds a table of cells, merges it for two positions of a brush, and checks each result against the native aggregate of the rows:

```sh
cargo run -p avenger-datafusion-aggregate-state --example sql_rollup
```

## Filters and nulls

A `FILTER` on a `state` call filters rows. A `FILTER` on a `merge` or a `merge_state` call takes or leaves whole states, and so does a `WHERE` on a table of states. A merge skips null states, such as the states that an outer join leaves missing. A `state` or a `merge_state` returns a state for each group, even a group whose rows a filter removes, and a merge of such states returns what the native aggregate returns for no rows: 0 for a count and null otherwise. `DISTINCT`, an aggregate `ORDER BY`, and `IGNORE NULLS` are errors.

## State encoding

A state is an Arrow struct with one field, such as `avg_state_v1`, whose struct holds the native accumulator's state columns. The field's metadata records the input types, the result type, and the DataFusion version, so a state keeps its identity through aliases and Arrow IPC. A merge rejects a state of another family, a missing or different signature, a state of another DataFusion version, and a state that lacks a required value. Keep the whole Arrow type, with its field metadata, when storing states.

The aggregates run DataFusion's own accumulators, grouped where DataFusion has a grouped one, so a merge follows the native aggregate's arithmetic, overflow, and type rules, and a regrouping can change floating-point rounding. A grouped `min` or `max` of floats starts from the largest or the smallest finite value, so a cell whose values are all positive or all negative infinity keeps that finite bound in its state.
