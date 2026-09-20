# Flight Delay Explorer

A native cross-filter dashboard built directly from Avenger's foundation crates. The initial scatterplot draws all **327,346 eligible flights** from the **336,776-row nycflights13 dataset**. It uses linear axes, retains negative delays and outliers, and performs no sampling.

```sh
cargo run --release -p winit-dataflow-flights
```

The bundled Parquet fixture makes the application usable offline. Python is needed only to reproduce that fixture.

## Explore

- **Brush the scatterplot.** Drag a rectangle to filter the airline bars, summary, and six destination histograms. Points outside the brush remain visible. Dragging continues outside the plot and commits on release.
- **Choose airlines.** Click a carrier row to toggle it. Airline counts exclude their own selection, so excluded carriers remain available. The scatter shows flights from included carriers.
- **Change local bins.** Each destination's button cycles through 15, 30, and 60 minutes. The six busiest destinations are discovered once and remain visible when empty.
- **Clear or reset.** Escape clears the brush. No airlines creates an active empty selection. All airlines restores the carrier catalog. Reset also restores 30-minute bins.
- **Hover.** Entering the scatter or airline plot warms eligible aggregate states. Point and histogram tooltips read installed data without additional queries.
- **Resize.** Panels reflow, scales update, and retained raw brush bounds are applied to the new pixel grid. Main plots stack when a narrow window has enough vertical room.

By default, both scatter projections use two-logical-pixel selection cells. Membership follows the same scale kernels as mark coordinates. Bounds include the lower endpoint and exclude the upper endpoint. The displayed brush represents effective cells. `--exact-selection` uses raw bounds instead. Pixel and exact modes can select different flights near boundaries.

## Compare direct queries with preaggregation

```sh
cargo run --release -p winit-dataflow-flights -- --preaggregate auto --diagnostics
cargo run --release -p winit-dataflow-flights -- --preaggregate off --diagnostics
```

`auto` prepares one aggregate family per receiving view and warms it on focus entry. A drag reuses that family's stored states and binds the new retained-space predicate. A receiver that cannot use preaggregation executes its complete direct predicate. The scatter itself always uses a direct row query.

`off` skips optimized-family construction and hover warm-up. Both modes retain ordinary source and result caching. Use the same viewport, selection mode, and fresh brush bounds when comparing them. Measure initial focus warm-up separately from later drags. Repeating identical bounds can hit the final-result cache in either mode.

Other options:

| Option | Behavior |
| --- | --- |
| `--headless` | Replay a fixed interaction trace, print result tables and runtime reports, and compare direct and rollup results |
| `--sql` | Print diagnostic SQL for the scatter and focused materializations |
| `--exact-selection` | Omit pixel quantization |
| `--no-cache` | Disable completed-result retention, while retaining in-progress request sharing |
| `--cache-mib N` | Set the runtime result-cache budget, default 256 MiB |
| `--materialization-mib N` | Set the active materialization budget, default 512 MiB |
| `--data PATH` | Read a compatible local Parquet fixture |

Diagnostics report actual executed node names, cache hits, shared in-progress computations, elapsed query time, and warm-up state sizes. Repeated scoped node names are grouped with an instance count. A cache hit can still require partition discovery and gathering. Query time excludes preparation, scene conversion, and GPU rendering. The byte budgets do not include all DataFusion operator, scene, point-index, or GPU memory.

```sh
cargo run --release -p winit-dataflow-flights -- --headless
cargo run --release -p winit-dataflow-flights -- --headless --preaggregate off
```

The workspace's `release` profile uses optimization level 1. Use `--profile release-perf` for optimization level 3 measurements.

## Implementation map

| File | Responsibility |
| --- | --- |
| [dataflow.rs](src/dataflow.rs) | Named source and scatter calculations, direct/optimized extensions, destination scopes, scalar outputs, bindings, and reports |
| [queries.rs](src/queries.rs) | Compose selection splits with checked preaggregate templates and opaque aggregate states |
| [selection.rs](src/selection.rs) | Two producers sharing an intersected selection, self-filter exclusion, carrier tuples, and pixel grids |
| [layout.rs](src/layout.rs) | Panel hierarchy, responsive layout, fixed guide reservations, and coordinated histogram labels |
| [scene.rs](src/scene.rs) | Convert installed results into symbols, bars, guides, and widget frames |
| [controller.rs](src/controller.rs) | Event streams, between-stream dragging, bounded debounce, widgets, and generation-checked installation |
| [worker.rs](src/worker.rs) | Own foreground and warm-up query tasks, cancel superseded consumers, and deliver completion wakes |
| [picking.rs](src/picking.rs) | Shared dense-point buffers and a compact center index for tooltips |

The dataflow runtime owns result caching and request coalescing. The example retains a direct extension and at most one compatible optimized extension per focus producer. A changed fixed predicate or pixel grid replaces its affected preparation. There is no separate selection-level readiness registry or materialized-table history.

DataFusion scale UDFs compute scatter positions, histogram positions, airline bar lengths, band positions, and ordinal color indices. Histogram `bin_minutes` inputs belong to destination scopes. A named local projection resolves the bin input before preaggregation. Zero-fill joins finish the aggregate results outside the preaggregate rewrite.

The event-loop thread handles lightweight state changes and scene assembly. Query execution, preparation, Arrow conversion, and point-index construction run on a Tokio worker. Superseded foreground tasks are aborted. Dataflow keeps a shared prerequisite alive while another consumer needs it. Synchronous kernels or conversions can finish after cancellation, so result installation also checks generations. Existing scenes remain visible while work runs.

## Dataset

The fixture is a narrow export of [nycflights13](https://github.com/tidyverse/nycflights13) revision `df98ef215aa8216fe0838a0b8ac5bada646d814c`. The package is distributed under CC0 and credits the Bureau of Transportation Statistics for flight data. See its [field definitions](https://nycflights13.tidyverse.org/reference/flights.html).

The analytical cohort excludes **9,430 rows** missing either departure or arrival delay. These exclusions do not all represent cancellations. `flight_id` is the zero-based original row position. Scheduled departure minutes equal `hour * 60 + minute`. The export retains every source row and preserves null delays. The dataflow applies the cohort filter.

[manifest.json](data/manifest.json) records source and export checksums, revision, counts, and byte sizes. [airlines.json](data/airlines.json) preserves the carrier-name lookup. Reproduce the export with pinned Python dependencies:

```sh
uv run examples/winit-dataflow-flights/scripts/export_data.py
```

## Verification

```sh
cargo test --release -p winit-dataflow-flights
cargo run --release -p winit-dataflow-flights --example snapshot -- /tmp/flights.png
cargo run --release -p winit-dataflow-flights --example snapshot -- /tmp/flights-narrow.png 900 1150
cargo run --release -p winit-dataflow-flights --example snapshot -- /tmp/flights-empty.png 1280 940 empty
```

Tests check known counts and means, complete ordered flight IDs, direct/rollup agreement, scoped cache reuse, select-none, raw-bound preservation on regrid, outside-plot dragging, final-release flushing, widget consumption, stale completion rejection, hover buffer reuse, and viewport bounds. Snapshot rendering uses the native scene builder. The empty snapshot replays the No airlines widget through the app event API.
