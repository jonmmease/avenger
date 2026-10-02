# avenger-datafusion-aggregate-state

Materializable aggregate states for Apache DataFusion **54.1.0**. Applications can aggregate raw rows once, retain the resulting Arrow batches, and merge selected states in later queries.

The crate exposes ClickHouse-style aggregate combinators through DataFusion UDFs. It uses native DataFusion accumulators and result types. Applications own query construction, materialization, storage, and caching.

## Operations

Each family has four functions:

| Function | Kind | Result |
|---|---|---|
| `avgState(x)` | Aggregate | An average state built from raw values |
| `avgMerge(s)` | Aggregate | The average of all values represented by the input states |
| `avgMergeState(s)` | Aggregate | One combined average state, reusable in another query |
| `avgFinalize(s)` | Scalar | A final value for each individual state row |

States retain the information needed for a correct rollup. For example, average states contain a count and a typed sum, so merging averages accounts for unequal group sizes. Variance and standard deviation use DataFusion's centered-moment states: count, mean, and the sum of squared deviations from the mean.

| Aggregate | SQL prefix | Rust prefix |
|---|---|---|
| Count | `count` | `count` |
| Sum | `sum` | `sum` |
| Minimum | `min` | `min` |
| Maximum | `max` | `max` |
| Average | `avg` | `avg` |
| Sample variance | `varSamp` | `var_samp` |
| Population variance | `varPop` | `var_pop` |
| Sample standard deviation | `stddevSamp` | `stddev_samp` |
| Population standard deviation | `stddevPop` | `stddev_pop` |

Append `State`, `Merge`, `MergeState`, or `Finalize` to a SQL prefix. Unquoted names are case insensitive; quoted camel-case and lowercase names are registered. Each family owns a distinct state type, including the four variance/stddev families. A `varSamp` state cannot be passed to `stddevSampMerge`.

Use `countState()` to count rows and `countState(expr)` to count non-null values. `countState(*)` is outside the supported SQL API.

## SQL and Rust APIs

`register_all` installs all functions for SQL use. Repeated installation is permitted; conflicting function names return an error. Factories in `functions` also allow selective registration.

```rust
use datafusion::{common::Result, prelude::SessionContext};
use avenger_datafusion_aggregate_state::register_all;

# async fn example() -> Result<()> {
let mut ctx = SessionContext::new();
register_all(&mut ctx)?;

// Collecting establishes a materialization boundary between the two queries.
let batches = ctx.sql(
    "SELECT avgState(x) AS s FROM (VALUES (10.0), (20.0)) AS raw(x)"
).await?.collect().await?;
ctx.register_batch("states", batches[0].clone())?;
let result = ctx.sql("SELECT avgMerge(s) FROM states").await?.collect().await?;
assert_eq!(result[0].num_rows(), 1);
# Ok(())
# }
# tokio::runtime::Runtime::new().unwrap().block_on(example()).unwrap();
```

For an interactive chart, first collect and register the result of a cell query:

```sql
SELECT airline, delay_cell, avgState(distance) AS distance_state
FROM flights
GROUP BY airline, delay_cell
```

Then reuse that materialized table for changing brush regions:

```sql
SELECT airline, avgMerge(distance_state) AS average_distance
FROM flight_states
WHERE delay_cell BETWEEN 10 AND 30
GROUP BY airline
```

The `expr_fn` module supplies ordinary DataFusion expressions:

```rust
use datafusion::logical_expr::col;
use avenger_datafusion_aggregate_state::expr_fn::{avg_state, avg_merge};

let partial = avg_state(col("distance")).alias("distance_state");
let complete = avg_merge(col("distance_state")).alias("average_distance");
```

Use these expressions with `LogicalPlanBuilder::aggregate` or DataFrames. They carry their function implementations, so programmatic plans need no SQL registration. Each expression helper has a matching factory, such as `functions::avg_state_udaf()` or `functions::avg_finalize_udf()`. `expr_fn::count_star_state()` is the Rust row-count helper.

Runnable examples build source data, print tables, and check results against native raw-data aggregation:

```sh
cargo run -p avenger-datafusion-aggregate-state --example sql_rollup
cargo run -p avenger-datafusion-aggregate-state --example logical_rollup
```

Both examples collect cell states, evaluate two brush regions, and materialize a further rollup with `MergeState`.

## Types, filters, and nulls

Coercion and result types come from the pinned native aggregate. The tested type matrix includes:

| Family | Tested inputs |
|---|---|
| Count | Row count; nullable numeric values |
| Sum | Signed/unsigned 64-bit integers, Float32/64, Decimal128 |
| Average | Signed/unsigned 64-bit integers, Float32/64, Decimal128 with multiple precision/scale pairs |
| Min/max | Numeric values, Decimal128, strings, booleans, dates, timestamps |
| Variance/stddev | Float64, including empty groups, singleton groups, and non-finite values |

