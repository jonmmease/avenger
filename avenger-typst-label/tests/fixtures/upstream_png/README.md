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

This is intentionally the first and only upstream-parity test suite for now.
Do not prepare SVG or PDF parity scaffolding while building this suite. If a
case exposes an SVG/PDF concern, record it in the scratch plan and keep the PNG
case focused on raster equivalence.

Copy-paste first-build prompt for an implementation agent:

```text
Build the PNG-only upstream Typst parity suite for avenger-typst-label.

Scope is PNG only. Do not add SVG/PDF parity, chart baselines, scenegraph
tests, browser tests, or renderer integration. Keep everything inside
avenger-typst-label and behind the existing raster feature.

Create or repair:
- src/bin/generate_upstream_png_refs.rs
- tests/upstream_png_parity.rs
- tests/fixtures/upstream_png/README.md
- tests/fixtures/upstream_png/cases.toml
- tests/fixtures/upstream_png/src/{id}.typ
- tests/fixtures/upstream_png/ref/{id}.png

Use upstream Typst only from the generator, via ../typst. The integration test
must be offline and compare Avenger raster output only against checked-in
ref/*.png files.

Start with exactly two or three smoke cases: one text decoration case, one
math fraction/root case, and one symbol case. Do not expand the corpus in the
first commit.

Run all commands in release mode:
  cargo run --release -p avenger-typst-label --features raster --bin generate_upstream_png_refs
  cargo test --release -p avenger-typst-label --features raster upstream_png_parity -- --nocapture
  cargo test --release -p avenger-typst-label --features raster -- --nocapture

Before committing, inspect every generated ref PNG directly or in a temporary
mosaic. Confirm non-blank output, no clipping, intended Lato/Lete Sans Math
fonts, and visible exercise of the target feature. For generator/comparator
changes, perturb one checked-in ref, verify useful expected/actual/diff
failure artifacts, restore/regenerate, rerun, and commit only suite files and
curated refs.
```

There are two modes:

- Existing suite mode: add coverage for one implemented label feature family.
- Build-from-zero mode: recreate the manifest/generator/test harness only if
  the suite has been deliberately removed or is being rebuilt after a major
  refactor.

Agent checklist:

1. Read this README and `cases.toml` before editing.
2. Add one supported feature family at a time.
3. Read the relevant upstream Typst test under `../typst/tests/suite/...` and
   preserve the smallest label-sized expression that exercises the behavior.
4. Add label-only snippets under `src/{id}.typ`.
5. Add matching `[[case]]` entries with upstream file/test attribution.
6. Regenerate references with the release-mode generator.
7. Inspect every changed `ref/*.png` directly or in a temporary mosaic.
8. Run the PNG parity test in release mode.
9. If label-engine code changed, run the full crate raster suite.
10. Stage only suite files and intentional crate metadata.

Definition of done for the first PNG-only build:

- `cases.toml` drives the suite; Rust tests do not hard-code case IDs.
- The generator can rebuild all checked-in `ref/*.png` files from snippets in
  `src/*.typ` using upstream Typst from `../typst`.
- The integration test never invokes upstream Typst; it only reads the
  checked-in PNG references.
- The initial corpus has two or three smoke cases, not a broad feature sweep.
- A deliberately perturbed reference fails with a useful case-id message and
  writes `expected.png`, `actual.png`, and `diff.png` artifacts.
- A clean run passes both the generator and parity test commands below in
  release mode.

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
3. Read the relevant upstream Typst test file from `../typst/tests/suite`.
   Keep the expression, not the whole document:
   - remove upstream page setup, show rules, loops, tables, columns, and other
     document-level scaffolding;
   - keep only syntax that belongs to the supported single-line label subset;
   - if the upstream case requires unsupported scripting or document layout,
     add an unsupported-syntax unit test instead of a PNG fixture.
4. Add or update one feature family at a time. Prefer unit tests first, then add
   small label-only snippets under `src/{id}.typ`.
5. Add matching `[[case]]` entries with upstream file/test attribution.
6. Run the reference generator:

   ```sh
   cargo run --release -p avenger-typst-label --features raster --bin generate_upstream_png_refs
   ```

7. Inspect every changed `ref/*.png` directly or in a temporary mosaic. Confirm
   each image is non-blank, unclipped, uses the intended Lato/Lete Sans Math
   fonts, and visibly exercises the intended feature.
8. Run the parity test:

   ```sh
   cargo test --release -p avenger-typst-label --features raster upstream_png_parity -- --nocapture
   ```

9. If label-engine code changed, run the full crate raster suite:

   ```sh
   cargo test --release -p avenger-typst-label --features raster -- --nocapture
   ```

10. Review `git diff` and stage only the intentional generator/test changes,
   `cases.toml`, source snippets, and curated `ref/*.png` files.

## PNG-Only Agent Task Card

Use this condensed task card for a fresh agent working on the suite:

1. Confirm the scope: PNG only, `avenger-typst-label` only, `raster` feature
   only.
2. Run `git status --short` and call out unrelated dirty files before editing.
3. Read this README and `cases.toml`.
4. Pick one already-implemented feature family.
5. Read the matching upstream Typst test under `../typst/tests/suite/...`.
6. Reduce the upstream behavior to one small label snippet under `src/`.
7. Add or update exactly one `[[case]]` entry with upstream attribution.
8. Run the generator in release mode.
9. Inspect every changed reference PNG directly or in a temporary mosaic.
10. Run the release-mode parity test.
11. If generator/comparator code changed, perturb one ref, confirm useful
    failure artifacts, then restore/regenerate.
12. Stage only suite files and curated refs.

Hard stop conditions:

- The case needs SVG/PDF output to be meaningful.
- The upstream test needs document layout, scripting, imports, counters,
  tables, matrices, or show/set rules not in the retained label subset.
- The Avenger feature is not implemented yet. Add or keep unit coverage first;
  add PNG parity after the feature works.
- The generated PNG is blank, clipped, uses the wrong font, or does not visibly
  exercise the target feature.

## Choosing Upstream Cases

Start from the upstream Typst suite, but convert tests into small label
fixtures rather than copying page-level render tests verbatim.

Good PNG parity cases:

- fit naturally on one label line;
- have deterministic visual output with the bundled fonts;
- exercise one retained feature family clearly;
- do not require Typst scripting, `#set`, `#show`, imports, counters, layout
  containers, matrix/table layout, or full-page behavior;
- can be attributed to one upstream file and test name in `cases.toml`.

Bad PNG parity cases:

- depend on page layout, block layout, columns, tables, counters, loops, or
  full Typst evaluation;
- require unsupported functions that should currently error;
- are mostly redundant with an existing visual fixture;
- hide the feature under a large expression where a one-line reduced case is
  clearer.

When reducing a Typst test, keep the upstream spelling of the feature under
test. Change surrounding values only to make the case label-sized,
deterministic, and readable.

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

For a first build, stop after the manifest, generator, integration test, and
two or three smoke references are working. Do not expand the corpus in the same
commit. For later extensions, add exactly one implemented feature family per
commit.

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
