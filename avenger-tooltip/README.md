# avenger-tooltip

Tooltips for an avenger app: an overlay that shows the data under the pointer,
with the event streams that find it.

## Add tooltips to an app

Give the marks that tooltips describe their rows as scene data, and add a
`Tooltips` overlay to the app:

```rust
let points = InstanceTarget::default().with_header("species").with_fields([
    TooltipField::new("sepal_length")
        .with_title("Sepal length")
        .with_value_format(".2f")
        .with_value_label("#value cm"),
]);
let app = app.with_overlay(Arc::new(Tooltips::over([points])))?;
```

A field's values read as an axis's ticks do: `value_format` is the pattern that
the text engine formats them with, as `tick_format` is, and `value_label` is
Typst markup where `#value` is the formatted value, as `tick_label` is with
`#tick`.

`Tooltips::new()` shows the nearest instance within 40 px of the pointer, among
every mark with rows, with its row's columns.

The tooltips find their data in the app's hit-testing tree and draw in its
overlay, so a hover never builds the scene or the tree. The text engine formats
the values, so it needs a number formatter for numbers and a datetime formatter
for dates and times.

## Targets

The tooltips try their targets in order, and the first that finds data at the
pointer shows it, so a target of points comes before a target of the regions
beneath them. There are two kinds.

An `InstanceTarget` shows one instance's row: a header field and a row for each
of its fields. It names the marks that it searches and the `FocusStrategy` that
finds the instance among them. Its tooltip is tinted by the instance's color.

| Strategy | Finds |
| --- | --- |
| `Exact` | The topmost instance under the pointer, such as a bar or a map region. |
| `Nearest { max_radius }` | The instance nearest the pointer. |
| `NearestX`, `NearestY { max_radius }` | The instance nearest the pointer along one axis. |

A `GroupTarget` shows each series' instance nearest the pointer's x, such as
each line's vertex in a time series: a header from the first series' row, and a
row for each series with its swatch, its title, and its value, under a
crosshair. Its entries are those of the series' legend, one mark and one title
for each series, in the legend's order:

```rust
let series = GroupTarget::new(marks, titles)?
    .with_max_radius(20.0)
    .with_limit(8)
    .with_header(TooltipField::new("step").with_title("Step"))
    .with_value(TooltipField::new("value").with_value_format(".2f"));
```

Its rows come in the entries' order, or, with `GroupOrder::Position`, down the
scene, as the series lie under the crosshair. With a limit, it shows the series
nearest the pointer's y.

With the `Data` anchor, a tooltip stands beside its data with a callout to the
data's edge. With the `Pointer` anchor, it stands below and right of the
pointer, as Vega's tooltips do, and follows it over an outline of the data.

`with_pinning()` lets a click pin a tooltip until a click elsewhere or Escape.
A pinned tooltip is a surface over the chart: it takes the presses and clicks on
its box from the app's streams, so a drag that starts on it doesn't pan, and it
shows the arrow over it. `with_delay(ms)` shows a new tooltip once the pointer
rests.
`with_style(TooltipStyle)` sets the look: the box and its text, the tint, the
crosshair, and the outline of the data.

A pinned tooltip keeps its data when the app builds its scene again. While a
mark's rows are the same batch, it keeps its row. When they change, a target's
`with_key(column)` finds the row that holds the pinned row's key, such as an id,
and without a key, the tooltip hides rather than show another row.

## Drawing your own overlay

The `draw` module holds the tooltip's marks without the app: `Tooltip`, a box
of a header and rows that `of_member` and `of_group` fill from a focus's rows
as a target describes them, and draws with `marks`, beside its data with a
callout, or `marks_beside`, below and right of the pointer. `instance_color`, `Swatch`, `highlight`, and
`crosshair` draw the rest of the packaged overlay, for an overlay that needs a
different behavior but the same look.
