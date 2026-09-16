# avenger-datafusion-dataflow

Build a graph from named DataFusion logical plans and scalar expressions, then query selected outputs against immutable scalar and table inputs.

This crate implements phases 1 through 3: graph construction and correct evaluation of root, composite-key, and nested scopes. Every demanded named computation runs once per defining instance per query. The runtime creates fresh physical plans during each query and retains no computed results between queries. Graph fusion, column pruning across nodes, compiled-plan reuse, and cross-query caching are later phases.

The crate has no dependencies on Avenger chart, scene graph, or rendering crates. It re-exports `datafusion` and `arrow` so consumers can use matching types.

## Run the complete example

```sh
cargo run --release -p avenger-datafusion-dataflow --example parameterized_graph
```

The [example](examples/parameterized_graph.rs) builds this dependency chain:

```text
sales table → revenue totals → maximum table → maximum scalar
                  │                                  │
                  │                     fraction → threshold
                  │                                  │
selection table ──┴──────────────────────────────────┴─→ visible rows
```

It queries table and scalar outputs together, changes a scalar parameter, then publishes a replacement selection table. Its sample data produces:

| Fraction | Selected regions | Threshold | Visible totals |
|---|---|---|---|
| 0.5 | East, West, North | 100 | East: 200, West: 150 |
| 0.8 | East, West, North | 160 | East: 200 |
| 0.5 | West | 100 | West: 150 |

The example prints the actual Arrow tables and per-query execution reports. All three evaluations execute five graph nodes. The shared totals computation executes once within each evaluation.

## Public API

```rust
use std::sync::Arc;
use avenger_datafusion_dataflow::{
    arrow::{
        array::Int64Array,
        datatypes::{DataType, Field, Schema},
        record_batch::RecordBatch,
    },
    datafusion::{
        common::ScalarValue,
        logical_expr::{col, LogicalPlanBuilder},
    },
    GraphBuilder, Runtime, RuntimeConfig, TableSnapshot,
};
# async fn example() -> avenger_datafusion_dataflow::Result<()> {

let schema = Arc::new(Schema::new(vec![
    Field::new("value", DataType::Int64, false),
]));
let mut graph = GraphBuilder::new();
let source = graph.table_input("source", schema.clone())?;
let cutoff = graph.scalar_input("cutoff", DataType::Int64)?;
let cutoff_value = graph.add_expr("cutoff_value", cutoff.expr_ref())?;
let filtered = graph.add_plan(
    "filtered",
    LogicalPlanBuilder::from(source.plan_ref())
        .filter(col("value").gt(cutoff_value.expr_ref()))?
        .build()?,
)?;
let rows = graph.table_output("rows", &filtered)?;
let threshold = graph.scalar_output("threshold", &cutoff_value)?;

let runtime = Runtime::new(RuntimeConfig::default())?;
let prepared = runtime.prepare(&graph.finish()?).await?;
let batch = RecordBatch::try_new(
    schema.clone(),
    vec![Arc::new(Int64Array::from(vec![1, 2, 3]))],
)?;
let inputs = prepared.inputs()
    .table(&source, TableSnapshot::from_batches(schema, vec![batch])?)?
    .scalar(&cutoff, ScalarValue::Int64(Some(1)))?
    .finish()?;

let result = prepared.query(&[rows], &[threshold], &inputs).await?;
assert_eq!(result.table(&rows)?.num_rows(), 2);
assert_eq!(result.scalar(&threshold)?, &ScalarValue::Int64(Some(1)));

let next_inputs = inputs.edit()
    .scalar(&cutoff, ScalarValue::Int64(Some(2)))?
    .finish()?;
let next = prepared.query(&[rows], &[], &next_inputs).await?;
assert_eq!(next.table(&rows)?.num_rows(), 1);
# Ok(())
# }
# let runtime = tokio::runtime::Runtime::new().unwrap();
# runtime.block_on(example()).unwrap();
```

### Graph definitions and names

Register inputs and computations before referencing them. `plan_ref()` returns a private logical extension leaf, and `expr_ref()` returns a typed scalar placeholder. A reference records identity rather than copying a producer's full lineage. Registration validates graph ownership and dependencies. Insertion order provides a topological execution order, so references cannot form a cycle.

Input names, computation names, and output names have separate namespaces within each scope. Child scope names are unique among siblings. Plans and expressions share the computation namespace. Output declarations make selected node values available to callers. An output may have the same name as its producer. Handles from another graph are rejected.

