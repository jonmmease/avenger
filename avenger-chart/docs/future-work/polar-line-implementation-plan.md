# Polar Line Geometry Space Implementation Plan

## Purpose

This is the implementation checklist for adding `Line<Polar>` and the
`geometry_space` option. It turns the design decisions in
[polar-geometry-space.md](polar-geometry-space.md) into phase-sized code
changes that an agent can implement, test, check off, and commit.

The goal is a shippable v1 for polar line marks, not the full future mark
effects system.

## Closed Decisions

- `Line<Polar>` is in scope for this implementation. `Text<Polar>` remains a
  follow-up.
- `GeometrySpace` is a mark semantic option, not a channel and not a scale
  input.
- `GeometrySpace` should live in `avenger-chart-core` and be re-exported by
  `avenger-chart`.
- The compiled mark state should preserve `geometry_space` through
  serialization with a backward-compatible default.
- Only `Line<Polar>` needs to expose `geometry_space` publicly in this plan.
  Future marks can adopt the same state field later.
- `Line<Polar>` defaults to `GeometrySpace::Coordinate`.
- `GeometrySpace::Display` projects input vertices first and connects them
  with display-space chords.
- `GeometrySpace::Coordinate` interpolates scaled `r` and `theta` before
  projection, then emits display-space samples by passing sampled scaled
  channels through the coordinate transform.
- Theta interpolation is always as given. There is no shortest-path,
  clockwise, counterclockwise, or wrap policy.
- `Polar::transform` should support mixed scalar/array `r` and `theta` by
  broadcasting scalar channels to the array length. Constant-radius arcs are a
  normal use case, not an edge case.
- Coordinate-space polar lines emit densified `SceneLineMark` geometry in v1.
- Do not add a new line-arc scene mark in v1.
- Do not route v1 through `ScenePathMark` unless `ScenePathMark` first gains
  line parity for dash behavior, source rows, retargeting, and interaction.
- Densification uses deterministic internal tolerances. Do not add public
  tolerance options in v1.
- Densified samples are render geometry only. Current line interaction
  semantics are mark-level, not vertex-level; the implementation should
  preserve that behavior rather than inventing nearest-row line hits.
- Style partitioning, `details`, `defined`, `order`, stroke dash, stroke cap,
  stroke join, opacity, and legends should match `Line<Cartesian>` semantics.
- For coordinate-space output, an emitted `SceneLineMark` may contain more
  vertices than source rows. `RenderedMarkData::source_row_indices` should
  still list the original source rows for that emitted scene mark, and tests
  should verify this does not break mark-level line event datum lookup.
- Positional scale preferences should match current polar point-positioned
  marks: string-like `r` and `theta` use point scales, and other positional
  data types use the standard data-type default.

## Agent Operating Instructions

- Before each phase, run `git status --short` and identify unrelated changes.
  Do not stage or revert unrelated worktree files.
- Implement one phase at a time. Do not combine phases unless the current
  phase cannot compile without the next one.
- Keep the working tree buildable at every commit.
- After completing a phase and passing its listed checks, update this file by
  changing that phase's completed tasks from `[ ]` to `[x]`.
- Stage only the files that belong to the phase, including this checklist
  update.
- Commit after every completed phase using a Conventional Commit message.
- If a phase is blocked or a listed check fails, leave its boxes unchecked,
  add a short note under `Progress Notes`, and do not commit failing code.
- If visual baselines intentionally change, include the generated baselines in
  the same phase commit and call that out in the commit message body.
- Do not refresh existing visual baselines in Phases 1-4 just to make tests
  pass. Baseline updates belong in Phase 5 unless an earlier phase explicitly
  changes an existing visual contract, in which case investigate and document
  the cause before committing.

Useful commit message shape:

```text
feat(polar): add polar line mark rendering

- Implements Line<Polar> for coordinate and display geometry space
- Preserves source-row identity for densified coordinate-space samples
- Updates the polar line implementation checklist

Checks:
- cargo test -p avenger-chart test_polar_line -- --nocapture
```

## Phase 0: Preflight

- [ ] Confirm the branch and working tree status.
- [ ] Note unrelated modified or untracked files before editing.
- [ ] Read the current implementations of `Line<Cartesian>`,
  `Symbol<Polar>`, `Polar::transform`, `SceneLineMark`, and `ScenePathMark`.
