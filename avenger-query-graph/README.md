# avenger-query-graph

Caches intermediate results across parameterized DataFusion queries. Compile the unoptimized plans of a document, such as the charts of a dashboard, into one graph, then run it with values for its parameters and selections. Each run reuses the cached results whose dependencies did not change, and computes the rest.

```rust
use std::collections::BTreeMap;

use avenger_query_graph::{Inputs, Outcome, ResultStore, SourceOptions, StoreConfig};
use avenger_selection::SelectionState;
use datafusion::{common::ParamValues, prelude::SessionContext, scalar::ScalarValue};

#[tokio::main(flavor = "current_thread")]
async fn main() -> datafusion::common::Result<()> {
    let mut ctx = SessionContext::new();
    avenger_selection_datafusion::register(&mut ctx)?;
    ctx.sql(
        "CREATE TABLE flights AS SELECT * FROM (VALUES \
         ('AA', 2023, 5.0), ('DL', 2023, 40.0), ('AA', 2024, 12.0), ('UA', 2023, -3.0)) \
         AS v(carrier, year, delay)",
    )
    .await?
    .collect()
    .await?;

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
    // Another year reruns the year filter and the count over the cached scan.
    graph.run(&inputs(2024)).await?;
    // The first year again is a cache hit for the count, which reads nothing below it.
    let again = graph.run(&inputs(2023)).await?;
    assert!(matches!(again.outcomes.last(), Some(Outcome::Hit(_))));
    Ok(())
}
```

## Contract

- Compile **unoptimized plans** with **unbound placeholders**, such as `DataFrame::into_unoptimized_plan` gives. A selection's state is a placeholder in a `selected` call of avenger-selection-datafusion.
- `Run::batches[i]` holds the result of the `i`th plan that `compile` took.
- A run spawns its computations, so it must run inside a Tokio runtime.
- `StoreConfig::budget` and `StoreConfig::max_result` bound the bytes of cached results, not the memory of the computations in progress.
- `SourceOptions::materialize` applies to the graphs that the store compiles after it changes. `SourceOptions::ttl` applies at the next run of every graph.

## How results are reused

The graph cuts each plan into nodes where its dependencies change (see [Where a plan is cut](#where-a-plan-is-cut)). A node's cache key holds its subplan, the providers of the tables that it scans and their versions, the values of the parameters that it reads, the selection clauses that it reads, and the session's settings. A node reads only the clauses of the owners that its `selected` calls keep, as `avenger_selection_datafusion::selection_reads` reports them. Moving a brush therefore keeps the cached results of the nodes that skip that brush's owner, such as the brushed plot's own cross filter.

A node caches only the columns that the nodes above it read. A run learns them from the bound, optimized plans of those nodes. When a cached result lacks a column that a new reader needs, the node is computed again with both sets of columns.

The store keeps cached results within `budget` and evicts the least recently used. A result larger than `max_result` or than `budget` is used by its run and not cached. Identical cacheable subplans that are cut into nodes, such as a CTE with two references, share one result.

## Sources and Task integration

`ResultStore::set_source_options` sets two options for a table:

- `materialize`: each scan of the table is a node, so later runs can reuse the scan's cached result instead of reading the table again. This suits files that a scan parses on every read, such as CSV.
- `ttl`: the minimum interval between checks of the source. When several names of one provider set one, the shortest applies. A listing table gets a new version when the paths, sizes, modification times, or entity tags of its files change, and at its first check. A table of another kind gets a new version at each check. The default is `ttl: None`, and a provider is unchecked only when none of its known names sets a time to live.

A run checks each expired source that it reads before it computes any key. `ResultStore::revalidate` checks every expired source, and returns whether any source has a new version. `ResultStore::table_changed` gives a table a new version after a known change inside it, such as an `INSERT INTO` a `MemTable`. Each provider has one version and its own checks, so two sessions can register different tables under one name, and a change through any name of a provider reaches every graph that reads it.

An app runs the graph in an avenger-app `Task` whose request key is `QueryGraph::key`. The key identifies the inputs that the plans read and the focus, so the task ignores a gesture that changes nothing that the plans read. `key` checks no source, so a host that reloads on an interval calls `revalidate` and requests a run when it returns true.

## Concurrency and cancellation

Independent nodes, such as the branches of a `UNION ALL`, compute at the same time. Concurrent runs with the same node key share its computation when it computes the columns that they need, and a run that needs another column starts a wider one. A computation that fails or panics leaves the store, so the next run computes the node again. A computation holds the computations whose results it reads. When a newer request replaces a run, the computations that nothing else waits for continue until the next run of the same graph has computed its keys. That run takes over each computation that it needs, and stops the others.

## Pre-aggregation

A run's inputs can name a `Focus`: the selection that is about to move. The run then prepares the plans with avenger-selection-datafusion's `preaggregate::prepare`, so each eligible aggregate that the selection filters becomes a rollup over a cells subplan, which does not read the moving clause. Which aggregates are eligible is described in [the selection crate's README](../avenger-selection-datafusion/README.md#which-plans-qualify). When the cells are a node of their own, a run with a focus computes them even when its rollups are cache hits, and a move recomputes the rollups over the cached cells while the store keeps them. Cells inside a subquery expression are not a node of their own, so a move recomputes them.

The focus changes how a run computes its results while it preserves the query's semantics. A node that holds a rollup has the cache key of its counterpart in the direct plans, so a result that one focus computed serves a run with another focus or without one. When `Focus::serves` rejects the selection's state, such as a brush of another grid, the run uses the direct plans. Floating-point aggregates of a rollup can differ from the direct plan's in their rounding.

## Inspecting a graph

`QueryGraph::nodes` returns the nodes of the direct plans, or of the variant for a focus. Each node has its plan, with the scan of each node that it reads named `input_<position>`, the inputs that its own operators read, and the nodes that it reads. `Run::outcomes` tells, for each node, whether the run hit its cached result, computed it, computed it without caching it, or did not read it, with its rows, bytes, columns, and time. `QueryGraph::to_dot` and `Run::to_dot` write the same as Graphviz DOT graphs.

## Where a plan is cut

A cut goes below each operator that reads an input that the operators below it do not read: a parameter, the clauses of a selection's owners, or a call of a volatile or stable function, such as `random()` or `now()`. Everything below the cut is one node whose result serves every value of that input. A filter's conjunction is first split into a stack of filters, ordered by the number of inputs that each conjunct reads, so `WHERE year = $year AND selected($brush)` makes the year filter a node, and a brush move reruns only the node above it.

Some subplans are never cut:

- A subplan that calls a volatile or stable function stays with the operator that reads it, and its node is computed on every run.
- Nothing inside a subquery expression or a recursive query is cut.
- A table scan is a node of its own only when its source's scans are materialized.

Each node is bound and optimized apart from the others, so a filter that reads an input never moves through a cut. A cut that the optimizer removes, such as one below a filter that is false, does not run. A cached result keeps the order of its rows, so a sort below a cut and a limit above it select the same rows as the plan without the cut.
