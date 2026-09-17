# avenger-datafusion-dataflow

Build a graph from named DataFusion logical plans and scalar expressions, then query selected outputs against immutable scalar, table, and expression inputs.

The native lifecycle is `DataflowBuilder → Dataflow → Runtime::prepare() → PreparedDataflow::query()`. It supports root, composite-key, and nested scopes, bounded completed-result caching, protobuf serialization, and an optional JSON/SQL adapter. A demanded computation runs at most once per defining instance per query. Cache hits skip prerequisite execution and physical planning. Misses create fresh DataFusion plans. Fusion, column pruning across nodes, and retained physical plans remain deferred.

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

The example prints the actual Arrow tables and per-query execution reports. The file source has its own named boundary. Changing the fraction or selection reuses resident source and aggregate results. Each report shows cache hits, source executions, and physical plans.

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
    DataflowBuilder, Runtime, RuntimeConfig, TableSnapshot,
};
# async fn example() -> avenger_datafusion_dataflow::Result<()> {

let schema = Arc::new(Schema::new(vec![
    Field::new("value", DataType::Int64, false),
]));
let mut graph = DataflowBuilder::new();
let source = graph.table_input("source", schema.clone())?;
let cutoff = graph.scalar_input("cutoff", DataType::Int64)?;
let cutoff_value = graph.add_scalar("cutoff_value", cutoff.expr_ref())?;
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

### Expression inputs and scalar computations

`scalar_input(name, type)` accepts one value per scope instance. `add_scalar(name, expr)` registers a named `ScalarNode` that computes one value per scope instance. `expr_input(name, type)` accepts a caller-supplied DataFusion `Expr` that runs in each consuming operation's row context. It can be a filter predicate, a projected value, or an argument to an aggregate already in the definition.

```rust,ignore
let selection = graph.expr_input("selection", DataType::Boolean)?;
let filtered = graph.add_plan(
    "filtered",
    LogicalPlanBuilder::from(source.plan_ref())
        .filter(selection.expr_ref())?
        .build()?,
)?;
// After finishing and preparing the definition:
let inputs = prepared.inputs()
    .expr(&selection, col("amount").gt(lit(20_i64)))?
    .finish()?;
```

Declarations need only a name and return type. Binding validates column references and the exact return type against every usage site, including consumers outside the requested outputs. Unqualified columns resolve independently in each consumer. Qualified columns must match each consumer's schema. Bindings cannot change output names, types, or conservative nullability. An unused declaration receives structural and volatility checks without a fabricated row schema or a return-type check. A use in a standalone scalar computation has an actual empty row context and rejects bare columns. All root inputs remain required.

The initial binding subset includes literals, columns, arithmetic, comparisons, Boolean and null operations, `CASE`, casts, lists for `IN`, and immutable scalar UDFs. Subqueries, aggregates, windows, graph references, placeholders, session variables, outer references, unnesting, higher-order functions, and Stable or Volatile functions are rejected in bindings. Existing scalar computations keep their normal volatility rules. Validation performs no reads or function evaluation. Native Rust bindings do not need a protobuf codec.

Expression trees are substituted before query optimization on cache misses. Their structural identity contributes only to dependent result keys. Returning from A to B to A can reuse A while its result remains resident. This does not push a predicate across a materialized named boundary. Keep an expensive source in its own named node to reuse it across expression changes. See the [file-backed expression example](examples/expression_inputs.rs):

```sh
cargo run -p avenger-datafusion-dataflow --example expression_inputs
```

Use `.expr()` in root bindings, `scope_defaults()`, and `.at()` overrides. Scoped `.unset_expr()` removes the edited entry, restoring default inheritance when removing an override. Table, scalar, and expression inputs have corresponding declaration, binding, scoped-removal, and typed name-lookup methods.

`dataflow.interface().root().inputs()` enumerates directly declared inputs in declaration order. Each `InputHandle::Table`, `InputHandle::Scalar`, or `InputHandle::Expr` contains its typed handle. The handle exposes its name and schema or field. Child interfaces obtained with `.scope(name)` enumerate their own declarations without flattening ancestors. Enumeration and name lookup retain no source assets or plan lineage.

### Dataflow definitions and names