- [ ] Grep for all `MarkState {` and `CompiledMarkState {` literal
  construction sites so Phase 1 can update them deliberately.
- [ ] Confirm `avenger-chart-marks` can depend on the crates needed by the
  shared line helpers without introducing a dependency cycle. If not, choose a
  different helper home before starting Phase 2.
- [ ] Verify that the recommended test filters in this plan exist, or replace
  them with valid focused commands before using them as gates.
- [ ] Confirm this plan is still current against the codebase.

Recommended checks:

```text
git status --short
rg -n "struct SceneLineMark|impl Mark<Polar>|impl Mark<Cartesian> for Line|GeometrySpace" \
  avenger-chart-core avenger-chart-cartesian avenger-chart-polar avenger-chart-marks avenger-scenegraph
rg -n "MarkState \\{|CompiledMarkState \\{" avenger-chart-core avenger-chart avenger-chart-* -g '*.rs'
```

Commit: no commit is required for preflight unless the plan itself changes.

## Phase 1: Core Geometry Space Contract

- [ ] Add `GeometrySpace` to `avenger-chart-core`.
- [ ] Derive `Clone`, `Copy`, `Debug`, `Default`, `PartialEq`, `Eq`,
  `Serialize`, and `Deserialize`.
- [ ] Use kebab-case serde names.
- [ ] Make `Coordinate` the enum default.
- [ ] Add `geometry_space: Option<GeometrySpace>` to `MarkState`.
- [ ] Add `geometry_space: Option<GeometrySpace>` to `CompiledMarkState` with
  `#[serde(default)]`.
- [ ] Copy `geometry_space` in `CompiledMarkState::from_mark_state`.
- [ ] Initialize `geometry_space: None` in `impl_mark_base!`.
- [ ] Update every direct `MarkState` and `CompiledMarkState` struct literal
  found in Phase 0.
- [ ] Add a helper on `CompiledMarkState`, or use direct field access, so mark
  implementations can resolve `state.geometry_space.unwrap_or(default)`.
- [ ] Re-export `GeometrySpace` from `avenger-chart-core/src/lib.rs`.
- [ ] Re-export `GeometrySpace` from the top-level `avenger-chart` prelude.
- [ ] Add serialization tests that prove an old payload with the field absent
  deserializes, and a non-default `GeometrySpace::Display` value survives an
  end-to-end serialize/deserialize round trip.
- [ ] Document in test names or comments that adding the optional field changes
  shared compiled mark state serialization intentionally.

Recommended checks:

```text
cargo fmt --all
cargo test -p avenger-chart-core
cargo test -p avenger-chart test_prelude -- --nocapture
```

Commit: `feat(chart): add geometry space mark option`

## Phase 2: Shared Line Rendering Support

`Line<Polar>` should not duplicate the entire `Line<Cartesian>` style
partitioning implementation. Extract the reusable pieces first, then prove
Cartesian line behavior still works.

- [ ] Move the generic `DetailColumns` helper out of
  `avenger-chart-cartesian` so both Cartesian and Polar marks can use it.
  `avenger-chart-core` is the preferred home because it already owns
  `detail_array_column_name` and `CompiledMarkCore`.
- [ ] Make the moved detail helper visible to coordinate crates and update
  Cartesian imports accordingly.
- [ ] Extract line style partitioning into shared helpers in
  `avenger-chart-marks`, for example `line_render.rs`.
- [ ] The primary helper should return style partitions over original data row
  indices, not final geometry. This lets Cartesian use original vertices while
  Polar coordinate space can densify within each partition.
- [ ] Each style partition should include original row indices, style values,
  detail key identity, and the resolved `stroke`, `stroke_width`,
  `stroke_dash`, `stroke_cap`, `stroke_join`, and opacity-applied color.
- [ ] Add a small helper for constructing a `SceneLineMark` from resolved
  style values plus caller-provided display-space `x`, `y`, and `defined`
  arrays.
- [ ] Preserve Cartesian behavior for uniform styles, varying stroke,
  varying stroke width, varying dash, varying opacity, and `details`
  partitioning.
- [ ] Update `CompiledCartesianLine` to use the helper without changing public
  behavior.
- [ ] Add or retain tests that cover multi-series Cartesian line partitioning
  after the extraction.

Recommended checks:

