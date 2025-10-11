# Faceting System

> **Note**: Code examples in this document assume `use datafusion::prelude::*;` and relevant aggregate function imports like `use datafusion::functions_aggregate::first::first;`. Actual implementation will require proper imports.

## Overview

Faceting marks group data by column values and render filtered subplots for each group (data-driven replication). Each facet shows a subset of the data filtered by the faceting variable(s).

## Faceting Mark Types

| Mark | Layout | Use Case |
|------|--------|----------|
| `Facet` | Manual data-driven | Custom layouts, scatterpie, variable positioning/sizing |
| `FacetRow` | Automatic horizontal | Single-row horizontal arrangement |
| `FacetColumn` | Automatic vertical | Single-column vertical arrangement |
| `FacetWrap` | Automatic grid wrap | Small multiples with wrapping |
| `FacetGrid` | Automatic row×col matrix | Two-variable matrix layout |

## Common Features

All faceting marks:
- Group data by one or more faceting columns
- Render a complete inner `Plot` for each facet group
- Support nested coordinate systems (e.g., Cartesian outer, Polar inner)
- Pass data to each inner plot based on mark facet strategies
- Support multiple marks within inner plots with different data strategies

Inner plot marks support:
- **FacetStrategy** to control data filtering (Filter/Broadcast/Skip)
  - Filter: Mark sees only its facet's data (default)
  - Broadcast: Mark sees all data (for reference lines)
  - Skip: Conditional rendering based on data

Automatic layout marks (`FacetRow`, `FacetColumn`, `FacetWrap`, `FacetGrid`) additionally support:
- Scale sharing modes (Shared, Free, SharedX, SharedY, SharedRows, SharedCols)
- Axis display modes (All, Edges, None)

---

## Facet - Manual Control

Fully data-driven positioning and sizing. You specify x, y, width, and height as aggregate or scalar expressions.

### API

```rust
Facet::new()
    // Position channels (required)
    .x(expr)
    .x_with(expr, |config| { ... })
    .y(expr)
    .y_with(expr, |config| { ... })

    // Size channels (required)
    .width(expr)
    .width_with(expr, |config| { ... })
    .height(expr)
    .height_with(expr, |config| { ... })

    // Faceting (required)
    .facet_by(vec!["col1", "col2", ...])

    // Subplot (required)
    .subplot(Plot::<CoordSystem>::new()...)
```

### Position and Size Channels

All channels must use:
- **Aggregate expressions** (e.g., `first(col("x"))`, `mean(col("value"))`)
- **Scalar values** (e.g., `100.0`, `lit(150.0)`)
- **Expressions using only facet_by columns** (e.g., `col("row") * lit(200.0)`)

Available channels depend on outer coordinate system:
- **Cartesian:** `x`, `y`, `width`, `height`
- **Polar:** `r`, `theta`, `width`, `height`

### Examples

#### Basic Scatterpie

```rust
Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Facet::new()
            // Position at data points
            .x(first(col("center_x")))
            .y(first(col("center_y")))

            // Fixed size pies
            .width(100.0)
            .height(100.0)

            // One pie per location
            .facet_by(vec!["pie_id"])

            // Polar inner plot for pie slices
            .subplot(
                Plot::<Polar>::new()
                    .mark(
                        Arc::new()
                            .r(col("value"))
                            .theta(col("angle"))
                            .fill(col("category"))
                    )
                    .configure_guide(
                        PolarGuide::default()
                            .show_radial_axis(false)
                            .show_angular_axis(false)
                    )
            )
    )
```

#### Data-Driven Sizes

```rust
Facet::new()
    .x(first(col("x_pos")))
    .y(first(col("y_pos")))

    // Size based on total value with sqrt scaling
    .width_with(sum(col("value")), |c| {
        c.scale_with::<Sqrt>(|s| {
            s.domain((lit(0.0), lit(1000.0)))
             .range_interval(lit(50.0), lit(200.0))
        })
        .legend_with(|l| {
            l.title("Total Value")
             .position(LegendPosition::Right)
        })
    })
    .height_with(sum(col("value")), |c| {
        c.scale_with::<Sqrt>(|s| {
            s.domain((lit(0.0), lit(1000.0)))
             .range_interval(lit(50.0), lit(200.0))
        })
    })

    .facet_by(vec!["region"])
    .subplot(...)
```

#### Grid Layout via Facet Columns

```rust
Facet::new()
    // Position computed from facet columns
    .x(col("grid_col") * lit(150.0))
    .y(col("grid_row") * lit(150.0))

    .width(130.0)
    .height(130.0)

    // Facet by grid coordinates
    .facet_by(vec!["grid_row", "grid_col"])

    .subplot(...)
```

