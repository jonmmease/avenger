# avenger-query-web

A WebAssembly library that runs a document's queries in a page, so its avenger charts follow the reader's selections without a server. A host, such as `avenger render` or avenger-cli's Quarto engine, plans the queries of a document's charts into one graph of physical plans and writes the graph's file, with the results of the parts that read no selection. The page decodes the file and runs the parts that read a selection again when the reader changes one, so a chart that a brush filters with `WHERE brush` takes its new rows. The library holds avenger-chart-web's library, whose exports its module holds too, and DataFusion's execution of the operators and the functions that the plans of charts hold, without SQL, a planner, or readers of files.

## The library

```js
import init, { ChartGraph, ChartView, addFonts } from "./pkg/avenger_query_web.js";

await init();
const graph = new ChartGraph(bytes); // a graph's file
const charts = await graph.run(); // each output's chart bytes, or null
const view = await ChartView.create(canvas, charts[0], {
  onChange: async (change) => {
    if (graph.change(change.bytes)) redraw(await graph.run());
  },
  reports: "every",
});
```

- `new ChartGraph(bytes)` decodes a document's graph, as avenger-cli's Quarto engine writes its file. It throws for bytes of no graph, or of a graph whose plans name an operator or a function that the library lacks.
- `reads` gives the names of the selections that the graph's queries read.
- `change(bytes)` takes the selections of a view's change message and returns whether the graph reads one of them.
- `run()` resolves to each output's chart bytes, or `null`, for the clauses that it took.
- `ChartView`, `addFonts`, and the element are avenger-chart-web's.

The `<avenger-chart>` element of avenger-chart-web draws an element with a `graph` attribute from its graph when the page loads this library, through `start`'s `glue` and `wasm`, `avenger_query_web.js` and `avenger_query_web_bg.wasm`, as avenger-chart-web's README describes.

## Build

```bash
avenger-chart-web/scripts/build.sh avenger-query-web [OUT_DIR]
```

## Sizes

The build of the Quarto runtime measured these sizes on 2026-11-26, with avenger-sql's library:

| File | Raw | gzip | Brotli |
|---|---|---|---|
| `avenger_query_web_bg.wasm` | 18.82 MB | 6.29 MB | 4.21 MB |

Over avenger-chart-web's module, it holds DataFusion's operators of the plans of charts, their decoders, DataFusion's scalar, nested, aggregate, and window functions, the functions of avenger's marks and compositions, and the functions of avenger-sql's library, which decode by their names and state. It links avenger-sql's library alone: avenger-sql's parser, its analysis, and DataFusion's planners and optimizer stay out of the module.
