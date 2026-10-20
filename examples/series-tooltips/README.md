# series-tooltips

Thirty random walks of 500 samples each, drawn as lines, with a tooltip that
lists the eight series nearest the pointer at the step under it.

- Move the pointer across the plot to read the series at each step.
- Move the pointer up and down to change which eight series the tooltip lists.

Each line's hit policy is `LineHit::Vertices`, so the hit-testing tree holds a
point for each vertex, whose instance index is the vertex's index. The scene
builder returns each series' rows, a step and a value for each vertex, as scene
data under the line's name. The app's tooltips, an avenger-tooltip `Tooltips`
overlay, have one `GroupTarget`, whose entries are those that a legend of the
series would take: each series' line and title, in order. For each line with a
vertex within 20 px of the pointer's x, it finds the vertex nearest the
pointer's x, and of those, the eight nearest the pointer's y. A line that the
tooltip lists keeps its place until another line is nearer by more than a
pixel, so the rows don't flicker between lines that are about as near.

The tooltip has the step as its header, and a row for each listed series, in
the entries' order, with a swatch in the line's stroke, the series' title, and
its value. A crosshair stands at the step, with a marker on each listed line,
and the tooltip's callout points at the crosshair at the pointer's height. A
pointer move builds only the overlay, and never the scene or its hit-testing
tree.

## Run the example

Run the example natively:

```sh
cargo run --release -p series-tooltips
```

Or build the example for the web, serve this directory, and open
`http://localhost:8773`:

```sh
wasm-pack build --target web --release
python3 -m http.server 8773
```