Register inputs and computations before referencing them. `plan_ref()` returns a private logical extension leaf, and `expr_ref()` returns a typed placeholder for a scalar value or an input expression. A reference records identity rather than copying a producer's full lineage. Registration validates graph ownership and dependencies. Insertion order provides a topological execution order, so references cannot form a cycle.

Input names, computation names, and output names have separate namespaces within each scope. Child scope names are unique among siblings. Plans and expressions share the computation namespace. Output declarations make selected node values available to callers. An output may have the same name as its producer. Handles from another graph are rejected.

Bindings use handles. A chart compiler can maintain a scoped name-to-handle map without putting visualization naming rules into this runtime.

### Inputs and stores

`TableSnapshot` holds a schema, a shared collection of record batches, and an opaque identity. Constructors validate schemas and allocate an ID from a process-wide atomic counter. Identical contents constructed separately receive different IDs. Clones share Arrow buffers and preserve identity. Empty tables preserve their declared schema.

`TableStore` optionally owns the current snapshot. Its clones share that owner. Replacing its snapshot validates the schema and atomically publishes the new data and identity. Republishing an old snapshot preserves the old identity.

`InputsBuilder::finish()` requires every declared root input, including root inputs irrelevant to the requested output subset. It validates supplied scoped bindings, while queries require effective scoped values only for discovered instances and demanded computations that read them. Scalar types and table schemas must match exactly. Typed scalar nulls are supported. `Inputs` captures its snapshots, so changing a store does not change existing bindings. Capturing several stores separately does not provide a transaction across them.

### Scoped sub-dataflows

`partition_by(name, source_plan, key_expressions, callback)` defines a child template. Its callback runs once during construction and returns any Rust value, usually a struct of handles. Output declarations determine which values are queryable. A callback that fails or panics makes `DataflowBuilder::finish()` fail, so partially constructed handles cannot identify a different definition later.

`ScopeBuilder` provides the same input, plan, expression, output, and partition operations as `DataflowBuilder`. `scope.rows()` returns one automatically bound `PlanNode` with the source schema. It can be exported directly. Plans can read current-scope and ancestor handles. Sibling and descendant references are rejected, including inside subqueries. Child collections cannot be consumed by parent computations through this API.

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

For standalone dataflows, the query signature stays `query(tables, scalars, inputs)`. Scoped output handles request all observed instances of their definition. Descendant requests expose ancestor navigation without exposing unrequested ancestor outputs. Scopes outside requested paths are unavailable. Both empty slices perform no discovery. A wrong-scope output access fails, and `.get()` returns `None` for absent or type-incompatible keys.

Discovery uses the source before child transforms. Empty sources have no observed instances, and a present parent can have an empty child collection. Composite keys include only observed tuples, without generating a crossed grid. Iteration order is unspecified. A computed key does not add a public source column.

The evaluator materializes a source and its keys once per containing instance, groups row indices, and gathers local batches before running child frames. It does not run one full-source filter query per key. Result lookup performs no planning, filtering, or deferred gathering. Views borrow their result, while cloned table snapshots keep buffers alive independently. Instance addresses retain no tables or results.

#### Supported key types

Keys support `Null`, Boolean, signed and unsigned integers of 8 through 64 bits, UTF-8 and binary values in ordinary, large, and view representations, `Date32`, `Date64`, all timestamp units with their declared timezone, and `Decimal128`/`Decimal256` with their declared precision and scale. Grouping and lookup use the same typed tuple equality and hashing. Typed nulls form ordinary groups.

Preparation rejects other key types, including floating-point, nested, dictionary, fixed-size binary, time-of-day, duration, interval, and 32/64-bit decimal types. Keys have no implicit casts and are not serialized display strings. A local `PartitionKey` is portable between compatible key schemas. A `ScopeInstance` includes graph and definition identities along its full path.

Prepared physical reuse, grouped execution, explicit facet domains, ancestor-specific defaults, and requests restricted to selected instance addresses are future work. The current scoped API is a correctness baseline and does not promise interactive latency for hundreds of facets.

### Evaluation semantics

`query(tables, scalars, inputs)` evaluates both output sets in one execution. Either slice can be empty. Duplicate output handles do not duplicate execution. Two empty slices execute no graph nodes. Output handles are small `Copy` values, and result access remains typed.

