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

- Drag a map to pan it. The place under the cursor stays under the cursor,
  anywhere on the globe. The globe turns the shortest way, so it can roll, and
  north can tilt away from up.
- Scroll over a map to zoom it about the cursor. The globe stays in place and
  turns toward the cursor.
- Double-click a map to restore its first view.
- Hover over a map to read the place under the cursor below it. Hover near an
  earthquake for a tooltip of its magnitude, depth, and date, and over a
  country for a tooltip of its name and continent.

Each map keeps its projection in the app's state, and an avenger-app-tools
`MapPanZoom` tool for each map pans and zooms the projection in the map's
rectangle, within the map's zoom limits. A drag pans the projection from where
the drag started, and every frame projects the map's layers again.
A press that the window cancels, as when it loses focus before the release,
ends its drag where it is.
The graticule draws through `Graticule::view`, and its labels stand in layout
slabs that `Graticule::view_extent` sizes, so the layout keeps its slabs while
the maps move. In a window too short for the maps at its width, the maps
shrink and keep their aspects. The maps draw the world gallery's layers, which
the example embeds: the countries, the plate boundaries, and the earthquakes.
Over tiles, which show the land, the maps draw no countries. The layers are
Git LFS objects, so run `git lfs pull` when they are pointer files.
[The data README](../../avenger-guides-geo/examples/data/README.md) lists
their sources and licenses.

Only the earthquakes and the countries take hits. The maps' hit-testing tree
holds them, with their rows, which each map's marks share under names of their
own. The readouts and the tooltips draw in scenes over the maps, so a hover
sets only those scenes, and never the maps. The tooltips come from
avenger-app-tools' `Tooltips` tool: the nearest earthquake within 10 pixels of
the cursor takes a tooltip beside it, and otherwise the country under the
cursor takes one beside the cursor.
A drag's moves and a zoom's steps don't rebuild the tree, so the tooltips hide
while the gesture lasts, since the tree shows the maps as they were. The tree
rebuilds when the gesture comes to rest: when a drag pauses for 100 ms or
ends, when the wheel stops for 100 ms, whether or not the cursor moves
meanwhile, and after a double-click or a resize.

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
