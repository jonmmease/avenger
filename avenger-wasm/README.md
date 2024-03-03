# Avegner wasm
This crate provides WASM bindings to the pure Rust avenger crates.

## Setup

First, [install wasm-pack](https://rustwasm.github.io/wasm-pack/installer/)

Then use wasm-pack to compile the crate from within this directory

```
wasm-pack build
```

## Try it out

The `avenger-wasm-app` directory contains a simple app (created with [`create-wasm-app`](https://github.com/rustwasm/create-wasm-app) and then manually updated to WebPack 5) that invokes DataFusion and writes results to the browser console.

From within the `avenger-wasm/avenger-wasm-app` directory:

```
npm install
npm run start
```