```text
cargo fmt --all
cargo test -p avenger-chart test_stroke_dash -- --nocapture
cargo test -p avenger-chart visual_tests::test_line -- --nocapture
cargo test -p avenger-chart visual_tests::test_line_multi_series -- --nocapture
```

Commit: `refactor(chart): share line scene rendering`

## Phase 3: Polar Transform And Line Sampling Utilities

Build the coordinate-space sampling logic before wiring it into the public
mark. Keep it small and deterministic.

- [ ] Add an internal polar line sampling module, either beside the polar line
  mark or inside `avenger-chart-polar/src/marks/line.rs`.
- [ ] Update `Polar::transform` to broadcast mixed scalar/array `r` and
  `theta` channels to the common array length before projection.
- [ ] Add `Polar::transform` tests for scalar `r` plus array `theta`, array
  `r` plus scalar `theta`, scalar/scalar, array/array, and mismatched
  array/array lengths.
- [ ] Implement display-space preparation that returns original scaled `r` and
  `theta` arrays for projection through `coord.transform`.
- [ ] Implement coordinate-space sampling using linear interpolation in scaled
  `r` and scaled `theta`, returning sampled scaled `r/theta` arrays.
- [ ] Project sampled coordinate-space arrays through `coord.transform`; do
  not duplicate polar projection math in the sampler.
- [ ] Preserve `defined` gaps exactly: no segment should be sampled across a
  false or missing `defined` row.
- [ ] Broadcast scalar `r`, `theta`, and `defined` values to the mark length
  before sampling or display-space projection.
- [ ] Use theta as given. Do not normalize or unwrap theta during sampling.
- [ ] Use an internal deterministic subdivision policy based on angular delta
  and display-space chord length.
- [ ] Add a conservative maximum subdivision count per input segment so bad
  input cannot produce unbounded vertex growth.
- [ ] Avoid duplicate vertices at shared segment endpoints.
- [ ] Return sampled display-space vertices separately from source-row indices.
  Source-row indices should always refer to original data rows, not densified
  samples.
- [ ] Add unit tests for radial, circular arc, spiral-like, gap, scalar
  broadcast, theta-as-given wrap, and subdivision cap cases.

Recommended initial constants:

```text
MAX_THETA_STEP = PI / 90.0
MAX_DISPLAY_SEGMENT_PX = 6.0
```

Recommended checks:

```text
cargo fmt --all
cargo test -p avenger-chart-polar polar_line -- --nocapture
```

Commit: `feat(polar): add polar line geometry sampling`

## Phase 4: Public `Line<Polar>` API And Renderer

- [ ] Add `avenger-chart-polar/src/marks/line.rs`.
- [ ] Implement `Mark<Polar> for Line<Polar>`.
- [ ] Add `CompiledPolarLine`.
- [ ] Add `line` to `avenger-chart-polar/src/marks/mod.rs`.
- [ ] Export `CompiledPolarLine`.
- [ ] Add `PolarLinePositionChannels` with `r`, `r_with`, `theta`,
  `theta_with`, and `geometry_space`.
- [ ] Export `PolarLinePositionChannels` from `avenger-chart-polar/src/lib.rs`.
- [ ] Re-export `PolarLinePositionChannels` from `avenger-chart/src/prelude.rs`
  and `avenger-chart/src/polar/mod.rs`.
- [ ] Support channels `r`, `theta`, `stroke`, `stroke_width`,
  `stroke_dash`, `stroke_cap`, `stroke_join`, `opacity`, `defined`, and
  `order`.
- [ ] Return `supports_order() == true`.
- [ ] Return `details_partition_continuous_geometry() == true`.
- [ ] Use line defaults from `line_channel_defaults`.
- [ ] Use line legend behavior matching `Line<Cartesian>`, with no legends for
  `r`, `theta`, `defined`, `order`, `stroke_cap`, or `stroke_join`.
- [ ] Use the same positional scale preferences as `Symbol<Polar>` for `r`
  and `theta`.
- [ ] In render, require array data as `Line<Cartesian>` does.
- [ ] Coerce scaled `r` and `theta` channels.
- [ ] Resolve `geometry_space`, defaulting to `GeometrySpace::Coordinate`.
- [ ] Implement `radius_expression` explicitly, returning `None` for `r` and
  `theta` to match `Symbol<Polar>`.
- [ ] For display space, project original vertices and build scene marks from
  the shared style partitions.