Bindings use handles. A chart compiler can maintain a scoped name-to-handle map without putting visualization naming rules into this runtime.

### Inputs and stores

`TableSnapshot` holds a schema, a shared collection of record batches, and an opaque identity. Constructors validate schemas and allocate an ID from a process-wide atomic counter. Identical contents constructed separately receive different IDs. Clones share Arrow buffers and preserve identity. Empty tables preserve their declared schema.

`TableStore` optionally owns the current snapshot. Its clones share that owner. Replacing its snapshot validates the schema and atomically publishes the new data and identity. Republishing an old snapshot preserves the old identity.

`InputsBuilder::finish()` requires every declared root input, including root inputs irrelevant to the requested output subset. It validates supplied scoped bindings, while queries require effective scoped values only for discovered instances and demanded computations that read them. Scalar types and table schemas must match exactly. Typed scalar nulls are supported. `Inputs` captures its snapshots, so changing a store does not change existing bindings. Capturing several stores separately does not provide a transaction across them.

### Scoped sub-dataflows

`partition_by(name, source_plan, key_expressions, callback)` defines a child template. Its callback runs once during construction and returns any Rust value, usually a struct of handles. Output declarations determine which values are queryable. A callback that fails or panics makes `GraphBuilder::finish()` fail, so partially constructed handles cannot identify a different definition later.

`ScopeBuilder` provides the same input, plan, expression, output, and partition operations as `GraphBuilder`. `scope.rows()` returns one automatically bound `PlanNode` with the source schema. It can be exported directly. Plans can read current-scope and ancestor handles. Sibling and descendant references are rejected, including inside subqueries. Child collections cannot be consumed by parent computations through this API.

Run the [nested and composite facet example](examples/scoped_facets.rs):

```sh
cargo run --release -p avenger-datafusion-dataflow --example scoped_facets
```

It discovers regions and years from sales rows, captures root and regional scalars, binds a selection table per year, and changes East / 2025 through its returned instance address. The instance remains present when its filtered output becomes empty. The same graph also demonstrates flat partitioning by `[region, year]`.

| API | Contract |
|---|---|
| `scope.key(values)` | Validate an ordered local key with exact scalar types |
| `scope.instance(values)` | Address a top-level instance without discovering or creating it |
| `parent.child(&scope, values)` | Address an immediate child using the full ancestor path |
| `inputs.scope_defaults(&scope, callback)` | Edit defaults for inputs declared directly in that definition |
| `inputs.at(&instance, callback)` | Edit sparse overrides at an exact address, including absent instances |
| Scoped `.scalar()` / `.table()` | Set an explicit value, including a typed null or empty table |
| Scoped `.unset_scalar()` / `.unset_table()` | Remove only that entry, restoring inheritance when removing an override |
| `result.scope(&scope)` / `panel.scope(&scope)` | Access an available immediate child collection |
| `collection.iter()` / `.get(&key)` | Iterate observed instances or look up a compatible local key |
| `panel.instance()` | Borrow the complete address, which can be cloned for interaction events |
| `panel.table(&output)` / `.scalar(&output)` | Borrow already materialized requested values |

Resolve each scoped input from its exact override, then its definition default. Repeated writes replace one entry without copying resolved defaults. Changing a default preserves explicit overrides. Missing bindings fail only when a demanded computation reads them. An absent-instance override remains configuration and can become active after source replacement.

The query signature stays `query(tables, scalars, inputs)`. Scoped output handles request all observed instances of their definition. Descendant requests expose ancestor navigation without exposing unrequested ancestor outputs. Scopes outside requested paths are unavailable. Both empty slices perform no discovery. A wrong-scope output access fails, and `.get()` returns `None` for absent or type-incompatible keys.

Discovery uses the source before child transforms. Empty sources have no observed instances, and a present parent can have an empty child collection. Composite keys include only observed tuples, without generating a crossed grid. Iteration order is unspecified. A computed key does not add a public source column.

The evaluator materializes a source and its keys once per containing instance, groups row indices, and gathers local batches before running child frames. It does not run one full-source filter query per key. Result lookup performs no planning, filtering, or deferred gathering. Views borrow their result, while cloned table snapshots keep buffers alive independently. Instance addresses retain no tables or results.

#### Supported key types

Keys support `Null`, Boolean, signed and unsigned integers of 8 through 64 bits, UTF-8 and binary values in ordinary, large, and view representations, `Date32`, `Date64`, all timestamp units with their declared timezone, and `Decimal128`/`Decimal256` with their declared precision and scale. Grouping and lookup use the same typed tuple equality and hashing. Typed nulls form ordinary groups.