The evaluator visits demanded nodes in dependency order, materializes each value, and supplies those values to downstream nodes. All consumers of a shared table read the same completed snapshot. Consumers can project different columns without changing their producer. This baseline preserves full table schemas at node boundaries.

Named prerequisites are strict. If a requested node references another named expression, that prerequisite runs even when the consuming expression puts its reference inside a false condition. Its failure fails the query. Conditional expressions and supported correlated subqueries inside one submitted section keep DataFusion's semantics. Standalone expression nodes cannot contain unresolved row references, bare aggregates, or window functions.

Scalar subqueries return a typed null for zero rows, a scalar for one row, and an error for more than one row. A private compatibility projection preserves nullable scalar-subquery fields with DataFusion 54.1, including when the underlying table field is non-nullable.

Function signatures determine volatility. `Stable` and `Volatile` dependencies propagate evaluation-local scope through the graph. A named volatile scalar produces one value per defining instance per query, shared by its consumers. Root values are shared across descendants, and regional values are shared by that region's nested instances. Separate named volatile nodes evaluate independently. Volatile expressions in table plans retain their row-wise semantics. Every node sees the same captured query start time, so `now()` agrees across independently planned regions.

The runtime returns results only after all requested outputs succeed. Errors retain their source and node context, with complete instance addresses for scoped failures. Dropping a query future releases its work and reservations. Concurrent queries have separate inputs, values, and planning state. Cross-query work coalescing and its cancellation protocol are later phases.

### Preparation and diagnostics

`prepare()` validates and analyzes all declared output paths. `PreparedDataflow` retains definitions, dependency metadata, and analyzed plans. It does not retain input snapshots or compiled physical plans.

`prepared.explain()` reports reachable nodes, definition paths, partition key schemas, ancestor captures, dependencies, external inputs, volatility, and reuse scope. `result.report()` reports executed nodes, physical planning count, captured query time, charged materialization, and per-definition instance, execution, and partitioned-row counts. Routine reports omit data-derived key values. `Reusable` marks nodes eligible for completed-result retention. Reports also expose cache hits, misses, bypasses, source executions, and retained byte charges.

`Runtime::with_session_state()` captures DataFusion configuration and function registries and installs the crate's snapshot planner. Native relational plans, values, and supported subqueries execute through DataFusion. Finite stable external scans are supported. Providers that hide a logical program are rejected; register that program directly so its dependencies and volatility can be analyzed. Session variables, DDL, DML, control statements, recursive queries, and unknown logical extensions are rejected. Known unbounded physical sources fail before execution.

### Resource limits

`RuntimeConfig` exposes `ExecutionConfig`: maximum active queries and an aggregate byte budget for materialized values owned by active queries. The defaults are four queries and 256 MiB. Regions and scope instances within one query execute sequentially. All frames share one query permit, captured timestamp, and materialization budget.

The byte budget conservatively charges Arrow array allocations, scalar copies, partition indices, instance addresses, and frame/result metadata, potentially counting shared buffers more than once. Charges accumulate until the query ends, including temporary partition buffers already released. This can reject a query below the budget in actual live memory. A query that cannot retain its required values fails with `ResourceExhausted`. This budget does not bound caller-owned inputs, returned results, or DataFusion operator memory. Operator memory uses DataFusion's runtime environment. Fixed graph assets are also excluded from active materialization charges. Retained results have a separate bounded cache budget.

## Prepared extensions

Construct additional computations on demand with `DataflowBuilder::with_base(&base.interface())`. The resulting definition is an ordinary `Dataflow`. `base.prepare_extension(&additional_dataflow)` returns a reusable `PreparedExtension` attached to that exact base preparation. It preserves the base's analyzed plans and cache identity.

| Prepared value | Execution method | Binding builder |
|---|---|---|
| `PreparedDataflow` | `query(tables, scalars, inputs)` | `base.inputs()` binds the base declarations |
| `PreparedExtension` | `query(tables, scalars, base_inputs, additional_inputs)` | `extension.inputs()` binds the additional declarations |

The following example assumes `source` exposes an Int64 column named `value`. It prepares once and queries two different selections:

