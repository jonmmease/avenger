# avenger-streaming

Streaming queries over the events of a chart. A query over the built-in `events` table runs frame by frame: each frame's events pass through the query's stages once, the rows that the stages keep update the query's front, and the rest of the query runs as a batch plan over the front's output, only on frames where that output changed. After every frame, the result equals the batch plan of the whole query over every event so far.

`avenger_sql::streaming::split` splits a query into its stages, its front, and its downstream:

- The stages are the stateless operators that read `events`: `WHERE`, `SELECT`, `EXTEND`, `SET`, `DROP` and `RENAME`, without subqueries or functions whose value changes between calls, such as `CURRENT_TIMESTAMP`.
- The front is an `AGGREGATE`, grouped or ungrouped, or an `ORDER BY ... LIMIT` whose last key is `seq`. An `AGGREGATE` keeps DataFusion's accumulators of each group, and an `ORDER BY ... LIMIT` keeps the first rows of its order. Neither needs a retraction, since `events` only grows.
- The downstream is the rest of the query, which reads the front's output.

A query of another shape is a `Rejection` that names the operator, and its caller runs the batch plan over the stored events.

```rust
let split = avenger_sql::streaming::split(&plan, &events_table)?;
let mut query = StreamingQuery::new(ctx, split, None).await?;
if query.push(frame_events).await? {
    let rows = query.result();
}
```

`set_params` binds the query's parameters. A parameter that the stages or the front read rebuilds the front from the events so far, which the query stores. A parameter that only the downstream reads runs the downstream again.

A known difference: DataFusion's grouped `MIN` and `MAX` over `NaN` differ from its ungrouped ones, and the streaming front keeps the ungrouped accumulators, so a grouped `MIN` or `MAX` over `NaN` differs from the batch plan.

## Tests

`tests/equivalence.rs` runs a corpus of queries over seeded sessions of pointer events, with `NULL`, `NaN`, the infinities and zeros of either sign, split into frames at random, and compares the streaming result with the batch plan's after every frame. The ignored test `tests/benchmark.rs` times both at sessions of 1,000 to 100,000 events and writes its table to `tests/output/benchmark.md`:

```bash
cargo test --release -p avenger-streaming --test benchmark -- --ignored --nocapture
```
