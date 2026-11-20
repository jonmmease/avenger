# avenger-chart-web

A WebAssembly library that renders avenger chart messages in HTML canvases, and its npm package. A server builds a chart, such as with the builders of `avenger-chart`, and sends the bytes of its message. A page draws the chart with the tools that the chart declares, such as navigation and tooltips. The library holds no DataFusion and no SQL, embeds its fonts, and draws through the browser's WebGPU.

The npm package in `js/` has three entries:

| Entry | Holds | Loads |
|---|---|---|
| `avenger-chart-web` | a wrapper that loads charts as they near the screen: `mount`, `configure`, `preload` | at once, with the page |
| `avenger-chart-web/core` | the library itself: `ChartView`, `loadFonts`, `init` | when the wrapper or the page imports it |
| `avenger-chart-web/proto` | TypeScript bindings of the chart format | when the page imports it |

## Build

```bash
cd avenger-chart-web/js && npm ci && npm run build
```

`npm run build` runs `scripts/size.sh`, which builds the module with the `release-size` profile and `wasm-opt -Oz` into `js/wasm/`, generates the bindings, and compiles the TypeScript into `js/dist/`. On macOS, a crate with C code builds for wasm32 with Homebrew's LLVM, which `size.sh` uses when it is installed. `js/README.md` describes the package.

## Use

```js
import { mount } from "avenger-chart-web";

for (const element of document.querySelectorAll("[data-chart]")) {
  mount(element, element.dataset.chart, { label: element.title });
}
```

`mount(container, source, options)` returns a handle at once, and a page awaits nothing. The wrapper loads the module once for the page, when the first chart nears the screen, and creates each chart's view as its container comes within `margin` of the screen. The container sets the chart's size, and the chart fills it. A source is the bytes of a chart message, or the URL of the bytes, which the wrapper fetches when the chart needs them.

| Member of the handle | Behavior |
|---|---|
| `state` | `"idle"`, `"loading"`, `"ready"`, `"error"`, or `"destroyed"` |
| `ready` | resolves when the chart first becomes interactive, and rejects when it fails before that |
| `setChart(source)` | replaces the chart, and resolves when the view takes it, or rejects with the reader's error |
| `setStylesheet(css)` | replaces the stylesheet, or returns to the base stylesheet for `null`, and resolves to its diagnostics |
| `destroy()` | removes the chart from its container and ends the handle |

`setChart` and `setStylesheet` work at any time. Before the chart is ready, they replace the handle's latest chart and stylesheet, and the view is created with the latest of each. After it is ready, they go to the view, which keeps its pans, zooms, and selections where the new chart has the same structure. A call that a later call replaces settles with the result of the later call. The wrapper handles every promise that it returns, so an ignored promise raises no `unhandledrejection` event.

