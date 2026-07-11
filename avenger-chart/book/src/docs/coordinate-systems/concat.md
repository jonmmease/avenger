# Concat Containers

Concat containers arrange complete child plots without splitting data. Use them
when you want to compose different plots side by side, build a dashboard grid,
or manually author the layout that higher-level repeat containers generate.

## Available Containers

| Container | Layout | Common use |
| --- | --- | --- |
| `HConcat` | One row, many columns | Side-by-side comparisons |
| `VConcat` | One column, many rows | Stacked panels |
| `GridConcat` | Explicit rows and columns | Dashboards, manual matrices |
| `WrapConcat` | Row-major wrapping | Responsive galleries |

Each child is an ordinary `Subplot::new(child_plot)` mark. The child plot keeps
its own coordinate system, marks, axes, legends, params, tools, and event
bindings.

## Horizontal And Vertical Concat

```rust,ignore
let left = Plot::<Cartesian>::new().mark(
    Symbol::new()
        .x(col("sepal_length"))
        .y(col("sepal_width")),
);

let right = Plot::<Cartesian>::new().mark(
    Rect::new()
        .x(col("species"))
        .y(lit(0.0))
        .y2(count()),
);

let plot = Plot::<HConcat>::new()
    .data(df)
    .mark(Subplot::new(left).key("scatter").label("Scatter"))
    .mark(Subplot::new(right).key("counts").label("Counts"));
```

Use `VConcat` the same way when the plots should stack vertically.

## Grid Concat

`GridConcat` places children at explicit grid cells:

```rust,ignore
let plot = Plot::<GridConcat>::new()
    .data(df)
    .configure_coord(|c| c.rows(2).columns(2))
    .mark(Subplot::new(top_left).grid_cell(0, 0))
    .mark(Subplot::new(top_right).grid_cell(0, 1))
    .mark(Subplot::new(bottom_left).grid_cell(1, 0));
```

Missing cells are holes. They keep their row/column tracks, and guide ownership
uses the outer non-empty cells in each row or column.

Grid concat is useful when you want full control. For example, a scatterplot
matrix can be authored manually with a grid of subplots and named domain groups
before you reach for the repeat convenience API.

## Wrapped Concat

`WrapConcat` lays children out row-major:

```rust,ignore
let plot = Plot::<WrapConcat>::new()
    .data(df)
    .configure_coord(|c| c.columns(3))
    .mark(Subplot::new(plot_a).key("a"))
    .mark(Subplot::new(plot_b).key("b"))
    .mark(Subplot::new(plot_c).key("c"))
    .mark(Subplot::new(plot_d).key("d"));
```

Use `responsive_columns(width)` when the chart has a canvas-constrained width
and should choose a column count from an approximate target cell width:

```rust,ignore
let plot = Plot::<WrapConcat>::new()
    .canvas_constraint(CanvasConstraint::width(width_param.expr()))
    .plot_constraint(PlotConstraint::height(160.0))
    .configure_coord(|c| c.responsive_columns(180.0))
    .mark(Subplot::new(plot_a))
    .mark(Subplot::new(plot_b))
    .mark(Subplot::new(plot_c));
```

The target is the approximate leaf plot-area width. The plot height is usually
plot-constrained so the wrapped layout can grow downward as columns change.

## Domain Coordination

Concat does not automatically share domains. Configure child plot channels with
`CoordinationScope` and optional named domain groups:

```rust,ignore
Symbol::new()
    .x_with(col("height"), |c| {
        c.with_domain_scope(CoordinationScope::Shared)
            .with_domain_group("height")
    })
    .y_with(col("weight"), |c| {
        c.with_domain_scope(CoordinationScope::Shared)
            .with_domain_group("weight")
    })
```

The group name is semantic. If one subplot uses a value on x and another uses
that same value on y, giving both channels the same domain group links their
domains even though the visual axis names differ.

## Axis Guide Visibility

`GridConcat` and `WrapConcat` can compact child axes:

```rust,ignore
let plot = Plot::<GridConcat>::new()
    .configure_coord(|c| {
        c.axis_guide_visibility(AxisGuideVisibilityPolicy::OuterEdges)
    });
```

`OuterForEquivalentDomainGroups` is the matrix-style policy. It hides interior
axes only when aligned cells use equivalent domain coordination targets:

```rust,ignore
let plot = Plot::<GridConcat>::new()
    .configure_coord(|c| {
        c.axis_guide_visibility(AxisGuideVisibilityPolicy::OuterForEquivalentDomainGroups)
    });
```

Repeat matrix axes use this policy internally, but manual concat grids can use
it directly.

## When To Use Concat

Use concat when:

- sibling plots have different mark types or coordinate systems;
- you need holes or an irregular grid;
- you want a manual layout with explicit child ids and keys;
- you want to combine a detail view with a summary or legend-like plot.

Use [repeat](repeat.md) when the layout is generated from lists of fields or
expressions and each cell follows a template.
