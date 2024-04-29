## PNG export in Deno

Build wasm package for Deno in `avenger-vega-renderer/`

```
pixi run build-vega-renderer-deno
```

Then from this directory

```
deno run --allow-net --allow-write --unstable-webgpu export_png.js
```

This should generate a `chart.png` file in the current directory