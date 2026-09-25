# Formatting reference fixtures

Generate Rust test expectations from D3 and Vega with Node 20.19.6:

```sh
cd tools/format-reference
npm ci --ignore-scripts
npm run generate
```

The package lock pins the reference implementations. Rust tests consume the generated JSON without Node or network access. Review fixture changes together with dependency upgrades. The fixtures cover shared D3/Vega behavior. Rust tests separately check the float formatter's intentional trimming differences with custom numerals, affixes, and padding.

Step-format fixtures pair Vega's `formatSpan` output with the interval selected by D3's `tickStep`. Rust tests pass that interval and the largest endpoint magnitude to `prepare_number_step_format`.

Number locale definitions come from d3-format 3.1.2. The generator reads the bundled `en-US` definition and the `de-DE`, `fr-FR`, and `ja-JP` definitions in `avenger-format-number/tests/fixtures/locales/`. The upstream license is included beside both sets of locale files.

Regenerate the currency fraction table with `node generate-currencies.mjs`. The script downloads the pinned CLDR 48.0.0 currency data and includes the union of historical region codes and fraction records. Rust formatting tests cover the resulting 0, 2, 3, and 4 digit defaults and explicit precision overrides without network access.
