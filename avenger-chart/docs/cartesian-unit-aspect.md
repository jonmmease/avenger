# Cartesian Unit Aspect

`Cartesian::unit_aspect(ratio)` constrains the displayed length of x and y data
units. The ratio is:

```text
pixels_per_y_unit / pixels_per_x_unit
```

Use `Cartesian::new().unit_aspect(1.0)` or the shorthand
`Cartesian::new().equal_units()` when one x unit and one y unit should draw with
the same screen length. This is useful for maps in projected coordinates,
geometry diagrams, image-coordinate plots, and any chart where slopes or shapes
should not be distorted by the plot-area aspect.

```rust
use avenger_chart::prelude::*;
use datafusion::prelude::*;

let plot = Plot::with_coord(Cartesian::new().equal_units())
    .plot_size(600.0, 300.0)
    .mark(
        Line::new()
            .x_with(col("x"), |x| {
                x.scale_with::<Linear>(|s| {
                    s.domain((-5.0, 5.0)).nice(false).zero(false)
                })
            })
            .y_with(col("y"), |y| {
                y.scale_with::<Linear>(|s| {
                    s.domain((-5.0, 5.0)).nice(false).zero(false)
                })
            }),
    );
```

The constraint preserves the original domain centers and expands the minimum
required axis. It does not shrink domains or clip data. Guides, axes, rendered
marks, hit testing, and interaction coordinate scopes use the expanded domains.

## Ratios

`unit_aspect(2.0)` means one y unit is twice as long on screen as one x unit.
For example, a coordinate-metric square brush under `unit_aspect(2.0)` has
`dy/dx = 1/2`, because half as many y units draw with the same screen length as
the x span.

## Facets And Repeat

Facet and generated-repeat plots can share unit-aspect constrained domains. The
layout solver expands coordinated domains once for the group so each cell keeps
the requested unit ratio.

Authored concat/grid/wrap plots reject cases where a unit-aspect child also
shares the constrained x or y domain across child frames. Use local child
domains, facet/repeat sharing, or remove `unit_aspect` for those authored
concat/grid cases.

## Box Tools

Box tools do not constrain drags automatically. Opt in when the tool should
respect a Cartesian unit-aspect coordinate:

```rust
let zoom = BoxZoom::cartesian().unit_aspect();
let brush = BoxSelection::cartesian("brush").unit_aspect();
```

`BoxZoom::unit_aspect()` defaults to `UnitAspectBox::Viewport`, which constrains
the dragged box to the plot viewport aspect so the selected raw domains can
become the next view without immediate extra unit-aspect expansion.

`BoxSelection::unit_aspect()` defaults to `UnitAspectBox::CoordinateMetric`,
which constrains the dragged data rectangle so its x and y edges have equal
screen length under the frozen start-scale geometry.

Use the explicit mode when needed:

```rust
BoxSelection::cartesian("brush")
    .unit_aspect_box(UnitAspectBox::Viewport);

BoxZoom::cartesian()
    .unit_aspect_box(UnitAspectBox::CoordinateMetric);
```

The unit-aspect box options require an active coordinate metric for the tool's
x/y channels. `Cartesian::unit_aspect(...)` exposes that metric, so the tools do
not define a separate fixed-aspect brush independent of the coordinate unit
aspect.

## Restrictions

- `ratio` must be positive and finite.
- The constrained x and y channels must each resolve to exactly one distinct
  continuous linear numeric scale.
- Scale domains and ranges must have positive finite spans after normal scale
  inference, padding, `zero`, `nice`, explicit domains, and raw-domain
  parameters have been applied.
- Unit aspect expands scale domains; it does not shrink or letterbox the plot
  area.
- The expanded domain is not re-niced after the constraint is applied.
- Authored concat/grid/wrap plots reject cross-child sharing of constrained
  domains; facet and generated-repeat sharing are supported.
- Box zoom and box selection require explicit `.unit_aspect()` or
  `.unit_aspect_box(...)` opt-in.
