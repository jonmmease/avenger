# Avegner wasm
This crate provides WASM bindings to the pure Rust avenger crates and a Vega renderer plugin

## Setup

First, [install wasm-pack](https://rustwasm.github.io/wasm-pack/installer/)

Then use wasm-pack to compile the crate from within this directory

```
npm run build
```

## Try it out

The `examples/vega-renderer` directory contains a simple app (created with [`create-wasm-app`](https://github.com/rustwasm/create-wasm-app) and then manually updated to WebPack 5) that uses the Vega renderer.

From within the `examples/vega-renderer` directory:

```
npm install
npm run start
```