```rust
use avenger_datafusion_dataflow::{
    arrow::datatypes::DataType,
    datafusion::logical_expr::{col, lit, LogicalPlanBuilder},
    DataflowBuilder, DataflowResult, Inputs, PreparedDataflow, Result, TableOutput,
};

async fn query_selections(
    base: &PreparedDataflow,
    source: TableOutput,
    base_inputs: &Inputs,
) -> Result<Vec<DataflowResult>> {
    let mut additional = DataflowBuilder::with_base(&base.interface());
    let rows = additional.import_table("source", &source)?;
    let selection = additional.expr_input("selection", DataType::Boolean)?;
    let filtered = additional.add_plan(
        "filtered",
        LogicalPlanBuilder::from(rows.plan_ref())
            .filter(selection.expr_ref())?
            .build()?,
    )?;
    let output = additional.table_output("rows", &filtered)?;
    let extension = base.prepare_extension(&additional.finish()?).await?;
    let mut results = Vec::new();
    for cutoff in [10_i64, 20_i64] {
        let additional_inputs = extension.inputs()
            .expr(&selection, col("value").gt(lit(cutoff)))?
            .finish()?;
        results.push(extension.query(&[output], &[], base_inputs, &additional_inputs).await?);
    }
    Ok(results)
}
```

Retain the `PreparedExtension` across interactions and edit its immutable bindings for subsequent queries. Repeated calls to `prepare_extension` create independent additional cache namespaces. Clones share the same preparation. Ordinary base queries need no extension or empty additional definition.

### Imports and bindings

`import_table` and `import_scalar` accept published root outputs and return ordinary `PlanNode` and `ScalarNode` handles. Imports share the original producer's materialized value and perform no wrapper execution. Re-export an import to request it alongside additional outputs. `Dataflow::num_nodes()` counts local computations, excluding import aliases. The additional interface exposes its own declared inputs and outputs.

Use existing base root input handles directly in additional plans and expressions. They read `base_inputs`. Additional scalar, table, and expression inputs read `additional_inputs`. Both binding sets retain their own ownership and completeness checks, including when input names match. An empty additional binding set comes from `extension.inputs().finish()?`. Existing base expression bindings are validated against their additional usage schemas before data execution without changing the original binding.

Only the associated base's root inputs and published root outputs can cross this boundary. Internal base computation handles remain private to their definition. Additional definitions can create nested scopes over imported root tables and use ordinary scoped defaults and overrides in `additional_inputs`. Importing an existing base facet instance requires a future scope-mapping API.

### Shared evaluation and retention

A composed query has one evaluation ID, query start time, execution permit, and active-memory reservation. Shared base computations run at most once within that evaluation. Stable and Volatile dependencies remain evaluation-local across both programs. Preparation and execution reports distinguish base and additional work, and `explain().imports` identifies each imported producer.

Resident base results serve ordinary queries and every extension attached to that preparation. Additional cache keys include their transitive base and local bindings. An additional result hit can skip base execution entirely. `base.clear_results()` also removes dependent additional entries and prevents in-flight requests from publishing results derived under the old clear epoch. `extension.clear_results()` removes only that extension's entries. Ordinary eviction of an upstream value does not invalidate retained downstream results.

The extension keeps its base alive. Dropping the last extension clone releases its additional cache namespace without clearing other users' base results. Both programs share the runtime's LRU budget and execution limits. Concurrent cold queries can duplicate work. Query misses still optimize and physically plan named computations.

### Generated pre-aggregation example

```sh
cargo run -p avenger-datafusion-dataflow --example additional_dataflow
```

The [flight example](examples/additional_dataflow.rs) warms a CSV source in the base, then constructs a delay-to-carrier pre-aggregation with a local active predicate. Changing the delay interval reuses the pre-aggregation, changing fixed filters recomputes it, and returning to a previous interval can reuse the final result. The example compares every result with a direct aggregate and prints actual execution reports. It groups by exact delay values and does not implement selection-aware binning or aggregate rewriting.

Extensions currently use native Rust construction against one standalone base. A protobuf- or JSON-loaded base can supply its interface. Serializing an additional definition returns an explicit external-import error. Portable import manifests, JSON extension requests, multiple bases, and extensions of extensions remain deferred.

## Fixed sources and caching

