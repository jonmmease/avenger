# chart-web

A static gallery of chart messages that avenger-chart-web draws in the browser. A build step builds six charts with the builders of `avenger-chart`, over the movies, the penguins, and the iris flowers of the repository, and writes the bytes of each chart's message into a manifest beside the page, with the bundled fonts and the library's `<avenger-chart>` element. The page writes an element for each chart, which loads as it nears the screen, frees its view when it leaves, and draws the chart with its navigation and its tooltip. Its Dark button switches the page's `quarto-dark` class, which every chart follows, and `?font=lobster`, `?font=inter`, or `?font=roboto-slab` draws the charts in a Google font.

`linked.html` holds two charts of the penguins in separate elements, which share a selection. A box drawn in either one reaches the other through the page, which passes each `avenger-change` event's selections to the other element with `setState(bytes, { modes: false, views: false })`, so both fade the penguins outside it, and each keeps its own mode and its own pans and zooms. Its Brush and Pan buttons call `setMode` on both, and the page keeps the mode, the selections, and each chart's views in its address, as parts of the charts' `state()`, so a reload or a copied link restores the brush and the pans and zooms of both charts.

## Run

Write the charts, the fonts, the element, and `manifest.json` into `site/`:

```bash
cargo run --release -p chart-web --bin build-charts
```

Build the library into `site/lib/`:

```bash
avenger-chart-web/scripts/build.sh examples/chart-web/site/lib
```

Serve the site:

```bash
python3 -m http.server 8778 -d examples/chart-web/site
```

Then open `http://localhost:8778/` in a browser with WebGPU. The `chart-web` entry of `.claude/launch.json` serves the same directory.
