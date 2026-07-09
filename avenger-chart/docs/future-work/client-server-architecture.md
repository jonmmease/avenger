# Client/Server Data Execution For Wasm

## Status

Ready for design spike. The best first seam is not the renderer, the pan/zoom
event loop, or a special-case raster endpoint. It is a hybrid DataFusion
execution seam:

- the browser keeps a `PlotSession`, params/stores/selections, pan/zoom tools,
  scenegraph rendering, and stale-preview retargeting,
- the browser keeps enough DataFusion to build plans, compile plots, and run
  small local computations,
- remote source tables are represented in the browser as schema-known table
  providers,
- eligible logical plan fragments are pushed to a stateless backend,
- results return as Arrow IPC record batches plus Avenger metadata.

This should support heavy view-dependent materializations such as
`Rasterize2D`, but it should also support simpler synchronous queries such as
regular histograms, scalar aggregates, scale-domain queries, and sampled table
previews.

This note assumes the client may still include DataFusion in Wasm. Avoiding
DataFusion entirely in the browser is a separate, more disruptive target.

## Goal

Let Avenger run interactive charts in the browser while large source data and
expensive transforms stay on a server. For the Datashader-style workflow, the
server should perform everything up through rasterization and send only the
raster result needed for the current viewport. Pan and zoom should stay smooth
because interaction updates are local and never wait on backend execution.

The same architecture should work for non-raster examples:

- a histogram where bin aggregation runs on the server,
- a scalar aggregate used for a legend or annotation,
- a scale domain inferred from a large remote table,
- a client plot that joins a small local selection table with a large remote
  table when the join can be pushed down or partially pushed down.

## Current Architecture Fit

Several current pieces already line up with this direction:

- `CompiledPlot` is serializable and can be sent from a server to a client.
- `CompiledPlot::instantiate(ctx)` only needs a DataFusion `SessionContext`.
- `PlotSession` owns evaluation state, cached materializations, params, and
  redraw scheduling boundaries.
- `Rasterize2D` and `ScalarAggregate` already use the generic materialization
  executor registry.
- The async rasterized mark design already separates smooth interaction from
  deferred materialization readiness.

The missing part is durable remote data identity. Current serialized data frame
support can inline a `MemTable` or refer to a named `MemTable` in a local
session, but a browser/server protocol needs a stronger source reference:

```text
source_ref:
  endpoint: "https://data.example.com/avenger"
  catalog: "demo"
  table: "taxi_pickups"
  revision: "2026-07-05T12:00:00Z"
  schema_fingerprint: "..."
```

That source reference must survive serialization, must be authorized on the
server, and must encode enough identity for cache keys and stale-result checks.

## Recommended Seam

Use DataFusion logical plans as the primary compute request format, and use
Avenger materialization metadata to describe why the plan is being run.

The browser should not send a rendered scene request like "give me an image for
this chart" as the core API. It should send a plan fragment for the data product
needed by the chart:

- a `Rasterize2D` logical plan whose output schema is a raster struct row,
- a grouped aggregate plan for a histogram,
- a projection/filter/limit plan for a preview table,
- a scalar aggregate plan for a domain or statistic.

The backend executes the plan against real server-side table providers and
returns Arrow IPC. The browser then consumes the returned batches through the
same mark/materialization path it would use for local computation.

This keeps the renderer and interaction code local, while moving expensive
dataflow work behind DataFusion planning and Arrow transport.

## Two Authoring Modes

### Server-Authored Compiled Plot

The server constructs the plot, compiles it, and sends the client:

- serialized `CompiledPlot`,
- a remote data-source manifest,
- UDF/UDAF and transform registry versions,
- initial params and optional initial materialized results.

The client deserializes the plot, creates a Wasm `SessionContext`, registers
remote table providers from the manifest, instantiates a `PlotSession`, and
renders. When the plot session evaluates and needs data, the remote table
providers and optimizer push eligible plan fragments back to the server.

This mode is useful when the server owns the application definition, access
policy, or plot authoring environment.

### Client-Authored Plot From Catalog Schema

The server sends a catalog manifest first:

