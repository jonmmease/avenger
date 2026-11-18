# avenger-sql-editor

A lightweight desktop editor of avenger-sql queries. CSV and Parquet files become tables of a DataFusion session with the functions of avenger-sql, and the result of each query shows as a chart, or as a table whose chart columns show the chart of each row. The editor holds six sample tables and a gallery of example queries over them.

## Run

```bash
cargo run --release -p avenger-sql-editor
```

Each path argument is a CSV or Parquet file to open as a table. `--query <FILE>` opens a query and runs it once the files are open. `--screenshot <PNG>` saves the window to a PNG once the query and its charts finish, and then exits. Before the screenshot, `--cursor <INDEX>` puts the cursor before the character at the index, each `--press <KEYS>` presses keys in order, such as `alt+Space` to open the completions or `F12` to go to the definition, and `--hover` holds the pointer over the character after the cursor.

## Run in a browser

The editor also builds for the web with wasm-pack, from the repository root:

```bash
wasm-pack build avenger-sql-editor --target web --release --out-dir web/pkg
```

```bash
python3 -m http.server 8776 -d avenger-sql-editor/web
```

Then open `http://localhost:8776`. DataFusion compresses Arrow IPC with zstd, which compiles C, and Apple's clang has no wasm32 target, so on macOS set `CC_wasm32_unknown_unknown` and `AR_wasm32_unknown_unknown` to the `clang` and `llvm-ar` of Homebrew's LLVM, such as `/opt/homebrew/opt/llvm/bin/clang`. In the browser, the editor reads the sample tables only, and a query or a chart cell runs on the page's one thread, so the page waits while it runs.

## The window

- **Data**, on the left, lists the tables of your files, the sample tables, each with its columns, and the functions that avenger-sql adds, by their families in its library: the marks, the compositions, the constructors, and the transforms. "Add" opens CSV and Parquet files, and so does dropping files on the window. A table takes its name from its file: the file stem in lowercase, with `_` for each character that is not an ASCII letter, a digit, or `_`. Clicking a column or a function, or "Insert" under a table, inserts its name at the cursor. The sample tables are in the schema `samples`, such as `samples.penguins`, so a file named `penguins.csv` is the table `penguins` beside them. [`samples/README.md`](samples/README.md) gives the source and the terms of each.
- **The editor** reads a `Document` of avenger-sql-language for the query, over the tables of the session. It colors the tokens by the document's highlights, which the analysis refines, so that a mark, a column, and a table each take their own color, and it underlines each error that the document reports. Option+Space (Ctrl+Space on other platforms) opens the document's completions at the cursor, which the typed name filters: the arrows move the selection, Enter or Tab inserts it, and Escape closes the list. Hover over a name to read its documentation, over the name of an argument to read its parameter, or over an underlined error to read its message and help. ⌘. (Ctrl+.) on an error lists its fixes, and Enter applies the selected one. While the cursor is on a name, its definition and each use of it show on a faint background, and F12 or ⌘-click (Ctrl-click) on a name selects its definition. ⌘↵ (Ctrl+Enter on other platforms) or Run runs the query. ⌘/ (Ctrl+/) comments the lines of the selection out with `--`, or back in when they all are comments. Format, or ⇧⌥F (Shift+Alt+F), formats the query in the style of avenger-sql's `STYLE.md`, unless it has a syntax error. A query that runs for more than 200 ms can be canceled from the status bar.
- **Result** shows a result of one row and one column that holds a chart as an interactive chart that fills the pane. It shows every other result as a table. Each chart column of the table shows the chart of each visible row as a static image.
  - Drag the edge of a column header to change the width of a column, and drag the bottom edge of a row to change the height of every row. The charts draw at the new size in each frame of the drag, and so does a single chart while the window resizes.
  - Hover over a chart cell and click the expand button at its top-right corner, or double-click the cell, to open its chart in the whole result pane. A bar above it names the row by its other columns. **Back**, or Escape while neither the pointer nor the keyboard is on the chart, returns to the table where it was. A new result closes the chart.
  - Turn on **Interactive charts**, above the table, to pan, zoom, and select in every visible chart of the table. It is off when the editor starts, so that the wheel scrolls the table over the charts. A chart that scrolls out of view, or a result that replaces the table, drops its pans and zooms.
- **Examples**, the view of the result pane when the editor starts, shows a card for each example query of [`examples.md`](examples.md), by section, with a thumbnail of its chart, and the number of its charts for a result of several. A card runs its query the first time it comes into view, four at a time, and keeps its result while the editor runs. Hover over a card to read what its chart shows. Click a card to put its query in the editor, under its title and description as comments, and run it. ⌘Z (Ctrl+Z) in the editor brings back the text before it. The search box keeps the examples whose title, description, section, or query holds the text.
- **Lifted SQL** shows the query as the staged query that its mark calls with statistics plan, as `avenger_sql::lift` prints it, with the text that it wrote highlighted, and **Plan** shows the optimized logical plan of the last result.

While a query has an error, the result pane keeps the last result that ran.

## Limits

- One query in the editor.
- No Delta tables, no remote files, and no saved sessions yet.
- Charts in table cells have no tooltips while **Interactive charts** is off.
