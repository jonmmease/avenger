# Avenger Vega Renderer
This crate provides WASM bindings to the pure Rust Avenger crates and a Vega renderer plugin

## Build

Build the package with:

```
pixi run build-vega-renderer
```

This will compile to WASM and copy the WASM and JavaScript files to the `dist/` directory.

## Test
Run the playwright tests with:

```
pixi run test-vega-renderer
```

These tests compare the result of rendering a variety of charts with Vega's default svg renderer and with Avenger. The test app is located in the `test_server/` directory.

## Typecheck
The JavaScript files in the `js/marks` directory use TypeScript compatible JSDoc types, and are type checked with:

```
npm run type-check
```

## Try it out

The `examples/vega-renderer` directory contains a simple app (created with [`create-wasm-app`](https://github.com/rustwasm/create-wasm-app) and then manually updated to WebPack 5) that uses the Vega renderer.

From within the `examples/vega-renderer` directory:

```
npm install
npm run start
```
