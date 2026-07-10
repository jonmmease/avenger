# chart-bake-server-client

The flagship `CompiledPlot::bake` example: a **server** process that owns the
data bakes a chart into a self-contained artifact, and a **client** process
renders it interactively with zero access to the original sources.

```sh
# 1. Server: generates data/sales.parquet on first run, compiles the chart,
#    bakes it (the param-free scan below the $min filter folds into embedded
#    tables), prints the bake report, and writes artifact/baked-chart.bin.
cargo run --release -p chart-bake-server-client --bin server

# 2. Client: loads the artifact into a FRESH SessionContext (no parquet, no
#    registered tables) and opens an interactive window. Move the cursor
#    horizontally to sweep the $min threshold — the embedded rows
#    re-aggregate per event through the live residual plan.
cargo run --release -p chart-bake-server-client --bin client
```

What to look at:

- `src/lib.rs` — the chart: per-region `SUM(value)` totals above a live
  `$min` filter, with `$min` bound to the cursor's x position through a
  `ChartEventBinding` (bindings serialize with the plot, so the baked chart
  stays interactive for free).
- `src/bin/server.rs` — bake + report + artifact write. The `$min` filter
  sits *below* the aggregate, so partial evaluation folds only the raw scan:
  the artifact embeds the rows and the client re-aggregates them per param
  value.
- `src/bin/client.rs` — deserialize, fresh session, render. The manifest
  registration happens inside `evaluate`; the client does nothing special.

Note: the server loads the parquet into memory before compiling — DataFusion
54's logical-plan codec cannot round-trip `ParquetFormat`, so compiling over
a direct parquet scan fails at plan serialization. Once fixed upstream this
becomes `register_parquet` + bake, with identical artifacts.
