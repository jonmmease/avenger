# Formatting reference fixtures

Generate Rust test expectations from D3 and Vega with Node 20.19.6:

```sh
cd tools/format-reference
npm ci --ignore-scripts
npm run generate
```

The package lock pins the reference implementations. Rust tests consume the generated JSON without Node or network access. Review fixture changes together with dependency upgrades. The fixtures cover shared D3/Vega behavior. Rust tests separately check the float formatter's intentional trimming differences with custom numerals, affixes, and padding.

Number locale definitions come from d3-format 3.1.2. Its license is included beside the locale files.
