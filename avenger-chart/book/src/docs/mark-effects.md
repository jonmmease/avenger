# Mark Effects

Mark effects run after ordinary mark data has been prepared, channel encodings
have been evaluated, and positions have been transformed into plot-space
coordinates. They are useful when a mark needs display-space behavior such as a
pixel nudge, grouped dodge, halo, label, or overlap-aware label placement.

Effects are intentionally narrow. They are available as inherent methods on
built-in primitive marks, not as blanket methods on `Mark`, `IntoPlotMark`,
`MarkGroup`, `Subplot`, or compound/statistical mark builders.

## Effect Inputs

Effect closures receive an item accessor. The accessor exposes three families
of DataFusion expressions:

- `item.channel("x")`: the current post-scale, post-coordinate channel value.
- `item.data("field")`: a prepared source-data field for the current item.
- `item.bbox().left()`, `right()`, `top()`, and `bottom()`: display-space item
  bounds when the mark renderer exposes them.

These expressions are only valid inside mark effect closures. They are not
ordinary data expressions and cannot be used in normal mark channels, scale
domains, guide inputs, or pre-scale data transforms.

```rust,no_run
use avenger_chart::prelude::*;
use datafusion::{dataframe::DataFrame, prelude::*};

async fn expression_adjustment(
    ctx: &SessionContext,
    df: DataFrame,
) -> Result<(), Box<dyn std::error::Error>> {
    let plot = Plot::<Cartesian>::new()
        .data(df)
        .mark(
            Symbol::new()
                .x(col("x"))
                .y(col("y"))
                .adjust(|point| {
                    point
                        .x(point.channel("x") + lit(6.0))
                        .y(point.channel("y") - lit(4.0))
                }),
        );

    let _compiled = plot.compile(ctx).await?;
    Ok(())
}
```

Expression adjustments are evaluated in order. A later adjustment reads the
current item frame, including earlier adjustments.

## Adjustment Transforms

Use `.adjust_transform(...)` when an effect needs reusable code or access to
runtime context. A transform compiles to a serializable
`CompiledMarkAdjustmentTransform` plus typed output handles. The routing
closure maps those output handles back to mark channels.

```rust,no_run
use avenger_chart::prelude::*;
use datafusion::{dataframe::DataFrame, prelude::*};

async fn transform_adjustment(
    ctx: &SessionContext,
    df: DataFrame,
) -> Result<(), Box<dyn std::error::Error>> {
    let plot = Plot::<Cartesian>::new()
        .data(df)
        .mark(
            Symbol::new()
                .x(col("x"))
                .y(col("y"))
                .adjust_transform(Nudge::new(6.0, -4.0), |point, nudge| {
                    point.x(nudge.x()).y(nudge.y())
                }),
        );

    let _compiled = plot.compile(ctx).await?;
    Ok(())
}
```

Built-in adjustment transforms include:

- `Nudge::new(dx, dy)`: adds a fixed pixel offset to `x` and `y`.
- `Jitter::x()` / `Jitter::y()`: applies seeded random displacement along one
  axis. This is validated for `Symbol<Cartesian>`.
- `Dodge::x()` / `Dodge::y()`: offsets items by a source-data grouping field
  within the current plot area. This is validated for `Symbol<Cartesian>`.

Transform implementations can request plot-area metadata, a derived source
item frame, a base plot-area scene, or text measurement. The v1 scene-aware
path is available to custom adjustment transforms; base mark adjustments
receive only the plot-area metadata unless the renderer explicitly provides
more.

## Supported Primitive Marks

The implemented Cartesian primitive effect hosts are:

| Mark | Item grain | Supported adjustment writes |
| --- | --- | --- |
| `Symbol<Cartesian>` | one item per symbol | `x`, `y` |
| `Text<Cartesian>` | one item per text label | `x`, `y`, `defined`, `text`, and text layout channels used by the text adjustment frame |
| `Rule<Cartesian>` | one item per rule segment | `x`, `y`, `x2`, `y2` |
| `Rect<Cartesian>` | one item per rectangle | `x`, `y`, `x2`, `y2` |
| `Image<Cartesian>` | one item per image | `x`, `y`, `width`, `height` |
| `PathMark<Cartesian>` | one item per path instance | `x`, `y` anchor |
| `Line<Cartesian>` | one item per line vertex | `x`, `y` |
| `Trail<Cartesian>` | one item per trail vertex | `x`, `y` |
| `Area<Cartesian>` | one item per area vertex | `x`, `y`, `x2`, `y2` |

For style channels such as `fill`, `stroke`, and `stroke_width`, use ordinary
mark channels unless the renderer documents effect copy-back support for that
channel.

Adjusted marks preserve event-datum lineage. Instance-like marks keep one event
row per rendered item. Vertex-grain marks such as `Line`, `Trail`, and `Area`
may split one source mark into multiple rendered scene marks when style or
detail channels vary, but each rendered partition carries the corresponding
source row ids in vertex order.

## Derived Primitive Marks

`.derive(...)` creates additional built-in primitive marks from each source
item. Derived marks read post-scale source geometry through `item.channel(...)`
and source data through `item.data(...)`. They do not participate in scale
domain inference, guides, legends, or layout planning.

The implemented public source hosts are `Symbol<Cartesian>` and
`Rect<Cartesian>`. Derived output is limited to built-in primitive
`Symbol`, `Rule`, `Rect`, and `Text` marks. Derived output is one level deep:
recursive derived graphs, mark-local data on derived output, data transforms on
derived output, `MarkGroup` output, compound output, and external mark output
are rejected.

```rust,no_run
use avenger_chart::prelude::*;
use datafusion::{dataframe::DataFrame, prelude::*};

async fn derived_labels(
    ctx: &SessionContext,
    df: DataFrame,
) -> Result<(), Box<dyn std::error::Error>> {
    let plot = Plot::<Cartesian>::new()
        .data(df)
        .mark(
            Symbol::new()
                .x(col("x"))
                .y(col("y"))
                .size(80.0)
                .derive(|point| {
                    Text::new()
                        .x(point.channel("x") + lit(8.0))
                        .y(point.channel("y"))
                        .text(point.data("label"))
                        .align("left")
                        .baseline("middle")
                }),
        );

    let _compiled = plot.compile(ctx).await?;
    Ok(())
}
```

Derived marks inherit the source item identity. Event datum rows and hit-test
lineage therefore still refer back to the source mark's prepared data row.

## Compound Mark Child Effects

Compound and statistical marks are not public effect hosts. For the implemented
box plot outlier case, `BoxPlot::outliers(...)` exposes a child-style hook for
the generated outlier `Symbol<Cartesian>` primitive. Effects configured there
run on that generated primitive child.

```rust,no_run
use avenger_chart::prelude::*;
use datafusion::{dataframe::DataFrame, prelude::*};

async fn box_plot_outlier_effect(
    ctx: &SessionContext,
    df: DataFrame,
) -> Result<(), Box<dyn std::error::Error>> {
    let plot = Plot::<Cartesian>::new()
        .data(df)
        .mark(
            BoxPlot::new()
                .x(col("value"))
                .y(col("group"))
                .outliers(|outliers| {
                    outliers.adjust_transform(Nudge::new(4.0, 0.0), |point, nudge| {
                        point.x(nudge.x()).y(nudge.y())
                    })
                }),
        );

    let _compiled = plot.compile(ctx).await?;
    Ok(())
}
```

The generated outlier child keeps its event-datum identity and target path.
Internally derived outlier children, such as halos or labels, remain excluded
from scale-domain, guide, legend, and layout planning.
