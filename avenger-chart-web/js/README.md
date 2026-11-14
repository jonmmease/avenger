# The npm package of avenger-chart-web

This directory builds the npm package `avenger-chart-web`, which `../README.md` describes for its users. The package is private until it is published.

## Build

```bash
npm ci
npm run build
```

`npm run build` runs `scripts/build.sh`, which:

1. Runs `../scripts/size.sh wasm`, which builds the WebAssembly module with the `release-size` profile and `wasm-opt -Oz`, and writes the module, its glue, and its types into `wasm/`.
2. Runs `buf generate` with `buf.gen.yaml` over `../../avenger-chart/proto`, which writes the bindings of the chart format into `src/gen/`.
3. Runs `tsc`, which compiles `src/` into `dist/`.
4. Copies the repository's `LICENSE` into `LICENSE`, and the licenses of the fonts that the module embeds, from `avenger-fonts`, into `licenses/`, which the package ships with the fonts.

`wasm/`, `src/gen/`, `dist/`, `LICENSE`, `licenses/`, and `test-output/` are build output, which Git ignores.

## Layout

| Path | Holds |
|---|---|
| `src/index.ts` | the entry `avenger-chart-web`: `configure`, `preload`, `mount`, and the types |
| `src/loader.ts` | the shared load of the module and the fonts |
| `src/handle.ts` | a chart's handle: its latest chart and stylesheet, its state, and its promises |
| `src/mount.ts` | `mount`: the container's states, its canvas, and the observers of the screen |
| `src/proto.ts` | the entry `avenger-chart-web/proto` over the generated bindings |
| `wasm/` | the entry `avenger-chart-web/core`, which wasm-bindgen writes |

The wrapper imports the core library only through a dynamic `import()`, and never imports `src/proto.ts`, so a page that mounts charts loads neither the module nor the bindings until it needs them.

## Tests

```bash
npm run check
npm test
```

`npm run check` type-checks the package and its tests. `npm test` runs `scripts/test.sh`, which writes the gallery's charts with `build-charts` into `test-output/site`, compiles the tests, and runs them with `node --test`. The tests of `src/handle.ts` drive a handle with a host that records what the handle asks of the page, so they need no browser. The browser checks run the examples in `examples/chart-web` and `examples/chart-server`.