Use `table_snapshot(name, snapshot)` for immutable graph-owned data. Use `table_input()` for a table that changes between requests. A finite external provider in `add_plan()` is fixed for the preparation lifetime: if its files or database contents change, create a new preparation or call `clear_results()` after coordinating the source change. The runtime does not poll files or hash source contents.

Give an expensive source its own named node before parameter-dependent transforms. Retention happens at named boundaries with complete schemas. A scan buried inside a filter node is only reused when that whole node's bindings match.

The default runtime LRU retains at most 128 MiB and 1,024 entries across all its preparations. Configure `CachePolicy::Lru(CacheConfig { max_bytes, max_entries })` or use `CachePolicy::Disabled` for the uncached baseline. Oversized results bypass retention. Charges include retained Arrow allocations and key metadata, conservatively counting shared buffers more than once.

Cache keys include the preparation namespace, node, complete scope address, relevant scalar values, input snapshot identities, and structural expression bindings. Irrelevant inputs do not invalidate a result. Clones of a prepared value share its namespace; separate preparations do not. Scoped overrides invalidate affected values while siblings can hit. Discovery tables can hit, but grouping indices and gathering local rows still run each query.

`Stable`/`Volatile` functions and their descendants remain query-local. Caches hold only complete successful node results. A failed query can leave successful prerequisites cached. Concurrent misses run independently. `clear_results()` removes one namespace and advances its epoch, preventing active older requests from repopulating it. Dropping the last prepared clone releases its entries. Caller-held results remain valid after eviction or clearing.

Cache hits count against active materialization limits. `runtime.cache_stats()` reports runtime-wide retention. The active and retained budgets bound different owners; neither bounds all process memory.

## Binary serialization

```rust,ignore
let bytes = dataflow.to_bytes()?;
let loaded = destination_runtime.decode_dataflow(&bytes)?;
let names = loaded.interface().root();
let marks = names.table_output("marks")?;
let prepared = destination_runtime.prepare(&loaded).await?;
```

The version 1 protobuf envelope imports DataFusion **54.1.0** logical-plan and expression messages. It includes declarations, scopes, outputs, semantic requirements, and Arrow IPC streams for fixed assets. A narrow side table covers IN/EXISTS subqueries, outer references, and set comparisons absent from DataFusion's expression format. Generated DataFusion types are reused through Prost `extern_path`. Vendored upstream `.proto` imports and a vendored build-time `protoc` keep code generation reproducible.

Decoding resolves functions immediately and produces the same native `Dataflow` used by Rust builders. Loading allocates fresh graph and snapshot identities; recover typed handles through `interface().root()`, `.scope(name)`, and the input/output lookup methods. Interfaces and immutable bindings do not retain source lineage or graph-owned assets.

Artifacts contain no request bindings, warm result cache, execution frames, or physical plans. Shared assets are encoded once by snapshot identity. Stable and volatile expressions remain live. External scans cannot be exported in this phase; provide fixed snapshots. Baking and client/server splitting remain future work.

`SemanticConfig` records the required time zone and application function versions. The destination must match them. Built-ins require the pinned DataFusion version. Custom functions need explicit `kind:name` version entries (`scalar:my_udf`, `aggregate:...`, `window:...`, `higher_order:...`) in both the definition and `RuntimeConfig`. Register matching functions before decoding, or supply configured-function payload codecs through `to_bytes_with_codec()` and `Runtime::with_session_state_and_codec()`. Codecs do not transport arbitrary Rust code. Higher-order functions and lambdas currently fail export because the pinned DataFusion codec cannot encode them. The decoder checks declared function volatility as well as version requirements.

```sh
cargo run --release -p avenger-datafusion-dataflow --example serialized_dataflow
```

## JSON definitions and requests

Enable the `json` feature to load `DataflowSpec` and submit `QueryRequest`. The SQL parser and file-format adapters are optional; the native/protobuf API does not require this feature.

```rust,ignore
let spec: DataflowSpec = serde_json::from_str(definition_json)?;
let dataflow = runtime.load_spec(&spec, &FileSourceResolver::new(base_directory)).await?;
let prepared = runtime.prepare(&dataflow).await?;
let request: QueryRequest = serde_json::from_str(request_json)?;
let result = prepared.query_request(&request, &AssetBindings::new()).await?;
```