| Option of `mount` | Default | Meaning |
|---|---|---|
| `lazy` | `"visible"` | `"visible"` loads the chart as its container nears the screen, and `"eager"` loads it at once |
| `margin` | `"200px"` | how near the screen a container is before its chart loads |
| `release` | `"offscreen"` | `"offscreen"` frees the chart's view when its container leaves three times `margin`, and `"never"` keeps it |
| `stylesheet` | none | the chart's stylesheet |
| `wheel` | `"focus"` | when wheel events reach the chart, as [Input](#input) describes |
| `label` | none | the accessible name of the canvas, which takes `role="img"` |
| `onError` | logs with `console.error` | the callback of the chart's failures |

A released chart frees its view, its canvas, and its GPU buffers, about 14 MB for a 520 by 340 pixel chart at a pixel ratio of 2. When it comes back, the wrapper creates a view from the latest chart and stylesheet, at the chart's fitted domains, so the chart loses the pans and zooms of the freed view. A page whose charts must keep them sets `release: "never"`.

### States

The container carries the chart's state in its `data-avenger-chart` attribute. While a chart loads, and while a chart near the screen waits for its source, the container shows a slow, faint pulse of its own text color, from 150 ms after the load starts, so a chart from the cache shows none. With `prefers-reduced-motion: reduce`, the pulse is still. After a failure, the container shows one line of muted text: "This chart needs WebGPU" in a browser without WebGPU, and "This chart did not load" otherwise. The wrapper's styles have no specificity, so a page's own rules win:

```css
.chart[data-avenger-chart] { height: 420px; color: #888; }
.chart .avenger-chart-status > span { font-size: 14px; }
```

A container without a height of its own is 300 pixels high. A chart in `error` takes a new source through `setChart`, which tries the chart again. A failure of the shared load, such as a font file that does not load, remains.

### Configuration and caching

`configure(options)` sets where the shared load finds the module and the page's fonts, before the load starts, and `preload()` starts the load before any chart needs it.

| Option of `configure` | Default |
|---|---|
| `moduleUrl` | the package's module, beside its glue |
| `extraFonts` | none: the URLs of the page's own font files, as [Fonts](#fonts) describes |

The glue finds the module with `new URL(..., import.meta.url)`, so a bundler such as Vite, webpack 5, or esbuild emits it as a hashed asset, which the browser caches for as long as their content does not change. The core library loads through a dynamic `import()`, which a bundler puts in a chunk of its own. A page that serves the module from a CDN sets `moduleUrl`.

## The core library

A page that manages loading itself imports `avenger-chart-web/core`:

```js
import init, { ChartView, loadFonts } from "avenger-chart-web/core";

await init();
const view = await ChartView.create(canvas, bytes, { stylesheet: css, wheel: "focus" });
view.setChart(newBytes);
const diagnostics = view.setStylesheet(darkCss);
view.destroy();
```

- `loadFonts(url)` fetches a font file and registers its fonts, after the fonts that the module embeds, with the views made after the load. `ChartView.create` waits for the loads that the page started.
- `ChartView.create(canvas, bytes, options)` decodes the chart message, builds its app at the canvas's CSS size and the display's pixel ratio, and draws it. It rejects with the decoder's error for an invalid message, and with an error that says so in a browser without WebGPU.
- `setChart(bytes)` replaces the chart, and throws the decoder's error for an invalid message.
- `destroy()` removes the view's listeners and its resize observer, and frees its GPU resources.
- The views of a page share one WebGPU device, so their pipelines compile once.

## Styles

The `stylesheet` option and `setStylesheet(css)` style the chart with CSS text from the page. The base stylesheet, `std:base.css`, comes first, and the page's stylesheet after it, so the page's rules win over the base rules. The chart's own style fields win over both, as inline styles do. A page stylesheet that imports `std:reset.css` removes the declarations of the base stylesheet.

```css
chart { background: #1e1e21; color: #ececee; }
grid { stroke: #303036; }
```

Only the `std:` stylesheets import, because the library fetches no stylesheets. A page that has several stylesheets joins their text. As in CSS, a rule or a declaration that does not parse is skipped, and the others apply. The diagnostics of a stylesheet are objects with `severity`, `message`, `line`, and `column`, and the library logs none of them. A restyle keeps the chart's views and selections.

The base stylesheet mixes the colors of guide lines with the background that it reads from `env(background)`, which a page cannot set yet. A page that changes the background also sets the colors of the guide lines, as the dark theme above does.

## Input

- The canvas takes `tabindex="0"`, so a click or a tap focuses it, and a page can show the focus with `:focus-visible`. Escape gives the focus back to the page, after the chart has the key.
- The `wheel` option is `"focus"` by default: wheel events reach the chart only while its canvas has the focus, so a page scrolls past a chart that the user has not picked. `"always"` sends every wheel event over the canvas to the chart, for a page that does not scroll.
- On touch, one finger scrolls the page, and a tap moves the cursor to its point and clicks there, so tooltips and selections work. Two fingers pan and zoom the chart: the movement of their midpoint is a drag, and the change of their distance is wheel steps at the midpoint. The canvas takes `touch-action: pan-x pan-y`.
- The canvas keeps the pointer during a drag that leaves it, and shows no context menu.

## Fonts

The module embeds the fonts that `avenger-fonts` bundles: Lato in four styles, DejaVu Sans Mono, and Lete Sans Math in two weights. Labels name Lato by `sans-serif`, DejaVu Sans Mono by `monospace`, and Lete Sans Math in math, so a chart draws its text as native rendering does, with nothing to load. The fonts keep the licenses of `avenger-fonts`, which the npm package ships in `licenses/`. A page adds its own fonts with `extraFonts`, or `loadFonts` in the core library: font files that are TrueType, OpenType, or collections.

### Fonts from Google Fonts

A page takes a Google font by the URL of its full file, which the `google/fonts` repository holds and jsDelivr serves with CORS headers, and names the family in the chart's stylesheet:

```js
import { configure, mount } from "avenger-chart-web";

const GOOGLE_FONTS = "https://cdn.jsdelivr.net/gh/google/fonts@c30b221beb0c1da59d563feb91ec8656a1fa972a";
configure({ extraFonts: [`${GOOGLE_FONTS}/ofl/inter/Inter%5Bopsz,wght%5D.ttf`] });
mount(element, source, { stylesheet: `chart { font-family: "Inter", sans-serif; }` });
```

- Pin a commit of the repository, so the file does not change under the page. A family's path names its license directory, such as `ofl` or `apache`, and its file, which is a variable font such as `Inter[opsz,wght].ttf` for most families, with the brackets URL-encoded.
- For a variable font, the engine reads the weight and optical-size axes, so each weight draws as it does in the page.
- Google's stylesheets, such as `https://fonts.googleapis.com/css2?family=Inter`, do not serve these files: they serve each family split into WOFF2 files for ranges of characters, and the engine draws a family from one file. A page gives `extraFonts` the full file instead.
- The page's fonts come after the bundled fonts, so the bundled fonts remain the fonts of math and code.

## Chart messages in TypeScript

`avenger-chart-web/proto` holds TypeScript bindings of the chart format, which protobuf-es generates from `avenger-chart/proto`. A page or a Node program reads and changes a chart message with them, without the module or WebGPU:

```ts
import { decodeChart, encodeChart } from "avenger-chart-web/proto";

const chart = decodeChart(bytes);
const root = chart.root?.node;
if (root?.case === "plot" && root.value.title) {
  root.value.title.text = "A new title";
}
await handle.setChart(encodeChart(chart));
```

- Each message and enum of the format is a TypeScript type, with the comments of its `.proto` file as its documentation. The entry also exports the generated schemas, for the JSON form and the other functions of `@bufbuild/protobuf`, its one run-time dependency.
- The data tables and `Chart.values` are Arrow IPC streams, which the bindings give as bytes. A page that reads their rows decodes them with a library of its own, such as `apache-arrow`.
- The bindings do not check charts or infer defaults. The module's reader checks a changed chart when the page gives it to `setChart`, and rejects an invalid chart with its error.
- The bindings and the module of one release read the same version of the format.

## Sizes

`scripts/size.sh` and esbuild measured these sizes on 2026-11-11:

| File | Raw | gzip | Brotli |
|---|---|---|---|
| `avenger_chart_web_bg.wasm` | 10.62 MB | 4.26 MB | 3.24 MB |
| `avenger_chart_web.js` | 0.08 MB | 0.02 MB | 0.01 MB |
| the wrapper, minified | 9.2 KB | 3.5 KB | not measured |
| `avenger-chart-web/proto` with its runtime, minified | 143 KB | 39 KB | 34 KB |

The module holds Arrow's compute kernels, the chart evaluator, the label engine and its fonts, and wgpu's WebGPU interface. It does not hold `wgpu-core` or `naga`, which only WebGL needs.

## Known limitations

- Tooltips draw inside the canvas, so a tooltip near an edge of a small chart is clipped.
- The library does not draw geo plots, whose tiles and projections are a feature of `avenger-chart-eval` that it does not turn on.
- A page cannot hear selection changes or tooltip contents yet, and cannot set the chart's modes or selections.

## Examples

- `examples/chart-web`: a static gallery of chart messages that a build step writes, mounted with the wrapper.
- `examples/chart-server`: a server that builds a chart on each request, and a page that mounts it and gives it each answer.
