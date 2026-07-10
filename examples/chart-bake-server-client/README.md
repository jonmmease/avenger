# chart-bake-server-client

The flagship `CompiledPlot::bake` example: a **server** process that owns the
data pre-evaluates everything param-independent and bakes the chart into a
small self-contained artifact; a **client** process renders it interactively
with zero access to the original sources.

The chart is daily `SUM(value)` totals per region over ~4M raw parquet
events, with a live `$min` threshold ABOVE the aggregate. That shape is what
makes baking pay: the aggregate is param-free, so partial evaluation executes
it once at bake time and the artifact embeds only the few thousand
pre-aggregated rows — interactions on the client filter those, never the
raw events.

```sh
# 1. Server: generates data/trips.parquet on first run, compiles the chart,
#    bakes it, prints a measured cost comparison (below), and writes
#    artifact/baked-chart.bin.
cargo run --release -p chart-bake-server-client --bin server

# 2. Client: loads the artifact into a FRESH SessionContext (no parquet, no
#    registered tables) and opens an interactive window. Move the cursor
#    horizontally to sweep the $min threshold.
cargo run --release -p chart-bake-server-client --bin client
```

The server prints what a `$min` change costs under three architectures
(numbers from one machine, `--release`; rerun to reproduce locally):

```
per-interaction cost of a $min change:
  architecture                    ships to client    per interaction
  1. thin client (server query)      31.2 MB            31.5 ms + round trip
  2. unbaked spec (rows embedded)   297.6 MB           383.4 ms
  3. baked spec (pre-evaluated)       104 KB            24.4 ms
```

- **Thin client**: the client owns nothing; every param change is a server
  query over parquet plus a network round trip, and the server stays in the
  loop forever.
- **Unbaked spec**: self-containment without baking means embedding every
  raw row in the spec, and every interaction re-runs the full aggregate over
  them.
- **Baked spec**: the aggregate ran once on the server. The client gets a
  spec three orders of magnitude smaller that evaluates faster than the
  server could even answer a query — with no server at all.

What to look at:

- `src/lib.rs` — the chart definition: `$min` is bound to the cursor's x
  position through a `ChartEventBinding`, which serializes with the plot, so
  the baked chart stays interactive with zero client-side wiring.
- `src/bin/server.rs` — the pipeline (aggregate below a live filter), the
  three-way measurement, the bake report, and the artifact write.
- `src/bin/client.rs` — deserialize, fresh session, render. Baked-table
  manifest registration happens inside `evaluate`; the client does nothing
  special.

Note: the chart compiles over an unnamed in-memory scan of the parquet rows
rather than the parquet scan itself, for two reasons: unnamed scans serialize
inline (which is what makes the UNBAKED spec genuinely self-contained for the
comparison), and DataFusion 54's logical-plan codec cannot round-trip
`ParquetFormat` (compiling over a direct parquet scan fails at plan
serialization). Once that is fixed upstream, `register_parquet` becomes the
natural source here.
