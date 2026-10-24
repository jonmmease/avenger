# avenger-tooltip

Tooltips for Avenger apps: a layer that describes the data under the pointer, from the rows behind
the marks of the layers below it.

```rust
use avenger_tooltip::{InstanceTarget, TooltipAnchor, TooltipField, Tooltips};

let tooltips = Tooltips::over([
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

`Tooltips` is a `Layer` for any app state, which an app adds over its chart. It keeps the pointer
in its own state, builds again only when the pointer moves, and draws marks that take no hits, so
every event reaches the layers below it.

## Targets

An `InstanceTarget` finds one instance at the pointer in the hit-testing trees of the layers below,
from the top layer down:

- `exact()` finds the topmost instance under the pointer.
- `nearest(max_radius)` finds the instance whose drawn edge lies nearest the pointer, within the
  radius. `with_metric` measures the distance along x or y alone.
- `with_marks(names)` limits the search to the marks with those names.

Only instances whose marks have rows are found, since a tooltip describes a row. A tooltip tries
its targets in order, and the first that finds an instance shows it. A target skips a tree that is
stale, as during a pan or a zoom, since a stale tree places the instances where they were.

## Fields

A tooltip shows its target's header, if any, as a bold first line beside a swatch of the instance's
color, unless the target is `without_swatch()`, and then a row for each field, with the field's
title on the left and its value on the right. Without fields, it shows every column of the row,
titled by the column's name.

A `TooltipField` reads as an axis's ticks read. `value_format` is the pattern of the text engine's
number or datetime provider, as an axis's `tick_format` is, and `value_label` is Typst markup with
`#value` bound to the formatted value, as an axis's `tick_label` is. A null value is empty. A field
whose column the rows lack draws no tooltip, and the layer logs why.

## Placement

- `TooltipAnchor::Data`, the default, sets the box beside the instance's drawn edge, with a callout
  to it, on the first of its right, left, top, and bottom sides where the box fits in the layer.
- `TooltipAnchor::Pointer` sets the box below and to the right of the pointer, and flips it to the
  pointer's other side across or down where it would leave the layer. It suits instances without
  one point to point at, such as a map's regions.

The tooltip outlines its instance's fill, clipped as the instance's mark is. `TooltipStyle` sets
the box's font, colors, padding, gaps, corner radius, callout, pointer offset, and outline.
