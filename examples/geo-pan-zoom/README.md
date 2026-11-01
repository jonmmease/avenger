# geo-pan-zoom

A globe and a Web Mercator world map of the same earthquakes, which pan and
zoom on their own in two of avenger-geo's pan modes:

| Map | Projection | Pan mode |
| --- | --- | --- |
| Pacific Ring of Fire | Orthographic | `PanMode::Sphere`: a drag turns the globe as a trackball turns |
| The world | Web Mercator | `PanMode::Longitude`: a sideways drag turns the world, and a vertical drag slides it |

The example has two basemaps, and the same code draws both:

| Basemap | Native binary | Web page |
| --- | --- | --- |
| Countries, colored by continent | `geo-pan-zoom` | `index.html` |
| OpenStreetMap tiles | `geo-pan-zoom-tiles` | `index.html?tiles` |

Legends beside the maps show the earthquakes' magnitudes and depths, and the
plate boundaries. Over countries, a legend of the continents comes first.

- Drag a map to pan it in the Pan mode, which the Drag buttons in the header
  choose. The place under the cursor stays under the cursor, anywhere on the
  globe. The globe turns the shortest way, so it can roll, and north can tilt
  away from up.
- In the Box mode, drag on a map to draw a box between two meridians and two
  parallels, and drag inside the box or on its edge to move or resize it. A
  click clears the box. The maps ring the earthquakes inside the box, by their
  longitudes and latitudes.
- In the Lasso mode, drag on a map to draw a closed path around earthquakes,
  and Shift+drag to add another. A click clears the paths. The maps ring the
  earthquakes inside them.
- Scroll over a map to zoom it about the cursor. The globe stays in place and
  turns toward the cursor.
- Double-click a map to restore its first view.
- Hover over a map to read the place under the cursor below it. Hover near an
  earthquake for a tooltip of its magnitude, depth, and date, and over a
  country for a tooltip of its name and continent. The Show tooltips check box
  in the header hides the tooltips.
- Drag across text to select it, including the description's two lines.
  Copy the selection with Cmd+C or Ctrl+C. A double-click selects a word,
  and a triple-click selects a line. The equation in the magnitude legend
  selects as a whole and copies as Typst markup. A `TextSelection` tool
  draws highlights in a scene above the others.

A `Brushes` tool from `avenger-app-tools` draws the box and the lasso in its own
scene over both maps. Its `KeyedIntervalBrush` and `KeyedLassoBrush` cover both
maps, keyed by map, through the `MapProjection` of each map's projection from
`avenger-coords`, and claim presses only in their modes, so a `KeyedMapPanZoom`
pans in the Pan mode. Each keeps one selection, in longitudes and latitudes,
that both maps show through their own projections, so a drag on one map moves
the box or the lasso on the other too. Both maps ring the selected earthquakes.

Each map stores its projection in app state. A `KeyedMapPanZoom` tool from
`avenger-app-tools`, keyed by map, pans and zooms each projection in the map's
own mode, within its rectangle and zoom limits. Dragging uses the projection
saved at the press. Each pan or zoom update projects the layers again.
Cancellation, such as focus loss, ends the drag at its last processed position.

`Graticule::view` draws the graticule. `Graticule::view_extent` sizes the layout
slabs for its labels, keeping the layout stable while maps move. Maps shrink
while preserving their aspect ratios if the window is too short. The embedded
world gallery data supplies countries, plate boundaries, and earthquakes. The
tile basemap omits country marks. If these Git LFS objects are still pointer
files, run `git lfs pull`. [The data
README](../../avenger-guides-geo/examples/data/README.md) lists their sources
and licenses.

Only earthquakes and countries participate in hit testing. Their marks share
source rows under separate names for each map. Readouts and tooltips occupy
separate scenes above the maps, so hovering updates those scenes without
rebuilding the maps. `Tooltips` from `avenger-app-tools` prefers the nearest
earthquake within 10 pixels, with a box beside it. Otherwise, it describes the
country under the cursor in a box beside the cursor.

Pan and zoom updates leave the map tree stale, so tooltips hide until it
rebuilds. A drag requests a tree rebuild after a 100 ms pause, on release, or on
cancellation. Wheel zoom requests one after the wheel stops for 100 ms,
regardless of pointer movement. Double-click and resize also rebuild the tree.

## Tiles

Over tiles, each map draws its basemap with avenger-guides-geo's `TileLayer`,
and both fetch their tiles through one avenger-image `ImageCache`, which wakes
the app through `AvengerApp::with_images` as tiles arrive. Each tile draws on
the frame after it arrives, and until then, the coarser tile that the cache
holds draws in its place. When the cursor rests over a map for 150 ms, a
debounced event stream prefetches the tiles that zooming one tile zoom in or out
there would show, without rebuilding the scene, so a zoom at the cursor shows
its tiles at once.
A map zooms in until its tiles reach their source's deepest zoom. Over
countries, it zooms in to 400 times its first scale.

On native targets, the fetches send the example's User-Agent and keep the
tiles in a disk cache in `target/geo-pan-zoom-tiles`, as OpenStreetMap's
[tile usage policy](https://operations.osmfoundation.org/policies/tiles/)
asks. In the browser, the browser's HTTP cache keeps them. The credits along
the bottom credit the tiles. `--tiles <TEMPLATE>` draws the tiles of another
server, where `<TEMPLATE>` is a URL with `{z}`, `{x}`, and `{y}`, as in:

```sh
cargo run --release -p geo-pan-zoom --bin geo-pan-zoom-tiles -- --tiles 'https://tile.openstreetmap.org/{z}/{x}/{y}.png'
```

## Run the example

Run the maps over countries natively:

```sh
cargo run --release -p geo-pan-zoom
```

Run the maps over tiles natively:

```sh
cargo run --release -p geo-pan-zoom --bin geo-pan-zoom-tiles
```

Or build the example for the web, serve this directory, and open
`http://localhost:8766` for countries, or `http://localhost:8766/?tiles` for
tiles:

```sh
wasm-pack build --target web --release
python3 -m http.server 8766
```