#### Multiple Inner Marks

```rust
Facet::new()
    .x(first(col("center_x")))
    .y(first(col("center_y")))
    .width(120.0)
    .height(120.0)

    .facet_by(vec!["pie_id"])

    .subplot(
        Plot::<Polar>::new()
            // Multiple marks in each subplot
            .mark(
                Arc::new()
                    .r(col("value"))
                    .theta(col("angle"))
                    .fill(col("category"))
                    .stroke("#fff")
                    .stroke_width(1.0)
            )
            .mark(
                Symbol::new()
                    .r(col("value") * lit(0.5))
                    .theta(col("angle"))
                    .size(50.0)
                    .fill("#000")
            )
    )
```

#### Scale Configuration

```rust
Facet::new()
    // Categorical position scale
    .x_with(first(col("category")), |c| {
        c.scale_with::<Band>(|s| {
            s.padding(0.2)
        })
        .axis_with(|a| {
            a.title("Category")
             .label_angle(45.0)
        })
    })

    .y_with(first(col("metric")), |c| {
        c.scale_with::<Band>(|s| {
            s.padding(0.1)
        })
    })

    .width(150.0)
    .height(150.0)

    .facet_by(vec!["category", "metric"])
    .subplot(...)
```

---

## FacetRow - Horizontal Layout

Arranges facets in a single horizontal row with automatic positioning and uniform sizing.

### API

```rust
FacetRow::new()
    // Faceting (required)
    .facet_by(vec!["column"])

    // Layout options
    .spacing(px)           // Default: 10.0
    .scale_sharing(mode)   // Default: ScaleSharing::Shared
    .axis_display(mode)    // Default: AxisDisplay::All

    // Size overrides (optional)
    .width(expr)           // Override computed width
    .height(expr)          // Override computed height

    // Subplot (required)
    .subplot(Plot::<CoordSystem>::new()...)
```

### Examples

#### Basic Horizontal Comparison

```rust
Plot::<Cartesian>::new()
    .data(df)
    .mark(
        FacetRow::new()
            .facet_by(vec!["quarter"])  // Q1, Q2, Q3, Q4
            .spacing(20.0)
            .subplot(
                Plot::<Cartesian>::new()
                    .mark(
                        Rect::new()
                            .x(col("category"))
                            .y(col("value"))
                            .fill(col("category"))
                    )
            )
    )
```

#### Free X Scales

```rust
FacetRow::new()
    .facet_by(vec!["region"])
    .spacing(15.0)
    .scale_sharing(ScaleSharing::SharedY)  // Shared Y scale, each region has own X scale
    .subplot(
        Plot::<Cartesian>::new()
            .mark(
                Line::new()
                    .x(col("date"))
                    .y(col("sales"))
            )
    )
```

#### Override Width

```rust
FacetRow::new()
    .facet_by(vec!["category"])
    .spacing(10.0)

    // Data-driven width instead of uniform
    .width_with(sum(col("count")), |c| {
        c.scale_with::<Linear>(|s| {
            s.range_interval(lit(100.0), lit(300.0))
        })
    })

    .subplot(...)
```

---

## FacetColumn - Vertical Layout

Arranges facets in a single vertical column with automatic positioning and uniform sizing.

### API

```rust
FacetColumn::new()
    // Faceting (required)
    .facet_by(vec!["column"])

    // Layout options
    .spacing(px)           // Default: 10.0
    .scale_sharing(mode)   // Default: ScaleSharing::Shared
    .axis_display(mode)    // Default: AxisDisplay::All

    // Size overrides (optional)
    .width(expr)           // Override computed width
    .height(expr)          // Override computed height

    // Subplot (required)
    .subplot(Plot::<CoordSystem>::new()...)
```

### Examples

#### Vertical Stacking

```rust
Plot::<Cartesian>::new()
    .data(df)
    .mark(
        FacetColumn::new()
            .facet_by(vec!["metric"])
            .spacing(15.0)
            .scale_sharing(ScaleSharing::SharedX)  // Shared X scale, each metric has own Y scale
            .subplot(
                Plot::<Cartesian>::new()
                    .mark(
                        Line::new()
                            .x(col("date"))
                            .y(col("value"))
                            .stroke("#2196F3")
                    )
            )
    )
```

#### Dashboard Layout

```rust
FacetColumn::new()
    .facet_by(vec!["panel"])
    .spacing(20.0)
    .scale_sharing(ScaleSharing::SharedX)  // Shared X scale, each panel has independent Y
    .subplot(
        Plot::<Cartesian>::new()
            .mark(
                Rect::new()
                    .x(col("time_bucket"))
                    .y(col("value"))
                    .fill(col("status"))
            )
    )
```

