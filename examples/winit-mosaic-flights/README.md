# Mosaic flights: three cross-filtered histograms

A native `avenger-app` / winit example inspired by Mosaic's [Cross-Filter Flights (10M)](https://idl.uw.edu/mosaic/examples/flights-10m.html). It uses the same 10-million-row dataset and shows arrival delay, departure time, and flight distance.

## Run

Download the dataset once, outside the repository:

```sh
mkdir -p "$HOME/datasets/mosaic"
curl -L --fail \
  'https://pub-1da360b43ceb401c809f68ca37c7f8a4.r2.dev/data/flights-10m.parquet' \
  -o "$HOME/datasets/mosaic/flights-10m.parquet"

cargo run --release -p winit-mosaic-flights -- \
  --data "$HOME/datasets/mosaic/flights-10m.parquet"
```

Add `--preaggregate off` to compare direct execution. The default is `auto`. Both modes use the same 1-pixel selection cells, 5 ms debounce, and ordinary result caching. Repeating a previous brush can hit the final-result cache in either mode.

## Interaction

- Hover over a histogram to warm the two other histograms' aggregate states.
- Drag horizontally to select an interval. Each histogram excludes its own brush and applies the other two.
- Click without dragging, or right-click, to clear one brush. Press Escape to clear all three.

The three panels are fixed at 600×200 pixels, including axes. Their display bins are 10 minutes of arrival delay, one hour of departure time, and 200 miles of distance. Arrival delay is clipped to −60 through 180 minutes. `DEP_TIME` in the reference dataset already contains fractional hours. Empty bins have no rectangle, and an empty result retains its axes.

## Code structure

| File | Responsibility |
| --- | --- |
| [dataflow.rs](src/dataflow.rs) | One prepared graph with three direct histogram queries and six focus-to-target materialization/rollup pairs |
| [selection.rs](src/selection.rs) | Plot definitions and `avenger-selection` producers, cross-filter predicates, and pixel grids |
| [controller.rs](src/controller.rs) | `avenger-eventstream` hover, drag, debounce, and result installation |
| [worker.rs](src/worker.rs) | Background queries, cancellation, and native completion notifications |
| [layout.rs](src/layout.rs) | Fixed panel arrangement through `avenger-layout` and `avenger-panels` |
| [scene.rs](src/scene.rs) | Native `avenger-scales`, `avenger-guides` axes, bars, and brush overlays |

The graph is prepared once. Hover primes the two relevant state tables and the focused histogram. Each materialization depends on an expression input containing the other active selections. Changes to that input invalidate its cached states. The changing brush is checked by `avenger-datafusion-preaggregate` and bound to the rollup's cell predicate. `avenger-datafusion-dataflow` owns caching, shared execution, and cancellation.

Queries return bin/count tables. Rendering applies native Avenger scales to the small results, using the same scales for axes and pointer inversion. Selection pixel cells use the DataFusion scale adapter. The example keeps the last completed histograms visible while a replacement query runs. New requests cancel superseded requests, and generation checks reject stale completions.

## Diagnostics

Add `--diagnostics` to print elapsed time, preaggregation strategy, cache hits, and executed node names. Warm-up is followed by rollups that reuse the state tables. Changing another brush can execute those state nodes again.

The footer's milliseconds include predicate binding and dataflow evaluation. They exclude event debounce, scene construction, and GPU presentation. The preaggregation count describes the chosen strategy, not cache hits. The example uses a 2 GiB result-cache budget and a 4 GiB active-materialization budget.