- [ ] For coordinate space, densify polar segments and pass the sampled
  display-space vertices plus original source-row indices into scene marks
  using the shared style helpers.
- [ ] Preserve `zindex`, clipping, interactivity, style channels, dash, caps,
  joins, and source-row identity.
- [ ] Preserve current line interaction semantics: `SceneLineMark`
  hit-testing remains mark-level with `instance_index: None`, even when
  densified vertex count exceeds source-row count.
- [ ] Add compile tests for default coordinate space and explicit display
  space.

Recommended checks:

```text
cargo fmt --all
cargo test -p avenger-chart-polar
cargo test -p avenger-chart test_prelude -- --nocapture
cargo test -p avenger-chart test_mark_channel_alignment -- --nocapture
```

Commit: `feat(polar): add line mark support`

## Phase 5: Visual And Behavioral Coverage

- [ ] Add polar line visual tests under `avenger-chart/tests/visual_tests/`.
- [ ] Put new polar line baselines in a dedicated category such as
  `avenger-chart/tests/baselines/polar_line/` unless the existing visual test
  organization strongly favors extending the current `polar/` category.
- [ ] Cover constant-radius coordinate-space arc versus display-space chord.
- [ ] Make the arc-versus-chord baseline use scalar `r` with array `theta` so
  it also exercises mixed scalar/array broadcasting in `Polar::transform`.
- [ ] Cover constant-theta radial segment.
- [ ] Cover varying `r` and `theta` spiral-like segment.
- [ ] Cover `defined` gaps.
- [ ] Cover dashed coordinate-space polar lines.
- [ ] Cover multi-series partitioning by stroke and/or dash.
- [ ] Cover `details` partitioning for multiple polar line series.
- [ ] Cover at least one categorical or ordinal theta/r case if it is expected
  to work.
- [ ] Cover clipping for a polar line whose coordinate-space interpolation
  crosses outside the visible polar plot area.
- [ ] Use stable baseline names that identify the behavior being locked down:
  `arc_vs_chord`, `radial_and_spiral`, `dashed_gaps`, `multi_series_details`,
  `categorical_theta`, and `clipping`.
- [ ] Keep each visual baseline focused. If a single image combines multiple
  behaviors, make sure each behavior remains visually distinguishable after
  anti-aliasing and downsampling.
- [ ] Add visual baselines only for intentional output.
- [ ] Add a non-visual assertion test that the default is coordinate space.
- [ ] Add a non-visual assertion test that theta interpolation is as given
  across a wrap boundary.
- [ ] Add a non-visual assertion test that a densified polar line can have
  more rendered vertices than source rows while event datum lookup keeps
  current mark-level line behavior.
- [ ] Add a non-visual assertion test or benchmark-style guard that
  densification respects the per-segment subdivision cap.

Recommended checks:

```text
cargo fmt --all
cargo test -p avenger-chart polar_line -- --nocapture
cargo test -p avenger-chart visual_regression -- --nocapture
```

Commit: `test(chart): cover polar line geometry space`

## Phase 6: Documentation And Final Verification

- [ ] Update `polar-geometry-space.md` to mark `Line<Polar>` decisions as
  implemented or implementation-ready.
- [ ] Add a user-facing example or guide snippet for `Line<Polar>` once the API
  is stable.
- [ ] Document that theta interpolation is as given and that authors should
  transform or unwrap data if they want a different crossing.
- [ ] Document why v1 uses densified `SceneLineMark` instead of open
  `ScenePathMark` or a new line-arc scene mark.
- [ ] Document that v1 line hit testing remains mark-level. Nearest source-row
  or nearest-segment line interactions require a separate interaction design.
- [ ] Run formatting.
- [ ] Run focused polar/chart tests.
- [ ] Run clippy for touched crates.
- [ ] Run the full chart test suite if time allows.
- [ ] Ensure `git status --short` contains only expected unrelated user files,
  or is clean.

Recommended checks:

```text
cargo fmt --all
cargo clippy -p avenger-chart-core --all-targets
cargo clippy -p avenger-chart-marks --all-targets
cargo clippy -p avenger-chart-polar --all-targets
cargo clippy -p avenger-chart-cartesian --all-targets
cargo test -p avenger-chart -- --nocapture
```

Commit: `docs(chart): document polar line geometry space`

## Progress Notes

Add dated notes here only when a phase is blocked, intentionally deferred, or
requires a deviation from the plan.
