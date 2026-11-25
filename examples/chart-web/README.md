# chart-web

A static gallery of chart messages that avenger-chart-web draws in the browser. A build step builds six charts with the builders of `avenger-chart`, over the movies, the penguins, and the iris flowers of the repository, and writes the bytes of each chart's message into a manifest beside the page, with the bundled fonts and the library's `<avenger-chart>` element. The page writes an element for each chart, which loads as it nears the screen, frees its view when it leaves, and draws the chart with its navigation and its tooltip. Its Dark button switches the page's `quarto-dark` class, which every chart follows, and `?font=lobster`, `?font=inter`, or `?font=roboto-slab` draws the charts in a Google font.

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