---

## FacetWrap - Grid Wrapping

Arranges facets in a grid that wraps after a specified number of columns. Similar to ggplot2's `facet_wrap`.

### API

```rust
FacetWrap::new()
    // Faceting (required)
    .facet_by(vec!["column"])

    // Layout options
    .columns(n)            // Number of columns (default: auto-compute)
    .spacing(px)           // Default: 10.0
    .scale_sharing(mode)   // Default: ScaleSharing::Shared
    .axis_display(mode)    // Default: AxisDisplay::All

    // Size overrides (optional)
    .width(expr)           // Override computed width
    .height(expr)          // Override computed height

    // Subplot (required)
    .subplot(Plot::<CoordSystem>::new()...)
```

### Examples

#### Classic Small Multiples

```rust
Plot::<Cartesian>::new()
    .data(df)
    .mark(
        FacetWrap::new()
            .facet_by(vec!["species"])
            .columns(3)
            .spacing(15.0)
            .subplot(
                Plot::<Cartesian>::new()
                    .mark(
                        Symbol::new()
                            .x(col("sepal_length"))
                            .y(col("sepal_width"))
                            .fill(col("species"))
                    )
            )
    )
```

#### Free Scales for Different Ranges

```rust
FacetWrap::new()
    .facet_by(vec!["country"])
    .columns(4)
    .spacing(12.0)
    .scale_sharing(ScaleSharing::Free)  // Each country gets own scales
    .subplot(
        Plot::<Cartesian>::new()
            .mark(
                Line::new()
                    .x(col("year"))
                    .y(col("gdp"))
                    .stroke("#4CAF50")
            )
    )
```

#### Auto-Compute Columns

```rust
FacetWrap::new()
    .facet_by(vec!["region"])
    // No .columns() specified - automatically computed based on number of facets
    .spacing(10.0)
    .subplot(...)
```

#### Polar Subplots in Grid

```rust
FacetWrap::new()
    .facet_by(vec!["region"])
    .columns(3)
    .spacing(20.0)
    .subplot(
        Plot::<Polar>::new()  // Nested coordinate system!
            .mark(
                Arc::new()
                    .r(col("value"))
                    .theta(col("angle"))
                    .fill(col("category"))
            )
            .configure_guide(
                PolarGuide::default()
                    .show_radial_axis(false)
            )
    )
```

---

## FacetGrid - Row×Column Matrix

Arranges facets in a matrix based on two faceting variables. Similar to ggplot2's `facet_grid`.

### API

```rust
FacetGrid::new()
    // Faceting (at least one required)
    .rows(column)          // Variable for rows
    .cols(column)          // Variable for columns

    // Layout options
    .spacing(px)           // Default: 10.0
    .scale_sharing(mode)   // Default: ScaleSharing::Shared
    .axis_display(mode)    // Default: AxisDisplay::All

    // Size overrides (optional)
    .width(expr)           // Override computed width
    .height(expr)          // Override computed height

    // Subplot (required)
    .subplot(Plot::<CoordSystem>::new()...)
```

### Examples

#### Two-Way Faceting

```rust
Plot::<Cartesian>::new()
    .data(df)
    .mark(
        FacetGrid::new()
            .rows("year")       // 3 unique years = 3 rows
            .cols("continent")  // 4 continents = 4 columns
            .spacing(12.0)
            .subplot(
                Plot::<Cartesian>::new()
                    .mark(
                        Line::new()
                            .x(col("month"))
                            .y(col("temperature"))
                            .stroke("#F44336")
                    )
            )
    )
```

#### Mixed Scale Sharing

```rust
FacetGrid::new()
    .rows("cylinder")
    .cols("origin")
    .spacing(10.0)
    .scale_sharing(ScaleSharing::SharedX)  // Share X (time), independent Y (values)
    .subplot(
        Plot::<Cartesian>::new()
            .mark(
                Symbol::new()
                    .x(col("weight"))
                    .y(col("mpg"))
                    .fill(col("origin"))
            )
    )
```

#### Rows Only

```rust
FacetGrid::new()
    .rows("region")  // Only rows, no columns
    .spacing(15.0)
    .subplot(...)
```

#### Columns Only

```rust
FacetGrid::new()
    .cols("category")  // Only columns, no rows
    .spacing(15.0)
    .subplot(...)
```

#### Variable Row Heights

```rust
FacetGrid::new()
    .rows("category")
    .cols("year")
    .spacing(10.0)

    // Data-driven height per category
    .height_with(max(col("value")), |c| {
        c.scale_with::<Linear>(|s| {
            s.range_interval(lit(100.0), lit(300.0))
        })
    })

    .subplot(...)
```

---
