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
  north can tilt away from up. The cursor is a hand over a map, and grabs while
  a drag pans it.
- Scroll over a map to zoom it about the cursor. The globe stays in place and
  turns toward the cursor.
- Double-click a map to restore its first view.
- Hover over a map to read the place under the cursor below the map.
- Hover over an earthquake to show its date, magnitude, and depth in a tooltip,
  and over a country to show its name and continent and outline it. Click to
  pin the tooltip, and click elsewhere to unpin it. Where earthquakes lie over
  each other, a pinned tooltip's arrows page through them. A drag on a pinned
  tooltip doesn't pan the map, and its text selects like the rest.
- Drag across any text, such as the header, a legend's entries, or the footer,
  to select it, and copy it with Cmd+C or Ctrl+C.
  Double-click selects a word, and triple-click a line. The magnitude legend's
  title holds math, which selects whole and copies as its Typst markup.

Each map keeps its projection in the app's state. A drag pans the projection
from where the drag started, and every frame projects the map's layers again.
A window that loses focus mid-drag may never hear the release, so losing focus
ends the drag where it is.
The graticule draws through `Graticule::view`, and its labels stand in layout
slabs that `Graticule::view_extent` sizes, so the layout keeps its slabs while
the maps move. In a window too short for the maps at its width, the maps
shrink and keep their aspects. The maps draw the world gallery's layers, which
the example embeds: the countries, the plate boundaries, and the earthquakes.
Over tiles, which show the land, the maps draw no countries. The layers are
Git LFS objects, so run `git lfs pull` when they are pointer files.
[The data README](../../avenger-guides-geo/examples/data/README.md) lists
their sources and licenses.

Only the earthquakes and the countries take hits. The app's hit-testing tree
holds them, and the scene builder returns their rows as each map's scene data.
The app's tooltips, an avenger-tooltip `Tooltips` overlay, have two targets,
tried in order: the earthquake within 8 px of the cursor, and else the country
under it. Each target names the columns that its tooltip shows and their
formats. An earthquake's tooltip stands beside it, in a box tinted by its depth
color, with a callout to its edge, and a country's tooltip stands below and
right of the cursor, which it follows across the country, in a box tinted by its
continent's color, over an outline of the country. A hover builds only the
overlay, never the scene or the tree. A drag's moves and a zoom's steps don't rebuild the tree, and the
tooltip hides while they last, unless a click pinned it. The drag's end, the
first move after a zoom, a reset, or a resize rebuilds the tree, and a pinned
tooltip then finds its earthquake or country where it now lies, or hides when
it is off the map. Over tiles, the maps draw no countries, so only the
earthquakes have tooltips.

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
