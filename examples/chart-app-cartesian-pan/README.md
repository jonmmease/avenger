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
`cache_metrics()` to JS; the loader puts it on `window`, so in the browser
console it is just `cache_metrics()`:

1. After first render, call `cache_metrics()` — expect committed
   entries > 0 and no panics (the web-time clock working).
2. Start a hands-free logger BEFORE grabbing the chart, then pan for a
   few seconds, release, wait a beat, and stop it:

   ```js
   const t = setInterval(() => console.log(Date.now() % 100000, cache_metrics()), 400);
   // pan around ... release ... wait ~2s
   clearInterval(t);
   ```

   Expected timeline for THIS chart (validated 2026-07-10):
   - idle after startup: `hits > 0` with a small committed entry count —
     the initialization evaluations reuse each other;
   - mid-drag: `misses` climb steeply while `hits` and `admitted_writes`
     stay FLAT — every pan frame embeds a fresh view domain (unique
     fingerprints by design), and preview evaluations are observe-only,
     so no admission churn;
   - after release: all counters freeze — the settle evaluation is served
     entirely by avenger's session artifacts and issues no queries;
   - throughout: `entries`/`bytes` bounded, and
     `fingerprint_nanos_total / misses` in the tens of microseconds.
3. The pass criteria are therefore: no panics anywhere (the web-time
   clock), startup `hits > 0`, admissions flat during the gesture, and
   bounded entries/bytes. Do NOT expect mid-gesture hit growth on this
   tiny fully-view-dependent chart — that payoff belongs to param-driven
   and heavier charts (pinned by the native integration suite).
4. Note the first evaluation's feel vs later ones; first-touch content
   hashing of the (tiny) table is unmeasurable here — large-table
   first-hash cost is a documented watch item, not part of this
   validation.

Record the outcome in `scratch/physical-cache-avenger-integration-plan.md`
(Phase 9 sign-off).
