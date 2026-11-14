# avenger-streaming

Queries over the events of a chart, which run frame by frame. Each query's result equals its batch plan over every event so far, after every frame.

An `EventLog` holds a chart's events in the order of arrival. It numbers them in the column `seq` from 1, so the order of the events is a column that queries read and that has no ties, and it is the table `events` of avenger-sql's catalog. The log merges the small batches that frames append into larger ones, since a plan reads many small batches slowly.

`EventQueries` holds the queries over one log. A query that `avenger_sql::streaming::split` splits runs incrementally: each frame's events pass through its stateless stages once, the rows that they keep update its front, and the rest of the query runs as a batch plan over the front's output, only on frames where that output changed. The fronts are an `AGGREGATE`, grouped or ungrouped, which keeps DataFusion's accumulators of each group, and an `ORDER BY ... LIMIT` whose last key is `seq`, which keeps the first rows of its order. Neither needs a retraction, since events only grow. Another query runs as its batch plan over the log on each frame with events, and `register` returns the reason.

```rust
let log = EventLog::new(pointer_event_schema)?;
let catalog = MemoryCatalog::new().with_table(&["events"], log.table());
let mut queries = EventQueries::new(ctx, log.clone());

let plan = avenger_sql::analyze(&syntax, &catalog, &library, &options).plan.unwrap();
if let Mode::Batch(rejection) = queries.register("brush", plan).await? {
    println!("brush runs as its batch plan: {rejection}");
}

// Each frame: the frame's events, without `seq`.
for id in queries.push(frame_events).await? {
    let rows = queries.result(&id).unwrap();
}
```

`set_params` sets parameter values by name. A query that reads a changed parameter in its stages or its front rebuilds the front from the log, and one that reads it only after the front runs the rest of its plan again. A query needs a value for each parameter that it reads before it registers. A query registered after events catches up on the log.

A known difference: DataFusion's grouped `MIN` and `MAX` over `NaN` differ from its ungrouped ones, and the streaming front keeps the ungrouped accumulators, so a grouped `MIN` or `MAX` over `NaN` differs from the batch plan.

## Tests

`tests/equivalence.rs` runs a corpus of queries over seeded sessions of pointer events, with `NULL`, `NaN`, the infinities and zeros of either sign, split into frames at random, and compares each result with the batch plan's after every frame. `tests/registry.rs` tests the log's numbering and merging, the batch plans of queries that don't split, a query registered after events, and the queries that a frame reports. The ignored test `tests/benchmark.rs` pushes each frame of a session, as a chart does, times each query as a streaming query and as its batch plan over the log at sessions of 1,000 to 100,000 events, and writes its table to `tests/output/benchmark.md`:

```bash
cargo test --release -p avenger-streaming --test benchmark -- --ignored --nocapture
```
