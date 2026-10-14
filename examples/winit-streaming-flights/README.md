# Streaming Mosaic flights

A separate native `avenger-app` example for streaming ingestion and asynchronous cross-filtering. It appends Arrow record batches through `TableStore::append_batch`, then queries complete immutable snapshots. It keeps three linked histograms and a carrier summary responsive by reading cached pre-aggregations while the dataflow scheduler warms newer snapshots.

This is the performance baseline before incremental calculation. Each newly warmed snapshot recomputes its materializations over the complete captured input. Pre-aggregation and ordinary result caching are enabled. No append lineage, delta planning, or incremental aggregate maintenance is implemented. The existing `winit-mosaic-flights` example remains independently runnable.

## Run

Use the prepared BTS 2016–2019 dataset outside the repository:

```sh
cargo run --release -p winit-streaming-flights -- \
  --data "$HOME/datasets/avenger-bts-2016-2019/sample-10000000" \
  --batch-rows 100000 --interval-ms 1000
```

Use the `full` directory for the 25,457,251-row dataset. A single Parquet file is also accepted. Required fields are `arr_delay: Int32`, `distance: Int32`, `scheduled_minute: UInt16`, and `carrier: Utf8`, with no null values. Extra fields are ignored. This is the normalized BTS schema, which differs from the original Mosaic file's uppercase columns.

Files replay in filename order. Monthly filenames therefore give month ordering, without implying chronological ordering within a file. One batch contains at most `--batch-rows` rows, and file boundaries can produce shorter batches. Only one batch is decoded ahead of publication. The decoder opens each requested file range by an explicit row offset, which keeps restart and canceled decoding independent of mutable reader state.

The default development dataset contains 10 million rows. `--max-batches N` stops after a bounded prefix. `--paused` opens the application before ingestion starts. `--diagnostics` prints foreground timing, cache hits, executed nodes, and target availability observations to stderr.

## Interact

- Hover over a histogram to select the active cross-filter context. Drag horizontally to brush. Each histogram excludes its own brush, while the carrier summary applies all three.
- Click without dragging or right-click to clear a brush. Escape clears all brushes.
- Space plays or pauses ingestion. An already accepted append finishes when paused.
- N pauses and appends one batch when no append is pending.
- R restarts from an empty store and the first file, preserving the current brushes and play/pause state.
- `+` speeds up ingestion and `-` slows it down. The interval is a pacing target, subject to decoding and the application's 50 ms observation timer.
- W retries warming the newest snapshot if target availability stops advancing. Background execution errors are available through tracing, for example with `RUST_LOG=warn`.

The header shows ingested rows and rows represented by the displayed results. They can differ while warming is running. On a cache miss, the last completed charts remain visible with a pending indication. Old completions cannot replace a newer foreground request.

Arrival delay is clipped only for histogram display. Carrier mean and sample standard deviation use the original delay in minutes. Scheduled departure time is converted from minutes since midnight to hours. Selection uses the same 1-pixel grid and 5 ms debounce as the non-streaming Mosaic example.

Histogram domains are fixed. Values outside a domain, such as distances above 5,000 miles, have no visible bin in that histogram. Those flights remain available to the other histograms and carrier summary, subject to the current brushes. A value exactly at the upper boundary belongs to the last bin.

## Query flow

The graph is prepared once over a native table input. Each focus has pre-aggregation state nodes for the other histograms and the carrier summary. The focused histogram is itself a cache target because its result is unchanged by its own brush. Any direct-query fallback is also a required target. Every expensive path is therefore included in background warming.

The application keeps the newest input, one captured warming candidate, and one observed compatible fallback. Appends update only the newest input while a useful candidate is warming. A timer probes that exact captured version with `CacheRead::CachedOnly`, so completed intermediate versions are not lost when ingestion is faster than computation. Once its targets are observed, it becomes a fallback and the newest pending snapshot is scheduled next. A successful foreground read can also observe the warming candidate and advance to the newest pending snapshot. The runtime owns the warming job and its single background slot.

Foreground reads use `CacheRead::FromCachedTargets`. Candidate overrides replace only the source snapshot and inherit the current brush expressions. A selected target is held for the duration of that read. Holding a source snapshot does not pin its derived cache entries for future reads. A miss never falls back to a raw foreground calculation.

Changing focus or a fixed brush replaces the warming context and discards incompatible fallbacks. Restart creates a new store and invalidates pending requests. Pausing does not cancel a committed append. Shutdown uses the existing application background-task lifecycle. Already admitted dataflow warming follows the scheduler's existing finish-on-drop policy.

DataFusion uses four target partitions. The dataflow runtime has four execution slots, with at most one warming job, a 2 GiB result cache, and a 4 GiB active-materialization budget. Background queries still compete for CPU with brush queries. This example makes that contention measurable and does not guarantee a fixed interaction latency. Those budgets do not bound process RSS or caller-held source data.

## Record a baseline

The headless mode emits one JSON object per append to stdout. It omits ingestion delays and rendering, warms the same target bundle, and changes the active delay brush through ten deterministic ranges while warming and another ten after target availability. The first batch has no older fallback, so its during-warming sample is empty. Every successful foreground sample checks that executed nodes are rollups only.

```sh
cargo run --release -p winit-streaming-flights -- \
  --data "$HOME/datasets/avenger-bts-2016-2019/sample-10000000" \
  --batch-rows 100000 --max-batches 20 --headless \
  > /tmp/streaming-flights-baseline.jsonl
```

The JSON includes input rows and batches, append time, cached target rows, time until targets are observed, individual brush durations, and summary percentiles. Append time measures snapshot publication after Parquet decoding. Target availability time includes binding, scheduler queueing, computation, and a 2 ms probe interval. It is not an isolated execution-time measurement. Foreground durations include binding, cache selection, query execution, and result decoding, but exclude UI debounce, scene construction, and presentation. Ten samples per phase are descriptive, not a statistically stable tail-latency estimate.

Use the same dataset, prefix, batch sizes, build profile, and machine when comparing a later incremental implementation. Workspace `release` uses optimization level 1. `--profile release-perf` uses level 3 and is available for longer performance runs. Save the JSON output outside the repository and record the code revision alongside it. The headless command fails after 120 seconds without target availability, rather than waiting indefinitely after a warming error or an unretainable target bundle.

## Code and checks

| File | Responsibility |
|---|---|
| [replay.rs](src/replay.rs) | Ordered bounded decoding and snapshot append publication |
| [dataflow.rs](src/dataflow.rs) | Graph construction, complete warming targets, sparse snapshot fallbacks, and result decoding |
| [controller.rs](src/controller.rs) | Candidate ownership, replay controls, context changes, brushing, and completion handling |
| [baseline.rs](src/baseline.rs) | Headless measurements during and after warming |
| [selection.rs](src/selection.rs), [layout.rs](src/layout.rs), [scene.rs](src/scene.rs) | Selection semantics and native chart rendering |

```sh
cargo test -p winit-streaming-flights
cargo check -p winit-streaming-flights -p winit-mosaic-flights
```

Tests use generated data and temporary Parquet files. They compare append and fallback results with complete direct queries, cover all focus choices and richer aggregates, preserve current brush values across older snapshot candidates, check empty results and cache misses, and verify that ingestion does not replace an unobserved warming candidate.
