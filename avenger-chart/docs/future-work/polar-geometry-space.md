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
- `SceneLineMark` renders line-like geometry from display-space `x/y`
  arrays.
- `SceneTextMark` already supports display-space positions and degree
  rotation.
- The implemented [mark-effects.md](mark-effects.md) surface provides
  post-scale adjustments and derived marks that polar line/text support can
  eventually consume, but polar `GeometrySpace` remains a separate mark and
  coordinate-system semantic.

Implemented v1 pieces:

- `GeometrySpace` is a shared mark option in `avenger-chart-core` and is
  re-exported by `avenger-chart`.
- `Line<Polar>` supports `r`, `theta`, `r_with`, `theta_with`, and
  `.geometry_space(...)`.
- `Line<Polar>` defaults to `GeometrySpace::Coordinate`.
- Coordinate-space polar lines use deterministic path subdivision and emit
  densified `SceneLineMark` geometry.
- Display-space polar lines project source vertices and connect them with
  straight display-space chords.
- Mixed scalar/array polar positions broadcast scalars to arrays, so constant
  radius arcs are a normal line case.
- Visual baselines cover arc-versus-chord, radial and spiral-like segments,
  gaps, dashes, multi-series details, categorical theta, and clipping.
- `Text<Polar>` supports `r`, `theta`, `r_with`, `theta_with`, and
  `.geometry_space(...)`.
- `Text<Polar>` defaults to `GeometrySpace::Coordinate`.
- Coordinate-space polar text interprets `angle = 0` as radial outward and
  `angle = 90` as tangential in the positive-theta direction.
- Display-space polar text preserves Cartesian text angle semantics.
- `Text<Polar>` adjustments run after projection and coordinate-space angle
  conversion, so `x`, `y`, and `angle` are display-space text item channels.
- Visual baselines cover default radial orientation, explicit coordinate
  orientation, tangential orientation, display-space orientation, coordinate
  versus display overlay, leader lines, scaled theta, categorical theta, and
  keep-upright adjustment examples.

Remaining future pieces:

- public keep-upright or label-orientation adjustment transforms, if they
  prove generally useful outside tests,
- richer polar/local-basis metadata in generic effect frames,
- mark effects for `Line<Polar>` geometry frames,
- additional mark families that might expose `GeometrySpace`.

## Recommended Direction

Add `GeometrySpace` as a mark semantic option shared by line, text, and future
marks whose geometry can be interpreted before or after coordinate projection.

For polar, default to coordinate-space geometry where the mark's visual shape
is naturally part of the coordinate system:

- `Line<Polar>` should default to `GeometrySpace::Coordinate`.
- `Text<Polar>` defaults to `GeometrySpace::Coordinate`, prioritizing polar
  label semantics. Authors can opt into `GeometrySpace::Display` for
  screen-oriented labels.

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
geometry. The v1 implementation densifies scaled `r/theta` arrays and then
emits a `SceneLineMark` with the resulting display-space `x/y` samples.

That keeps existing line rendering, stroke, dash, cap, join, z-index, and event
behavior intact. It also means dash placement follows the approximated
coordinate-space path.

The v1 implementation deliberately does not introduce a line-arc scene mark.
It also avoids routing through `ScenePathMark`, because path marks do not yet
have line parity for dash behavior, source-row identity, retargeting, and
interaction.

Possible subdivision policies:

- fixed maximum angular step, such as every few degrees,
- fixed maximum display-space segment length,
- adaptive subdivision based on display-space chord error,
- user-configurable tolerance with a conservative default.

The implemented policy combines an angular step limit with a display-space
segment-length estimate and caps subdivisions per source segment. The tolerance
is internal for v1; public quality/performance knobs can be added later if
real charts need them.

### Angle Wrapping

Theta interpolation should be as given. There should be no shortest-path,
clockwise, counterclockwise, or wrap policy in the mark.

This keeps `GeometrySpace::Coordinate` literal: interpolate the scaled
coordinate values present in the mark evaluation frame. If an author wants a
different crossing at the angular wrap boundary, they should transform or
unwrap the data before it reaches the mark.

### Event Datum Identity

Coordinate-space polar lines can render more vertices than source rows. Those
inserted vertices are geometry samples only; retained event datum rows still
refer to the original source rows for the emitted scene line mark.

Current line hit testing is mark-level: `SceneLineMark` hits have
`instance_index: None`, not a nearest vertex or nearest source-row index. That
behavior is preserved for densified polar lines. Nearest-row or
nearest-segment line interactions need a separate interaction design.

### Example

```rust
use avenger_chart::prelude::*;

let plot = Plot::<Polar>::new().mark(
    Line::<Polar>::new()
        .r("radius")
        .theta("angle")
        .geometry_space(GeometrySpace::Coordinate)
        .stroke("#2563eb")
        .stroke_width(2.0),
);
```

Use `GeometrySpace::Display` when the desired geometry is a straight
display-space chord between projected polar points:

```rust
let plot = Plot::<Polar>::new().mark(
    Line::<Polar>::new()
        .r("radius")
        .theta("angle")
        .geometry_space(GeometrySpace::Display),
);
```

## `Text<Polar>`

`Text<Polar>` uses the existing `angle` channel, with `GeometrySpace` defining
the frame that angle is measured in.

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

The current polar implementation computes the display angle from scaled
`theta`:

```text
display_angle = theta.to_degrees() + angle
```

This is the same as using the direction of increasing `r` at fixed `theta` as
the local zero direction for the standard polar transform.

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

The effect API can express this with a transform-like adjustment:

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

`KeepUpright` should use the final display angle, not raw theta data. That way
it works for numeric, temporal, categorical, and custom-scaled angular
channels. The v1 implementation keeps this as a test-only adjustment transform;
there is no automatic mark-level flipping policy.

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

Future polar effect work may need to expose richer frame data:

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

`Line<Polar>` v1 is implemented; see
[polar-line-implementation-plan.md](polar-line-implementation-plan.md) for the
phase checklist, verification notes, and deferred refactoring items.

`Text<Polar>` v1 is implemented with coordinate-space and display-space angle
semantics, adjustment support, event datum preservation, and visual baselines.
The keep-upright behavior remains an adjustment-layer example rather than mark
default behavior.

## Decisions Needed

`Line<Polar>` and `Text<Polar>` v1 decisions are implemented. Remaining
decisions are about future mark families and higher-level effects:

- Which additional marks should expose `geometry_space`: maybe `Area`,
  `Trail`, `PathMark`, `Image`, and `Symbol` orientation.
- How `GeometrySpace` should compose with mark effects, especially
  adjustment transforms that need local coordinate bases.
