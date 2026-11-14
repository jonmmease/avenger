# chart-web

A static gallery of chart messages that avenger-chart-web draws in the browser. A build step builds six charts with the builders of `avenger-chart`, over the movies, the penguins, and the iris flowers of the repository, and writes the bytes of each chart's message beside the page. The page mounts each chart with the wrapper of the package, which loads the chart as it nears the screen and frees its view when it leaves, and draws it with its navigation and its tooltip. Its Dark button restyles every chart with a page stylesheet, including the charts that have not loaded yet, and `?font=lobster`, `?font=inter`, or `?font=roboto-slab` draws the charts in a Google font, which loads with the library.

## Run

Build the npm package of avenger-chart-web in `avenger-chart-web/js`:

```bash
cd avenger-chart-web/js && npm ci && npm run build
```

Write the charts into `site/charts` and `site/manifest.json`:

```bash
cargo run --release -p chart-web --bin build-charts
```

Link the package into the site, and serve the site:

```bash
ln -sfn ../../../avenger-chart-web/js examples/chart-web/site/avenger-chart-web
```

```bash
python3 -m http.server 8778 -d examples/chart-web/site
```

Then open `http://localhost:8778/` in a browser with WebGPU. The `chart-web` entry of `.claude/launch.json` serves the same directory.
