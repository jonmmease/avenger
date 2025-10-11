# Repeat System

> **Note**: Code examples in this document assume `use datafusion::prelude::*;` and relevant aggregate function imports. Actual implementation will require proper imports.

## Repeat Marks

Repeat marks create subplots by iterating over a list of **variable names** (columns in the dataset), allowing you to show the same visualization for different data dimensions. Unlike faceting which partitions data into groups, repeat shows the full dataset in each subplot with different column mappings.

### Key Differences from Faceting

| Aspect | Faceting | Repeat |
|--------|----------|--------|
| **Driven by** | Data values (groups) | Variable/column names |
| **Data per subplot** | Filtered (group's data) | Full dataset (all rows) |
| **What varies** | Which rows are shown | Which columns are encoded |
| **Use case** | "Show for each species" | "Show for each measurement variable" |
| **Example** | One plot per species value | One plot per measurement column |

### Mark Types

| Mark | Layout | Use Case |
|------|--------|----------|
| `Repeat` | Manual data-driven | Variable space exploration, statistics-based positioning |
| `RepeatRow` | Automatic horizontal | Horizontal comparison of variables |
| `RepeatColumn` | Automatic vertical | Vertical stacking of variables |
| `RepeatWrap` | Automatic grid wrap | Small multiples across variables |
| `RepeatGrid` | Automatic row×col matrix | SPLOM (scatter plot matrix) |

### Common Features

All repeat marks:
- Iterate over a list of variable names
- Render a complete inner `Plot` for each variable
- Pass the full dataset to each subplot
- Support subplot closures that create plots based on variable names
- Support nested coordinate systems

Automatic layout marks (`RepeatRow`, `RepeatColumn`, `RepeatWrap`, `RepeatGrid`) support:
- Scale sharing modes (Shared, Free, SharedX, SharedY, PerVariable)
- Axis display modes (All, Edges, None)

---

## Repeat - Variable Space Exploration

Create subplots positioned based on variable statistics. This enables "meta-visualizations" where subplot positions reveal relationships between variables.

### API

```rust
Repeat::new()
    // Position channels (closures that generate expressions)
    .x(|(var, idx)| expr)
    .x_with(|(var, idx)| expr, |config| { ... })
    .y(|(var, idx)| expr)
    .y_with(|(var, idx)| expr, |config| { ... })

    // Size channels
    .width(|(var, idx)| expr)
    .width_with(|(var, idx)| expr, |config| { ... })
    .height(|(var, idx)| expr)
    .height_with(|(var, idx)| expr, |config| { ... })

    // Variables to iterate over (required)
    .variables(vec!["col1", "col2", ...])

    // Subplot specification (required)
    .subplot(|(var, idx)| Plot::<CoordSystem>::new()...)
```

### Position and Size Closures

Closures receive `(var: &str, idx: usize)` and return expressions:
- **Aggregate expressions**: `mean(col(var))`, `stddev(col(var))`
- **Scalar values**: `lit(100.0 + idx as f64 * 50.0)`
- **Computed statistics**: `max(col(var)) - min(col(var))`

The expressions are evaluated once per variable to determine subplot positions and sizes.

### Examples

#### Position by Mean and Variance

```rust
Plot::<Cartesian>::new()
    .data(sensor_df)
    .mark(
        Repeat::new()
            .variables(vec!["temperature", "humidity", "pressure", "wind_speed"])

            // Position based on statistical properties
            .x(|(var, idx)| mean(col(var)))
            .y(|(var, idx)| variance(col(var)))

            // Size based on data coverage
            .width_with(|(var, idx)| count(col(var)), |c| {
                c.scale_with::<Sqrt>(|s| {
                    s.range_interval(lit(100.0), lit(250.0))
                })
                .legend_with(|l| l.title("Sample Size"))
            })
            .height_with(|(var, idx)| count(col(var)), |c| {
                c.scale_with::<Sqrt>(|s| {
                    s.range_interval(lit(100.0), lit(250.0))
                })
            })

            // Histogram for each variable
            .subplot(|(var, idx)| {
                Plot::<Cartesian>::new()
                    .mark(
                        Histogram::new()
                            .x(col(var))
                            .bins(30)
                            .fill(lit(COLORS[idx % COLORS.len()]))
                    )
            })
    )
    .title("Sensor Variable Space")
```

**Result:** Histograms positioned at (mean, variance) revealing relationships between variable distributions.

#### Correlation-Based Layout

```rust
Repeat::new()
    .variables(vec!["feature1", "feature2", "feature3", "feature4", "feature5"])

    // X = correlation with target variable
    .x(|(var, idx)| correlation(col(var), col("target_metric")))

    // Y = use index for vertical separation
    .y(|(var, idx)| lit(idx as f64 * 200.0))

    // Width based on correlation strength
    .width_with(|(var, idx)| abs(correlation(col(var), col("target_metric"))), |c| {
        c.scale_with::<Linear>(|s| {
            s.range_interval(lit(200.0), lit(500.0))
        })
    })
    .height(|(var, idx)| lit(150.0))

    // Time series for each feature
    .subplot(|(var, idx)| {
        Plot::<Cartesian>::new()
            .title(var.replace("_", " ").to_title_case())
            .mark(
                Line::new()
                    .x(col("date"))
                    .y(col(var))
                    .stroke("#2196F3")
            )
    })
```

**Result:** Features ordered by importance (correlation strength determines width and x-position).

#### Grid by Index

```rust
Repeat::new()
    .variables(vec!["metric1", "metric2", "metric3", "metric4"])

    // Manual grid using indices
    .x(|(var, idx)| lit((idx % 2) as f64 * 400.0))
    .y(|(var, idx)| lit((idx / 2) as f64 * 300.0))

    .width(|(var, idx)| lit(380.0))
    .height(|(var, idx)| lit(280.0))

    .subplot(|(var, idx)| {
        Plot::<Cartesian>::new()
            .title(var.to_title_case())
            .mark(
                Rect::new()
                    .x(col("category"))
                    .y(col(var))
                    .fill(col("category"))
            )
    })
```

**Result:** 2×2 grid with manual positioning control via index arithmetic.

#### Variable Properties Visualization

```rust
Repeat::new()
    .variables(vec!["price", "volume", "market_cap", "pe_ratio"])

    // Position reveals distribution shape
    .x(|(var, idx)| mean(col(var)))
    .y(|(var, idx)| stddev(col(var)))

    // Size shows data range
    .width_with(|(var, idx)| percentile(col(var), 0.95) - percentile(col(var), 0.05), |c| {
        c.scale_with::<Sqrt>(|s| {
            s.range_interval(lit(80.0), lit(300.0))
        })
        .legend_with(|l| l.title("Value Range (90% interval)"))
    })
    .height_with(|(var, idx)| percentile(col(var), 0.95) - percentile(col(var), 0.05), |c| {
        c.scale_with::<Sqrt>(|s| {
            s.range_interval(lit(80.0), lit(300.0))
        })
    })

    // Polar plot showing distribution
    .subplot(|(var, idx)| {
        Plot::<Polar>::new()
            .mark(
                Arc::new()
                    .theta(bin(col(var), 12))
                    .r(count())
                    .fill(lit(COLORS[idx % COLORS.len()]))
                    .stroke("#ffffff")
                    .stroke_width(1.0)
            )
            .configure_guide(
                PolarGuide::default()
                    .show_radial_axis(false)
                    .show_angular_axis(false)
            )
    })
```

**Result:** Polar histograms positioned by (mean, stddev) with size indicating range.

---

## RepeatRow - Horizontal Variable Comparison

Arrange subplots horizontally, one for each variable.

### API

```rust
RepeatRow::new()
    // Variables (required)
    .variables(vec!["var1", "var2", ...])

    // Layout options
    .spacing(px)           // Default: 10.0
    .scale_sharing(mode)   // Default: ScaleSharing::Shared
    .axis_display(mode)    // Default: AxisDisplay::All

    // Subplot specification (required)
    .subplot(|(var, idx)| Plot::<CoordSystem>::new()...)
```

### Examples

#### Time Series Comparison

```rust
Plot::<Cartesian>::new()
    .data(stock_df)
    .mark(
        RepeatRow::new()
            .variables(vec!["sales", "profit", "revenue"])
            .spacing(20.0)
            .scale_sharing(ScaleSharing::SharedX)  // Share time axis
            .subplot(|(var, idx)| {
                Plot::<Cartesian>::new()
                    .title(var.to_title_case())
                    .mark(
                        Line::new()
                            .x(col("date"))
                            .y(col(var))
                            .stroke("#2196F3")
                    )
            })
    )
    .title("Business Metrics Over Time")
```

#### Multi-Year Comparison

```rust
RepeatRow::new()
    .variables(vec!["2020", "2021", "2022", "2023"])
    .spacing(15.0)
    .subplot(|(year, idx)| {
        Plot::<Cartesian>::new()
            .title(format!("Year {}", year))
            .mark(
                Rect::new()
                    .x(col("category"))
                    .y(col(year))  // Different column each subplot
                    .fill("#4CAF50")
            )
    })
```

---

## RepeatColumn - Vertical Variable Stacking

Arrange subplots vertically, one for each variable.

### API

```rust
RepeatColumn::new()
    // Variables (required)
    .variables(vec!["var1", "var2", ...])

    // Layout options
    .spacing(px)           // Default: 10.0
    .scale_sharing(mode)   // Default: ScaleSharing::Shared
    .axis_display(mode)    // Default: AxisDisplay::All

    // Subplot specification (required)
    .subplot(|(var, idx)| Plot::<CoordSystem>::new()...)
```

### Examples

#### Dashboard Stack

```rust
Plot::<Cartesian>::new()
    .data(df)
    .mark(
        RepeatColumn::new()
            .variables(vec!["cpu_usage", "memory_usage", "disk_io", "network_traffic"])
            .spacing(15.0)
            .scale_sharing(ScaleSharing::SharedX)  // Shared time axis
            .subplot(|(metric, idx)| {
                Plot::<Cartesian>::new()
                    .title(metric.replace("_", " ").to_title_case())
                    .mark(
                        Area::new()
                            .x(col("timestamp"))
                            .y(col(metric))
                            .fill(lit(COLORS[idx % COLORS.len()]))
                            .opacity(0.7)
                    )
            })
    )
    .title("System Metrics")
```

#### Vertical Variable Exploration

```rust
RepeatColumn::new()
    .variables(vec!["sepal_length", "sepal_width", "petal_length", "petal_width"])
    .spacing(10.0)
    .scale_sharing(ScaleSharing::Free)  // Each variable has own scale
    .subplot(|(var, idx)| {
        Plot::<Cartesian>::new()
            .mark(
                Violin::new()
                    .x(col("species"))
                    .y(col(var))
                    .fill(col("species"))
            )
    })
```

---

## RepeatWrap - Grid of Variables

Arrange subplots in a grid that wraps after a specified number of columns.

### API

```rust
RepeatWrap::new()
    // Variables (required)
    .variables(vec!["var1", "var2", ...])

    // Layout options
    .columns(n)            // Number of columns (default: auto-compute)
    .spacing(px)           // Default: 10.0
    .scale_sharing(mode)   // Default: ScaleSharing::Shared
    .axis_display(mode)    // Default: AxisDisplay::All

    // Subplot specification (required)
    .subplot(|(var, idx)| Plot::<CoordSystem>::new()...)
```

### Examples

#### Weather Variables Grid

```rust
Plot::<Cartesian>::new()
    .data(weather_df)
    .mark(
        RepeatWrap::new()
            .variables(vec![
                "temperature", "precipitation", "wind_speed",
                "humidity", "pressure", "cloud_cover"
            ])
            .columns(3)  // 2×3 grid
            .spacing(15.0)
            .scale_sharing(ScaleSharing::SharedX)  // Shared time axis
            .subplot(|(var, idx)| {
                Plot::<Cartesian>::new()
                    .title(var.replace("_", " ").to_title_case())
                    .mark(
                        Line::new()
                            .x(col("date"))
                            .y(col(var))
                            .stroke(lit(COLORS[idx % COLORS.len()]))
                    )
            })
    )
    .title("Weather Station Data")
```

#### Distribution Overview

```rust
RepeatWrap::new()
    .variables(vec!["age", "income", "education_years", "household_size"])
    .columns(2)
    .spacing(20.0)
    .scale_sharing(ScaleSharing::Free)
    .subplot(|(var, idx)| {
        Plot::<Cartesian>::new()
            .title(var.replace("_", " ").to_title_case())
            .mark(
                Histogram::new()
                    .x(col(var))
                    .bins(30)
                    .fill("#2196F3")
            )
    })
```

---

## RepeatGrid - Scatter Plot Matrix (SPLOM)

Create a matrix of subplots based on two lists of variables. Perfect for scatter plot matrices where each cell shows a different variable pairing.

### API

```rust
RepeatGrid::new()
    // Variables (at least one required)
    .rows(vec!["var1", "var2", ...])    // Variables for rows
    .cols(vec!["var1", "var2", ...])    // Variables for columns

    // Layout options
    .spacing(px)           // Default: 10.0
    .scale_sharing(mode)   // Default: ScaleSharing::PerVariable
    .axis_display(mode)    // Default: AxisDisplay::Edges
    .show_upper(bool)      // Show upper triangle (default: true)
    .show_lower(bool)      // Show lower triangle (default: true)
    .show_diagonal(bool)   // Show diagonal (default: true)

    // Subplot specification (required)
    .subplot(|(row_var, row_idx), (col_var, col_idx)| Plot::<CoordSystem>::new()...)
```

### Subplot Specification

The subplot closure receives two tuples for row and column variables:
```rust
|(row_var, row_idx), (col_var, col_idx)| {
    // row_var: &str - variable name for Y axis
    // row_idx: usize - row index (0-based)
    // col_var: &str - variable name for X axis
    // col_idx: usize - column index (0-based)

    // Can check if diagonal
    if row_idx == col_idx {
        // Diagonal cell: single variable
    } else {
        // Off-diagonal: two variables
    }
}
```

### Examples

#### Classic SPLOM

```rust
Plot::<Cartesian>::new()
    .data(iris_df)
    .mark(
        RepeatGrid::new()
            .rows(vec!["sepal_length", "sepal_width", "petal_length", "petal_width"])
            .cols(vec!["sepal_length", "sepal_width", "petal_length", "petal_width"])
            .spacing(3.0)
            .scale_sharing(ScaleSharing::PerVariable)
            .axis_display(AxisDisplay::Edges)
            .subplot(|(row_var, row_idx), (col_var, col_idx)| {
                if row_idx == col_idx {
                    // Diagonal: distribution
                    Plot::<Cartesian>::new()
                        .mark(
                            Histogram::new()
                                .x(col(row_var))
                                .bins(20)
                                .fill(col("species"))
                                .opacity(0.6)
                        )
                } else {
                    // Off-diagonal: scatter
                    Plot::<Cartesian>::new()
                        .mark(
                            Symbol::new()
                                .x(col(col_var))
                                .y(col(row_var))
                                .fill(col("species"))
                                .size(20.0)
                                .opacity(0.7)
                        )
                }
            })
    )
    .title("Iris Scatter Plot Matrix")
```

#### Upper/Lower Triangle Differentiation

```rust
RepeatGrid::new()
    .rows(vars.clone())
    .cols(vars.clone())
    .spacing(2.0)
    .subplot(|(row_var, row_idx), (col_var, col_idx)| {
        use std::cmp::Ordering;

        match row_idx.cmp(&col_idx) {
            Ordering::Equal => {
                // Diagonal: density plot
                Plot::<Cartesian>::new()
                    .mark(
                        Density::new()
                            .x(col(row_var))
                            .fill(col("category"))
                            .opacity(0.5)
                    )
            }
            Ordering::Less => {
                // Upper triangle: scatter
                Plot::<Cartesian>::new()
                    .mark(
                        Symbol::new()
                            .x(col(col_var))
                            .y(col(row_var))
                            .fill(col("category"))
                    )
            }
            Ordering::Greater => {
                // Lower triangle: smoothed trend
                Plot::<Cartesian>::new()
                    .mark(
                        Smooth::new()
                            .x(col(col_var))
                            .y(col(row_var))
                            .stroke(col("category"))
                            .method(SmoothMethod::Loess)
                    )
            }
        }
    })
```

#### Lower Triangle Only

```rust
RepeatGrid::new()
    .rows(vec!["var1", "var2", "var3", "var4"])
    .cols(vec!["var1", "var2", "var3", "var4"])
    .show_upper(false)    // Hide upper triangle
    .show_diagonal(true)  // Show diagonal
    .show_lower(true)     // Show lower triangle
    .spacing(5.0)
    .subplot(|(row_var, row_idx), (col_var, col_idx)| {
        if row_idx == col_idx {
            Plot::<Cartesian>::new()
                .mark(Histogram::new().x(col(row_var)))
        } else {
            Plot::<Cartesian>::new()
                .mark(Symbol::new().x(col(col_var)).y(col(row_var)))
        }
    })
```

#### Asymmetric Grid

```rust
RepeatGrid::new()
    .rows(vec!["sales", "profit", "revenue"])  // 3 rows
    .cols(vec!["Q1", "Q2", "Q3", "Q4"])        // 4 columns
    .spacing(10.0)
    .subplot(|(metric, metric_idx), (quarter, quarter_idx)| {
        Plot::<Cartesian>::new()
            .title(format!("{} - {}", quarter, metric))
            .mark(
                Rect::new()
                    .x(col("category"))
                    .y(col(format!("{}_{}", metric, quarter)))
                    .fill(col("category"))
            )
    })
```

**Result:** 3×4 grid showing different metrics across quarters.

#### Correlation Matrix Alternative

```rust
RepeatGrid::new()
    .rows(vars.clone())
    .cols(vars.clone())
    .spacing(1.0)
    .axis_display(AxisDisplay::Edges)
    .subplot(|(row_var, row_idx), (col_var, col_idx)| {
        if row_idx == col_idx {
            // Diagonal: variable name
            Plot::<Cartesian>::new()
                .mark(
                    Text::new()
                        .x(lit(0.5))
                        .y(lit(0.5))
                        .text(lit(row_var))
                        .font_size(14.0)
                )
        } else {
            // Off-diagonal: compute and show correlation
            let corr_expr = correlation(col(row_var), col(col_var));

            Plot::<Cartesian>::new()
                .mark(
                    Rect::new()
                        .x(lit(0.0))
                        .y(lit(0.0))
                        .width(lit(1.0))
                        .height(lit(1.0))
                        .fill_with(corr_expr.clone(), |c| {
                            c.scale_with::<Linear>(|s| {
                                s.domain((lit(-1.0), lit(1.0)))
                                 .range_colors(vec!["#d73027", "#f7f7f7", "#4575b4"])
                            })
                        })
                )
                .mark(
                    Text::new()
                        .x(lit(0.5))
                        .y(lit(0.5))
                        .text(format!("{:.2}", corr_expr))
                        .fill("#000000")
                )
        }
    })
```

**Result:** Heatmap-style correlation matrix with values displayed.

---

## Scale Sharing for Repeat

Repeat marks support the same scale sharing modes as faceting, plus a special `PerVariable` mode for SPLOM.

### PerVariable (SPLOM-specific)

Each variable gets one consistent scale domain that applies wherever that variable appears (on either X or Y axis).

```rust
RepeatGrid::new()
    .rows(vec!["var1", "var2", "var3"])
    .cols(vec!["var1", "var2", "var3"])
    .scale_sharing(ScaleSharing::PerVariable)  // Default for RepeatGrid
    .subplot(|(row_var, row_idx), (col_var, col_idx)| {
        Plot::<Cartesian>::new()
            .mark(Symbol::new().x(col(col_var)).y(col(row_var)))
    })
```

**Result:**
- All cells with "var1" on X axis share the same X scale
- All cells with "var1" on Y axis share the same Y scale
- "var1" has consistent meaning throughout the matrix
- Enables proper visual comparison across all cells

**Scale resolution:**
1. Compute domain for each variable across full dataset
2. Create one scale per variable
3. For cell (row_i, col_j):
   - X scale = variable_scales[col_j]
   - Y scale = variable_scales[row_i]

### Other Modes

All other ScaleSharing modes work as with faceting:

```rust
// All cells share all scale domains
.scale_sharing(ScaleSharing::Shared)

// Each cell independent
.scale_sharing(ScaleSharing::Free)

// Shared X, independent Y
.scale_sharing(ScaleSharing::SharedX)

// Shared Y, independent X
.scale_sharing(ScaleSharing::SharedY)

// Share within rows
.scale_sharing(ScaleSharing::SharedRows)

// Share within columns
.scale_sharing(ScaleSharing::SharedCols)
```

---

## Faceting vs Repeat Comparison

### When to Use Faceting

Use faceting when you want to **partition data** into groups:

```rust
// Show distribution for EACH species (filtered data)
FacetWrap::new()
    .facet_by(vec!["species"])
    .subplot(
        Plot::<Cartesian>::new()
            .mark(Histogram::new().x(col("petal_length")))
    )
```

**Data flow:** Group by species → each subplot sees only its species' data

### When to Use Repeat

Use repeat when you want to show **different variables**:

```rust
// Show distribution of EACH measurement (all data)
RepeatWrap::new()
    .variables(vec!["sepal_length", "sepal_width", "petal_length", "petal_width"])
    .subplot(|(var, idx)| {
        Plot::<Cartesian>::new()
            .mark(Histogram::new().x(col(var)))
    })
```

**Data flow:** For each variable → subplot shows full dataset mapped to that variable

### Comparison Table

| Feature | Faceting | Repeat |
|---------|----------|--------|
| **Driven by** | Data values in columns | Column/variable names |
| **Subplots created for** | Each unique group | Each variable |
| **Data per subplot** | Filtered (group's subset) | Full dataset (all rows) |
| **What varies** | Which data rows shown | Which column is encoded |
| **Groups defined at** | Runtime (data-dependent) | Design-time (schema) |
| **Example** | One plot per species | One plot per measurement |
| **Use case** | Comparing groups | Comparing variables |

### Can You Combine Them?

**Yes!** Faceting and repeat can be nested:

```rust
// Facet by species, repeat over measurements within each
Plot::<Cartesian>::new()
    .data(iris_df)
    .mark(
        FacetWrap::new()
            .facet_by(vec!["species"])  // One subplot per species
            .subplot(
                Plot::<Cartesian>::new()
                    .mark(
                        RepeatRow::new()  // Within each species, show all measurements
                            .variables(vec!["sepal_length", "sepal_width", "petal_length"])
                            .subplot(|(var, idx)| {
                                Plot::<Cartesian>::new()
                                    .mark(Histogram::new().x(col(var)))
                            })
                    )
            )
    )
```

**Result:** For each species, show a row of histograms (one per measurement variable).

Or reverse:

```rust
// Repeat over measurements, facet by species within each
Plot::<Cartesian>::new()
    .data(iris_df)
    .mark(
        RepeatRow::new()
            .variables(vec!["sepal_length", "sepal_width", "petal_length"])
            .subplot(|(var, idx)| {
                Plot::<Cartesian>::new()
                    .mark(
                        FacetColumn::new()
                            .facet_by(vec!["species"])
                            .subplot(
                                Plot::<Cartesian>::new()
                                    .mark(Histogram::new().x(col(var)))
                            )
                    )
            })
    )
```

**Result:** For each measurement, show a column of histograms (one per species).

---

## Scale Sharing

All automatic layout faceting and repeat types (`FacetRow`, `FacetColumn`, `FacetWrap`, `FacetGrid`, `RepeatRow`, `RepeatColumn`, `RepeatWrap`, `RepeatGrid`) support scale sharing modes. Some modes are specific to certain layout types.

### ScaleSharing Enum

```rust
pub enum ScaleSharing {
    Shared,       // All facets/repeats share same scale domains (default for faceting)
    Free,         // Each facet/repeat has independent scale domains
    SharedX,      // Share X scale, independent Y scales
    SharedY,      // Share Y scale, independent X scales
    SharedRows,   // Share domains within each row, independent across rows (Grid only)
    SharedCols,   // Share domains within each column, independent across columns (Grid only)
    PerVariable,  // Each variable gets one scale used wherever it appears (RepeatGrid only, default for SPLOM)
}
```

### Shared (Default)

All facets share the same scale domains. Best for comparing values across facets.

```rust
FacetWrap::new()
    .facet_by(vec!["species"])
    .columns(3)
    .scale_sharing(ScaleSharing::Shared)  // Default
    .subplot(...)
```

**Result:** All subplots have identical axis ranges, making comparisons easy.

### Free

Each facet computes its own scale domains independently. Best when facets have very different data ranges.

```rust
FacetWrap::new()
    .facet_by(vec!["metric"])
    .columns(2)
    .scale_sharing(ScaleSharing::Free)
    .subplot(...)
```

**Result:** Each subplot optimizes its axes for its own data, maximizing space usage.

### SharedX

Share X scale domain, independent Y scale domains. Common for time series comparisons where you want aligned time axes.

```rust
FacetColumn::new()
    .facet_by(vec!["stock"])
    .scale_sharing(ScaleSharing::SharedX)
    .subplot(
        Plot::<Cartesian>::new()
            .mark(
                Line::new()
                    .x(col("date"))  // Shared X scale (aligned dates)
                    .y(col("price")) // Independent Y scales (different price ranges)
            )
    )
```

### SharedY

Share Y scale domain, independent X scale domains. Less common but useful for certain comparisons.

```rust
FacetRow::new()
    .facet_by(vec!["product"])
    .scale_sharing(ScaleSharing::SharedY)
    .subplot(...)
```

### SharedRows

Share domains within each row, independent across rows. Only applicable to `FacetGrid`. Useful when row categories have different data ranges but you want comparison within each row.

```rust
FacetGrid::new()
    .rows("vehicle_type")  // car, truck, motorcycle
    .cols("year")          // 2020, 2021, 2022
    .scale_sharing(ScaleSharing::SharedRows)
    .subplot(
        Plot::<Cartesian>::new()
            .mark(
                Symbol::new()
                    .x(col("weight"))
                    .y(col("mpg"))
            )
    )
```

**Result:**
- Each row (vehicle type) has its own X and Y scale domains
- All facets within a row share those domains (years are comparable within vehicle type)
- Different rows have different scales (car vs truck vs motorcycle have very different ranges)

**When to use:**
- Row categories have different data ranges (e.g., different vehicle types)
- Want to compare across columns within each row (e.g., compare years for each vehicle type)
- Don't need to compare across rows (different vehicle types not directly comparable)

### SharedCols

Share domains within each column, independent across columns. Only applicable to `FacetGrid`. Useful when column categories have different data ranges but you want comparison within each column.

```rust
FacetGrid::new()
    .rows("year")         // 2020, 2021, 2022
    .cols("metric")       // temperature, pressure, humidity
    .scale_sharing(ScaleSharing::SharedCols)
    .subplot(
        Plot::<Cartesian>::new()
            .mark(
                Line::new()
                    .x(col("month"))
                    .y(col("value"))
            )
    )
```

**Result:**
- Each column (metric) has its own X and Y scale domains
- All facets within a column share those domains (years are comparable within metric)
- Different columns have different scales (temperature vs pressure vs humidity have different units/ranges)

**When to use:**
- Column categories have different data ranges (e.g., different metrics with different units)
- Want to compare across rows within each column (e.g., compare years for each metric)
- Don't need to compare across columns (different metrics not directly comparable)

### Comparison Table

| Mode | X Domain | Y Domain | Applicable To | Use Case |
|------|----------|----------|---------------|----------|
| `Shared` | Shared across all | Shared across all | All layouts | Comparing all facets directly |
| `Free` | Independent per facet | Independent per facet | All layouts | Each facet optimizes its own view |
| `SharedX` | Shared across all | Independent per facet | All layouts | Aligned X axes (e.g., time), varying Y (e.g., different stocks) |
| `SharedY` | Independent per facet | Shared across all | All layouts | Varying X, aligned Y axes (e.g., price comparisons) |
| `SharedRows` | Shared within row | Shared within row | `FacetGrid` only | Row categories have different ranges |
| `SharedCols` | Shared within column | Shared within column | `FacetGrid` only | Column categories have different ranges |

**Note:** `SharedRows` and `SharedCols` only make sense for `FacetGrid` since other layouts don't have both rows and columns. If used with other layouts, they behave like `Free`.

### Example: Product Performance by Region and Quarter

```rust
FacetGrid::new()
    .rows("region")      // North, South, East, West
    .cols("quarter")     // Q1, Q2, Q3, Q4
    .scale_sharing(ScaleSharing::SharedRows)
    .subplot(
        Plot::<Cartesian>::new()
            .mark(
                Rect::new()
                    .x(col("product"))
                    .y(col("sales"))
                    .fill(col("product"))
            )
    )
```

**Result:**
- Each region (row) has its own Y scale (North might have 10x sales of South)
- Within each region, quarters are comparable (same Y scale)
- Quarters aligned across regions (X axis shared)
- Easy to see quarterly trends within each region
- Harder to compare absolute values across regions (by design)

### Example: Environmental Sensors Over Time

```rust
FacetGrid::new()
    .rows("location")    // Site A, Site B, Site C
    .cols("sensor_type") // Temperature, Humidity, Pressure
    .scale_sharing(ScaleSharing::SharedCols)
    .subplot(
        Plot::<Cartesian>::new()
            .mark(
                Line::new()
                    .x(col("timestamp"))
                    .y(col("reading"))
                    .stroke("#2196F3")
            )
    )
```

**Result:**
- Each sensor type (column) has its own Y scale (temperature, humidity, pressure have different units)
- Within each sensor type, locations are comparable (same Y scale)
- Time axis (X) shared across all for alignment
- Easy to compare locations for each sensor type
- Can't compare temperature to humidity (by design - different units)

---

## Axis Display

All automatic layout types (`FacetRow`, `FacetColumn`, `FacetWrap`, `FacetGrid`) support controlling which facets show axis labels and titles. This works with any coordinate system by passing grid position information to the guide, which decides how to configure its axes.

### AxisDisplay Enum

```rust
pub enum AxisDisplay {
    All,      // Every facet shows full axes (default)
    Edges,    // Guide decides based on grid position
    None,     // No axes shown
}
```

### FacetPosition Information

When rendering each facet, the guide receives position information:

```rust
pub struct FacetPosition {
    pub row: usize,           // 0-indexed row in grid
    pub col: usize,           // 0-indexed column in grid
    pub is_top_row: bool,     // True if in top row
    pub is_bottom_row: bool,  // True if in bottom row
    pub is_left_col: bool,    // True if in leftmost column
    pub is_right_col: bool,   // True if in rightmost column
    pub is_first: bool,       // True if first in sequence (Row/Column/Wrap)
    pub is_last: bool,        // True if last in sequence
}
```

The coordinate system's guide implementation uses this to decide which axes to show. This keeps faceting extensible - any coordinate system can define its own logic.

### All (Default)

Every facet shows complete axes with labels and titles.

```rust
FacetWrap::new()
    .facet_by(vec!["species"])
    .columns(3)
    .axis_display(AxisDisplay::All)  // Default
    .subplot(...)
```

**Result:** Every subplot has full axis labels. Uses most space but easiest to read independently.

### Edges

Each guide implementation decides which axes to show based on grid position. This is the traditional small multiples pattern.

```rust
FacetGrid::new()
    .rows("year")
    .cols("continent")
    .spacing(10.0)
    .axis_display(AxisDisplay::Edges)
    .subplot(
        Plot::<Cartesian>::new()
            .mark(Line::new().x(col("month")).y(col("temperature")))
    )
```

**Cartesian Guide Behavior:**
- Shows X axis labels only on top row (`is_top_row`) or bottom row (`is_bottom_row`)
- Shows Y axis labels only on left column (`is_left_col`) or right column (`is_right_col`)
- All facets show tick marks for reference
- Interior facets have no labels

**Polar Guide Behavior:**
- Could hide angular labels unless on edge
- Could hide radial axis unless `is_left_col`
- Implementation defined by `PolarGuide`

**Custom Coordinate Systems:**
External crates define their own logic by checking `FacetPosition` flags in their guide implementation.

### None

No axes shown in any facet. Guide receives this instruction and hides all axes.

```rust
FacetWrap::new()
    .facet_by(vec!["region"])
    .columns(4)
    .axis_display(AxisDisplay::None)
    .subplot(...)
```

**Result:** Clean subplots with no axis labels. Maximizes space for data.

### Combining with Scale Sharing

Axis display is independent of scale sharing:

```rust
FacetGrid::new()
    .rows("category")
    .cols("year")
    .scale_sharing(ScaleSharing::Shared)    // Shared domains
    .axis_display(AxisDisplay::Edges)       // But only show labels on edges
    .subplot(...)
```

**Result:** All facets use the same scale ranges (good for comparison), but only edge facets show labels (saves space).

```rust
FacetWrap::new()
    .facet_by(vec!["country"])
    .columns(3)
    .scale_sharing(ScaleSharing::Free)     // Each facet has own scales
    .axis_display(AxisDisplay::All)         // But all show their own labels
    .subplot(...)
```

**Result:** Each facet optimizes its scale to its data AND shows those scale values explicitly.

### Example: Compact Cartesian Grid

```rust
Plot::<Cartesian>::new()
    .data(df)
    .mark(
        FacetGrid::new()
            .rows("cylinder")
            .cols("origin")
            .spacing(8.0)
            .scale_sharing(ScaleSharing::Shared)
            .axis_display(AxisDisplay::Edges)
            .subplot(
                Plot::<Cartesian>::new()
                    .mark(
                        Symbol::new()
                            .x(col("weight"))
                            .y(col("mpg"))
                            .fill("#2196F3")
                    )
            )
    )
```

**Result:** Compact 3×3 grid where CartesianGuide:
- Shows X axis labels only on top/bottom rows
- Shows Y axis labels only on left/right columns
- All facets show tick marks for reference
- Shared scales make cross-facet comparison easy

### Example: Polar Subplots with Edge Axes

```rust
FacetWrap::new()
    .facet_by(vec!["region"])
    .columns(3)
    .axis_display(AxisDisplay::Edges)
    .subplot(
        Plot::<Polar>::new()
            .mark(Arc::new().r(col("value")).theta(col("angle")))
    )
```

**Result:** PolarGuide decides how to handle edges (e.g., only show angular labels on outer pies).

### How It Works Internally

The faceting system automatically handles axis display:

1. **User specifies display mode** on the faceting mark:
   ```rust
   FacetGrid::new()
       .axis_display(AxisDisplay::Edges)  // User config
       .subplot(...)
   ```

2. **For each facet during rendering**, the system:
   - Computes grid position (`FacetPosition`)
   - Clones the inner plot's guide
   - Calls `guide.configure_for_facet(position, display_mode)` internally
   - The guide adjusts its axes based on position
   - Renders the configured guide

3. **User never calls `configure_for_facet`** - it's an internal hook

### Guide Implementation Hook (For Coordinate System Authors)

For coordinate system implementors, the guide trait includes:

```rust
trait CoordinateGuide {
    /// Configure guide for faceted rendering (called internally by faceting system)
    ///
    /// # Arguments
    /// * `position` - Grid position information
    /// * `display_mode` - User's axis display preference
    ///
    /// # Implementation
    /// Check `display_mode` and `position` flags to decide which axes to show.
    /// This is called automatically during faceting - users never call this directly.
    fn configure_for_facet(
        &mut self,
        position: FacetPosition,
        display_mode: AxisDisplay
    );
}
```

**Example CartesianGuide Implementation:**
```rust
impl CoordinateGuide for CartesianGuide {
    fn configure_for_facet(
        &mut self,
        position: FacetPosition,
        display_mode: AxisDisplay
    ) {
        match display_mode {
            AxisDisplay::All => {
                // Keep all axes visible
            }
            AxisDisplay::Edges => {
                // Check each axis and only show if facet is at matching edge

                // X axis - check if it should be shown based on its position
                if let Some(x_axis) = &mut self.x_axis {
                    let show_x = match x_axis.position {
                        AxisPosition::Top => position.is_top_row,
                        AxisPosition::Bottom => position.is_bottom_row,
                        _ => false, // X axis not on horizontal edge
                    };
                    if !show_x {
                        x_axis.show_labels = false;
                        x_axis.show_title = false;
                    }
                }

                // Y axis - check if it should be shown based on its position
                if let Some(y_axis) = &mut self.y_axis {
                    let show_y = match y_axis.position {
                        AxisPosition::Left => position.is_left_col,
                        AxisPosition::Right => position.is_right_col,
                        _ => false, // Y axis not on vertical edge
                    };
                    if !show_y {
                        y_axis.show_labels = false;
                        y_axis.show_title = false;
                    }
                }
            }
            AxisDisplay::None => {
                // Hide all axes
                if let Some(x_axis) = &mut self.x_axis {
                    x_axis.show_labels = false;
                    x_axis.show_title = false;
                }
                if let Some(y_axis) = &mut self.y_axis {
                    y_axis.show_labels = false;
                    y_axis.show_title = false;
                }
            }
        }
    }
}
```

**Key Design:**
- Each axis knows its position (Top/Bottom/Left/Right)
- Grid position flags match axis positions perfectly:
  - `AxisPosition::Top` → show only if `is_top_row`
  - `AxisPosition::Bottom` → show only if `is_bottom_row`
  - `AxisPosition::Left` → show only if `is_left_col`
  - `AxisPosition::Right` → show only if `is_right_col`
- Works regardless of where user positioned their axes

This design keeps faceting extensible - any coordinate system can implement its own axis hiding logic based on axis positions and grid location.

---

## Facet Labels

Facet marks can display labels that identify what each subplot represents. These labels show the faceting values (e.g., "setosa", "Q1", "Europe") and are positioned relative to the facet grid structure. They are separate from the inner plot's axes and identify the faceting grouping itself.

### Label Configuration

All automatic layout faceting marks support label configuration:

```rust
pub enum LabelPosition {
    Top,      // Above subplot
    Bottom,   // Below subplot
    Left,     // Left of subplot
    Right,    // Right of subplot
}
```

### FacetWrap - Tile Titles

Display a title above (or beside) each tile showing the facet value:

```rust
FacetWrap::new()
    .facet_by(vec!["species"])
    .columns(3)
    .with_labels(true)                    // Show labels (default: true)
    .label_position(LabelPosition::Top)   // Position (default: Top)
    .label_format(|value| {               // Optional formatter
        format!("Species: {}", value)
    })
    .label_font_size(14.0)                // Optional styling
    .label_color("#333333")
    .subplot(...)
```

**Result:** Each tile shows "Species: setosa", "Species: versicolor", "Species: virginica" at the top.

#### Basic Labels

```rust
FacetWrap::new()
    .facet_by(vec!["region"])
    .columns(4)
    .with_labels(true)  // Just shows "North", "South", "East", "West"
    .subplot(...)
```

#### No Labels

```rust
FacetWrap::new()
    .facet_by(vec!["country"])
    .columns(3)
    .with_labels(false)  // Clean tiles with no identification
    .subplot(...)
```

### FacetRow - Strip Labels

Show labels above or below each panel in the row:

```rust
FacetRow::new()
    .facet_by(vec!["quarter"])
    .with_labels(true)                    // Show labels (default: true)
    .label_position(LabelPosition::Top)   // Above each panel
    .label_format(|value| {
        format!("Q{}", value)  // "Q1", "Q2", "Q3", "Q4"
    })
    .subplot(...)
```

**Result:** Labels appear as a strip above (or below) each panel.

#### Bottom Labels

```rust
FacetRow::new()
    .facet_by(vec!["category"])
    .with_labels(true)
    .label_position(LabelPosition::Bottom)  // Below each panel
    .subplot(...)
```

### FacetColumn - Strip Labels

Show labels left or right of each panel in the column:

```rust
FacetColumn::new()
    .facet_by(vec!["metric"])
    .with_labels(true)                     // Show labels (default: true)
    .label_position(LabelPosition::Right)  // Right side (default for columns)
    .label_format(|value| {
        value.to_uppercase()
    })
    .subplot(...)
```

**Result:** Labels appear as a strip on the right (or left) side of each panel.

#### Left Labels

```rust
FacetColumn::new()
    .facet_by(vec!["stock"])
    .with_labels(true)
    .label_position(LabelPosition::Left)  // Left side
    .subplot(...)
```

### FacetGrid - Row and Column Labels

Grid facets have separate labels for rows and columns:

```rust
FacetGrid::new()
    .rows("cylinder")
    .cols("origin")

    // Row labels (on right side by default)
    .with_row_labels(true)                      // Show row labels (default: true)
    .row_label_position(LabelPosition::Right)   // Right side (default)
    .row_label_format(|value| {
        format!("{} cyl", value)
    })

    // Column labels (on top by default)
    .with_col_labels(true)                      // Show column labels (default: true)
    .col_label_position(LabelPosition::Top)     // Top (default)
    .col_label_format(|value| {
        value.to_uppercase()
    })

    .subplot(...)
```

**Result:** Shows "4 cyl", "6 cyl", "8 cyl" on the right side and "USA", "EUROPE", "JAPAN" on top.

#### Rows Only Grid

```rust
FacetGrid::new()
    .rows("region")
    .with_row_labels(true)
    .row_label_position(LabelPosition::Right)
    .subplot(...)
```

**Result:** Row labels on the right, no column labels (no columns specified).

#### Columns Only Grid

```rust
FacetGrid::new()
    .cols("year")
    .with_col_labels(true)
    .col_label_position(LabelPosition::Top)
    .subplot(...)
```

**Result:** Column labels on top, no row labels (no rows specified).

#### All Combinations

```rust
FacetGrid::new()
    .rows("category")
    .cols("year")

    // Row labels on left
    .with_row_labels(true)
    .row_label_position(LabelPosition::Left)

    // Column labels on bottom
    .with_col_labels(true)
    .col_label_position(LabelPosition::Bottom)

    .subplot(...)
```

**Result:** Grid with row labels on the left and column labels at the bottom.

#### Hide Grid Labels

```rust
FacetGrid::new()
    .rows("x")
    .cols("y")
    .with_row_labels(false)    // No row identification
    .with_col_labels(false)    // No column identification
    .subplot(...)
```

**Result:** Clean grid with no facet identification.

### Facet - Manual Control

For manual positioning, labels can optionally be shown at each subplot location:

```rust
Facet::new()
    .x(first(col("center_x")))
    .y(first(col("center_y")))
    .width(100.0)
    .height(100.0)
    .facet_by(vec!["region"])

    // Optional labels at each location
    .with_labels(true)
    .label_position(LabelPosition::Top)    // Relative to each subplot
    .label_format(|value| {
        format!("Region: {}", value)
    })

    .subplot(...)
```

**Result:** Each subplot shows its region name above it.

**Note:** Labels with `Facet` may not always be desired since positions are custom. Setting `.with_labels(false)` produces clean subplots.

### Label Formatting

All faceting marks support label formatting functions:

#### Simple String Formatting

```rust
.label_format(|value| {
    format!("Category: {}", value)
})
```

#### Multi-Column Formatting (Grid)

```rust
FacetGrid::new()
    .rows("year")
    .cols("region")

    .row_label_format(|value| {
        format!("Year {}", value)
    })

    .col_label_format(|value| {
        value.to_uppercase()
    })

    .subplot(...)
```

#### Conditional Formatting

```rust
.label_format(|value| {
    if value.parse::<i32>().unwrap_or(0) > 2020 {
        format!("Future: {}", value)
    } else {
        format!("Past: {}", value)
    }
})
```

### Label Styling

Control label appearance with styling options:

```rust
FacetWrap::new()
    .facet_by(vec!["species"])
    .columns(3)

    // Typography
    .label_font_size(16.0)           // Default: 12.0
    .label_font_weight("bold")       // Default: "normal"
    .label_font_family("Arial")      // Default: theme font

    // Color
    .label_color("#2196F3")          // Default: theme text color
    .label_background("#f5f5f5")     // Optional background

    // Padding
    .label_padding(8.0)              // Padding around label (default: 5.0)

    .subplot(...)
```

### Grid-Specific Styling

`FacetGrid` can style row and column labels independently:

```rust
FacetGrid::new()
    .rows("category")
    .cols("year")

    // Row label styling
    .row_label_font_size(14.0)
    .row_label_color("#FF5722")
    .row_label_background("#fff3e0")

    // Column label styling
    .col_label_font_size(16.0)
    .col_label_color("#2196F3")
    .col_label_background("#e3f2fd")

    .subplot(...)
```

### Example: Styled Small Multiples

```rust
Plot::<Cartesian>::new()
    .data(df)
    .mark(
        FacetWrap::new()
            .facet_by(vec!["species"])
            .columns(3)

            // Custom labels with formatting
            .with_labels(true)
            .label_position(LabelPosition::Top)
            .label_format(|species| {
                format!("Iris {}", species)
            })

            // Styling
            .label_font_size(14.0)
            .label_font_weight("bold")
            .label_color("#1976D2")
            .label_background("#E3F2FD")
            .label_padding(10.0)

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

**Result:** Each tile shows "Iris setosa", "Iris versicolor", "Iris virginica" with blue text on light blue background.

### Example: Dashboard with Grid Labels

```rust
Plot::<Cartesian>::new()
    .data(df)
    .mark(
        FacetGrid::new()
            .rows("vehicle_type")
            .cols("year")
            .spacing(15.0)

            // Row labels on right with custom styling
            .with_row_labels(true)
            .row_label_position(LabelPosition::Right)
            .row_label_format(|v| v.to_uppercase())
            .row_label_font_size(16.0)
            .row_label_font_weight("bold")
            .row_label_color("#424242")
            .row_label_background("#f5f5f5")

            // Column labels on top
            .with_col_labels(true)
            .col_label_position(LabelPosition::Top)
            .col_label_format(|year| format!("Year {}", year))
            .col_label_font_size(14.0)
            .col_label_color("#1976D2")

            .scale_sharing(ScaleSharing::SharedRows)
            .axis_display(AxisDisplay::Edges)

            .subplot(
                Plot::<Cartesian>::new()
                    .mark(
                        Symbol::new()
                            .x(col("weight"))
                            .y(col("mpg"))
                            .fill(col("vehicle_type"))
                    )
            )
    )
```

**Result:** Professional dashboard with clear row and column identification.

### Default Behavior

If not specified:
- **FacetWrap**: Labels shown on top by default
- **FacetRow**: Labels shown on top by default
- **FacetColumn**: Labels shown on right by default
- **FacetGrid**: Row labels on right, column labels on top by default
- **Facet**: Labels can be enabled but off by default (custom positioning may not need identification)

### Labels vs Axes

Important distinction:

| Feature | Facet Labels | Coordinate System Axes |
|---------|--------------|----------------------|
| Purpose | Identify which facet/group | Show data scale values |
| Controlled by | Faceting mark | Inner plot's coordinate guide |
| Content | Faceting column values | Scale tick values |
| Example | "Species: setosa" | "0, 2, 4, 6, 8, 10" |
| Position | Relative to grid structure | Relative to plot area |
| Configuration | `.with_labels()`, `.label_position()` | `.configure_guide()`, `.axis_with()` |

Both can appear in the same visualization:
- **Facet labels** identify which group you're looking at
- **Axes** show the scale of the data within that group

---

## Facet Data Strategy

Marks within the inner plot can control whether they receive filtered facet data or all data. This is useful for reference lines, global statistics, or conditional rendering.

### FacetStrategy Enum

```rust
pub enum FacetStrategy {
    Filter,     // Mark sees only its facet's data (default)
    Broadcast,  // Mark sees all data across all facets
    Skip,       // Skip mark if facet columns not present in data
}
```

### Filter (Default)

The mark receives only the data for its facet. This is the normal behavior.

```rust
FacetWrap::new()
    .facet_by(vec!["species"])
    .columns(3)
    .subplot(
        Plot::<Cartesian>::new()
            .mark(
                Symbol::new()
                    .x(col("sepal_length"))
                    .y(col("sepal_width"))
                    // Default: sees only this facet's species data
            )
    )
```

**Result:** Each subplot's symbol mark sees only its species' data points.

### Broadcast

The mark receives ALL data across all facets. Useful for reference lines or global statistics.

```rust
FacetWrap::new()
    .facet_by(vec!["species"])
    .columns(3)
    .subplot(
        Plot::<Cartesian>::new()
            // Faceted data - shows points for this facet only
            .mark(
                Symbol::new()
                    .x(col("sepal_length"))
                    .y(col("sepal_width"))
                    .facet_strategy(FacetStrategy::Filter)  // Default
            )

            // Global reference line - same in all facets
            .mark(
                Rule::new()
                    .y(mean(col("sepal_width")))  // Global mean across all species
                    .stroke("#ff0000")
                    .stroke_width(2.0)
                    .facet_strategy(FacetStrategy::Broadcast)  // Sees all data
            )
    )
```

**Result:**
- Symbol marks show filtered data per facet
- Rule mark computes global mean and shows the same line in every facet

### Skip

The mark is skipped if facet columns are not present in its data. Useful for conditional marks.

```rust
FacetGrid::new()
    .rows("year")
    .cols("region")
    .subplot(
        Plot::<Cartesian>::new()
            .mark(
                Line::new()
                    .x(col("month"))
                    .y(col("temperature"))
                    .facet_strategy(FacetStrategy::Filter)
            )

            // Optional annotation mark - only shows if has year/region columns
            .mark(
                Text::new()
                    .x(col("event_month"))
                    .y(col("event_temp"))
                    .text(col("event_name"))
                    .facet_strategy(FacetStrategy::Skip)  // Skip if no year/region in data
            )
    )
```

**Result:** Text mark only renders in facets where the annotation data has matching year/region values.

### Example: Global Reference with Faceted Data

Common pattern - show faceted data with global statistics:

```rust
// Compute global statistics
let global_stats = df
    .aggregate(vec![], vec![
        mean(col("value")).alias("global_mean"),
        min(col("value")).alias("global_min"),
        max(col("value")).alias("global_max"),
    ])
    .await?;

Plot::<Cartesian>::new()
    .data(df)
    .mark(
        FacetWrap::new()
            .facet_by(vec!["category"])
            .columns(3)
            .subplot(
                Plot::<Cartesian>::new()
                    // Faceted histogram
                    .mark(
                        Rect::new()
                            .x(col("bin"))
                            .y(col("count"))
                            .fill("#4CAF50")
                            .facet_strategy(FacetStrategy::Filter)  // Per-facet data
                    )

                    // Global mean line
                    .data(global_stats.clone())
                    .mark(
                        Rule::new()
                            .x(col("global_mean"))
                            .stroke("#FF5722")
                            .stroke_width(2.0)
                            .stroke_dash(vec![5.0, 5.0])
                            .facet_strategy(FacetStrategy::Broadcast)  // Same in all facets
                    )
            )
    )
```

**Result:** Each category facet shows:
- Its own histogram (filtered data)
- Global mean line (same across all facets)

### Example: Scatterpie with Global Reference

```rust
Facet::new()
    .x(first(col("center_x")))
    .y(first(col("center_y")))
    .width(100.0)
    .height(100.0)
    .facet_by(vec!["pie_id"])
    .subplot(
        Plot::<Polar>::new()
            // Pie slices - filtered to this pie's data
            .mark(
                Arc::new()
                    .r(col("value"))
                    .theta(col("angle"))
                    .fill(col("category"))
                    .facet_strategy(FacetStrategy::Filter)
            )

            // Reference circle at mean value - same in all pies
            .mark(
                Arc::new()
                    .r(mean(col("value")))  // Global mean
                    .theta_range(0.0, 2.0 * PI)
                    .fill_opacity(0.0)
                    .stroke("#999")
                    .stroke_width(1.0)
                    .facet_strategy(FacetStrategy::Broadcast)
            )
    )
```

**Result:** Each pie shows its own data slices plus a reference circle at the global mean value.

### How It Works

1. **Outer plot data** is grouped by `facet_by` columns
2. **For each facet group**:
   - Marks with `FacetStrategy::Filter` receive only that group's data
   - Marks with `FacetStrategy::Broadcast` receive the full ungrouped data
   - Marks with `FacetStrategy::Skip` are checked for facet column presence
3. **Inner plot's own data** (via `.data()`) works with facet strategy:
   - If mark has explicit data, facet strategy applies to that data
   - Allows mixing multiple data sources with different strategies

### Configuration on Marks

All mark types support facet strategy configuration:

```rust
Symbol::new()
    .x(col("x"))
    .y(col("y"))
    .facet_strategy(FacetStrategy::Broadcast)
```

```rust
Line::new()
    .x(col("x"))
    .y(col("y"))
    .facet_strategy(FacetStrategy::Filter)  // Default, can be explicit
```

---

## Channel Configuration Pattern

All faceting marks support the `_with` pattern for channel configuration:

### Scale Configuration

```rust
.x_with(expr, |c| {
    c.scale_with::<ScaleType>(|s| {
        s.domain(...)
         .range(...)
         .option(...)
    })
})
```

### Axis Configuration

```rust
.x_with(expr, |c| {
    c.axis_with(|a| {
        a.title("X Position")
         .label_angle(45.0)
         .tick_count(10)
    })
})
```

### Legend Configuration

```rust
.width_with(expr, |c| {
    c.legend_with(|l| {
        l.title("Size")
         .position(LegendPosition::Right)
         .orientation(LegendOrientation::Vertical)
    })
})
```

### Combined Configuration

```rust
.y_with(expr, |c| {
    c.scale_with::<Linear>(|s| {
        s.domain((lit(0.0), lit(100.0)))
         .nice(true)
    })
    .axis_with(|a| {
        a.title("Value")
    })
})
```

---

## Subplot Configuration

The subplot is a full `Plot` object with all standard features:

### Multiple Marks

```rust
.subplot(
    Plot::<Cartesian>::new()
        .mark(Rect::new()...)       // Background layer
        .mark(Line::new()...)        // Line layer
        .mark(Symbol::new()...)      // Point layer
)
```

### Plot-Level Data

```rust
.subplot(
    Plot::<Cartesian>::new()
        // Inner plot can have its own data
        // (in addition to filtered outer data)
        .data(reference_df)
        .mark(Line::new()...)  // Uses reference_df
)
```

### Scales and Legends

```rust
.subplot(
    Plot::<Cartesian>::new()
        .mark(Symbol::new().fill(col("category")))
        .scale("fill", Scale::ordinal().range_colors(colors))
        .legend("fill", |l| l.title("Category"))
)
```

### Guides and Axes

```rust
.subplot(
    Plot::<Polar>::new()
        .mark(Arc::new()...)
        .configure_guide(
            PolarGuide::default()
                .show_radial_axis(false)
                .show_angular_axis(true)
                .radial_grid_stroke("#cccccc")
        )
)
```

### Titles and Subtitles

```rust
.subplot(
    Plot::<Cartesian>::new()
        .title("Subplot Title")
        .subtitle("Additional context")
        .mark(...)
)
```

### Margins

```rust
.subplot(
    Plot::<Cartesian>::new()
        .margins(Margins::uniform(5.0))
        .mark(...)
)
```

---

## Common Patterns

### Mixing Facets with Regular Marks

```rust
Plot::<Cartesian>::new()
    .data(df)

    // Background layer - regular mark
    .mark(
        Rect::new()
            .x(col("region_x"))
            .y(col("region_y"))
            .fill("#f0f0f0")
            .opacity(0.3)
    )

    // Faceted subplots
    .mark(
        Facet::new()
            .x(first(col("center_x")))
            .y(first(col("center_y")))
            .width(100.0)
            .height(100.0)
            .facet_by(vec!["pie_id"])
            .subplot(Plot::<Polar>::new()...)
    )

    // Annotation layer - regular mark
    .mark(
        Symbol::new()
            .x(col("annotation_x"))
            .y(col("annotation_y"))
            .shape("cross")
    )
```

### Nested Different Coordinate Systems

```rust
// Cartesian outer, Polar inner
Plot::<Cartesian>::new()
    .mark(
        FacetWrap::new()
            .subplot(Plot::<Polar>::new()...)
    )

// Could also do Polar outer, Cartesian inner
Plot::<Polar>::new()
    .mark(
        Facet::new()
            .r(...)
            .theta(...)
            .width(...)
            .height(...)
            .subplot(Plot::<Cartesian>::new()...)
    )
```

### Shared Data with Reference Lines

```rust
// Outer plot data
let data = ctx.read_csv("data.csv").await?;
let mean_value = data.select(vec![mean(col("value"))]).await?;

Plot::<Cartesian>::new()
    .data(data.clone())
    .mark(
        FacetWrap::new()
            .facet_by(vec!["category"])
            .columns(3)
            .subplot(
                Plot::<Cartesian>::new()
                    // Uses filtered data from outer plot
                    .mark(Symbol::new().x(col("x")).y(col("value")))

                    // Add reference line using separate data
                    .data(mean_value.clone())
                    .mark(Rule::new().y(col("mean")).stroke("#ff0000"))
            )
    )
```

### Conditional Formatting in Subplots

```rust
Facet::new()
    .x(first(col("x_pos")))
    .y(first(col("y_pos")))
    .width(120.0)
    .height(120.0)
    .facet_by(vec!["region"])
    .subplot(
        Plot::<Cartesian>::new()
            .mark(
                Rect::new()
                    .x(col("category"))
                    .y(col("value"))
                    .fill_with(col("value"), |c| {
                        c.conditional(
                            col("value").gt(lit(100.0)),
                            lit("#4CAF50"),  // Green if > 100
                            lit("#F44336")   // Red otherwise
                        )
                    })
            )
    )
```

---

## Decision Guide

### Choose `Facet` when:
- Positions are data-driven (e.g., scatterpie at geographic coordinates)
- Sizes vary per facet based on data
- Custom spatial arrangements needed
- Irregular layouts

### Choose `FacetRow` when:
- Single horizontal row of facets
- Comparing across one dimension horizontally
- Dashboard-style horizontal panels

### Choose `FacetColumn` when:
- Single vertical column of facets
- Stacking time series or metrics vertically
- Dashboard-style vertical panels

### Choose `FacetWrap` when:
- Single faceting variable
- Want automatic grid layout with wrapping
- Traditional small multiples
- Don't care about exact positioning

### Choose `FacetGrid` when:
- Two faceting variables (rows and columns)
- Want matrix layout
- Comparing across two categorical dimensions
- Traditional facet_grid pattern

---

## Complete Example: Scatterpie Dashboard

Combining multiple faceting approaches:

```rust
use avenger_chart::prelude::*;

Plot::<Cartesian>::new()
    .data(df)
    .title("Regional Sales Dashboard")
    .canvas_size(1200.0, 800.0)

    // Main scatterpie layer with data-driven positioning and sizing
    .mark(
        Facet::new()
            .x(first(col("region_x")))
            .y(first(col("region_y")))

            // Size based on total sales
            .width_with(sum(col("sales")), |c| {
                c.scale_with::<Sqrt>(|s| {
                    s.domain((lit(0.0), lit(1000000.0)))
                     .range_interval(lit(80.0), lit(250.0))
                })
                .legend_with(|l| l.title("Total Sales"))
            })
            .height_with(sum(col("sales")), |c| {
                c.scale_with::<Sqrt>(|s| {
                    s.domain((lit(0.0), lit(1000000.0)))
                     .range_interval(lit(80.0), lit(250.0))
                })
            })

            .facet_by(vec!["region"])

            // Polar pie charts showing category breakdown
            .subplot(
                Plot::<Polar>::new()
                    .mark(
                        Arc::new()
                            .r(col("sales"))
                            .theta(col("angle"))
                            .fill(col("product_category"))
                            .stroke("#ffffff")
                            .stroke_width(2.0)
                    )
                    .configure_guide(
                        PolarGuide::default()
                            .show_radial_axis(false)
                            .show_angular_axis(false)
                    )
                    .legend("fill", |l| {
                        l.title("Product Category")
                         .position(LegendPosition::Right)
                    })
            )
    )
```

---
