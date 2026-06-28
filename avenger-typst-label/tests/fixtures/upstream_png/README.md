# Upstream Typst PNG Parity Fixtures

This directory contains the PNG-only parity corpus for `avenger-typst-label`.
It compares Avenger's label raster output with small reference PNGs generated
by upstream Typst for the same single-line label snippets.

## Scope

- Build PNG parity only here.
- Do not add SVG, PDF, scenegraph, browser, or chart baseline fixtures in this
  suite.
- The generator may call upstream Typst, but the integration test must be
  offline and must only read checked-in `ref/*.png` files.
- Keep snippets label-sized and single-line where possible. Do not copy
  Typst's full page-level render refs into this tree.

## Layout

- `cases.toml`: one manifest entry per case.
- `src/{id}.typ`: label snippet only, without page setup.
- `ref/{id}.png`: curated upstream Typst PNG reference.

Each `cases.toml` entry records the upstream source file/test name so future
agents can trace the case back to `../typst/tests/suite/...`.

## Build And Test

Run from the repository root, always in release mode:

```sh
cargo run --release -p avenger-typst-label --features raster --bin generate_upstream_png_refs
cargo test --release -p avenger-typst-label --features raster upstream_png_parity -- --nocapture
```

The generator expects an upstream Typst checkout at `../typst`. It creates
temporary wrapped Typst documents under `target/typst-parity/src`, decompresses
Avenger's bundled Lato and Lete Sans Math fonts into
`target/typst-parity/fonts`, and calls Typst with explicit font paths plus
`--ignore-system-fonts`.

The test harness does not call upstream Typst. If a comparison fails, it writes
`expected.png`, `actual.png`, and `diff.png` under
`target/tests/upstream_png_parity/{case_id}/`.

## Adding A Case

1. Add a small snippet to `src/{id}.typ`.
2. Add a matching `[[case]]` entry to `cases.toml`.
3. Run the generator command.
4. Review the generated `ref/{id}.png` directly or in a temporary mosaic.
   Confirm it is non-blank, unclipped, and visibly exercises the intended
   feature.
5. Run the parity test command.
6. Commit only `cases.toml`, `src/{id}.typ`, and `ref/{id}.png` with the code
   change that made the feature pass.

Add cases one feature family at a time. Prefer unit tests first, then PNG
parity once the feature is implemented.

## Emoji Cases

Emoji cases should set `requires_system_emoji = true`. On macOS the generator
copies `/System/Library/Fonts/Apple Color Emoji.ttc` into the temporary parity
font directory. If the font is unavailable, the test skips only those cases and
prints the reason.

## Harness Sanity Check

When changing the comparator or generator, temporarily perturb one checked-in
reference PNG, run the parity test, confirm the failure message includes the
case id and artifact paths, then restore/regenerate the reference before
committing.