See the [definition fixture](tests/fixtures/facets.dataflow.json), [request fixture](tests/fixtures/facets.query.json), and [runnable example](examples/json_dataflow.rs). The `dataflow_spec_schema()` and `query_request_schema()` functions publish the matching [JSON Schemas](schemas/), regenerated with the `json_schemas` example. Version 1 requests use the adapter's grammar without a separate version field.

Each scope has `inputs`, `sources`, SQL-string `tables` and `scalars`, public `outputs`, and child `scopes`. A child declares a visible `partition.source`, ordered SQL `partition.keys`, and a local `rows` relation name. SQL names resolve lexically; local symbols shadow ancestors. Local inputs, sources, computations, and the rows alias share one namespace in JSON. SQL CTEs and aliases use DataFusion's normal rules. Computations can reference later definitions; unresolved or cyclic dependencies fail loading. Outputs are aliases and do not add SQL symbols. Only query SQL is accepted, and all relations must be declared.

Inputs have no declaration defaults. Requests combine root scalar/table/expression bindings, `scope_defaults`, full-path instance `overrides`, and table/scalar output selections. All root inputs are required; scoped inputs are required only where demanded. Each request is independent. A null or empty table is an explicit binding. Overrides for absent facets do not create facets. Results keep the native nested materialized shape.

Declare JSON expression inputs as `{"kind": "expr", "type": "boolean"}` and reference them with `$name` in definition SQL. Supply SQL expression strings in the request's `bindings.exprs` map. The same `exprs` map is available in scope defaults and instance overrides. Binding strings are single expressions with the native binding restrictions and cannot refer to other parameters. An unused input may contain unresolved column references, but still rejects unsupported or volatile expressions.

The [expression definition](tests/fixtures/expressions.dataflow.json) and [request](tests/fixtures/expressions.query.json) run in the JSON example, including a protobuf round trip and input discovery. Artifacts retain expression declarations and typed references, while requests supply the actual expressions.


### Sources and values

* Inline sources require a schema and `values`, an array of row objects. Schema order determines columns. Missing nullable values become null; unknown fields, missing non-nullable values, and incompatible types fail.
* File sources use `url`, `format` (`csv`, `parquet`, or `arrow`), optional `schema`, and `options`. CSV options are `has_header` (default true), a one-byte ASCII `delimiter` (default comma; use `\t` for TSV), and positive `schema_infer_max_records` (default 1,000). Arrow files use the IPC file format.
* Parsing performs no I/O. With an explicit schema, loading and preparation do not open the file. Without one, `SourceResolver::infer_schema()` runs once during loading. Actual reads occur on demanded misses. The built-in resolver handles local paths relative to its base directory; use a trailing separator for directory paths. Custom resolvers can return other deferred providers.
* Fixed `asset` sources resolve host-supplied `TableSnapshot` values. Repeated references share identity within a load. Request assets use `AssetBindings`; table bindings can also provide inline `values` using the input's declared schema.
* Supported JSON types are `null`, `boolean`, signed/unsigned 8–64-bit integers, `float32`, `float64`, `utf8`, `large_utf8`, `date32`, `date64`, and `{"timestamp":{"unit":"s|ms|us|ns","timezone":null}}`. Dates use `YYYY-MM-DD`. Timestamps accept integer epoch units or RFC 3339 strings. Integer decimal strings preserve 64-bit precision through JavaScript clients. Null is typed by the declaration. Nested, binary, decimal, interval, and other Arrow values remain available through the native/IPC API.

Inline request tables allocate new snapshot identities on every submission; host assets preserve identity. Reuse typed `Inputs` when that is preferable. JSON result serialization, SQL export, HTTP transport, and text round-trip fidelity are outside this adapter.

```sh
cargo run --release -p avenger-datafusion-dataflow --features json --example json_dataflow
```

## Validation

```sh
cargo test --release -p avenger-datafusion-dataflow --locked
cargo test --release -p avenger-datafusion-dataflow --features json --locked
cargo clippy -p avenger-datafusion-dataflow --features json --all-targets --locked -- -D warnings
```

The tests cover graph and scope ownership, immutable bindings, nested and composite discovery, supported key types, native DataFusion result comparisons, scalar-subquery cardinality, strict dependencies, per-instance volatile sharing, stable query time, result lookup, cancellation, and concurrent input isolation.
