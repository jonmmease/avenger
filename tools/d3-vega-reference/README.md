# Formatting reference fixtures

This tool generates number and datetime formatting fixtures from D3 and Vega. From the repository root, run these commands with Node 20.19.6:

```sh
cd tools/d3-vega-reference
npm ci --ignore-scripts
npm run generate
```

The package lock pins the reference versions. The generator writes `tests/fixtures/upstream.json` and `tests/fixtures/ticks.json` in `avenger-format-number-d3` and `avenger-format-datetime-d3`. Rust tests read these files without Node or network access. Review fixture changes when updating dependencies.

Number fixtures store each input's exact `f64` bits as hexadecimal text. Tick fixtures record D3's ticks for a domain with Vega's `formatSpan` labels, and Vega's `formatFloat` labels for values that span magnitudes.

Datetime fixtures cover D3 directives in UTC. Separate processes test selected patterns across display timezones and timezone transitions. Time tick fixtures record d3-time's ticks for local domains in each timezone, with labels from Vega's calendar multi-format.

The generator reads `en-US`, `de-DE`, `fr-FR`, and `ja-JP` from each crate's `locales/` directory. These definitions come from d3-format 3.1.2 and d3-time-format 4.1.0. Each directory includes the upstream license.

Separate Rust tests cover pattern validation and Avenger's intentional differences from D3 and Vega, including `%j` calendar ordinals and automatic trimming before localization and padding.
