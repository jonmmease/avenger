# Formatting reference fixtures

Generate Rust test expectations from D3 and Vega with Node 20.19.6:

```sh
cd tools/format-reference
npm ci --ignore-scripts
npm run generate
```

The package lock pins the reference implementations. Local datetime cases run in separate processes with explicit timezones. Rust tests consume the generated JSON without Node or network access. Review fixture changes together with dependency upgrades.

Locale definitions in the formatting crates come from d3-format 3.1.2 and d3-time-format 4.1.0. Their licenses are included beside the locale files.
