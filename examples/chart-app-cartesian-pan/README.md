# chart-app-cartesian-pan

Grouped scatter with cartesian pan/scroll-zoom, native and wasm. This
example also carries the **physical result cache browser validation** for
`avenger-datafusion-cache` (integration plan Phase 9).

## Run (native)

```bash
cargo run --release -p chart-app-cartesian-pan
```

## Run (browser)

```bash
wasm-pack build --target web --release examples/chart-app-cartesian-pan
cd examples/chart-app-cartesian-pan && python3 -m http.server 8765
# open http://localhost:8765
```

## Physical-cache validation (manual, browser)

The app builds its session context through
`avenger_chart::physical_cache::cached_session_context()` and exports
`cache_metrics()` to JS. In the browser console (with the wasm module
bound as `wasm` per the example's loader):

1. After first render, call `wasm.cache_metrics()` — expect committed
   entries > 0 and no panics (the web-time clock working).
2. Pan the chart, calling `cache_metrics()` during the gesture — expect
   `hits` to grow while `admitted_writes` stays FLAT (preview evaluations
   are observe-only).
3. Release and let the view settle — expect `admitted_writes` to grow
   (settled exact evaluations admit again), then further pans to hit.
4. Note the first evaluation's feel vs later ones; first-touch content
   hashing of the (tiny) table is unmeasurable here — large-table
   first-hash cost is a documented watch item, not part of this
   validation.

Record the outcome in `scratch/physical-cache-avenger-integration-plan.md`
(Phase 9 sign-off).
