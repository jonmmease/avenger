# avenger-query-graph

Keeps the results of the subplans of DataFusion queries, so that a change of one input reruns only the parts of a query that read it. The queries are those of one document, such as the charts of a dashboard, whose parameters and selection states are placeholders, such as `$year` and `$brush`. A `ResultStore` compiles them into one `QueryGraph`. Each run of the graph takes the inputs' values, reuses each kept result whose inputs did not change, and computes the rest.

## Where a plan is cut

The graph cuts each plan into nodes. A cut goes below each operator that reads an input that the operators below it do not read, such as a parameter, the clauses of a selection's owners, or a call of a volatile or stable function. Everything below the cut is one node whose result serves every value of that input. A filter's conjunction is first split into a stack of filters, so a conjunct that reads fewer inputs runs below one that reads more. In `WHERE year = $year AND selected($brush)`, the year filter is a node, and a brush move reruns only the node above it.

Some subplans are never cut. A subplan that calls a volatile or stable function, such as `random()` or `now()`, stays with the operator that reads it, and its node is computed on every run. Nothing inside a subquery expression is cut. A table scan is a node of its own only when its source's scans are materialized (see [Sources](#sources)). Equal subplans, such as a CTE with two references or equal plans of two charts, are one node.

Each node is optimized apart from the others, after its parameters and selections are bound, so a filter that reads an input never moves through a cut.

## Keys and kept results

A node's key holds its subplan, the values of the parameters that it reads, the fingerprints of the clauses of the owners that it reads, the version of each table that it scans, and the session's settings. The key holds only the owners that the node reads, as `avenger_selection_datafusion::selection_reads` reports them, so a cross filter's other plots keep their results while one plot is brushed.

A node keeps only the columns that the nodes above it read. A run learns them from the bound plans of the nodes above, so a `selected` call's columns are known once its state is bound. When a kept result lacks a column that a new reader needs, the node is computed again with both sets of columns.

The store keeps results within `StoreConfig::budget` and evicts the least recently used. A result larger than `StoreConfig::max_result` is used by its run and not kept.

## Sources

`ResultStore::set_source_options` sets two options for a table:

- `materialize`: each scan of the table is a node, so the table is read once. This suits files that a scan parses again on every read, such as CSV. It applies to the graphs that the store compiles afterwards.
- `ttl`: the time after a check of the source at which a run checks it again. A listing table gets a new version when the paths, sizes, modification times, or entity tags of its files change. A table of another kind gets a new version at each check.

A run checks each expired source that it reads before it computes any key. `ResultStore::revalidate` checks every expired source, and it returns whether any source has a new version, so a host can request a new run on its own interval. `ResultStore::table_changed` gives a table a new version, for a change inside the table, such as an `INSERT INTO` a `MemTable`.

## Runs

A run resolves the nodes from the top down. A node whose key has a kept result reads nothing below it. A node that misses is computed as soon as the nodes below it have results, so independent nodes, such as the branches of a `UNION ALL`, compute at the same time. Every node of a run gets the run's start time, so `now()` has one value in the run.

Concurrent runs that miss on one key share one computation. A run that needs a column that the computation does not compute starts a wider one. A computation that no run waits for, such as one of a run that a newer request replaced, continues until the next run of the same graph has computed its keys. That run takes over each such computation that it needs and stops the others.

The store does not replace runs. An app runs the graph in an avenger-app `Task`, whose request key is `QueryGraph::key`. The key holds the keys of the plans' top nodes and the focus, so the task ignores a gesture that changes nothing that the plans read.

## Pre-aggregation

A run's inputs can name a `Focus`: the selection that is about to move, as avenger-selection-datafusion's pre-aggregation defines it. The run then prepares the graph's plans with `preaggregate::prepare`, so each aggregate that the selection filters becomes a rollup over a cells subplan. The cells do not read the moving clause, so a move reruns only the rollups. A run with a focus computes the cells even when its rollups hit, so the first move after the pointer enters a plot reruns only the rollups.

The focus never changes a result. A node that holds a rollup has the identity and the key of its counterpart in the direct plans, so a result that one focus computed serves a run with another focus or without one. When `Focus::serves` rejects the selection's state, such as a brush of another grid, the run uses the direct plans. A rollup's floating-point aggregates can differ from the direct plan's in their rounding.

## Inspecting a graph

`QueryGraph::nodes` returns the nodes of the direct plans or of the variant of a focus: each node's plan, with each cut as a scan named `input_<position>`, the inputs that its own operators read, and the nodes that it reads. `Run::outcomes` tells, for each node, whether the run hit its result, computed it, computed it without keeping it, or did not read it, with its rows, bytes, columns, and time. `QueryGraph::to_dot` and `Run::to_dot` write the same as Graphviz DOT graphs.

## Example

```rust
use std::{collections::BTreeMap, sync::Arc};

use avenger_query_graph::{Inputs, Outcome, ResultStore, SourceOptions, StoreConfig};
use avenger_selection::SelectionState;
use datafusion::{
    arrow::{
        array::{Float64Array, Int64Array, RecordBatch, StringArray},
        datatypes::{DataType, Field, Schema},
    },
    common::ParamValues,
    prelude::SessionContext,
    scalar::ScalarValue,
};

# #[tokio::main(flavor = "current_thread")]
# async fn main() -> datafusion::common::Result<()> {
let mut ctx = SessionContext::new();
avenger_selection_datafusion::register(&mut ctx)?;
let schema = Arc::new(Schema::new(vec![
    Field::new("carrier", DataType::Utf8, false),
    Field::new("year", DataType::Int64, false),
    Field::new("delay", DataType::Float64, false),
]));
let flights = RecordBatch::try_new(
    schema,
    vec![
        Arc::new(StringArray::from(vec!["AA", "DL", "AA", "UA"])),
        Arc::new(Int64Array::from(vec![2023, 2023, 2024, 2023])),
        Arc::new(Float64Array::from(vec![5.0, 40.0, 12.0, -3.0])),
    ],
)?;
ctx.register_batch("flights", flights)?;

let store = ResultStore::new(StoreConfig {
    budget: 512 << 20,
    max_result: 64 << 20,
});
store.set_source_options(
    "flights",
    SourceOptions {
        materialize: true,
        ttl: None,
    },
);
let sql = "SELECT carrier, count(*) AS n FROM flights \
           WHERE year = $year AND selected($brush) GROUP BY carrier";
let plan = ctx.sql(sql).await?.into_unoptimized_plan();
let graph = store.compile(&ctx.state(), vec![plan])?;

let inputs = |year: i64| Inputs {
    params: ParamValues::from(vec![("year", ScalarValue::from(year))]),
    selections: BTreeMap::from([("brush".to_string(), SelectionState::default())]),
    focus: None,
};
// The first run computes the scan, the year filter, and the count.
let first = graph.run(&inputs(2023)).await?;
assert_eq!(first.batches[0].iter().map(|batch| batch.num_rows()).sum::<usize>(), 3);
assert!(first.outcomes.iter().all(|outcome| matches!(outcome, Outcome::Computed { .. })));
// Another year reruns the year filter and the count over the kept scan.
let other = graph.run(&inputs(2024)).await?;
assert!(matches!(other.outcomes[0], Outcome::Hit(_)));
// The first year again hits the count and reads nothing below it.
let again = graph.run(&inputs(2023)).await?;
assert!(matches!(again.outcomes[2], Outcome::Hit(_)));
assert!(matches!(again.outcomes[0], Outcome::NotRead));
# Ok(())
# }
```
