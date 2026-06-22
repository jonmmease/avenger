# Polar Geometry Space

## Goal Review

Polar coordinates currently support positioned symbols and positioned
subplots. Supporting polar line and text marks raises a semantic question that
does not exist in ordinary Cartesian plots: should geometry be constructed in
the polar coordinate frame, or after projection into display space?

For a line between two polar points, this is the difference between:

- connecting projected endpoints with a display-space chord, and
- interpolating scaled `r/theta` values first, then projecting the intermediate
  samples.

For text, this is the difference between:

- interpreting `angle = 0` as screen-horizontal, and
- interpreting `angle = 0` as the local coordinate-space zero direction,
  which is radial outward in polar coordinates.

The planned shared vocabulary is `GeometrySpace`:

```rust
pub enum GeometrySpace {
    Coordinate,
    Display,
}
```

This document records the polar-specific semantics. The concrete implementation
checklist for `Line<Polar>` is in
[polar-line-implementation-plan.md](polar-line-implementation-plan.md).

## Current System Fit

Useful existing pieces:

- `Polar` requires scaled `r` and `theta` channels and transforms them into
  `PointGeometry`.
- The default polar range maps `theta` to `0..2pi` and `r` to half the minimum
  plot-area dimension.
- `Symbol<Polar>` already uses the polar transform and emits ordinary
  scenegraph symbol marks.
- `SceneLineMark` already renders line-like geometry from display-space `x/y`
  arrays.
- `SceneTextMark` already supports display-space positions and degree
  rotation.
- The implemented [mark-effects.md](mark-effects.md) surface provides
  post-scale adjustments and derived marks that polar line/text support can
  eventually consume, but polar `GeometrySpace` remains a separate mark and
  coordinate-system semantic.

Missing pieces:

- `Line<Polar>` render implementation and polar line position-channel helpers,
- `Text<Polar>` render implementation and polar text position-channel helpers,
- a shared mark option for `geometry_space`,
- path resampling/subdivision for coordinate-space polar line segments,
- coordinate-basis calculation for polar text orientation,
- tests and visual baselines that distinguish coordinate-space and
  display-space behavior.

## Recommended Direction

Add `GeometrySpace` as a mark semantic option shared by line, text, and future
marks whose geometry can be interpreted before or after coordinate projection.

For polar, default to coordinate-space geometry where the mark's visual shape
is naturally part of the coordinate system:

- `Line<Polar>` should default to `GeometrySpace::Coordinate`.
- `Text<Polar>` should default to `GeometrySpace::Display` if compatibility
  with Cartesian text angle semantics is preferred, or
  `GeometrySpace::Coordinate` if polar label semantics are prioritized.
  This default should be chosen deliberately during the `Text<Polar>` spike.

The API should use one enum and mark-specific builder methods:

```rust
Line::<Polar>::new()
    .r("r")
    .theta("theta")
    .geometry_space(GeometrySpace::Coordinate);

Text::<Polar>::new()
    .r("r")
    .theta("theta")
    .text("label")
    .geometry_space(GeometrySpace::Coordinate)
    .angle(90.0);
```

Avoid overloading `interpolate` for this. In common visualization vocabulary,
`interpolate` usually describes line shape families such as linear, step,
basis, cardinal, or monotone. `GeometrySpace` describes where geometry is
constructed.

## `Line<Polar>`

### Coordinate-Space Lines

`GeometrySpace::Coordinate` means line segments are constructed in scaled polar
coordinates:

```text
r(t)     = r0 + t * (r1 - r0)
theta(t) = theta0 + t * (theta1 - theta0)
```

Then each intermediate sample is projected through the polar transform.

Special cases:

- constant `theta` produces a radial segment,
- constant `r` produces a circular arc,
- varying `r` and `theta` produces a spiral-like curve,
- undefined rows split the path exactly as Cartesian line gaps do.

This behavior treats the line as living in the polar coordinate system rather
than as an accidental chord in the framebuffer.

### Display-Space Lines

`GeometrySpace::Display` means each input row is first projected to local
plot-area `x/y`, then adjacent projected vertices are connected by straight
display-space segments.

This produces chords between polar points. It can be useful for annotations,
geometric overlays, or cases where authors explicitly want screen-space
straightness.

### Resampling

Coordinate-space polar lines need subdivision before emitting scenegraph
geometry. A first implementation can densify scaled `r/theta` arrays and then
emit a `SceneLineMark` with the resulting display-space `x/y` samples.

That keeps existing line rendering, stroke, dash, cap, join, z-index, and event
behavior intact. It also means dash placement follows the approximated
coordinate-space path.

Possible subdivision policies:

- fixed maximum angular step, such as every few degrees,
- fixed maximum display-space segment length,
- adaptive subdivision based on display-space chord error,
- user-configurable tolerance with a conservative default.

