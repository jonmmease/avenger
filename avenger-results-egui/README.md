# avenger-results-egui

The result of an avenger-sql chart script in egui, as the window of `avenger view` shows it. A result of one row whose one column holds a chart shows that chart alone, live, so it pans, zooms, selects, and shows its tooltips. Any other result shows its table: a column for each column of the result, and in each column of charts, the chart of each visible row at the cell's size. `avenger_chart::results::QueryResult` holds the result and decides its shape, so this crate and `avenger render` show a result alike.

```bash
cargo run --release -p avenger-results-egui --example results
```

## The view

`ResultView` is the view of a result. `set_result` shows a new result, as after the chart script runs again: a live chart takes the new chart and keeps its views and selections, and the cells of a table draw again. `show` draws the view in the rest of a `Ui` and returns the error of a live chart that fails to build. `restyle` restyles the charts after the color scheme or a stylesheet changes, and `busy` says whether a chart still works, so that the host asks for another frame.

`set_rows` shows a table without charts from a `RowSource`, which reads its rows a page at a time as the table scrolls, so a table of millions of rows shows at once and never collects. A source gives its schema, the count of its rows, which runs beside the reading so the scroll bar knows the table's length, and one stream of its rows in order. The table reads the rows that it shows and a page ahead, `PAGE_ROWS`, and keeps the rows that it has read, so scrolling back shows them without reading. The rows come from one run of the query, read forward, since a query need not give its rows in the same order each time it runs: pages of separate runs, with `OFFSET`, could repeat rows and miss others, as a view with `GROUP BY` does. A line over the table gives the count of the rows, or the rows read while the count runs.

In a table, the columns and the rows resize by dragging: a column by its header's edge, and the rows by the strip along the bottom of a cell. With `ResultView::interactive`, the chart cell under the pointer runs its chart's app. The expand button in the top right corner of a chart cell, or a double click, opens the cell's chart over the table, live, with a bar of the cell's row and a Back button, which Escape also presses.

## The chart's state

`ResultView::set_on_change` has a host hear the changes of the gestures on the chart that fills the view, as `avenger_chart_app::ChartChange`s, and `ResultView::chart_state` reads the chart's state as text, a `ChartState`: each selection's clauses, each widget's value, each scale's domain as the chart draws it, an interval or its categories, and the visible one of a panned or zoomed position scale, and the modes that are on. `describe_change` names what a change changed, as `brush: Horsepower 80.0 to 140.0`, for a host that shows the state apart from the chart, as a debugger's panes do.

## What a host gives

The charts of a view share what `Shared` holds: eframe's wgpu `RenderState`, whose device draws the charts into egui textures, an `Exec`, the runtime of the charts' apps and of the cells' background evaluations, natively on a multithreaded tokio runtime and on the web on the page's thread, the cascade and the reader's preferences, the label engine, and `Images`, the caches of the charts' images.

`Surface` gives the charts the background of egui's panels, so that a chart and the panel around it are one surface, and the color scheme of egui's visuals, with the preferences and the cascade of both from avenger-sql-project's `charts` module. `Images::new` takes the `ImageCacheOptions` of the host, so that each host fetches with its own User-Agent and keeps its own disk cache. `StaticChart` is the chart of a table cell, which evaluates without an app at any size.

The host keeps what belongs to it: running the chart script, its project, its tabs, and its controls, such as a switch of `interactive` or of the color scheme.
