# Formatting reference fixtures

This tool generates number and datetime formatting fixtures from D3 and Vega. From the repository root, run these commands with Node 20.19.6:

```sh
cd tools/format-reference
npm ci --ignore-scripts
npm run generate
```

The package lock pins the reference versions. The generator writes `tests/fixtures/upstream.json` in `avenger-format-number-d3` and `avenger-format-datetime-d3`. Rust tests read these files without Node or network access. Review fixture changes when updating dependencies.

Number fixtures store each input's exact `f64` bits as hexadecimal text. Step-format fixtures record Vega's `formatSpan` output, D3's `tickStep` interval, and the largest endpoint magnitude.

Datetime fixtures cover D3 directives in UTC. Separate processes test selected patterns across display timezones and timezone transitions.

The generator reads `en-US`, `de-DE`, `fr-FR`, and `ja-JP` from each crate's `locales/` directory. These definitions come from d3-format 3.1.2 and d3-time-format 4.1.0. Each directory includes the upstream license.

Separate Rust tests cover pattern validation and Avenger's intentional differences from D3 and Vega, including `%j` calendar ordinals and automatic trimming before localization and padding.