The `Line<Polar>` implementation plan starts with a simple deterministic
policy and records whether visual quality, dash placement, or performance
require adaptive refinement.

### Angle Wrapping

Theta interpolation should be as given. There should be no shortest-path,
clockwise, counterclockwise, or wrap policy in the mark.

This keeps `GeometrySpace::Coordinate` literal: interpolate the scaled
coordinate values present in the mark evaluation frame. If an author wants a
different crossing at the angular wrap boundary, they should transform or
unwrap the data before it reaches the mark.

## `Text<Polar>`

`Text<Polar>` should use the existing `angle` channel, but `GeometrySpace`
should define the frame that angle is measured in.

### Display-Space Orientation

`GeometrySpace::Display` preserves current text semantics:

- `angle = 0` means screen-horizontal,
- `angle = 90` means a 90-degree display-space rotation.

This is useful for labels that should remain upright and screen-oriented
regardless of where they appear in the polar plot.

### Coordinate-Space Orientation

`GeometrySpace::Coordinate` measures `angle` in the local polar basis:

- `angle = 0` means radial outward,
- `angle = 90` means tangential in the positive `theta` direction.

The implementation should compute a display-space basis from the scaled
`r/theta` values and the polar transform:

```text
e0 = direction of increasing r at fixed theta
e1 = direction of increasing theta at fixed r
```

Then the text's display angle is the basis angle plus the user-provided
`angle` channel.

This makes radial and tangential labels use the same ordinary text `angle`
channel:

```rust
Text::<Polar>::new()
    .r("r")
    .theta("theta")
    .text("label")
    .geometry_space(GeometrySpace::Coordinate)
    .angle(0.0);  // radial

Text::<Polar>::new()
    .r("r")
    .theta("theta")
    .text("label")
    .geometry_space(GeometrySpace::Coordinate)
    .angle(90.0); // tangential
```

### Upright Text

Keeping radial or tangential text mostly upright should be an adjustment, not
part of `GeometrySpace` itself.

The effect API can eventually express this with a transform-like adjustment:

```rust
Text::<Polar>::new()
    .r("r")
    .theta("theta")
    .text("label")
    .geometry_space(GeometrySpace::Coordinate)
    .angle(0.0)
    .adjust_transform(KeepUpright::new(), |text, upright| {
        text.angle(upright.angle())
    });
```

`KeepUpright` should use the scaled coordinate basis or final display angle,
not raw theta data. That way it works for numeric, temporal, categorical, and
custom-scaled angular channels.

## Interaction With Mark Effects

`GeometrySpace` is a mark semantic option, not an effect. It controls how the
mark builds its initial geometry frame.

Adjustments and derivations then operate on that frame:

- `KeepUpright` could adjust text angles after coordinate-space orientation is
  computed.
- `Nudge` can move text in display space after polar projection.
- smart label placement can avoid polar lines, symbols, and other scene
  obstacles using geometry indexes.
- derived labels can inherit source polar datums and use source geometry as
  anchors.

Polar marks should therefore expose enough frame data for effects:

- prepared rows,
- scaled `r/theta` encodings,
- display-space anchors,
- local coordinate basis for orientation-aware marks,
- source row and event datum identity.

## Alternate Paradigms

- **Always display-space**: simplest, because the existing point transform and
  scenegraph line mark already do this. It produces surprising chords for
  constant-radius polar lines.
- **Always coordinate-space**: best semantic default for polar data geometry,
  but it can be surprising for annotations that should be screen-straight.
- **Separate polar line/text mark types**: explicit, but it duplicates generic
  `Line` and `Text` APIs and makes composition harder.
- **Use path marks only**: authors could manually build SVG/path geometry, but
  that bypasses ordinary line/text encodings, scale-domain ownership, event
  datum identity, and mark effects.

## Readiness

`Line<Polar>` is ready for implementation planning; see
[polar-line-implementation-plan.md](polar-line-implementation-plan.md).

`Text<Polar>` remains ready for a design spike after line support lands.
Likely order:

1. Implement `Text<Polar>` with position channels and display-space angle.
2. Add coordinate-space text orientation and visual tests for radial and
   tangential labels.
3. Add `KeepUpright` only after the effect-stage output handle design is ready,
   or temporarily keep it as an internal spike helper.

## Decisions Needed

`Line<Polar>` decisions are closed in the implementation plan. Remaining
decisions are about future mark families:

- Whether `Text<Polar>` defaults to display space or coordinate space.
- Which additional marks should expose `geometry_space`: `Text`, maybe
  `Area`, `Trail`, `PathMark`, `Image`, and `Symbol` orientation.
- How `GeometrySpace` should compose with mark effects, especially
  adjustment transforms that need local coordinate bases.
