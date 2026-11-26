# avenger-sql-web

A WebAssembly library that builds avenger charts and projects from avenger-sql in a page, for an online editor: a project of files that the page holds in memory, its data scripts loaded into a session of DataFusion with their tables of CSV, JSON, Arrow, and Parquet files, the charts of its chart scripts, and the features of an editor at a position of a file. The library holds avenger-query-web's library, which runs the file of a document's baked graph, and avenger-chart-web's, which draws the charts, so its module holds their exports too.

## The library

```js
import init, { SqlProject, ChartView, addFonts } from "./pkg/avenger_sql_web.js";

await init();
const project = await SqlProject.open("sales", {
  "avenger.toml": '[project]\nname = "sales"\n',
  "data/sources.av.sql":
    "CREATE EXTERNAL TABLE sales OPTIONS (format = 'CSV', uris = ['data/files/sales.csv']);",
  "data/files/sales.csv": "region,revenue\neast,10\nwest,20\n",
  "charts/revenue.av.sql": "FROM sales |> AGGREGATE mark_bar(x => region, y => revenue) AS chart",
});
const [chart] = await project.charts("charts/revenue.av.sql");
const view = await ChartView.create(canvas, chart, {});

await project.setText("charts/revenue.av.sql", "FROM sales |> WHERE ");
const { range, items } = project.completions("charts/revenue.av.sql", 20);
const errors = project.diagnostics("charts/revenue.av.sql");
```

- `SqlProject.open(name, files)` opens the project of `files`, each a file's text or bytes by its path relative to the project's root, in a store in memory named `name`, and loads its data scripts. A session of one partition runs the queries.
- `chartScripts()` gives the paths of the chart scripts, and `text(path)` the text of a file.
- `charts(path)` resolves to the bytes of the charts of the chart script's last query, one for each row, which `ChartView` draws, or rejects with the script's error.
- `setText(path, text)` sets the text of a file, as an editor changes it: every file's analysis reads it, and a data script or a module loads again, with the scripts downstream of it.
- `diagnostics(path)`, `completions(path, at)`, `hover(path, at)`, `signatureHelp(path, at)`, and `symbols(path)` give avenger-sql-language's errors and warnings, completions, hover, forms of a call, and outline of a file, as plain objects. Positions and ranges count UTF-16 code units, as the strings of JavaScript do.

`project::SqlProject` is the same project in Rust, natively and in WebAssembly.

## Build

```bash
avenger-chart-web/scripts/build.sh avenger-sql-web [OUT_DIR]
```

`scripts/build.sh` measured these sizes on 2026-11-27:

| File | Raw | gzip | Brotli |
|---|---|---|---|
| `avenger_sql_web_bg.wasm` | 29.27 MB | 10.32 MB | 6.75 MB |

Over avenger-query-web's module, it holds avenger-sql's parser and analysis, DataFusion's SQL planner, the readers of CSV, JSON, Arrow, and Parquet files, avenger-sql-project's loading of a project, and avenger-sql-language's features of an editor.

## Tests

The tests open a project of a data script, a CSV file, a Parquet file, and a chart script, build its charts, change files, and ask for completions and diagnostics, natively and in a browser:

```bash
cargo test --release -p avenger-sql-web
```

```bash
wasm-pack test --release --headless --chrome avenger-sql-web
```