Preparation rejects other key types, including floating-point, nested, dictionary, fixed-size binary, time-of-day, duration, interval, and 32/64-bit decimal types. Keys have no implicit casts and are not serialized display strings. A local `PartitionKey` is portable between compatible key schemas. A `ScopeInstance` includes graph and definition identities along its full path.

Prepared physical reuse, grouped execution, caching, explicit facet domains, ancestor-specific defaults, and requests restricted to selected instance addresses are future work. The current scoped API is a correctness baseline and does not promise interactive latency for hundreds of facets.

### Evaluation semantics

`query(tables, scalars, inputs)` evaluates both output sets in one execution. Either slice can be empty. Duplicate output handles do not duplicate execution. Two empty slices execute no graph nodes. Output handles are small `Copy` values, and result access remains typed.

The evaluator visits demanded nodes in dependency order, materializes each value, and supplies those values to downstream nodes. All consumers of a shared table read the same completed snapshot. Consumers can project different columns without changing their producer. This baseline preserves full table schemas at node boundaries.

Named prerequisites are strict. If a requested node references another named expression, that prerequisite runs even when the consuming expression puts its reference inside a false condition. Its failure fails the query. Conditional expressions and supported correlated subqueries inside one submitted section keep DataFusion's semantics. Standalone expression nodes cannot contain unresolved row references, bare aggregates, or window functions.

Scalar subqueries return a typed null for zero rows, a scalar for one row, and an error for more than one row. A private compatibility projection preserves nullable scalar-subquery fields with DataFusion 54.1, including when the underlying table field is non-nullable.

Function signatures determine volatility. `Stable` and `Volatile` dependencies propagate evaluation-local scope through the graph. A named volatile scalar produces one value per defining instance per query, shared by its consumers. Root values are shared across descendants, and regional values are shared by that region's nested instances. Separate named volatile nodes evaluate independently. Volatile expressions in table plans retain their row-wise semantics. Every node sees the same captured query start time, so `now()` agrees across independently planned regions.

The runtime returns results only after all requested outputs succeed. Errors retain their source and node context, with complete instance addresses for scoped failures. Dropping a query future releases its work and reservations. Concurrent queries have separate inputs, values, and planning state. Cross-query work coalescing and its cancellation protocol are later phases.

### Preparation and diagnostics

`prepare()` validates and analyzes all declared output paths. `PreparedGraph` retains definitions, dependency metadata, and analyzed plans. It does not retain input snapshots or compiled physical plans.

`prepared.explain()` reports reachable nodes, definition paths, partition key schemas, ancestor captures, dependencies, external inputs, volatility, and reuse scope. `result.report()` reports executed nodes, physical planning count, captured query time, charged materialization, and per-definition instance, execution, and partitioned-row counts. Routine reports omit data-derived key values. `Reusable` means eligible for future result reuse. It does not mean caching is enabled in this version.

`Runtime::with_session_state()` captures DataFusion configuration and function registries and installs the crate's snapshot planner. Native relational plans, values, and supported subqueries execute through DataFusion. External table scans, session-variable expressions, DDL, DML, control statements, recursive queries, and unknown logical extensions are rejected. Bind external data as explicit snapshots.

### Resource limits

`RuntimeConfig` currently exposes `ExecutionConfig`: maximum active queries and an aggregate byte budget for materialized values owned by active queries. The defaults are four queries and 256 MiB. Regions and scope instances within one query execute sequentially. All frames share one query permit, captured timestamp, and materialization budget.

The byte budget conservatively charges Arrow array allocations, scalar copies, partition indices, instance addresses, and frame/result metadata, potentially counting shared buffers more than once. Charges accumulate until the query ends, including temporary partition buffers already released. This can reject a query below the budget in actual live memory. A query that cannot retain its required values fails with `ResourceExhausted`. This budget does not bound caller-owned inputs, returned results, or DataFusion operator memory. Operator memory uses DataFusion's runtime environment. Cache configuration and physical reuse policies will be added with their implementation phases.

## Validation

```sh
cargo test --release -p avenger-datafusion-dataflow --locked
cargo clippy --release -p avenger-datafusion-dataflow --all-targets --locked -- -D warnings
```

The tests cover graph and scope ownership, immutable bindings, nested and composite discovery, supported key types, native DataFusion result comparisons, scalar-subquery cardinality, strict dependencies, per-instance volatile sharing, stable query time, result lookup, cancellation, and concurrent input isolation.
