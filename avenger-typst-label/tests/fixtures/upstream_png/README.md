# Upstream PNG references

This corpus compares Avenger's rasterized labels with PNG renders that upstream Typst makes from
the same label sources. It is the ink-level check, for decorations, stretched glyphs, emoji and
anything else a frame comparison can't see. Frame and math IR references live next to it; see
[../README.md](../README.md).

## Files

- `cases.toml`: one entry per case, with `upstream_tests` and a `note` as provenance.
  - `upstream_tests` lists the upstream tests the case reduces. Names are unique across
    upstream's `tests/suite`.
  - Cases without `upstream_tests` were written for Avenger's label audits.
- `src/{id}.typ`: the label source, without page setup.
- `ref/{id}.png`: the upstream render.

## Generating

The generator wraps each source in a box on an auto-sized page with a 128 pt margin, so the
page is the box plus the margin on every side:

```typst
#set page(width: auto, height: auto, margin: 128pt, fill: white)
#set text(font: "Lato", size: 32pt, weight: 500)
#show math.equation: set text(font: "Lete Sans Math", weight: 500)
#show raw: set text(font: "DejaVu Sans Mono")
#box[<source>]
```

Each case sets its own fonts, size and weight. The generator renders at `72 * scale` ppi (`scale`
defaults to 2) with only the fixture fonts:

```sh
cargo run --release -p avenger-typst-label --features upstream-png-parity --bin generate_upstream_png_refs
```

- The CLI must be the pinned release: set `TYPST_BIN` to a binary built from it, or check out
  `../typst` at the pinned commit, which the generator builds with `--locked`.
- `-- --check` renders into `target/typst-parity/check` and fails if any reference differs from
  the checked-in one, without writing to `ref/`.
- Emoji cases set `requires_system_emoji = true`. They use the macOS Apple Color Emoji font,
  which the generator gives upstream and the test registers. The test skips them elsewhere.

Inspect every changed reference before committing it.

## Comparing

`tests/upstream_png_parity.rs` (features `raster` and `upstream-png-parity`) runs two checks per
case, and every case must pass both:

- `metrics`: the label's width and height equal the reference's page size minus the margins,
  within half a pixel. Typst rounds the page to whole pixels.
- `ink`: both images are aligned at the label's logical origin. In every em-sized tile, the
  similarity is one minus the mean, over pixels that are ink in either image, of each pixel's
  largest channel difference.
  The worst tile must reach `MIN_SIMILARITY` (0.90).

`png_comparison_rejects_mutations` checks that the comparison has teeth: no case but the emoji
ones may pass with its text 3% larger, bold, filled dark red or shifted by a pixel, or with `^2`
changed to `^3`, an `e` to `c` or `hat` to `tilde`.

Failing cases write `expected.png`, `actual.png` and `diff.png` to the gitignored
`tests/output/upstream_png/{id}/`.

## Adding a case

1. Reduce an upstream test from `../typst/tests/suite` to one label line:
   - Keep upstream's spelling of the feature under test.
   - Inline set rules as arguments.
   - Write colors as hex, because labels use CSS color names.
2. Add `src/{id}.typ` and a `cases.toml` entry with `upstream_tests`.
3. Regenerate, inspect the new reference, and run the test.
