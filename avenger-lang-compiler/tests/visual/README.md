# Avenger Language Fixture Baselines

This suite owns every `.avenger` source below `tests/fixtures/projects` through
`tests/fixtures/visual_cases.json`. The manifest assigns each source to exactly
one selected-chart case, so imported modules and local data resources are
covered without pretending they are independently renderable charts.

The current corpus contains 63 sources and 46 selected-chart cases:

- 45 valid chart entrypoints compile, serialize and deserialize, evaluate in isolated
  environments, render through WGPU, and compare with reviewed PNGs under
  `tests/baselines/visual`;
- one intentionally invalid case compares its complete structured compiler
  diagnostics with reviewed JSON baselines.

Every visual case has an explicit `review` status in the manifest:

- `reviewed` means the PNG has been inspected and represents intended output;
- `known_incorrect` pins a reproducible defect while its remediation is active;
- `weak` means the output is currently correct but does not exercise enough of
  its named feature;
- `intentional_blank` means a white canvas is expected and must be supported by
  a structural assertion rather than trusted as visual evidence by itself.

Review status never skips compilation, artifact round-trip, evaluation, or PNG
comparison. The inventory test fails when any visual case omits it.

The current remediation pass is closed: all 45 visual cases are `reviewed`.
There are no active `known_incorrect`, `weak`, or `intentional_blank` cases.
The provider-host fixture still renders a white canvas by design, but is
classified as `reviewed` because the harness directly asserts that both its
Iceberg catalog factory and Delta table factory were instantiated.

Use a non-final status only while a concrete follow-up is active. Record the
expected defect or coverage gap in the remediation plan, keep comparing the
current artifact on normal runs, and return the case to `reviewed` only after
its fixture contract and full-resolution PNG have both been inspected. Do not
use a review status to bless or skip a mismatch.

The suite installs built-in native widget factories, the downstream extension
registry used by `04_composed_extension`, and schema-only provider mocks used by
`phase9-providers`. These are real fixture host requirements, not exclusions.

## Run the suite

```sh
cargo test --release -p avenger-lang-compiler \
  --test fixture_visual_regression -- --nocapture
```

Run one root or project while developing:

```sh
AVENGER_LANG_FIXTURE_CASE=09_native_coordinate_families/treemap \
  cargo test --release -p avenger-lang-compiler \
  --test fixture_visual_regression fixture_visual_regression -- --nocapture
```

`AVENGER_LANG_FIXTURE_CASE` is a substring filter over manifest root paths.

## Review and update baselines

Intentional changes are blessed explicitly:

```sh
AVENGER_LANG_BLESS_FIXTURE_BASELINES=1 \
  cargo test --release -p avenger-lang-compiler \
  --test fixture_visual_regression -- --nocapture
```

Review every changed PNG at full resolution and every changed diagnostic JSON
before committing. Prefer a direct scene or fixture-host assertion for
important semantics that pixels alone cannot prove. Normal test runs never
rewrite reviewed artifacts. Missing or changed images, serialized round-trip
mismatches, and image diffs are written below
`target/tests/avenger-lang-fixture-visual`.

When adding, renaming, or removing any project fixture, update
`tests/fixtures/visual_cases.json`. The inventory test fails unless every
discovered `.avenger` file has exactly one manifest owner and every case root
parses as a chart.
