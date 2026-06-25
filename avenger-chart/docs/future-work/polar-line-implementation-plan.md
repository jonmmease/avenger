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
  serialization with a default for human-readable payloads that omit the field.
  Binary serializers used by the test suite require the option field to remain
  present in the stream, so do not use `skip_serializing_if` on
  `CompiledMarkState::geometry_space` without adding a versioned/custom binary
  encoding path.
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
- `Line<Polar>` should reject mark effects in v1. Correctly combining effects
  with densified geometry belongs with the later mark evaluation frame work.
- Shared line support should start with neutral helpers in `avenger-chart-core`
  (`DetailColumns`, dash/cap/join/color normalization, and index gathering).
  A fuller style-partition abstraction can still be extracted later if more
  coordinate systems need line rendering.
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
- cargo test --release -p avenger-chart test_polar_line -- --nocapture
```

## Phase 0: Preflight

- [x] Confirm the branch and working tree status.
- [x] Note unrelated modified or untracked files before editing.
- [x] Read the current implementations of `Line<Cartesian>`,
  `Symbol<Polar>`, `Polar::transform`, `SceneLineMark`, and `ScenePathMark`.
- [x] Grep for all `MarkState {` and `CompiledMarkState {` literal
  construction sites so Phase 1 can update them deliberately.
- [x] Grep for `CompiledMarkState::from_mark_state`, struct-update syntax, and
  default construction paths that could hide mark-state initialization.
- [x] Confirm `avenger-chart-marks` can depend on the crates needed by the
  shared line helpers without introducing a dependency cycle. If not, choose a
  different helper home before starting Phase 2.
- [x] Map the current `Line<Cartesian>` render dependencies, including
  `marks::util`, primitive effect plumbing, visual/item assignment helpers,
  and line style coercion. Use this map to choose the Phase 2 extraction
  boundary.
- [x] Verify that the recommended test filters in this plan exist, or replace
  them with valid focused commands before using them as gates.
- [x] Confirm this plan is still current against the codebase.

Recommended checks:

```text
git status --short
rg -n "struct SceneLineMark|impl Mark<Polar>|impl Mark<Cartesian> for Line|GeometrySpace" \
  avenger-chart-core avenger-chart-cartesian avenger-chart-polar avenger-chart-marks avenger-scenegraph
rg -n "MarkState \\{|CompiledMarkState \\{" avenger-chart-core avenger-chart avenger-chart-* -g '*.rs'
rg -n "CompiledMarkState::from_mark_state|\\.\\.Default::default\\(\\)|impl_mark_base!" \
  avenger-chart-core avenger-chart avenger-chart-* -g '*.rs'
```

Commit: no commit is required for preflight unless the plan itself changes.

## Phase 1: Core Geometry Space Contract

- [x] Add `GeometrySpace` to `avenger-chart-core`.
- [x] Derive `Clone`, `Copy`, `Debug`, `Default`, `PartialEq`, `Eq`,
  `Serialize`, and `Deserialize`.
- [x] Use kebab-case serde names.
- [x] Make `Coordinate` the enum default.
- [x] Add `geometry_space: Option<GeometrySpace>` to `MarkState`.
- [x] Add `geometry_space: Option<GeometrySpace>` to `CompiledMarkState` with
  `#[serde(default)]`. Do not use `skip_serializing_if` while bincode
  round-trips rely on stable field order.
- [x] Copy `geometry_space` in `CompiledMarkState::from_mark_state`.
- [x] Initialize `geometry_space: None` in `impl_mark_base!`.
- [x] Update every direct `MarkState` and `CompiledMarkState` struct literal
  found in Phase 0.
- [x] Add a helper on `CompiledMarkState`, or use direct field access, so mark
  implementations can resolve `state.geometry_space.unwrap_or(default)`.
- [x] Re-export `GeometrySpace` from `avenger-chart-core/src/lib.rs`.
- [x] Re-export `GeometrySpace` from the top-level `avenger-chart` prelude.
- [x] Add serialization tests that prove a human-readable payload with the
  field absent deserializes.
- [x] Add a serialization test that a non-default `GeometrySpace::Display`
  value survives an end-to-end bincode serialize/deserialize round trip.
- [x] Document in test names or comments that adding the optional field changes
  shared compiled mark state serialization intentionally.

Recommended checks:

```text
cargo fmt --all
cargo test --release -p avenger-chart-core
cargo test --release -p avenger-chart test_prelude -- --nocapture
```

Commit: `feat(chart): add geometry space mark option`

## Phase 2: Shared Line Rendering Support

`Line<Polar>` should not duplicate the entire `Line<Cartesian>` style
partitioning implementation. Extract the reusable pieces first, then prove
Cartesian line behavior still works. This phase must be deliberate because
`Line<Cartesian>` also owns primitive effect handling; the shared helper should
not accidentally absorb effect semantics that `Line<Polar>` is not ready to
support.

- [x] Move the generic `DetailColumns` helper out of
  `avenger-chart-cartesian` so both Cartesian and Polar marks can use it.
  `avenger-chart-core` is the preferred home because it already owns
  `detail_array_column_name` and `CompiledMarkCore`.
- [x] Make the moved detail helper visible to coordinate crates and update
  Cartesian imports accordingly.
- [x] Choose the shared helper home after the Phase 0 dependency check. It
  must be below both coordinate crates and may be `avenger-chart-marks` or
  `avenger-chart-core`; do not put shared helpers in `avenger-chart-cartesian`.
- [x] Enumerate the current Cartesian line render support surface before
  moving code. At minimum, account for `marks::util` helpers such as color
  coercion, stroke cap/join/dash coercion, optional dash handling,
  gather-by-indices behavior, and any primitive effect visual/item assignment
  inputs that influence line style partitioning.
- [x] Extract neutral style support helpers needed by both Cartesian and Polar:
  color string normalization, dash/cap/join normalization, optional dash
  handling, and gather-by-indices behavior. Keep primitive effect evaluation in
  Cartesian for now.
- [ ] Deferred refactor: the primary helper should return style partitions
  over original data row indices, not final geometry. This lets Cartesian use
  original vertices while Polar coordinate space can densify within each
  partition.
- [ ] Deferred refactor: each style partition should include original row
  indices, style values, detail key identity, and the resolved `stroke`,
  `stroke_width`, `stroke_dash`, `stroke_cap`, `stroke_join`, and
  opacity-applied color.
- [ ] Deferred refactor: add a small helper for constructing a
  `SceneLineMark` from resolved style values plus caller-provided display-space
  `x`, `y`, and `defined` arrays.
- [x] Preserve Cartesian behavior for uniform styles, varying stroke,
  varying stroke width, varying dash, varying opacity, and `details`
  partitioning by leaving the Cartesian renderer's partitioning path
  unchanged.
- [ ] Deferred refactor: update `CompiledCartesianLine` to use a fuller shared
  partition helper without changing public behavior.
- [x] Add or retain tests that cover multi-series Cartesian line partitioning
  after the extraction.

Recommended checks:

```text
cargo fmt --all
cargo test --release -p avenger-chart test_stroke_dash -- --nocapture
cargo test --release -p avenger-chart visual_tests::test_line -- --nocapture
cargo test --release -p avenger-chart visual_tests::test_line_multi_series -- --nocapture
```

Commit: `refactor(chart): share line scene rendering`

## Phase 3: Polar Transform And Line Sampling Utilities

Build the coordinate-space sampling logic before wiring it into the public
mark. Keep it small and deterministic.

- [x] Add an internal polar line sampling module, either beside the polar line
  mark or inside `avenger-chart-polar/src/marks/line.rs`.
- [x] Update `Polar::transform` to broadcast mixed scalar/array `r` and
  `theta` channels to the common array length before projection.
- [x] Add `Polar::transform` tests for scalar `r` plus array `theta`, array
  `r` plus scalar `theta`, scalar/scalar, array/array, and mismatched
  array/array lengths.
- [x] Add `Polar::transform` tests for scalar `r` plus array `theta`, array
  `r` plus scalar `theta`, and mismatched array/array lengths.
- [x] Preserve scalar/scalar `Polar::transform` output as scalar
  `PointGeometry`. Only mixed scalar/array inputs should newly broadcast.
- [x] Confirm that `order` is resolved upstream as row reordering before mark
  rendering, matching `Line<Cartesian>` assumptions.
- [x] Add a test that an ordered polar line samples along the ordered row
  sequence, not the original input order.
- [x] Implement display-space preparation that returns original scaled `r` and
  `theta` arrays for projection through `coord.transform`.
- [x] Implement coordinate-space sampling using linear interpolation in scaled
  `r` and scaled `theta`, returning sampled scaled `r/theta` arrays.
- [x] Project sampled coordinate-space arrays through `coord.transform`; do
  not duplicate polar projection math in the sampler.
- [x] Preserve `defined` gaps exactly: no segment should be sampled across a
  false or missing `defined` row.
- [x] Assert in tests that sampled `x`, `y`, and `defined` arrays have equal
  lengths and that inserted samples are never created inside a `defined` gap.
- [x] Broadcast scalar `r`, `theta`, and `defined` values to the mark length
  before sampling or display-space projection.
- [x] Use theta as given. Do not normalize or unwrap theta during sampling.
- [x] Use an internal deterministic subdivision policy based on angular delta
  and display-space chord length.
- [x] Add a conservative maximum subdivision count per input segment so bad
  input cannot produce unbounded vertex growth. Record the numeric cap and how
  it interacts with `MAX_THETA_STEP` and `MAX_DISPLAY_SEGMENT_PX`.
- [x] Avoid duplicate vertices at shared segment endpoints.
- [x] Return sampled display-space vertices separately from source-row indices.
  Source-row indices should always refer to original data rows, not densified
  samples.
- [x] Add unit tests for radial, circular arc, spiral-like, gap, scalar
  broadcast, theta-as-given wrap, and subdivision cap cases.

Recommended initial constants:

```text
MAX_THETA_STEP = PI / 90.0
MAX_DISPLAY_SEGMENT_PX = 6.0
```

Recommended checks:

```text
cargo fmt --all
cargo test --release -p avenger-chart-polar polar_line -- --nocapture
```

Commit: `feat(polar): add polar line geometry sampling`

## Phase 4: Public `Line<Polar>` API And Renderer

- [x] Add `avenger-chart-polar/src/marks/line.rs`.
- [x] Implement `Mark<Polar> for Line<Polar>`.
- [x] Add `CompiledPolarLine`.
- [x] Reject unsupported mark effects at compile time; do not implement effect
  application for `Line<Polar>` in v1.
- [x] Add `line` to `avenger-chart-polar/src/marks/mod.rs`.
- [x] Export `CompiledPolarLine`.
- [x] Add `PolarLinePositionChannels` with `r`, `r_with`, `theta`,
  `theta_with`, and `geometry_space`.
- [x] Export `PolarLinePositionChannels` from `avenger-chart-polar/src/lib.rs`.
- [x] Re-export `PolarLinePositionChannels` from `avenger-chart/src/prelude.rs`
  and `avenger-chart/src/polar/mod.rs`.
- [x] Support channels `r`, `theta`, `stroke`, `stroke_width`,
  `stroke_dash`, `stroke_cap`, `stroke_join`, `opacity`, `defined`, and
  `order`.
- [x] Return `supports_order() == true`.
- [x] Return `details_partition_continuous_geometry() == true`.
- [x] Use line defaults from `line_channel_defaults`.
- [x] Use line legend behavior matching `Line<Cartesian>`, with no legends for
  `r`, `theta`, `defined`, `order`, `stroke_cap`, or `stroke_join`.
- [x] Use the same positional scale preferences as `Symbol<Polar>` for `r`
  and `theta`.
- [x] In render, require array data as `Line<Cartesian>` does.
- [x] Implement `render_mark_data` and return
  `RenderedMarkData::with_source_row_indices(...)` for all emitted scene line
  partitions.
- [x] Implement `render_from_data` as a delegating wrapper around
  `render_mark_data`, mirroring `CompiledCartesianLine`.
- [x] Coerce scaled `r` and `theta` channels.
- [x] Resolve `geometry_space`, defaulting to `GeometrySpace::Coordinate`.
- [x] Implement `radius_expression` explicitly, returning `None` for `r` and
  `theta` to match `Symbol<Polar>`.
- [x] Reject non-empty mark effects during compile or render with a clear error
  message, matching the current conservative `Symbol<Polar>` posture.
- [x] For display space, project original vertices and build scene marks using
  the shared low-level style helpers.
- [x] For coordinate space, densify polar segments and pass the sampled
  display-space vertices plus original source-row indices into scene marks
  using the shared low-level style helpers.
- [x] Preserve `zindex`, clipping, interactivity, style channels, dash, caps,
  joins, and source-row identity.
- [x] Preserve current line interaction semantics: `SceneLineMark`
  hit-testing remains mark-level with `instance_index: None`, even when
  densified vertex count exceeds source-row count.
- [x] Add compile tests for default coordinate space and explicit display
  space.
- [x] Add a focused test that mark effects on `Line<Polar>` are rejected until
  densified-geometry effect semantics are designed.

Recommended checks:

```text
cargo fmt --all
cargo test --release -p avenger-chart-polar
cargo test --release -p avenger-chart test_prelude -- --nocapture
cargo test --release -p avenger-chart test_mark_channel_alignment -- --nocapture
```

Commit: `feat(polar): add line mark support`

## Phase 5: Visual And Behavioral Coverage

- [x] Add polar line visual tests under `avenger-chart/tests/visual_tests/`.
- [x] Put new polar line baselines in a dedicated category such as
  `avenger-chart/tests/baselines/polar_line/` unless the existing visual test
  organization strongly favors extending the current `polar/` category.
- [x] Cover constant-radius coordinate-space arc versus display-space chord.
- [x] Make the arc-versus-chord baseline use scalar `r` with array `theta` so
  it also exercises mixed scalar/array broadcasting in `Polar::transform`.
- [x] Cover constant-theta radial segment.
- [x] Cover varying `r` and `theta` spiral-like segment.
- [x] Cover `defined` gaps.
- [x] Cover dashed coordinate-space polar lines.
- [x] Cover multi-series partitioning by stroke and/or dash.
- [x] Cover `details` partitioning for multiple polar line series.
- [x] Cover at least one categorical or ordinal theta/r case if it is expected
  to work.
- [x] Cover clipping for a polar line whose coordinate-space interpolation
  crosses outside the visible polar plot area.
- [x] Make the clipping baseline actually exceed the polar radius, or use a
  non-square plot where the clipped geometry is visible in the rendered image.
- [x] Use stable baseline names that identify the behavior being locked down:
  `arc_vs_chord`, `radial_and_spiral`, `dashed_gaps`, `multi_series_details`,
  `categorical_theta`, and `clipping`.
- [x] Keep each visual baseline focused. If a single image combines multiple
  behaviors, make sure each behavior remains visually distinguishable after
  anti-aliasing and downsampling.
- [x] Add visual baselines only for intentional output.
- [x] Add a non-visual assertion test that the default is coordinate space.
- [x] Add a non-visual assertion test that theta interpolation is as given
  across a wrap boundary.
- [x] Add a non-visual assertion test that a densified polar line can have
  more rendered vertices than source rows.
- [x] Add a non-visual assertion that event datum lookup keeps current
  mark-level line behavior for densified polar line scene marks.
- [x] Add a deterministic non-visual assertion that densification respects the
  per-segment subdivision cap for a near-full-turn segment. Avoid timing-based
  benchmark gates.

Recommended checks:

```text
cargo fmt --all
cargo test --release -p avenger-chart polar_line -- --nocapture
cargo test --release -p avenger-chart visual_regression -- --nocapture
```

Commit: `test(chart): cover polar line geometry space`

## Phase 6: Documentation And Final Verification

- [x] Update `polar-geometry-space.md` to mark `Line<Polar>` decisions as
  implemented or implementation-ready.
- [x] Add a user-facing example or guide snippet for `Line<Polar>` once the API
  is stable.
- [x] Document that theta interpolation is as given and that authors should
  transform or unwrap data if they want a different crossing.
- [x] Document why v1 uses densified `SceneLineMark` instead of open
  `ScenePathMark` or a new line-arc scene mark.
- [x] Document that v1 line hit testing remains mark-level. Nearest source-row
  or nearest-segment line interactions require a separate interaction design.
- [x] Document that densified line geometry intentionally has more rendered
  vertices than retained source rows, and why this is safe for v1 line events:
  `SceneLineMark` hit geometry uses a single mark-level instance with
  `instance_index: None`.
- [x] Run formatting.
- [x] Run focused polar/chart tests.
- [x] Run clippy for touched crates.
- [x] Run the full chart library test suite if time allows.
- [x] Ensure `git status --short` contains only expected unrelated user files,
  or is clean.

Recommended checks:

```text
cargo fmt --all
cargo clippy --release -p avenger-chart-core --all-targets
cargo clippy --release -p avenger-chart-marks --all-targets
cargo clippy --release -p avenger-chart-polar --all-targets
cargo clippy --release -p avenger-chart-cartesian --all-targets
cargo test --release -p avenger-chart -- --nocapture
```

Commit: `docs(chart): document polar line geometry space`

## Progress Notes

- 2026-06-22: Implemented the core `GeometrySpace` contract, mixed
  scalar/array polar transform broadcasting, `Line<Polar>` rendering for
  coordinate and display geometry space, facade/prelude exports, and
  non-visual behavior tests. Added focused WGPU visual baselines for
  arc-versus-chord, radial/spiral geometry, gaps/dashes, multi-series
  partitioning, categorical theta, and clipping under
  `avenger-chart/tests/baselines/polar_line/`. The categorical theta baseline
  hides the theta axis because ordinal polar tick generation is still a
  separate guide limitation. The compiled state uses `#[serde(default)]`
  without `skip_serializing_if` because the repository's bincode round trips
  require a stable field stream. Shared line extraction is limited to neutral
  core helpers for now; a fuller style partition abstraction remains unchecked
  in Phase 2.
- 2026-06-22: Added the remaining focused behavioral assertions for
  human-readable absent-field serde, scalar/scalar and array/array polar
  transforms, ordered polar line row sequencing, theta-as-given wrap
  interpolation, subdivision caps, and mark-level event datum lookup for
  densified coordinate-space lines. Updated `polar-geometry-space.md` and the
  mark-effects future-work note so `Line<Polar>` v1 is documented as
  implemented. The only unchecked Phase 2 items are now explicitly deferred
  refactors for a fuller shared Cartesian/Polar line style-partition helper;
  the v1 renderer uses the implemented neutral shared helpers instead.
