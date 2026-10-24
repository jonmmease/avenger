# m4-series

A random walk of ten million samples, drawn as a line through at most four
samples for each pixel column, which a background task picks in DataFusion
for the view that the pan and zoom settle on.

- Drag the plot sideways to pan along the series.
- Scroll over the plot to zoom along the series about the cursor.
- Double-click the plot to restore the whole series.

M4 decimation keeps each pixel column's first, last, lowest, and highest
samples, so within the view the line draws the same pixels as the whole series
would. The line ends at the view's first and last samples. The app's state
holds the view's x scale, which an avenger-app-tools `PanZoom` tool pans and zooms,
and each gesture draws again. The view comes to rest when a drag pauses or the
wheel stops for 20 ms, at a release, and after a double-click, and the scene's
build then requests the line of the view, as it does for the first scene and a
resize. The request is an avenger-app `Task` keyed by the view's x domain and the
plot's width in device pixels, so a request for the view that the task is
decimating, or that the latest line shows, starts nothing.

The task's job rasterizes the series with avenger-datafusion-rasterize's
`Rasterize` into one row, with a cell for each device pixel column. Its
aggregates are the first and last samples' x and y, ordered by x, the lowest
and highest values with their x, through `first_value` ordered by y. The job
turns each column's four samples into points in x order, each sample once. The
scene draws the latest points as a line through the current view, so a pan
moves it and a zoom stretches it until the new view's line arrives. The y
domain spans the latest line's values, and keeps the previous line's when no
sample lies in the view. The space above and to the right of the plot, and the
y labels' width, have fixed minimums that hold the labels as the axes change,
so the plot stays put.

On native targets, the app's jobs and the host share one multi-thread Tokio
runtime, and the series splits into a partition for each core, so the
decimation runs in parallel. In the browser, the jobs run between the page's
tasks, and the series splits into 16 partitions, since a query yields to the
page between the polls of its partitions' tasks.

## Run the example

Run the example natively:

```sh
cargo run --release -p m4-series
```

Or build the example for the web, serve this directory, and open
`http://localhost:8770`. DataFusion's Arrow IPC support builds zstd's C code,
so the build needs a clang with a wasm target, such as Homebrew's LLVM:

```sh
CC_wasm32_unknown_unknown=/opt/homebrew/opt/llvm/bin/clang AR_wasm32_unknown_unknown=/opt/homebrew/opt/llvm/bin/llvm-ar wasm-pack build --target web --release
python3 -m http.server 8770
```