Other flat types accepted by the native implementation use its coercion and accumulator factories. Nested inputs are rejected except for count. Unsupported signatures return DataFusion planning errors. There is no conversion of every family's state to Float64.

SQL aggregate `FILTER` works at each invocation. A filter on `Merge` or `MergeState` includes or excludes **whole states**; it cannot remove individual raw rows already incorporated into a state. The crate has no separate filtering language.

- `State` and `MergeState` return a present state for every observed group, even when it represents only null or filtered-out values. A zero-row global aggregate also returns a present empty state.
- `Merge` and `MergeState` ignore outer null states, such as states missing after a join.
- `Finalize` propagates an outer null state to a null result, including for count.
- A present empty count state finalizes to zero. The other families return null for empty/all-null input. For finite singleton input, population variance/stddev is zero and sample variance/stddev is null.

`DISTINCT`, aggregate `ORDER BY`, and explicit null-treatment modifiers are rejected. Window usage is outside this release's supported API.

## Numerical behavior

Accumulation and merging use DataFusion's algorithms, including native overflow behavior. Regrouping can change floating-point rounding, overflow, and special-value results. This crate does not promise bitwise equality with a different query shape or ClickHouse's numeric semantics.

`Merge` follows native grouped or global evaluation as appropriate. `Finalize` follows native single-accumulator evaluation for each state. In DataFusion 54.1.0 these paths differ for some non-finite values:

- Population variance/stddev on one non-finite value can produce NaN in grouped execution; single-accumulator finalization returns zero.
- Grouped floating minimum starts from the largest finite value, and maximum from the smallest finite value. An all-positive-infinity minimum cell or all-negative-infinity maximum cell can therefore retain that finite bound. Subsequent merges consume the exported state; they cannot recover the original infinity.

Tests compare schemas and exact values for exact types. Ordinary finite floating fixtures use `abs(actual - expected) <= 1e-10 * (1 + abs(expected))`. A separate large-offset moment test uses small integer offsets around `1e12` and an independent reference, with an absolute tolerance of `5e-4`. These are test-fixture bounds, not general error guarantees.

## State encoding and compatibility

An exported state is an Arrow struct containing a family-specific versioned payload, for example `avg_state_v1`. Its child arrays contain native state columns. Nested field metadata records the coerced argument types, result type, and DataFusion version. Query aliases do not affect the state type.

Consumers validate the family, encoding version, invocation signature, field layout, and required payload validity. Missing metadata or incompatible states return errors. Retain the complete Arrow data type, including nested field metadata, when storing or transporting states.

Arrow IPC preserves this representation. An integration test writes a decimal average state, reads it into a fresh context, and merges it there. The normal UDF execution path performs **no IPC or byte serialization**. Grouped packing wraps existing arrays without copying their value buffers; grouping, filtering, and final result construction still allocate memory.

Native grouped accumulators are used where available. Scalar `Finalize` batches numeric implementations and extracts extrema results directly from their payloads. Grouped State/Merge operations fall back to native per-group accumulators for types without grouped support, such as boolean extrema. Public declarations use private macros; execution and validation use shared Rust adapters.

Compatibility is limited to this encoding version and the pinned DataFusion implementation. A DataFusion upgrade requires reviewing native state layout and semantics. Cross-version migration, ClickHouse binary interchange, Parquet round-trip guarantees, and DataFusion protobuf function codecs are not provided. IPC data serialization does not serialize the UDF implementations or their logical plans.

## Validation and performance

```sh
cargo test -p avenger-datafusion-aggregate-state --all-targets
cargo test -p avenger-datafusion-aggregate-state --doc
cargo clippy -p avenger-datafusion-aggregate-state --all-targets -- -D warnings
cargo bench -p avenger-datafusion-aggregate-state --bench grouped
```

The benchmark compares native cell aggregation with `State`, raw aggregation with `Merge`, filtered merges, batched numeric `Finalize`, extrema payload extraction, and grouped boolean accumulation through the native per-group fallback. It uses 200,000 rows, four partitions, and two group counts. Override `AGG_STATE_ROWS` and `AGG_STATE_ITERATIONS` to change the workload. Physical plans are prepared before timing; each measurement includes execution and collection. Materialization is performed outside merge timings. The result depends on how much the cell table reduces the raw data.

The package uses standalone dependency declarations and has no Avenger dependencies. It requires Rust 1.88 or later, matching DataFusion 54.1.0.

## Query planning

[`avenger-datafusion-preaggregate`](../avenger-datafusion-preaggregate) is an
optional consumer of these functions. It derives materialization and rollup
logical plans from queries with changing filters, including per-aggregate
filters and supported finishing operators. The state utility remains independent
of that planner and of execution runtimes.
