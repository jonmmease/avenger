# avenger-app-tools

Tools for Avenger apps: the reusable interactions that an app adds with avenger-app's
`AppBuilder::tool`. Each tool declares its own scenes and streams and keeps its own state, as
avenger-app's `Tool` trait describes.

- `tooltips::Tooltips` describes the data under the pointer, from the rows behind the marks of the
  scenes below its own.
- `navigation::PanZoom`, behind the `scales` feature, pans and zooms an app's continuous scales,
  and `navigation::MapPanZoom`, behind the `geo` feature, pans and zooms a map projection.

## Tooltips

```rust
use avenger_app_tools::tooltips::{InstanceTarget, TooltipAnchor, TooltipField, Tooltips};

let tooltips = Tooltips::new("tooltip", 100).over([
    InstanceTarget::nearest(10.0)
        .with_marks(["quakes"])
        .with_header(
            TooltipField::new("magnitude")
                .with_value_format(".1f")
                .with_value_label("Magnitude #value"),
        )
        .with_fields([
            TooltipField::new("depth_km")
                .with_title("Depth")
                .with_value_format(".0f")
                .with_value_label("#value km"),
            TooltipField::new("time").with_title("Date").with_value_format("%b %d, %Y"),
        ]),
    InstanceTarget::exact()
        .with_marks(["countries"])
        .with_anchor(TooltipAnchor::Pointer)
        .with_header("name")
        .with_fields(["continent"]),
]);
```

`Tooltips` is a tool for any app state, which an app adds with `AppBuilder::tool`. It declares its
own scene, here `"tooltip"` at z-order 100, above the scenes that it describes. It keeps the
pointer in its own `TooltipState`, changes its scene only when the pointer moves or a scene below
it changes, and draws marks that take no hits, so every event reaches the scenes below it.

### Targets

An `InstanceTarget` finds one instance at the pointer in the hit-testing trees of the scenes below,
from the top scene down:

- `exact()` finds the topmost instance under the pointer.
- `nearest(max_radius)` finds the instance whose drawn edge lies nearest the pointer, within the
  radius. `with_metric` measures the distance along x or y alone.
- `with_marks(names)` limits the search to the marks with those names.

Only instances whose marks have rows are found, since a tooltip describes a row. A tooltip tries
its targets in order, and the first that finds an instance shows it. A target skips a tree that is
stale, as during a pan or a zoom, since a stale tree places the instances where they were.

### Fields

A tooltip shows its target's header, if any, as a bold first line beside a swatch of the instance's
color, unless the target is `without_swatch()`, and then a row for each field, with the field's
title on the left and its value on the right. Without fields, it shows every column of the row,
titled by the column's name.

A `TooltipField` reads as an axis's ticks read. `value_format` is the pattern of the text engine's
number or datetime provider, as an axis's `tick_format` is, and `value_label` is Typst markup with
`#value` bound to the formatted value, as an axis's `tick_label` is. A null value is empty. A field
whose column the rows lack draws no tooltip, and the tool logs why.

### Placement

- `TooltipAnchor::Data`, the default, sets the box beside the instance's drawn edge, with a callout
  to it, on the first of its right, left, top, and bottom sides where the box fits in the window.
- `TooltipAnchor::Pointer` sets the box below and to the right of the pointer, and flips it to the
  pointer's other side across or down where it would leave the window. It suits instances without
  one point to point at, such as a map's regions.

The tooltip outlines its instance's fill, clipped as the instance's mark is. `TooltipStyle` sets
the box's font, colors, padding, gaps, corner radius, callout, pointer offset, and outline.

## Navigation

The `scales` feature adds `PanZoom`, which pans and zooms continuous scales, and the `geo` feature
adds `MapPanZoom`, which pans and zooms a map projection. The app keeps the scales or the
projection in its state and draws with them in its build. It gives the tool selectors for them
and for the area of a scene where the tool's gestures apply:

```rust
let chart = AvengerApp::builder(state, Chart, engine)
    .tool(PanZoom::xy(PLOT, |s: &ChartState| s.plot, |s: &mut ChartState| &mut s.x, |s| &mut s.y));

let globe = MapPanZoom::new(MAPS, |s: &MapState| s.area, |s: &mut MapState| s.globe.as_mut(), PanMode::Sphere)
    .with_home(|s| s.fitted())
    .with_scale_limits(|s, projection| s.limits(projection));
let map = AvengerApp::builder(state, Map, engine).tool(globe);
```

- A drag pans the view from where it was at the press, so the place under the press stays under
  the pointer. The wheel zooms the view about the pointer by 20% a notch, at most two notches for
  one event. Each move and each wheel turn requests the scene without its tree.
- A drag comes to rest when it pauses for the settle delay, 100 ms by default, at its release,
  and at a canceled press, and the wheel's zooms come to rest when the wheel stops for the settle
  delay. A gesture that moved the view since its scene's tree last caught up then requests the
  scene with its tree, so a pause of the pointer doesn't settle the wheel. An app that
  resamples its data for the view therefore requests the resample in its build when
  `Scenes::requested_tree` reports the scene.
- A double-click restores the view's home, which is the view at the tool's first gesture, unless
  `MapPanZoom::with_home` gives the home, such as the projection fitted to the map.
- `PanZoom` moves each scale through `ConfiguredScale::pan` and `zoom`, which the linear scale
  supports. Each scale's range spans the area, and the tool reads only the range's direction.
  `MapPanZoom` moves the projection through `Projection::pan` and `zoom` in its `PanMode`, in the
  area's frame, and the wheel stops at the limits of `with_scale_limits`.
- A gesture that starts outside the area doesn't belong to the tool, so two tools on one scene
  each take the gestures in their own area. The tools consume no events.
