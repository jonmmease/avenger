# avenger-chart-web

A WebAssembly library that renders avenger chart messages in HTML canvases. A server builds a chart, such as with the builders of `avenger-chart` or a script of avenger-sql, and sends the bytes of its message. A page draws the chart with the tools that the chart declares, such as navigation and tooltips. The library holds no DataFusion, no SQL, and no fonts, and draws through the browser's WebGPU. Its `<avenger-chart>` element, plain JavaScript in `element/`, loads it once for a page and draws each chart as it nears the screen.

## Build

```bash
wasm-pack build avenger-chart-web --target web --profile release-size
```

`scripts/size.sh [OUT_DIR]` builds the package into `pkg/`, or `OUT_DIR`, with `wasm-opt -Oz`, and prints the sizes of its module and its glue. On macOS, a crate with C code builds for wasm32 with Homebrew's LLVM, which the script uses when it is installed.

## The element

`element/avenger-chart.js` is an ES module with no build step. A page calls `start(config)` once, which loads the library, adds the fonts, and defines `<avenger-chart>`:

```js
import { start } from "./avenger-chart.js";

start({
  glue: "./lib/avenger_chart_web.js",
  wasm: "./lib/avenger_chart_web_bg.wasm",
  fonts: ["./fonts/Lato-Bold.ttf", "./fonts/Lato-Light.ttf" /* …the faces of avenger-fonts, in order */],
  styles: { entry: "chart.css", files: { "chart.css": "mark { opacity: 0.8; }" } },
  colorSchemes: ["light", "dark"],
});
```

```html
<avenger-chart width="640" height="400">
  <script type="application/vnd.avenger.chart+base64">…the chart's bytes…</script>
</avenger-chart>
```

- `wasm` and the font files load as they are, or as gzip when a URL ends in `.gz`, which the page decompresses with `DecompressionStream` while the module compiles.
- An element holds its chart's bytes as base64 in its inner script, and its size in CSS pixels in `width` and `height`. On a narrower page its canvas takes the width it gets, and the chart lays out again at that width. It creates its view as it nears the screen and frees it as it leaves.
- With one color scheme, a chart draws in it on its background. With two, a chart draws on a transparent canvas and follows the `quarto-light` and `quarto-dark` classes of `body`, as the light and dark themes of Quarto switch.
- Until its canvas draws, an element takes no room. Inside a Quarto cell or figure, it then hides the cell's other outputs, the chart's images, and shows them again when its view goes away, so a page without WebGPU, or whose library fails to load, keeps its images.

## The library

```js
import init, { ChartView, addFonts } from "./pkg/avenger_chart_web.js";

await init();
addFonts(latoBold); // each face of avenger-fonts, in order, as bytes
const view = await ChartView.create(canvas, bytes, { styles, colorScheme: "dark", transparent: true });
view.setChart(newBytes);
view.setColorScheme("light");
const diagnostics = view.setStyles(newStyles);
view.destroy();
```

- `addFonts(bytes)` registers the faces of a font file for the views made after it.
- `ChartView.create(canvas, bytes, options)` decodes the chart message, builds its app at the canvas's CSS size and the display's pixel ratio, and draws it. It rejects with the decoder's error for an invalid message, which includes a message of another format version, and with an error that says so in a browser without WebGPU. `options` gives the project's `styles`, the `colorScheme`, light by default, whether the view is `transparent`, and `wheel`.
- `setChart(bytes)` replaces the chart, and keeps the views of its scales and its selections where the new chart has the same structure. It throws the decoder's error for an invalid message.
- `setColorScheme(scheme)` draws the chart for a reader who prefers `"light"` or `"dark"`: its background, and the rules of its stylesheets' `prefers-color-scheme` conditions.
- `destroy()` removes the view's listeners and its resize observer, and frees its GPU resources. A page destroys the views that it no longer shows.
- The views of a page share one WebGPU device, so their pipelines compile once, and one label engine, with GoogleSQL's number and date formats, whose syntaxes the charts of avenger-sql name.

## Styles

The `styles` option and `setStyles(styles)` style the chart with a project's stylesheets: `{ entry, files }`, the path of the entry stylesheet and the text of each stylesheet by its path relative to the project's root. The entry stylesheet imports the others by paths relative to itself, as the stylesheets of an avenger-sql project do, and the `std:` stylesheets. The base stylesheet, `std:base.css`, comes first, and the project's stylesheets after it, so their rules win over the base rules. The chart's own style fields win over both, as inline styles do.

The view builds its cascade with `avenger_style::chart_cascade`, as the CLI builds the cascade of its images. Each stylesheet reads the color scheme's background, `#ffffff` or `#1e1e1e`, as its `env(background)`, which the base stylesheet mixes into the colors of guide lines. A transparent view adds `chart { background: transparent; }` after the base stylesheet, so the chart's own background is the page's, unless the project's stylesheets set one. As in CSS, a rule or a declaration that does not parse is skipped, and the others apply. `view.diagnostics` and `setStyles` give the diagnostics of the project's stylesheets: objects with `severity`, `message`, `file`, `line`, and `column`. `setStyles(null)` returns to the base stylesheet. A restyle keeps the chart's views and selections.

## Input

- The canvas takes `tabindex="0"`, so a click or a tap focuses it, and a page can show the focus with `:focus-visible`. Escape gives the focus back to the page, after the chart has the key.
- The `wheel` option is `"focus"` by default: wheel events reach the chart only while its canvas has the focus, so a page scrolls past a chart that the user has not picked. `"always"` sends every wheel event over the canvas to the chart, for a page that does not scroll.
- On touch, one finger scrolls the page, and a tap moves the cursor to its point and clicks there, so tooltips and selections work. Two fingers pan and zoom the chart: the movement of their midpoint is a drag, and the change of their distance is wheel steps at the midpoint. The canvas takes `touch-action: pan-x pan-y`.
- The canvas keeps the pointer during a drag that leaves it, and shows no context menu.

## Fonts

The module embeds no fonts, so the page adds them: the faces that `avenger-fonts` bundles first, in the order of `avenger_fonts::FACES` (Lato Bold and Light before Regular, so that weight ties resolve as CSS resolves them), then its own. Labels name Lato by `sans-serif`, DejaVu Sans Mono by `monospace`, and Lete Sans Math in math, so a chart draws its text as native rendering does. The host that serves the page, such as the gallery's build step, writes the faces from its own copy of `avenger-fonts`, and the fonts keep its licenses. A page's own fonts are TrueType, OpenType, or collection files, such as a Google font by the URL of its full file, which the `google/fonts` repository holds and jsDelivr serves with CORS headers.

## Sizes

`scripts/size.sh` measured these sizes on 2026-11-25:

| File | Raw | gzip | Brotli |
|---|---|---|---|
| `avenger_chart_web_bg.wasm` | 9.52 MB | 3.25 MB | 2.27 MB |
| `avenger_chart_web.js` | 0.08 MB | 0.02 MB | 0.01 MB |

The module holds Arrow's compute kernels, the chart evaluator, the label engine, and wgpu's WebGPU interface. It does not hold `wgpu-core` or `naga`, which only WebGL needs, or the fonts, whose seven faces are 2.09 MB raw and 1.19 MB as gzip.

## Known limitations

- Tooltips draw inside the canvas, so a tooltip near an edge of a small chart is clipped.
- The library does not draw geo plots, whose tiles and projections are a feature of `avenger-chart-eval` that it does not turn on, or image marks, which it does not fetch.
- A page cannot hear selection changes or tooltip contents yet, and cannot set the chart's modes or selections.
