# Offline raster tile fixtures

Basemap raster tiles used by warped-tile visual regression tests
(`avenger-chart-geo/tests/visual_regression.rs`), fetched once from the
CARTO basemap CDN (`basemaps.cartocdn.com/rastertiles/voyager_nolabels`)
and checked in so tests never touch the network.

- `1/{x}/{y}.png` — the four zoom-1 world tiles
- `4/{x}/{y}.png` — the eight zoom-4 tiles covering CONUS (x 2–5, y 5–6)

Attribution: © OpenStreetMap contributors, © CARTO.
