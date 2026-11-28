# avenger-query-web

A WebAssembly library that runs a document's queries in a page, so its avenger charts follow the reader's selections without a server. A host, such as `avenger render` or avenger-cli's Quarto engine, bakes the graph of a document's charts with avenger-query-graph and writes its file: the plans of the charts that read a selection, the tables that those plans scan, and each chart's rows before any selection changes. The page reads the file and runs the plans again when the reader changes a selection, so a chart that a brush filters with `WHERE brush` takes its new rows.

The library holds avenger-chart-web's library, whose exports its module holds too, the chart graph that runs the file, and DataFusion's optimizer, planner, and execution, without SQL or readers of files. A run in a page binds the selections' states, optimizes, plans, and keeps results as a native run of a query graph does.

## The library

```js
import init, { ChartGraph, ChartView, addFonts } from "./pkg/avenger_query_web.js";

await init();
const graph = new ChartGraph(bytes); // the file of a baked graph
const charts = await graph.run(); // each chart's bytes, or null
const view = await ChartView.create(canvas, charts[0], {
  onChange: async (change) => {
    if (graph.change(change.bytes)) redraw(await graph.run());
  },
  reports: "every",
});
```

- `new ChartGraph(bytes)` reads a document's file, as avenger-cli's Quarto engine writes it. It throws for bytes of no file, or of a file whose plans name a function or a node that the library lacks.
- `reads` gives the names of the selections that the graph's queries read.
- `change(bytes)` takes the selections of a view's change message and returns whether the graph reads one of them.
- `run()` resolves to each chart's bytes, or `null`, for the clauses that it took. The first run computes nothing, since the file holds each chart's rows for empty selections.
- `ChartView`, `addFonts`, and the element are avenger-chart-web's.

The `<avenger-chart>` element of avenger-chart-web draws an element with a `graph` attribute from its graph when the page loads this library, through `start`'s `glue` and `wasm`, `avenger_query_web.js` and `avenger_query_web_bg.wasm`, as avenger-chart-web's README describes.

## Build

```bash
avenger-chart-web/scripts/build.sh avenger-query-web [OUT_DIR]
```

## Tests

The test of the library loads the file of a real chart's baked graph and runs it for a brush, natively and in a browser:

```bash
cargo test --release -p avenger-query-web
```

```bash
wasm-pack test --release --headless --chrome avenger-query-web
```

## Sizes

The build of the Quarto runtime measured these sizes on 2026-11-27:

| File | Raw | gzip | Brotli |
|---|---|---|---|
| `avenger_query_web_bg.wasm` | 23.15 MB | 7.94 MB | 5.15 MB |

Over avenger-chart-web's module, which is 2.31 MB as Brotli, it holds DataFusion's logical plans with their decoder, optimizer, and planner, the operators and the functions that the plans of charts reach, the query graph, and the functions of avenger's marks and compositions.
