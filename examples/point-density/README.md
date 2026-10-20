# point-density

Five million points from a mixture of Gaussians, drawn as a heatmap of how many
points fall in each cell of a 400 × 400 grid over the view, which a background
task counts in DataFusion for the view that the pan and zoom settle on.

- Drag the plot to pan it.
- Scroll over the plot to zoom it about the cursor.
- Double-click the plot to restore its first view.

The app's state holds the view's x and y domains, and each gesture changes them
and draws again. A debounced event stream requests the heatmap of the view
once it stays for 20 ms.
The request is an avenger-app `Task` keyed by the view's domains, so a request for
the view that the task is counting, or that the latest heatmap shows, starts
nothing.

The task's job rasterizes the points with avenger-datafusion-rasterize's
`Rasterize`, which counts the rows in each cell of the grid, whatever the
plot's size. The job then shades the counts on a log scale through viridis,
with no color where no point lies, and returns the image with the largest
count. The scene draws the latest image as one mesh image quad over the view
that it was counted for, through the current view, so a pan moves it and a zoom
stretches it until the new view's heatmap arrives. The image stays on the GPU
across the scenes that draw it. The colorbar shows the latest heatmap's counts,
and its slab has a fixed least width, so the plot stays put as the first
heatmap arrives. The readout says when a count fails.

On native targets, the app's jobs and the host share one multi-thread Tokio
runtime, and the points split into a partition for each core, so the counts
run in parallel. In the browser, the jobs run between the page's tasks, and
the points split into 16 partitions, since a query yields to the page between
the polls of its partitions' tasks.

## Run the example

Run the example natively:

```sh
cargo run --release -p point-density
```

Or build the example for the web, serve this directory, and open
`http://localhost:8768`. DataFusion's Arrow IPC support builds zstd's C code,
so the build needs a clang with a wasm target, such as Homebrew's LLVM:

```sh
CC_wasm32_unknown_unknown=/opt/homebrew/opt/llvm/bin/clang AR_wasm32_unknown_unknown=/opt/homebrew/opt/llvm/bin/llvm-ar wasm-pack build --target web --release
python3 -m http.server 8768
```
