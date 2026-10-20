# avenger-tooltip

Tooltips for an avenger app: an overlay that shows the data under the pointer,
with the event streams that find it.

## Add tooltips to an app

Give the marks that tooltips describe their rows as scene data, and add a
`Tooltips` overlay to the app:

```rust
let tooltips = Tooltips::over(vec![TooltipTarget::default().with_content(
    TooltipContent::new().with_header("species").with_fields([
        TooltipField::new("sepal_length")
            .with_label("Sepal length")
            .with_format(".2f"),
    ]),
)]);
let app = app.with_overlay(Arc::new(tooltips))?;
```

`Tooltips::new()` shows the nearest instance within 40 px of the pointer, among
every mark with rows, with its row's columns.

The tooltips find their data in the app's hit-testing tree and draw in its
overlay, so a hover never builds the scene or the tree. The text engine formats
the values, so it needs a number formatter for numbers and a datetime formatter
for dates and times.

## Targets

A `TooltipTarget` names the marks that it searches, the `FocusStrategy` that
finds the data among them, the `TooltipAnchor` where the tooltip stands, and the
`TooltipContent` that it shows. The tooltips try their targets in order, and the
first that finds data at the pointer shows it, so a target of points comes
before a target of the regions beneath them.

| Strategy | Finds |
| --- | --- |
| `Exact` | The topmost instance under the pointer, such as a bar or a map region. |
| `Nearest { max_radius }` | The instance nearest the pointer. |
| `NearestX`, `NearestY { max_radius }` | The instance nearest the pointer along one axis. |
| `GroupX { max_radius, limit }` | Each mark's instance nearest the pointer's x, such as each series' vertex, and the `limit` of them nearest the pointer's y. |

With the `Data` anchor, a tooltip stands beside its data with a callout to the
data's edge. With the `Pointer` anchor, it stands below and right of the
pointer, as Vega's tooltips do, and follows it over an outline of the data. A tooltip of one instance is tinted by
the instance's color. A `GroupX` tooltip has a row for each member, labeled by
its mark's name with its swatch, and a crosshair at its x.

`with_pinning()` lets a click pin a tooltip until a click elsewhere or Escape,
and `with_delay(ms)` shows a new tooltip once the pointer rests.

## Drawing your own overlay

The `draw` module holds the tooltip's marks without the app: `Tooltip`, a box
of a header and rows that `of_member` and `of_group` fill from a focus's rows,
and draws with `marks`, beside its data with a callout, or `marks_beside`,
below and right of the pointer. `instance_color`, `Swatch`, `highlight`, and
`crosshair` draw the rest of the packaged overlay, for an overlay that needs a
different behavior but the same look.
