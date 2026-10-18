# geo-pan-zoom

A globe and a Web Mercator world map of the same earthquakes, which pan and
zoom on their own in two of avenger-geo's pan modes:

| Map | Projection | Pan mode |
| --- | --- | --- |
| Pacific Ring of Fire | Orthographic | `PanMode::Sphere`: a drag turns the globe as a trackball turns |
| The world | Web Mercator | `PanMode::Longitude`: a sideways drag turns the world, and a vertical drag slides it |

Legends beside the maps show the continents, the earthquakes' magnitudes and
depths, and the plate boundaries.

- Drag a map to pan it. The place under the cursor stays under the cursor,
  anywhere on the globe. The globe turns the shortest way, so it can roll, and
  north can tilt away from up.
- Scroll over a map to zoom it about the cursor. The globe stays in place and
  turns toward the cursor.
- Double-click a map to restore its first view.
- Hover over a map to read the place under the cursor, or an earthquake's
  magnitude, depth, and date.

Each map keeps its projection in the app's state. A drag pans the projection
from where the drag started, and every frame projects the map's layers again.
The graticule draws through `Graticule::view`, and its labels stand in layout
slabs that `Graticule::view_extent` sizes, so the layout keeps its slabs while
the maps move. In a window too short for the maps at its width, the maps
shrink and keep their aspects. The maps draw the world gallery's three layers, which the
example embeds. They are Git LFS objects, so run `git lfs pull` when they are
pointer files. [The data README](../../avenger-guides-geo/examples/data/README.md)
lists their sources and licenses.

Run the example natively:

```sh
cargo run --release -p geo-pan-zoom
```

Or build it for the web, serve this directory, and open
`http://localhost:8766`:

```sh
wasm-pack build --target web --release
python3 -m http.server 8766
```
