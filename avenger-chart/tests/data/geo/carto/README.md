# Offline raster tile fixtures

Basemap raster tiles used by warped-tile visual regression tests
(`avenger-chart-geo/tests/visual_regression.rs`), fetched once from the
CARTO basemap CDN (`basemaps.cartocdn.com/rastertiles/voyager_nolabels`)
and checked in so tests never touch the network.

- `1/{x}/{y}.png` — the four zoom-1 world tiles
- `4/{x}/{y}.png` — the eight zoom-4 tiles covering CONUS (x 2–5, y 5–6)
- `11/{x}/{y}.png` — nine zoom-11 tiles covering NYC (x 602–604, y 768–770),
  used by the categorical taxi raster capstone baselines
- `13/{x}/{y}.png` — nine zoom-13 tiles covering midtown Manhattan
  (x 2411–2413, y 3077–3079), used by the capstone's scatter-regime baseline

Attribution: © OpenStreetMap contributors, © CARTO.
