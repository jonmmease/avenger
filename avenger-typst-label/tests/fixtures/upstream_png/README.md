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

## Agent Instructions

Use this suite when an agent needs to validate that `avenger-typst-label`
matches upstream Typst for supported single-line label features. Build or
extend only the PNG path first. The generator may depend on a local upstream
Typst checkout at `../typst`, but the Rust test must be offline and compare
against checked-in references only.

Agent checklist:

1. Read this README and `cases.toml` before editing.
2. Add one supported feature family at a time.
3. Add label-only snippets under `src/{id}.typ`.
4. Add matching `[[case]]` entries with upstream file/test attribution.
5. Regenerate references with the release-mode generator.
6. Inspect every changed `ref/*.png` directly or in a temporary mosaic.
7. Run the PNG parity test in release mode.
8. If label-engine code changed, run the full crate raster suite.
9. Stage only suite files and intentional crate metadata.

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

## Build Existing Suite

Use this path when the suite already exists and an agent is adding cases or
updating references for implemented label behavior:

1. Run `git status --short` and identify unrelated dirty files before editing.
2. Read `cases.toml` and this README. Keep the suite PNG-only.
3. Add or update one feature family at a time. Prefer unit tests first, then add
   small label-only snippets under `src/{id}.typ`.
4. Add matching `[[case]]` entries with upstream file/test attribution.
5. Run the reference generator:

   ```sh
   cargo run --release -p avenger-typst-label --features raster --bin generate_upstream_png_refs
   ```

6. Inspect every changed `ref/*.png` directly or in a temporary mosaic. Confirm
   each image is non-blank, unclipped, uses the intended Lato/Lete Sans Math
   fonts, and visibly exercises the intended feature.
7. Run the parity test:

   ```sh
   cargo test --release -p avenger-typst-label --features raster upstream_png_parity -- --nocapture
   ```

8. If label-engine code changed, run the full crate raster suite:

   ```sh
   cargo test --release -p avenger-typst-label --features raster -- --nocapture
   ```

9. Review `git diff` and stage only the intentional generator/test changes,
   `cases.toml`, source snippets, and curated `ref/*.png` files.

## Agent Build Checklist

When building or extending this suite:

1. Keep this PNG-only. Do not add SVG/PDF fixtures, chart baselines,
   scenegraph tests, browser tests, or renderer integration here.
2. Add or update `cases.toml` first, with upstream file/test attribution.
3. Add label-only snippets under `src/{id}.typ`.
4. Run the generator in release mode to create `ref/{id}.png`.
5. Visually inspect every generated reference directly or in a temporary
   mosaic. It must be non-blank, unclipped, and visibly exercise the intended
   feature.
6. Run the parity test in release mode.
7. For generator/comparator changes, perturb one ref, confirm the test writes
   useful `expected.png`, `actual.png`, and `diff.png` artifacts, then
   restore/regenerate the ref.
8. Commit only `cases.toml`, `src/{id}.typ`, `ref/{id}.png`, the generator,
   the test harness, and required crate metadata. Never commit `target/`
   artifacts or upstream Typst render-reference images.

## Fresh Agent Handoff

Use this as the implementation prompt for a new agent:

```text
Build or extend the PNG-only upstream Typst parity suite for
avenger-typst-label.

Scope is PNG only. Do not add SVG/PDF parity, chart baselines, scenegraph
tests, browser tests, or renderer integration. Keep everything inside
avenger-typst-label and behind the existing raster feature.

If the suite already exists, follow the "Build Existing Suite" section in
tests/fixtures/upstream_png/README.md. If it does not exist, follow the
"Build From Zero" section. In both cases, start with PNG only; SVG/PDF parity
is intentionally deferred.

Use upstream Typst only in the reference generator. The integration test must
be offline and must compare Avenger raster output against checked-in curated
ref/*.png files. Run every command in release mode.

Required files:
- src/bin/generate_upstream_png_refs.rs
- tests/upstream_png_parity.rs
- tests/fixtures/upstream_png/README.md
- tests/fixtures/upstream_png/cases.toml
- tests/fixtures/upstream_png/src/{id}.typ
- tests/fixtures/upstream_png/ref/{id}.png

Required commands:
  cargo run --release -p avenger-typst-label --features raster --bin generate_upstream_png_refs
  cargo test --release -p avenger-typst-label --features raster upstream_png_parity -- --nocapture
  cargo test --release -p avenger-typst-label --features raster -- --nocapture

Before committing, inspect every generated ref PNG directly or in a temporary
mosaic. Confirm each image is non-blank, unclipped, uses the intended
Lato/Lete Sans Math fonts, and visibly exercises the intended feature. For
generator or comparator changes, temporarily perturb one reference PNG and
confirm the test fails with useful expected.png, actual.png, and diff.png
artifacts. Then restore/regenerate the reference and rerun the suite.

Commit only generator/test code, cases.toml, label snippets, curated ref PNGs,
and required crate metadata. Never commit target artifacts, temporary mosaics,
platform emoji font files, or upstream Typst render-reference images.
```

## Build From Zero

If this suite needs to be rebuilt deliberately:

1. Create `tests/fixtures/upstream_png/{src,ref}` and a manifest-driven
   `cases.toml`.
2. Start with exactly two or three smoke cases: one text decoration case, one
   math fraction/root case, and one symbol case. Add emoji only after the
   deterministic emoji font path is proven.
3. Implement `src/bin/generate_upstream_png_refs.rs`.
   - Read and sort `cases.toml` by `id`.
   - Wrap each label snippet in a tiny auto-sized Typst page with white fill and
     4pt inset.
   - Decompress Avenger's bundled Lato and Lete Sans Math fonts into
     `target/typst-parity/fonts`.
   - Call `../typst/crates/typst-cli` with explicit `--font-path`,
     `--ignore-system-fonts`, and `--ppi 72`.
   - Fail clearly if the upstream Typst checkout is missing.
4. Run the generator and inspect every generated reference PNG.
5. Implement `tests/upstream_png_parity.rs`.
   - Read the same manifest; do not hard-code cases.
   - Compile through `LabelEngine`, rasterize through `rasterize`, composite
     transparent output over white, crop expected/actual to content plus
     padding, and compare dimensions plus similarity.
   - On failure, write `expected.png`, `actual.png`, and `diff.png` under
     `target/tests/upstream_png_parity/{case_id}/`.
6. Prove the harness by perturbing one checked-in reference PNG, confirming the
   failure output is useful, restoring/regenerating, and rerunning the suite.
7. Commit the suite in a focused commit.

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
