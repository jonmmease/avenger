# avenger-chart-web

A WebAssembly library that renders avenger chart messages in HTML canvases. A server builds a chart, such as with the builders of `avenger-chart`, and sends the bytes of its message. A page draws the chart with the tools that the chart declares, such as navigation and tooltips. The library holds no DataFusion and no SQL, embeds its fonts, and draws through the browser's WebGPU.

## Build

```bash
wasm-pack build avenger-chart-web --target web --profile release-size
```

`scripts/size.sh` builds the package into `pkg/` with `wasm-opt -Oz`, and prints the sizes of its module and its glue. On macOS, a crate with C code builds for wasm32 with Homebrew's LLVM, which the script uses when it is installed.

## Use

```js
import init, { ChartView } from "./pkg/avenger_chart_web.js";

await init();
const view = await ChartView.create(canvas, bytes, { stylesheet: css, wheel: "focus" });
view.setChart(newBytes);
const diagnostics = view.setStylesheet(darkCss);
view.destroy();
```

- `loadFonts(url)` fetches a font file and registers its fonts, after the fonts that the module embeds, with the views made after the load. `ChartView.create` waits for the loads that the page started.
- `ChartView.create(canvas, bytes, options)` decodes the chart message, builds its app at the canvas's CSS size and the display's pixel ratio, and draws it. It rejects with the decoder's error for an invalid message, and with an error that says so in a browser without WebGPU.
- `setChart(bytes)` replaces the chart, and keeps the views of its scales and its selections where the new chart has the same structure. It throws the decoder's error for an invalid message.
- `destroy()` removes the view's listeners and its resize observer, and frees its GPU resources. A page destroys the views that it no longer shows.
- The views of a page share one WebGPU device, so their pipelines compile once.

## Styles

The `stylesheet` option and `setStylesheet(css)` style the chart with CSS text from the page. The base stylesheet, `std:base.css`, comes first, and the page's stylesheet after it, so the page's rules win over the base rules. The chart's own style fields win over both, as inline styles do. A page stylesheet that imports `std:reset.css` removes the declarations of the base stylesheet.

```css
chart { background: #1e1e21; color: #ececee; }
grid { stroke: #303036; }
```

Only the `std:` stylesheets import, because the library fetches no stylesheets. A page that has several stylesheets joins their text. As in CSS, a rule or a declaration that does not parse is skipped, and the others apply. `view.diagnostics` holds the diagnostics of the `stylesheet` option, and `setStylesheet` returns the diagnostics of its stylesheet: objects with `severity`, `message`, `line`, and `column`. The library logs nothing. `setStylesheet(null)` returns to the base stylesheet. A restyle keeps the chart's views and selections.

The base stylesheet mixes the colors of guide lines with the background that it reads from `env(background)`, which a page cannot set yet. A page that changes the background also sets the colors of the guide lines, as the dark theme above does.

## Input

- The canvas takes `tabindex="0"`, so a click or a tap focuses it, and a page can show the focus with `:focus-visible`. Escape gives the focus back to the page, after the chart has the key.
- The `wheel` option is `"focus"` by default: wheel events reach the chart only while its canvas has the focus, so a page scrolls past a chart that the user has not picked. `"always"` sends every wheel event over the canvas to the chart, for a page that does not scroll.
- On touch, one finger scrolls the page, and a tap moves the cursor to its point and clicks there, so tooltips and selections work. Two fingers pan and zoom the chart: the movement of their midpoint is a drag, and the change of their distance is wheel steps at the midpoint. The canvas takes `touch-action: pan-x pan-y`.
- The canvas keeps the pointer during a drag that leaves it, and shows no context menu.

## Fonts

The module embeds the fonts that `avenger-fonts` bundles: Lato in four styles, DejaVu Sans Mono, and Lete Sans Math in two weights. Labels name Lato by `sans-serif`, DejaVu Sans Mono by `monospace`, and Lete Sans Math in math, so a chart draws its text as native rendering does, with nothing to load. The fonts keep the licenses of `avenger-fonts`. A page adds its own fonts with `loadFonts`: font files that are TrueType, OpenType, or collections, such as a Google font by the URL of its full file, which the `google/fonts` repository holds and jsDelivr serves with CORS headers. The page's fonts come after the bundled fonts, so the bundled fonts remain the fonts of math and code.

## Sizes

`scripts/size.sh` measured these sizes on 2026-11-11:

| File | Raw | gzip | Brotli |
|---|---|---|---|
| `avenger_chart_web_bg.wasm` | 10.62 MB | 4.26 MB | 3.24 MB |
| `avenger_chart_web.js` | 0.08 MB | 0.02 MB | 0.01 MB |

The module holds Arrow's compute kernels, the chart evaluator, the label engine and its fonts, and wgpu's WebGPU interface. It does not hold `wgpu-core` or `naga`, which only WebGL needs.

## Known limitations

- Tooltips draw inside the canvas, so a tooltip near an edge of a small chart is clipped.
- The library does not draw geo plots, whose tiles and projections are a feature of `avenger-chart-eval` that it does not turn on.
- A page cannot hear selection changes or tooltip contents yet, and cannot set the chart's modes or selections.
