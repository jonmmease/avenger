# Nested Band Scale

> **Scale Type:** Categorical position

Nested band scales map struct-valued position data to hierarchical categorical
positions. They are useful when a single Cartesian axis should show categories
inside categories, such as grouped bars, Bokeh-style nested categorical axes,
and nested heatmaps.

## Data Model

Pass a DataFusion `Struct` expression to a Cartesian position channel. The
field order defines the nesting order, and the last field is the leaf band.
Every struct field is treated as categorical, including numeric, date, and
timestamp fields.

Use DataFusion's `named_struct` expression to build the nested position value:

```rust,no_run
use avenger_chart::prelude::*;
use datafusion::prelude::{col, lit, named_struct};

let quarter_team = named_struct(vec![
    lit("quarter"), col("quarter"),
    lit("team"), col("team"),
]);
```

An existing struct column works the same way. Its field order becomes the
nested level order, and its field names become default nested-axis titles.

## Grouped Bars

For standard grouped bars, use a nested x position and hide the leaf axis
level. A regular visual channel such as `fill` owns the legend.

```rust,no_run
use avenger_chart::prelude::*;
use datafusion::prelude::{col, lit, named_struct};

# fn example(df: datafusion::prelude::DataFrame) {
let quarter_team = named_struct(vec![
    lit("quarter"), col("quarter"),
    lit("team"), col("team"),
]);

let plot = Chart::<Cartesian>::new()
    .data(df)
    .legend("fill", |legend| legend.title("Team"))
    .mark(
        Rect::new()
            .x_with(quarter_team, |x| {
                x.axis(|axis| axis.title("Quarter").grid(false))
                    .level(0, |level| level.padding_inner(0.45).padding_outer(0.15))
                    .level(1, |level| {
                        level
                            .nest_scope(NestScope::Shared)
                            .padding_inner(0.08)
                            .axis(|axis| axis.visible(false))
                    })
            })
            .x2_with(col(":x"), |x| x.band(1.0))
            .y(lit(0.0))
            .y2(col("value"))
            .fill(col("team")),
    );
# let _ = plot;
# }
```

`NestScope::Shared` reserves the same leaf slots under every parent group. If
one quarter is missing a team, that team's slot is still reserved so the bars
align across quarters. Missing slots do not create marks.

## Nested Categorical Axis

For Bokeh-style nested categorical axes, leave the leaf level visible and use
the default `NestScope::Free` behavior. Parent groups become wider or narrower
according to how many leaf categories they contain, while the leaf bandwidth
stays constant.

```rust,no_run
use avenger_chart::prelude::*;
use datafusion::prelude::{col, lit, named_struct};

# fn example(df: datafusion::prelude::DataFrame) {
let cylinders_make = named_struct(vec![
    lit("cylinders"), col("cylinders"),
    lit("manufacturer"), col("manufacturer"),
]);

let plot = Chart::<Cartesian>::new().data(df).mark(
    Rect::new()
        .x_with(cylinders_make, |x| {
            x.axis(|axis| {
                axis.title("Manufacturer grouped by cylinders")
                    .grid(false)
                    .label_angle(-45.0)
            })
            .level(0, |level| level.padding_inner(0.35).padding_outer(0.2))
            .level(1, |level| level.padding_inner(0.06))
        })
        .x2_with(col(":x"), |x| x.band(1.0))
        .y(lit(0.0))
        .y2(col("mpg"))
        .fill(col("cylinders")),
);
# let _ = plot;
# }
```

## Parent Spans

`.band(t)` addresses the leaf band. Use `.level_band(level, t)` when a mark
needs the span of an ancestor level.

```rust,no_run
# use avenger_chart::prelude::*;
# use datafusion::prelude::{col, lit, named_struct};
# fn example(df: datafusion::prelude::DataFrame) {
# let region_item = named_struct(vec![lit("region"), col("region"), lit("item"), col("item")]);
let plot = Chart::<Cartesian>::new()
    .data(df)
    .mark(
        Rect::new()
            .x(region_item.clone())
            .x2_with(col(":x"), |x| x.band(1.0))
            .y(lit(0.0))
            .y2(col("value")),
    )
    .mark(
        Rule::new()
            .x_with(region_item, |x| x.level_band(0, 0.0))
            .x2_with(col(":x"), |x| x.level_band(0, 1.0))
            .y(lit(47.0))
            .y2(lit(47.0)),
    );
# let _ = plot;
# }
```

## Nested Heatmaps

Nested band scales can be used on both x and y. Set zero leaf padding when
heatmap cells should touch.

```rust,no_run
use avenger_chart::prelude::*;
use datafusion::prelude::{col, lit, named_struct};

# fn example(df: datafusion::prelude::DataFrame) {
let nested_x = named_struct(vec![
    lit("region"), col("region"),
    lit("product"), col("product"),
]);
let nested_y = named_struct(vec![
    lit("year"), col("year"),
    lit("quarter"), col("quarter"),
]);

let plot = Chart::<Cartesian>::new().data(df).mark(
    Rect::new()
        .x_with(nested_x, |x| {
            x.level(0, |level| level.padding_inner_px(10.0))
                .level(1, |level| level.padding_inner(0.0).padding_outer(0.0))
        })
        .x2_with(col(":x"), |x| x.band(1.0))
        .y_with(nested_y, |y| {
            y.level(0, |level| level.padding_inner_px(10.0))
                .level(1, |level| level.padding_inner(0.0).padding_outer(0.0))
        })
        .y2_with(col(":y"), |y| y.band(1.0))
        .fill(col("value")),
);
# let _ = plot;
# }
```

## Facets And Sharing

Facet sharing and nested sharing are independent. `domain_scope(...)` on a
nested level controls how that level's domain is coordinated across facets or
repeat containers. `nest_scope(...)` controls how child domains are arranged
inside one nested scale.

```rust,no_run
use avenger_chart::prelude::*;
use datafusion::prelude::{col, lit, named_struct};

# fn example(df: datafusion::prelude::DataFrame) {
let nested_x = named_struct(vec![
    lit("cylinders"), col("cylinders"),
    lit("manufacturer"), col("manufacturer"),
]);

let leaf = Plot::<Cartesian>::new().mark(
    Rect::new()
        .x_with(nested_x, |x| {
            x.level(0, |level| level.domain_scope(CoordinationScope::Shared))
                .level(1, |level| {
                    level
                        .domain_scope(CoordinationScope::Shared)
                        .nest_scope(NestScope::Free)
                })
        })
        .x2_with(col(":x"), |x| x.band(1.0))
        .y(lit(0.0))
        .y2(col("value")),
);

let plot = Chart::<FacetColumn>::new()
    .data(df)
    .mark(Subplot::new(leaf).column(col("market")));
# let _ = plot;
# }
```

In this example, cylinder groups align across facet columns. Manufacturer
domains are shared for each cylinder group, but different cylinders can still
have different manufacturer sets because the leaf level uses
`NestScope::Free`.

## Legends

Nested position levels do not create legends. They are position-domain levels,
so they are represented by the axis. If you want a legend for the inner group,
encode that field on a visual channel such as `fill`, `stroke`, or `shape` and
configure that channel's legend normally.