```text
tables:
  taxi_pickups:
    schema: Arrow schema
    source_ref: taxi_pickups@revision
    capabilities:
      - projection
      - filter
      - aggregate
      - rasterize2d
      - sort_limit
    statistics:
      row_count: approximate
      columns:
        pickup_x:
          min: ...
          max: ...
```

The client registers schema-known remote tables, constructs normal Avenger
plots with `ctx.table("taxi_pickups")`, compiles locally, and uses the same
remote execution seam during evaluation.

Schema is enough for construction and type checking. Richer authoring features
such as recommended encodings, min/max defaults, category lists, and sample
values need catalog statistics or explicit sample endpoints.

Both modes converge on the same runtime shape: a client `PlotSession` backed by
a local `SessionContext` whose remote tables can push computation to a server.

## Federated DataFusion Shape

The closest DataFusion ecosystem reference is
[datafusion-federation](https://github.com/datafusion-contrib/datafusion-federation).
It represents remote-capable tables with provider metadata, identifies the
largest subplans that a common remote engine can execute, and replaces those
subplans with an opaque federation node.

For Avenger, wrap this pattern behind Avenger-owned types rather than exposing
`datafusion-federation` directly:

```text
AvengerRemoteTableProvider
  - implements DataFusion table provider/source traits
  - exposes Arrow schema and stable source_ref
  - points at an AvengerRemoteProvider

AvengerRemoteProvider
  - groups sources by endpoint/catalog/auth/source revision
  - owns capability policy
  - supplies the optimizer and physical extension planner

RemoteLogicalPlanExec
  - implements DataFusion ExecutionPlan
  - serializes the inner logical plan and request metadata
  - POSTs to the backend over HTTP/fetch
  - decodes Arrow IPC into a RecordBatch stream
```

The browser should not serialize a federation node to the backend. The
federation node is a local execution placeholder. Its inner logical plan and
metadata are serialized into a backend request.

The server receives the inner logical plan, resolves source refs against its
real catalog, registers the same Avenger extension codec and UDF/UDAF names,
plans/executes in a server `SessionContext`, and streams Arrow IPC batches back.

### Dependency Stance

As checked on 2026-07-05, `datafusion-federation` is still labeled alpha, but
its current public design is a very close conceptual match. It is reasonable to
use it as a reference immediately and consider it as a dependency after the
next DataFusion update. If used as a dependency, keep it behind a small Avenger
wrapper crate so API churn does not leak into chart APIs.

Prefer a DataFusion-to-DataFusion logical-plan executor first. The generic SQL
unparser path is useful for external databases, but Avenger needs exact support
for extension nodes such as `Rasterize2D`, custom UDFs, and Avenger data types.

[Ballista](https://datafusion.apache.org/ballista/user-guide/rust.html) is a
heavier fit for the first browser/server target. It provides a remote
`SessionContext` and distributed execution, but it brings scheduler/executor
cluster semantics and gRPC/Tonic assumptions. A stateless HTTP + Arrow IPC
executor is a smaller first step for Wasm apps and can coexist with Ballista
later on the server side.

## Wire API

The API should be data-product oriented. One request computes one logical plan
fragment or one Avenger materialization target.

```text
POST /v1/execute-logical-plan

metadata:
  request_id: "uuid"
  client_epoch: 42
  priority: "preview" | "settled" | "blocking"
  source_refs:
    - taxi_pickups@revision
  udf_registry_version: "avenger-chart-transforms/..."
  logical_extension_codec: "avenger-core-v1"
  output_kind: "record_batches" | "rasterize2d" | "scalar"
  materialization_key: "hash(...)"
  params:
    view_x0: -8240000.0
    view_x1: -8210000.0
    view_y0: 4960000.0
    view_y1: 4990000.0
    pixel_width: 900
    pixel_height: 520

body:
  datafusion logical plan protobuf bytes
```

Response:

```text
status:
  request_id: "uuid"
  materialization_key: "hash(...)"
  source_revisions:
    taxi_pickups: "..."
  output_schema: Arrow schema
  warnings: []
  timing:
    plan_ms: 4.1
    execute_ms: 38.7
    encode_ms: 2.8

body:
  Arrow IPC stream containing one or more record batches
```

For HTTP, metadata can be a small JSON or protobuf envelope followed by Arrow
IPC bytes. For browser `fetch`, streaming IPC is ideal for table previews and
large grouped outputs, while small raster/scalar results may be buffered.

The server should support cancellation or best-effort abandonment by request
id. The client can also ignore stale responses using `client_epoch` and
`materialization_key`.

## Interactive Rasterization Scenario

This is the target Datashader-style flow for a Wasm taxi pickup density chart.

### Bootstrap

1. The browser loads the Avenger Wasm app.
2. The app fetches a catalog manifest containing the `taxi_pickups` schema,
   source revision, endpoint, and capabilities.
3. The client registers `taxi_pickups` as an `AvengerRemoteTableProvider`.
4. The plot is either received as a serialized `CompiledPlot` or constructed in
   the client from the catalog schema.
5. The plot session is instantiated in the browser.

The plot contains a view-scoped rasterization:

```rust
UniformRaster2D::new()
    .view(
        View::cartesian()
            .id("pickup-density")
            .preview_cached(true),
        |mark, view| {
            mark.transform(
                Rasterize2D::new(col("pickup_x"), col("pickup_y"))
                    .x(|x| x.extent(
                        view.x().domain_start(),
                        view.x().domain_end(),
                    ).bins(view.x().pixels()))
                    .y(|y| y.extent(
                        view.y().domain_start(),
                        view.y().domain_end(),
                    ).bins(view.y().pixels()))
                    .agg("count"),
                |mark, hist| {
                    mark.raster_with(hist.raster(), |r| {
                        r.x(hist.x_dim()).y(hist.y_dim())
                    })
                },
            )
        },
    )
```

### Initial Exact Evaluation

1. The client evaluates the plot session in exact mode.
2. Layout, guides, scales, and view domains are resolved locally.
3. Evaluation emits a materialization key for the initial `Rasterize2D` result:

   ```text
   hash(
     source_ref,
     rasterize2d spec,
     view x/y extents,
     pixel width/height,
     aggregate,
     color-domain dependencies
   )
   ```

4. If that key is not in the local materialization cache, the session enqueues
   a backend request.
5. Until the result is ready, the chart renders any available fallback state:
   empty raster, placeholder, or a coarser initial sample if provided.
6. The server executes the logical plan and returns an Arrow IPC stream with a
   raster struct row and metadata.
7. The browser stores the row in the materialization cache, invalidates the
   plot session, and reevaluates. `UniformRaster2D` consumes the cached raster
   row and produces a scene image mark.

### Pan And Zoom

During pointer or wheel interaction:

1. The browser updates raw domain params every frame.
2. The chart rerenders immediately by retargeting the last ready raster into
   the new viewport.
3. The client debounces preview raster requests. It may send one request every
   configured interval, not every frame.
4. Each preview request contains the current viewport extents, pixel size,
   source refs, logical plan, and materialization key.
5. The server returns Arrow IPC for the requested raster. Responses that no
   longer match the current epoch can be ignored or cached for later reuse.
6. When a fresh preview result is ready, the client swaps the cached data used
   by the mark and redraws.

The interaction invariant is:

```text
No pan/zoom event waits for remote execution.
Interaction mutates local params and renders with the best available raster.
Remote execution only improves the retained materialization cache.
```

### Settle

When the interaction settles:

1. The client marks the final viewport's raster key as the exact target.
2. It submits or reprioritizes a settled request.
3. The chart continues displaying the best available stale raster.
4. When the settled raster arrives, the client reevaluates and replaces the
   displayed raster with the exact result.

This flow is the browser/server version of the existing retained-view-result
idea in [async-rasterized-marks.md](async-rasterized-marks.md). The main
difference is that the materialization executor is an HTTP logical-plan
executor instead of a local DataFusion task.

## Regular Histogram Scenario

A histogram can use the same seam without view-dependent raster semantics:

1. The client builds a grouped aggregate plan:

   ```sql
   SELECT width_bucket(value, min, max, bins) AS bin, count(*) AS count
   FROM remote_table
   WHERE ...
   GROUP BY bin
   ORDER BY bin
   ```

2. The remote provider pushes the aggregate subplan to the server.
3. The server returns Arrow IPC record batches with `bin` and `count`.
4. The browser renders bars from the returned batches.

For small results, this can feel synchronous from the chart author's
perspective even though the runtime uses the same async request/cache plumbing.
For large or repeatedly updated histograms, the same request id, priority, and
stale-result handling apply.

## Serialization Requirements

The first implementation needs these codecs and registries:

- a durable `RemoteTableRef` representation in serialized logical plans,
- an Avenger logical extension codec that covers `Rasterize2D`, scalar
  aggregates, raster struct types, and any Avenger transform extension nodes,
- a shared UDF/UDAF registry contract by stable name and version,
- Arrow schema metadata for raster semantics, CRS, dimensions, aggregate
  names, and categorical planes,
- a source manifest format that can be sent with `CompiledPlot` or fetched by a
  client-authored plot UI.

Named session-local `MemTable` references remain useful for local examples and
tests, but they are not enough for a stateless backend.

## Wasm Runtime Notes

The Wasm frontend still needs DataFusion for the hybrid plan-building path.
However, expensive remote tables can avoid shipping source data to the browser,
and remote execution can keep most heavy transform work off the Wasm target.

Two runtime details need design:

- DataFusion physical execution in Wasm must be able to call `fetch` and yield
  a `RecordBatchStream` without assuming native Tokio networking.
- `PlotSession` materialization scheduling should go through a small spawn or
  executor abstraction so native builds can use Tokio while Wasm builds use
  browser tasks/promises.

This is also the point where a no-DataFusion frontend could be revisited. The
lighter frontend would need a separate plot/schema compiler contract and would
lose local computation unless another expression engine replaced DataFusion.
That should not block the federated DataFusion path.

## Server Contract

The backend is stateless with respect to interactive sessions. It may cache
results, but every request must carry enough information to validate and run by
itself:

- authenticated principal,
- source refs and revisions,
- logical plan bytes,
- registry versions,
- materialization key,
- output schema expectation,
- limits for rows, bytes, time, and memory.

The server must treat client logical plans as untrusted:

- authorize every source ref,
- reject unsupported functions and extension nodes,
- enforce output limits,
- enforce timeouts and cancellation,
- avoid exposing file paths or internal table names in errors,
- include source revisions in responses so stale cache entries can be detected.

## Open Decisions

- Should the first wire format use DataFusion protobuf logical plans directly,
  or an Avenger envelope with DataFusion bytes as one payload field?
- Should `datafusion-federation` be a direct dependency after the DataFusion
  update, or should Avenger copy the small subset of its pattern first?
- Which Avenger transforms are remote-capable in v1: only `Rasterize2D` and
  scalar aggregates, or any registered logical extension with a shared codec?
- How should mixed local/remote plans handle local tables, selections, and
  joins? For v1, prefer pushing only all-remote subplans and collecting small
  local inputs explicitly.
- Does the remote executor return raw raster structs only, or can it also
  optionally return pre-shaded image resources for very large rasters?
- What cache key fields are mandatory across exact, preview, and stale-retarget
  modes?

## Suggested Implementation Path

1. Define `RemoteTableRef` and a catalog/source manifest.
2. Add a remote table provider that exposes schema but cannot execute locally.
3. Prototype logical-plan-over-HTTP for simple projection/filter/aggregate
   queries, returning Arrow IPC.
4. Integrate the provider with a federation-style optimizer that pushes
   all-remote subplans.
5. Route `ScalarAggregate` materializations through the remote executor.
6. Route view-scoped `Rasterize2D` materializations through the remote executor
   and validate pan/zoom stale-preview behavior.
7. Add server-authored `CompiledPlot + manifest` bootstrap.
8. Add client-authored catalog/schema bootstrap.

The first demo should be the interactive rasterization scenario above, followed
by a regular histogram. Together they validate both view-dependent async
materialization and simple sync-looking aggregate pushdown.
