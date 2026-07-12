# Repeat Containers

Repeat containers generate repeated child plots from lists of data expressions.
They are the high-level way to build scatterplot matrices, wrapped variable
galleries, and other layouts where the same chart template should be reused for
several fields.

Repeat lowers to ordinary [concat containers](concat.md). That means repeat
uses the same child-frame layout, axes, legends, domain coordination, tools,
stores, and selections as a manually authored concat grid.

## Repeat Variables

A `RepeatVariable` has an id and a DataFusion expression:

```rust,ignore
let variables = vec![
    RepeatVariable::new("bill_length", col("bill_length_mm")).title("Bill length (mm)"),
    RepeatVariable::new("bill_depth", col("bill_depth_mm")).title("Bill depth (mm)"),
    RepeatVariable::new("flipper", col("flipper_length_mm")).title("Flipper length (mm)"),
    RepeatVariable::new("mass", col("body_mass_g")).title("Body mass (g)"),
];
```

For simple field-backed variables, use:

```rust,ignore
let variables = vec![
    RepeatVariable::field("bill_length_mm"),
    RepeatVariable::field("bill_depth_mm"),
    RepeatVariable::field("flipper_length_mm"),
];
```

The id is used for repeat metadata and generated domain groups. The expression
is what the repeated child plot uses for data.

## Placeholder Expressions

Inside a repeated child plot, use placeholders for the active variable:

```rust,ignore
let cell = Plot::<Cartesian>::new().mark(
    Symbol::new()
        .x(repeat::column())
        .y(repeat::row())
        .fill_with(col("species"), |c| c.legend(|l| l.title("Species")))
        .size(36.0),
);
```

Common placeholders are:

| Placeholder | Meaning |
| --- | --- |
| `repeat::column()` | Current column variable expression |
| `repeat::row()` | Current row variable expression |
| `repeat::item()` | Current wrapped item variable expression |
| `repeat::column_title()` / `repeat::row_title()` / `repeat::item_title()` | Variable title |
| `repeat::*_id()` | Variable id |
| `repeat::*_index()` | Zero-based repeat position |
| `repeat::cell_id()` | Stable generated cell id |

Position placeholders return channel expressions, so they can carry scale and
axis defaults. Metadata placeholders return ordinary DataFusion expressions.

## Scatterplot Matrix

`RepeatGrid` repeats over row and column variables:

```rust,ignore
let plot = Chart::<RepeatGrid>::new()
    .data(df)
    .configure_coord(|c| {
        c.rows(variables.clone())
            .columns(variables)
            .cell(cell)
            .matrix_domains()
            .matrix_axes()
    });
```

`matrix_domains()` coordinates domains by repeat variable id. If a variable is
used on x in one cell and y in another cell, those domains are linked through a
shared named group. `matrix_axes()` applies matrix-style outer axes and uses
repeat variable titles where possible.

Use `matrix_domains_with_scope(scope)` when the matrix is nested inside facets
and should coordinate domains at a particular logical level:

```rust,ignore
let plot = Chart::<RepeatGrid>::new()
    .configure_coord(|c| {
        c.rows(variables.clone())
            .columns(variables)
            .cell(cell)
            .matrix_domains_with_scope(CoordinationScope::Level(1))
            .matrix_axes()
    });
```

## Conditional Cells

Use `cell_when` to choose a different child plot for some repeat cells. The
first matching branch wins; otherwise the default `cell(...)` plot is used.

This example uses histograms on the diagonal and scatter plots elsewhere:

```rust,ignore
let scatter = Plot::<Cartesian>::new().mark(
    Symbol::new()
        .x(repeat::column())
        .y(repeat::row())
        .fill_with(col("species"), |c| c.legend(|l| l.title("Species"))),
);

let histogram = Plot::<Cartesian>::new().mark(
    Rect::new().transform(Bin::new(repeat::column()).maxbins(24), |mark, bin| {
        mark.x(bin.start())
            .x2(bin.end())
            .y(lit(0.0))
            .y2(count())
    }),
);

let plot = Chart::<RepeatGrid>::new()
    .data(df)
    .configure_coord(|c| {
        c.rows(variables.clone())
            .columns(variables)
            .cell(scatter)
            .cell_when(repeat::row_index().eq(repeat::column_index()), histogram)
            .matrix_domains()
            .matrix_axes()
    });
```

The histogram's `repeat::column()` resolves to the same expression as the
diagonal cell's x variable.

## Wrapped Repeat

Use `RepeatWrap` when there is one list of variables and the layout should wrap:

```rust,ignore
let plot = Chart::<RepeatWrap>::new()
    .data(df)
    .configure_coord(|c| {
        c.items(variables)
            .columns(3)
            .cell(
                Plot::<Cartesian>::new().mark(
                    Symbol::new()
                        .x(repeat::item())
                        .y(col("target"))
                        .size(36.0),
                ),
            )
            .item_domains()
    });
```

For canvas-width-driven workflows, use `responsive_columns(approx_width)`:

```rust,ignore
let plot = Chart::<RepeatWrap>::new()
    .canvas_constraint(CanvasConstraint::width(width_param.expr()))
    .plot_constraint(PlotConstraint::height(160.0))
    .configure_coord(|c| {
        c.items(variables)
            .responsive_columns(180.0)
            .cell(cell)
    });
```

Responsive repeat uses the same column-count behavior as `WrapConcat`.

## Pan And Scroll Zoom

`PanScrollZoom` works with repeat domain coordination. Attach it to the repeated
cell plot:

```rust,ignore
let cell = Plot::<Cartesian>::new()
    .tool(PanScrollZoom::cartesian())
    .mark(
        Symbol::new()
            .x(repeat::column())
            .y(repeat::row())
            .size(36.0),
    );

let plot = Chart::<RepeatGrid>::new()
    .data(df)
    .configure_coord(|c| {
        c.rows(variables.clone())
            .columns(variables)
            .cell(cell)
            .matrix_domains()
            .matrix_axes()
    });
```

In a matrix, horizontal pan updates the repeated column variable domain and
vertical pan updates the repeated row variable domain. Any other cell using the
same variable updates through the named domain group.

## Box Selection

`BoxSelection` can use repeat placeholders as selection dimensions:

```rust,ignore
let brush = BoxSelection::cartesian("brush")
    .dimensions(repeat::column(), repeat::row())
    .resolve(BoxSelectionResolve::Union);

let cell = Plot::<Cartesian>::new()
    .tool(brush.clone())
    .mark(
        Symbol::new()
            .x(repeat::column())
            .y(repeat::row())
            .fill_with(lit("#b8beca"), |c| {
                c.no_scale()
                    .when_value(brush.predicate(), lit("#2563eb"))
                    .no_legend()
            }),
    );
```

The generated selection clauses contain the resolved source-data expressions,
so `brush.predicate()` can also be used in sibling concat plots.

## Choosing Repeat Or Concat

Use repeat when:

- cells are generated from a list of fields or expressions;
- the same template applies to many cells;
- you want SPLOM-style matrix domain and axis defaults;
- repeat-aware pan/zoom or selection should follow variable ids.

Use [concat](concat.md) when:

- each child plot is authored independently;
- you need a hand-tuned dashboard layout;
- cells have unrelated coordinate systems;
- the grid has custom holes or irregular structure.
