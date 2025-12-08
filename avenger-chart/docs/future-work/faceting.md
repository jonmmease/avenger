# Faceting System

> **Status**: Partially Implemented
> - ✅ **FacetRow** - [Documentation](../book/src/docs/coordinate-systems/faceting/)
> - ✅ **FacetColumn** - [Documentation](../book/src/docs/coordinate-systems/faceting/)
> - ⏳ FacetWrap - Planned
> - ⏳ FacetGrid - Planned
> - ⏳ Manual Facet - Planned
>
> This document describes the original design specification. For implemented
> features, see the [Faceting Documentation](../book/src/docs/coordinate-systems/faceting/index.md).
>
> **Note**: Code examples assume `use datafusion::prelude::*;` and relevant aggregate function imports like `use datafusion::functions_aggregate::first::first;`.

## Overview

Faceting marks group data by column values and render filtered subplots for each group (data-driven replication). Each facet shows a subset of the data filtered by the faceting variable(s).

This is distinct from the [Repeat System](repeat.md) which iterates over variable names rather than data values.

## Prerequisites

### Implemented Infrastructure

These components have been implemented to support FacetRow and FacetColumn:

✓ **Layout System Enhancement** (`avenger-chart/src/layout/`)
  - Multi-subplot support with independent plot regions
  - Handles N subplots with shared or independent axes
  - Two-pass measurement and coordination system

✓ **Scale Domain Resolution**
  - Cross-facet domain calculation for shared scales
  - ScaleSharing::Free, Shared, and Level(n) modes implemented
  - Pre-compilation domain analysis

✓ **FacetStrategy enum** (`avenger-chart/src/marks/facet_strategy.rs`)
  - Filter, Broadcast, Skip variants implemented
  - Used to control mark data visibility across facets

✓ **Mark trait architecture** - Supports type-erased compilation and rendering

✓ **Coordinate system abstractions** - Cartesian, Polar, ZeroD

✓ **Aggregation system** - Plot::compile() already handles aggregate expressions

### Required for Future Features

Before additional faceting features (FacetWrap, FacetGrid, Manual Facet) can be implemented:

1. **Arc Mark for Polar** (`avenger-chart/src/polar/marks/arc.rs`)
   - Required for pie chart examples in this document
   - Should support `r`, `theta`, `start_angle`, `end_angle` channels
   - Currently only Symbol mark exists for Polar coordinates

---

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
- Group data by one or more faceting columns using DataFusion's `.group_by()`
- Render a complete inner `Plot<InnerC>` for each facet group
- Support nested coordinate systems (e.g., Cartesian outer positioning, Polar inner subplots)
- Pass data to each inner plot based on mark facet strategies
- Support multiple marks within inner plots with different data strategies

Inner plot marks support:
- **FacetStrategy** (already implemented) to control data filtering
  - `Filter`: Mark sees only its facet's data (default)
  - `Broadcast`: Mark sees all data (for reference lines)
  - `Skip`: Conditional rendering based on data presence

Automatic layout marks (`FacetRow`, `FacetColumn`, `FacetWrap`, `FacetGrid`) additionally support:
- Scale sharing modes (`ScaleSharing` enum)
- Axis display modes (`AxisDisplay` enum)

---

## Type System

### Facet Mark Generic Structure

Faceting marks are generic over TWO coordinate systems:

```rust
pub struct Facet<OuterC, InnerC>
where
    OuterC: CoordinateSystem,
    InnerC: CoordinateSystem
{
    // Position/size channels for facet positioning (OuterC space)
    state: MarkState,

    // Inner subplot specification
    subplot: Plot<InnerC>,

    // Faceting configuration
    facet_by: Vec<String>,

    _phantom: PhantomData<(OuterC, InnerC)>,
}
```

**Key points:**
- `OuterC`: Coordinate system for facet positioning (usually Cartesian)
- `InnerC`: Coordinate system for subplot content (can be Cartesian, Polar, etc.)
- Facet implements `Mark<OuterC>`, making it compatible with `Plot<OuterC>`
- The `.x()`, `.y()`, `.width()`, `.height()` channels use `OuterC`'s coordinate space
- The `.subplot()` accepts a `Plot<InnerC>`

**Example type expansion:**
```rust
// This:
Plot::<Cartesian>::new()
    .mark(
        Facet::new()
            .subplot(Plot::<Polar>::new()...)
    )

// Expands to:
Plot::<Cartesian>::new()
    .mark(
        Facet::<Cartesian, Polar>::new()
            .subplot(Plot::<Polar>::new()...)
    )
```

---

## Architectural Decision: Async Mark::compile()

**Decision**: Make `Mark::compile()` async to enable faceting marks to compile subplots naturally.

**Current signature**:
```rust
fn compile(&self, compiled_state: CompiledMarkState) -> Arc<dyn CompiledMark>;
```

**New signature**:
```rust
async fn compile(
    &self,
    compiled_state: CompiledMarkState,
    session_context: &SessionContext,
) -> Result<Arc<dyn CompiledMark>, AvengerChartError>;
```

**Benefits**:
1. Faceting marks can await subplot compilation: `subplot.compile(ctx).await?`
2. No special handling needed in `Plot::compile()` - facets are just marks
3. Simpler error propagation with `Result` return type
4. Natural fit for async DataFusion operations (grouping, filtering)

**Migration**: All existing marks need `async` keyword added (mechanical change via macro).

---

## Compilation Process

With async `Mark::compile()`, faceting marks compile like any other mark, but internally compile N subplots.

### Compilation Algorithm

```rust
// In FacetRow<InnerC>::compile()
async fn compile(
    &self,
    compiled_state: CompiledMarkState,
    session_context: &SessionContext,
) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
    // 1. Get data from compiled state
    let df = compiled_state.data.to_dataframe(session_context)?;

    // 2. Group data by facet columns
    let groups = df
        .group_by(vec![col(&self.facet_column)])?
        .aggregate(vec![], vec![])?
        .collect().await?;

    // 3. Compute shared scale domains (if needed)
    let shared_domains = if self.scale_sharing != ScaleSharing::Free {
        compute_shared_domains(&groups, &self.subplot, self.scale_sharing, session_context).await?
    } else {
        HashMap::new()
    };

    // 4. Compile each subplot
    let mut compiled_subplots = vec![];
    for batch in groups {
        for row_idx in 0..batch.num_rows() {
            // Extract facet value
            let facet_val = extract_facet_value(&batch, row_idx, &self.facet_column)?;

            // Filter data for this facet
            let group_df = df.clone().filter(
                col(&self.facet_column).eq(lit(facet_val.clone()))
            )?;

            // Clone subplot and configure
            let mut subplot = self.subplot.clone();
            subplot.data = Some(group_df);

            // Apply shared domains
            if !shared_domains.is_empty() {
                apply_shared_domains(&mut subplot, &shared_domains)?;
            }

            // Compile subplot (async!)
            let compiled = subplot.compile(session_context).await?;

            compiled_subplots.push((facet_val, compiled));
        }
    }

    // 5. Compute layout
    let layout = FacetLayout::horizontal(
        &compiled_subplots,
        self.spacing,
        self.width_override,
        self.height_override,
    )?;

    // 6. Return compiled facet mark
    Ok(Arc::new(CompiledFacetRow {
        subplots: compiled_subplots,
        layout,
        scale_sharing: self.scale_sharing,
        spacing: self.spacing,
        state: compiled_state,
    }))
}
```

### Scale Domain Resolution for Shared Scales

When `ScaleSharing::Shared` or similar is used, domains must be computed across all facets before subplot compilation:

```rust
fn compute_unified_domains(
    groups: Vec<(FacetValue, DataFrame)>,
    subplot_spec: &Plot<InnerC>
) -> HashMap<String, Domain> {
    let mut unified_domains = HashMap::new();

    // For each channel in the subplot
    for (channel_name, channel_config) in subplot_spec.channels() {
        // Collect data values across all groups
        let mut all_values = vec![];
        for (_facet_val, group_df) in &groups {
            let values = group_df.evaluate_channel(channel_name)?;
            all_values.extend(values);
        }

        // Compute unified domain
        let domain = infer_domain(all_values, channel_config.scale_type());
        unified_domains.insert(channel_name, domain);
    }

    unified_domains
}
```

---

## Facet - Manual Control

Fully data-driven positioning and sizing. You specify x, y, width, and height as aggregate or scalar expressions.

### API

```rust
impl<OuterC, InnerC> Facet<OuterC, InnerC>
where
    OuterC: CoordinateSystem,
    InnerC: CoordinateSystem
{
    /// Position channels (required) - use OuterC coordinate space
    fn x(self, expr: impl IntoExpr) -> Self;
    fn x_with<F>(self, expr: impl IntoExpr, config: F) -> Self
    where F: FnOnce(OuterC::PositionConfig) -> OuterC::PositionConfig;

    fn y(self, expr: impl IntoExpr) -> Self;
    fn y_with<F>(self, expr: impl IntoExpr, config: F) -> Self
    where F: FnOnce(OuterC::PositionConfig) -> OuterC::PositionConfig;

    /// Size channels (required)
    fn width(self, expr: impl IntoExpr) -> Self;
    fn width_with<F>(self, expr: impl IntoExpr, config: F) -> Self
    where F: FnOnce(SizeConfig) -> SizeConfig;

    fn height(self, expr: impl IntoExpr) -> Self;
    fn height_with<F>(self, expr: impl IntoExpr, config: F) -> Self
    where F: FnOnce(SizeConfig) -> SizeConfig;

    /// Faceting specification (required)
    fn facet_by(self, columns: Vec<impl Into<String>>) -> Self;

    /// Subplot specification (required)
    fn subplot(self, plot: Plot<InnerC>) -> Self;
}
```

### Position and Size Channel Constraints

Position and size channels must evaluate to **a single value per facet group**. This can be achieved by:

1. **Aggregate expressions**: Reduce each group to a single value
   - Examples: `first(col("x"))`, `mean(col("value"))`, `sum(col("amount"))`

2. **Scalar literals**: Same value for all facets
   - Examples: `100.0`, `lit(150.0)`

3. **Expressions using only facet_by columns**: Each group has unique values for grouping columns
   - Example: If faceting by `"grid_row"`, then `col("grid_row") * lit(150.0)` evaluates to one value per group

**Why this constraint?** After grouping by `facet_by` columns, each facet group needs exactly one (x, y, width, height) tuple to position that subplot in the outer coordinate space.

**Available channels depend on outer coordinate system:**
- **Cartesian outer:** `x`, `y`, `width`, `height`
- **Polar outer:** `r`, `theta`, `width`, `height`

### Examples

#### Basic Scatterpie (requires Arc mark)

> **Note**: This example requires `Arc` mark implementation for Polar coordinates (not yet available).

```rust
Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Facet::new()
            // Position at data points (first x/y in each pie group)
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
                        Arc::new()  // ← Requires implementation
                            .r(col("value"))
                            .theta(col("category"))
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

#### Data-Driven Sizes (Cartesian-in-Cartesian)

```rust
Plot::<Cartesian>::new()
    .data(df)
    .mark(
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
            .subplot(
                Plot::<Cartesian>::new()
                    .mark(
                        Symbol::new()
                            .x(col("metric_x"))
                            .y(col("metric_y"))
                            .fill(col("category"))
                    )
            )
    )
```

#### Grid Layout via Facet Columns

```rust
Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Facet::new()
            // Position computed from facet columns
            // If grid_col ∈ {0, 1, 2}, positions are {0, 150, 300}
            .x(col("grid_col") * lit(150.0))
            .y(col("grid_row") * lit(150.0))

            .width(130.0)
            .height(130.0)

            // Facet by grid coordinates
            .facet_by(vec!["grid_row", "grid_col"])

            .subplot(
                Plot::<Cartesian>::new()
                    .mark(
                        Rect::new()
                            .x(col("x"))
                            .y(col("y"))
                            .fill(col("value"))
                    )
            )
    )
```

#### Multiple Inner Marks

```rust
Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Facet::new()
            .x(first(col("center_x")))
            .y(first(col("center_y")))
            .width(120.0)
            .height(120.0)

            .facet_by(vec!["group_id"])

            .subplot(
                Plot::<Cartesian>::new()
                    // Background symbols
                    .mark(
                        Symbol::new()
                            .x(col("x"))
                            .y(col("y"))
                            .size(50.0)
                            .fill("#cccccc")
                            .facet_strategy(FacetStrategy::Broadcast) // See all data
                    )
                    // Foreground symbols - only group data
                    .mark(
                        Symbol::new()
                            .x(col("x"))
                            .y(col("y"))
                            .size(100.0)
                            .fill(col("category"))
                            .facet_strategy(FacetStrategy::Filter) // Default
                    )
            )
    )
```

---

## FacetRow - Horizontal Layout

Arranges facets in a single horizontal row with automatic positioning and uniform sizing.

### API

```rust
impl<InnerC> FacetRow<InnerC>
where InnerC: CoordinateSystem
{
    /// Faceting specification (required)
    fn facet_by(self, column: impl Into<String>) -> Self;

    /// Layout options
    fn spacing(self, px: f32) -> Self;  // Default: 10.0
    fn scale_sharing(self, mode: ScaleSharing) -> Self;  // Default: Shared
    fn axis_display(self, mode: AxisDisplay) -> Self;  // Default: Edges

    /// Size overrides (optional) - override ALL subplot dimensions
    fn width(self, value: f32) -> Self;  // Override computed width for each facet
    fn height(self, value: f32) -> Self;  // Override computed height for each facet

    /// Subplot specification (required)
    fn subplot(self, plot: Plot<InnerC>) -> Self;
}
```

### Enums

```rust
pub enum ScaleSharing {
    Shared,   // All facets share both X and Y scales
    Free,     // Each facet has independent X and Y scales
    SharedX,  // Shared X scale, independent Y scales
    SharedY,  // Shared Y scale, independent X scales
}

pub enum AxisDisplay {
    All,    // Show axes on all facets
    Edges,  // Show axes only on edge facets
    None,   // Hide all axes
}
```

**Note**: FacetRow only supports single-variable faceting (one column). For FacetRow, `SharedX`/`SharedY` control whether the shared dimension is X or Y.

### Examples

#### Basic Horizontal Comparison

```rust
Plot::<Cartesian>::new()
    .data(df)
    .mark(
        FacetRow::new()
            .facet_by("quarter")  // Q1, Q2, Q3, Q4
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
Plot::<Cartesian>::new()
    .data(df)
    .mark(
        FacetRow::new()
            .facet_by("region")
            .spacing(15.0)
            .scale_sharing(ScaleSharing::SharedY)  // Shared Y, each region has own X
            .subplot(
                Plot::<Cartesian>::new()
                    .mark(
                        Line::new()
                            .x(col("date"))
                            .y(col("sales"))
                    )
            )
    )
```

---

## FacetColumn - Vertical Layout

Arranges facets in a single vertical column with automatic positioning and uniform sizing.

### API

```rust
impl<InnerC> FacetColumn<InnerC>
where InnerC: CoordinateSystem
{
    /// Faceting specification (required)
    fn facet_by(self, column: impl Into<String>) -> Self;

    /// Layout options
    fn spacing(self, px: f32) -> Self;
    fn scale_sharing(self, mode: ScaleSharing) -> Self;
    fn axis_display(self, mode: AxisDisplay) -> Self;

    /// Size overrides (optional)
    fn width(self, value: f32) -> Self;
    fn height(self, value: f32) -> Self;

    /// Subplot specification (required)
    fn subplot(self, plot: Plot<InnerC>) -> Self;
}
```

### Examples

#### Vertical Stacking

```rust
Plot::<Cartesian>::new()
    .data(df)
    .mark(
        FacetColumn::new()
            .facet_by("metric")
            .spacing(15.0)
            .scale_sharing(ScaleSharing::SharedX)  // Shared X, each metric has own Y
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

---

## FacetWrap - Grid Wrapping

Arranges facets in a grid that wraps after a specified number of columns. Similar to ggplot2's `facet_wrap`.

### API

```rust
impl<InnerC> FacetWrap<InnerC>
where InnerC: CoordinateSystem
{
    /// Faceting specification (required)
    fn facet_by(self, columns: Vec<impl Into<String>>) -> Self;

    /// Layout options
    fn columns(self, n: usize) -> Self;  // Default: auto-compute via sqrt
    fn spacing(self, px: f32) -> Self;
    fn scale_sharing(self, mode: ScaleSharing) -> Self;
    fn axis_display(self, mode: AxisDisplay) -> Self;

    /// Size overrides (optional)
    fn width(self, value: f32) -> Self;
    fn height(self, value: f32) -> Self;

    /// Subplot specification (required)
    fn subplot(self, plot: Plot<InnerC>) -> Self;
}
```

### Auto-Computing Columns

When `.columns()` is not specified, use:

```rust
let n_facets = unique_facet_values.len();
let n_cols = (n_facets as f64).sqrt().ceil() as usize;
let n_rows = (n_facets as f64 / n_cols as f64).ceil() as usize;
```

This creates a roughly square grid. Example:
- 9 facets → 3×3 grid
- 10 facets → 4×3 grid (10 slots, 2 empty)
- 15 facets → 4×4 grid

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

#### Auto-Compute Columns

```rust
Plot::<Cartesian>::new()
    .data(df)
    .mark(
        FacetWrap::new()
            .facet_by(vec!["region"])
            // No .columns() specified - automatically computed
            .spacing(10.0)
            .subplot(
                Plot::<Cartesian>::new()
                    .mark(
                        Line::new()
                            .x(col("year"))
                            .y(col("value"))
                    )
            )
    )
```

---

## FacetGrid - Row×Column Matrix

Arranges facets in a matrix based on two faceting variables. Similar to ggplot2's `facet_grid`.

### API

```rust
impl<InnerC> FacetGrid<InnerC>
where InnerC: CoordinateSystem
{
    /// Faceting specification (at least one required)
    fn rows(self, column: impl Into<String>) -> Self;  // Variable for rows
    fn cols(self, column: impl Into<String>) -> Self;  // Variable for columns

    /// Layout options
    fn spacing(self, px: f32) -> Self;
    fn scale_sharing(self, mode: ScaleSharingGrid) -> Self;  // Extended enum
    fn axis_display(self, mode: AxisDisplay) -> Self;

    /// Size overrides (optional)
    fn width(self, value: f32) -> Self;   // Per-facet width
    fn height(self, value: f32) -> Self;  // Per-facet height

    /// Subplot specification (required)
    fn subplot(self, plot: Plot<InnerC>) -> Self;
}
```

### Extended Scale Sharing

```rust
pub enum ScaleSharingGrid {
    Shared,     // All facets share both X and Y
    Free,       // Each facet independent
    SharedX,    // Shared X across columns, Y varies
    SharedY,    // Shared Y across rows, X varies
    SharedRows, // Shared scales within each row
    SharedCols, // Shared scales within each column
}
```

**Semantics:**
- `SharedX`: All facets in the same **column** share the same X scale
- `SharedY`: All facets in the same **row** share the same Y scale
- `SharedRows`: All facets in the same **row** share both X and Y scales
- `SharedCols`: All facets in the same **column** share both X and Y scales

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
Plot::<Cartesian>::new()
    .data(df)
    .mark(
        FacetGrid::new()
            .rows("cylinder")
            .cols("origin")
            .spacing(10.0)
            .scale_sharing(ScaleSharingGrid::SharedX)  // X shared across columns
            .subplot(
                Plot::<Cartesian>::new()
                    .mark(
                        Symbol::new()
                            .x(col("weight"))
                            .y(col("mpg"))
                            .fill(col("origin"))
                    )
            )
    )
```

#### Rows Only

```rust
Plot::<Cartesian>::new()
    .data(df)
    .mark(
        FacetGrid::new()
            .rows("region")  // Only rows, no columns
            .spacing(15.0)
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

---

## Error Handling

### Empty Facets

When a facet group has no data:

```rust
pub enum EmptyFacetBehavior {
    Skip,   // Don't render that facet (default)
    Show,   // Render empty plot with axes
    Error,  // Fail compilation
}

FacetWrap::new()
    .facet_by(vec!["species"])
    .on_empty_facet(EmptyFacetBehavior::Skip)
    ...
```

### Missing Facet Columns

When facet_by column doesn't exist in data:

```rust
FacetWrap::new()
    .facet_by(vec!["nonexistent_column"])
    ...
// Returns: AvengerChartError::ColumnNotFound
```

### Null Values in Facet Columns

Null values in grouping columns are treated as a distinct group:

```rust
// Data: species = ["setosa", "versicolor", NULL]
// Produces 3 facets: setosa, versicolor, (null)
```

### Too Many Facets

When faceting produces excessive subplots (e.g., >100), consider:
- Sampling the data
- Using a different column with fewer unique values
- Warning in compilation logs

---

## Implementation Notes

### CompiledFacet Structure

```rust
#[derive(Serialize, Deserialize)]
pub struct CompiledFacet {
    /// Compiled subplots with their facet values
    subplots: Vec<(FacetValue, CompiledPlot)>,

    /// Layout information (positions and sizes)
    layout: FacetLayout,

    /// Scale sharing configuration
    scale_sharing: ScaleSharing,

    /// Mark state (for trait compliance)
    state: CompiledMarkState,
}
```

### Rendering Process

```rust
impl CompiledMark for CompiledFacet {
    fn evaluate_from_data(...) -> Result<Vec<SceneMark>, _> {
        let mut scene_marks = vec![];

        for (facet_val, subplot_plot) in &self.subplots {
            // Get subplot bounds from layout
            let bounds = self.layout.get_bounds(facet_val);

            // Render subplot to its bounds
            let subplot_scene = subplot_plot.render_to_bounds(
                bounds,
                context,
                coord
            )?;

            // Wrap in Group with clip region
            let group = SceneGroup {
                clip: Some(bounds),
                marks: subplot_scene,
                ...
            };

            scene_marks.push(SceneMark::Group(group));
        }

        Ok(scene_marks)
    }
}
```

### FacetValue Type

```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FacetValue {
    Single(ScalarValue),
    Multi(Vec<ScalarValue>),  // For multi-column faceting
}
```

### Integration with ChartLayout

The layout system needs enhancement to:
1. Detect CompiledFacet marks
2. Compute subplot regions based on faceting configuration
3. Handle shared vs independent axes
4. Manage axis label positioning for edge facets only

```rust
impl ChartLayout {
    fn layout_faceted_plot(...) -> LayoutSolution {
        // Compute grid of subplot regions
        // Position axes based on AxisDisplay mode
        // Handle shared scale axis positioning
        // Create clipping regions for subplots
        ...
    }
}
```

---

## Testing Strategy

### Unit Tests

1. **Grouping**: Verify data correctly grouped by facet columns
2. **Domain Resolution**: Test unified domain calculation for shared scales
3. **Layout Computation**: Validate subplot positioning algorithms
4. **Empty Facets**: Test EmptyFacetBehavior variants

### Integration Tests

1. **Cartesian-in-Cartesian**: FacetWrap with scatter plots
2. **Polar-in-Cartesian**: Scatterpie (requires Arc mark)
3. **Broadcast Strategy**: Reference lines across facets
4. **Scale Sharing**: Verify shared vs independent axes render correctly

### Visual Regression Tests

Add baseline images for:
- `test_facet_wrap_shared_scales`
- `test_facet_grid_free_scales`
- `test_facet_row_axis_display_edges`
- `test_scatterpie` (when Arc mark available)

---

## Future Enhancements

### Facet Labels

Add strip labels showing facet values:

```rust
FacetWrap::new()
    .facet_by(vec!["species"])
    .show_labels(true)
    .label_position(FacetLabelPosition::Top)
    .label_formatter(|col_name, col_val| {
        format!("{}: {}", col_name, col_val)
    })
    ...
```

### Facet Borders

```rust
FacetWrap::new()
    .facet_border(true)
    .facet_border_color("#cccccc")
    .facet_border_width(1.0)
    ...
```

### Independent Layout Sizing

Allow per-facet width/height based on data:

```rust
Facet::new()
    .width_with(count(col("id")), |c| {
        c.scale_with::<Linear>(|s| {
            s.range_interval(lit(100.0), lit(300.0))
        })
    })
    ...
```

### Nested Faceting

Facets within facets:

```rust
FacetWrap::new()
    .facet_by(vec!["year"])
    .subplot(
        Plot::new()
            .mark(
                FacetWrap::new()
                    .facet_by(vec!["month"])
                    .subplot(...)
            )
    )
```

---

## Relationship to Other Systems

- **[Repeat System](repeat.md)**: Schema-driven (iterate variable names) vs data-driven (iterate values)
- **[Layout System](layout.md)**: Faceting for data-driven subplots, Layout for manual composition
- **[Transform System](transform-system.md)**: Can pre-process data before faceting (e.g., bin then facet)
- **FacetStrategy** (implemented): Used by marks within faceted subplots

---

## Summary

Faceting provides powerful data-driven subplot replication with:
- ✓ Manual (`Facet`) and automatic (Row/Column/Wrap/Grid) layouts
- ✓ Nested coordinate systems (Cartesian, Polar)
- ✓ Flexible scale sharing modes
- ✓ Integration with existing FacetStrategy for mark-level control
- ✓ Type-safe API with dual coordinate system generics

**Dependencies**: Arc mark for polar examples, async Mark::compile(), enhanced layout system

**Status**: Comprehensive plan with phased implementation strategy (see below)

---

## Implementation Plan

This section outlines a phased approach to implementing faceting, starting with the most useful features (automatic layouts) and building toward the advanced manual layout API.

### Phase 0: Make Mark::compile() Async (Breaking Change)

**Duration**: 1 day

**Goal**: Enable faceting marks to compile subplots naturally

**Changes**:
```rust
// Before
trait Mark<C: CoordinateSystem> {
    fn compile(&self, compiled_state: CompiledMarkState) -> Arc<dyn CompiledMark>;
}

// After
trait Mark<C: CoordinateSystem> {
    async fn compile(
        &self,
        compiled_state: CompiledMarkState,
        session_context: &SessionContext,
    ) -> Result<Arc<dyn CompiledMark>, AvengerChartError>;
}
```

**Migration**:
- Update `Mark` trait definition in `avenger-chart/src/marks/mod.rs`
- Update `impl_mark_trait_common!` macro to generate async methods
- Add `async` keyword to all existing mark implementations (Line, Rect, Symbol for each coordinate system)
- Update `Plot::compile()` to await mark compilations
- Update all tests to use `await`

**Validation**: All existing tests pass after migration

---

### Phase 1: Core Infrastructure

**Duration**: 2-3 days

**Goal**: Build reusable types and utilities for all faceting marks

**Files to create**:

1. **`avenger-chart/src/marks/faceting/mod.rs`**
   ```rust
   pub mod enums;
   pub mod layout;
   pub mod value;
   pub mod domains;

   pub use enums::{ScaleSharing, ScaleSharingGrid, AxisDisplay, EmptyFacetBehavior};
   pub use value::FacetValue;
   pub use layout::FacetLayout;
   ```

2. **`avenger-chart/src/marks/faceting/enums.rs`**
   - `ScaleSharing` enum (Shared, Free, SharedX, SharedY)
   - `ScaleSharingGrid` enum (+SharedRows, SharedCols)
   - `AxisDisplay` enum (All, Edges, None)
   - `EmptyFacetBehavior` enum (Skip, Show, Error)

3. **`avenger-chart/src/marks/faceting/value.rs`**
   ```rust
   #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
   pub enum FacetValue {
       Single(ScalarValue),
       Multi(Vec<ScalarValue>),
   }
   ```

4. **`avenger-chart/src/marks/faceting/layout.rs`**
   ```rust
   pub struct FacetLayout {
       facet_bounds: HashMap<FacetValue, LayoutBounds>,
       total_width: f32,
       total_height: f32,
   }

   impl FacetLayout {
       pub fn horizontal(...) -> Result<Self, AvengerChartError>;
       pub fn vertical(...) -> Result<Self, AvengerChartError>;
       pub fn wrapped(...) -> Result<Self, AvengerChartError>;
       pub fn grid(...) -> Result<Self, AvengerChartError>;
   }
   ```

5. **`avenger-chart/src/marks/faceting/domains.rs`**
   ```rust
   pub async fn compute_shared_domains(
       groups: &[(FacetValue, DataFrame)],
       subplot: &Plot<InnerC>,
       scale_sharing: ScaleSharing,
       session_context: &SessionContext,
   ) -> Result<HashMap<String, Domain>, AvengerChartError>;
   ```

**Tests**:
- Enum serialization roundtrip
- FacetValue equality and hashing
- FacetLayout position calculations (unit tests with mock data)

---

### Phase 2: FacetRow Implementation

**Duration**: 3-4 days

**Goal**: First working faceting mark with horizontal layout

**Files to create**:

1. **`avenger-chart/src/marks/faceting/row.rs`**
   ```rust
   pub struct FacetRow<InnerC>
   where InnerC: CoordinateSystem
   {
       state: MarkState,
       facet_column: String,
       subplot: Plot<InnerC>,
       spacing: f32,
       scale_sharing: ScaleSharing,
       axis_display: AxisDisplay,
       width_override: Option<f32>,
       height_override: Option<f32>,
       _phantom: PhantomData<InnerC>,
   }

   impl<InnerC: CoordinateSystem> FacetRow<InnerC> {
       pub fn new() -> Self;
       pub fn facet_by(self, column: impl Into<String>) -> Self;
       pub fn subplot(self, plot: Plot<InnerC>) -> Self;
       pub fn spacing(self, px: f32) -> Self;
       pub fn scale_sharing(self, mode: ScaleSharing) -> Self;
       pub fn axis_display(self, mode: AxisDisplay) -> Self;
       pub fn width(self, value: f32) -> Self;
       pub fn height(self, value: f32) -> Self;
   }

   // FacetRow is always Mark<Cartesian> - positions subplots in Cartesian space
   impl<InnerC: CoordinateSystem> Mark<Cartesian> for FacetRow<InnerC> {
       async fn compile(...) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
           // Implementation from "Compilation Algorithm" section above
       }
   }
   ```

2. **`avenger-chart/src/marks/faceting/compiled_row.rs`**
   ```rust
   #[derive(Serialize, Deserialize)]
   pub struct CompiledFacetRow {
       subplots: Vec<(FacetValue, CompiledPlot)>,
       layout: FacetLayout,
       scale_sharing: ScaleSharing,
       spacing: f32,
       state: CompiledMarkState,
   }

   #[typetag::serde]
   impl CompiledMark for CompiledFacetRow {
       fn evaluate_from_data(...) -> Result<Vec<SceneMark>, AvengerChartError> {
           let mut scene_marks = vec![];

           for (facet_val, compiled_subplot) in &self.subplots {
               let bounds = self.layout.get_bounds(facet_val);

               // Render subplot within bounds
               let subplot_marks = compiled_subplot.render_to_bounds(bounds, context)?;

               // Wrap in Group with translation and clipping
               scene_marks.push(SceneMark::Group(SceneGroup {
                   x: bounds.x,
                   y: bounds.y,
                   clip: Some(ClipPath::Rect {
                       width: bounds.width,
                       height: bounds.height,
                   }),
                   marks: subplot_marks,
                   ..Default::default()
               }));
           }

           Ok(scene_marks)
       }
   }
   ```

**Tests**:
- **Unit**: Grouping by single column produces correct facet values
- **Unit**: Shared domain computation unifies X and Y correctly
- **Integration**: Compile with 3 facet groups, verify 3 subplots created
- **Integration**: ScaleSharing::SharedX produces unified X domain only
- **Visual**: `test_facet_row_iris_species.png` - 3 horizontal facets

**Blockers**: Need `CompiledPlot::render_to_bounds()` method (may need to add)

---

### Phase 3: FacetColumn Implementation

**Duration**: 1-2 days

**Goal**: Vertical faceting (same as FacetRow but vertical layout)

**Files to create**:
- `avenger-chart/src/marks/faceting/column.rs`
- `avenger-chart/src/marks/faceting/compiled_column.rs`

**Implementation**: 90% code reuse from FacetRow
- Change `FacetLayout::horizontal()` to `FacetLayout::vertical()`
- Stack facets vertically: `y += height + spacing` instead of `x += width + spacing`

**Tests**:
- Same test structure as FacetRow
- **Visual**: `test_facet_column_metrics.png` - 3 vertical facets

---

### Phase 4: FacetWrap Implementation

**Duration**: 2-3 days

**Goal**: Grid wrapping with auto-column computation

**Files to create**:
- `avenger-chart/src/marks/faceting/wrap.rs`
- `avenger-chart/src/marks/faceting/compiled_wrap.rs`

**New features**:
- `.columns(n)` option
- Auto-compute columns via `sqrt(n_facets).ceil()`
- Grid positioning logic in `FacetLayout::wrapped()`

**Implementation**:
```rust
pub struct FacetWrap<InnerC> {
    // Same as FacetRow plus:
    columns: Option<usize>,
}

impl FacetLayout {
    pub fn wrapped(
        subplots: &[(FacetValue, CompiledPlot)],
        columns: Option<usize>,
        spacing: f32,
        ...
    ) -> Result<Self, AvengerChartError> {
        let n = subplots.len();
        let n_cols = columns.unwrap_or_else(|| {
            (n as f64).sqrt().ceil() as usize
        });
        let n_rows = (n as f64 / n_cols as f64).ceil() as usize;

        let mut facet_bounds = HashMap::new();
        for (idx, (facet_val, _)) in subplots.iter().enumerate() {
            let col = idx % n_cols;
            let row = idx / n_cols;

            facet_bounds.insert(
                facet_val.clone(),
                LayoutBounds {
                    x: col as f32 * (facet_width + spacing),
                    y: row as f32 * (facet_height + spacing),
                    width: facet_width,
                    height: facet_height,
                },
            );
        }

        Ok(FacetLayout { facet_bounds, ... })
    }
}
```

**Tests**:
- Auto-compute: 9 facets → 3×3 grid, 10 facets → 4×3 grid
- Explicit columns: `.columns(4)` with 10 facets → 4×3 grid
- **Visual**: `test_facet_wrap_auto_columns.png` - grid layout

---

### Phase 5: FacetGrid Implementation

**Duration**: 3-4 days

**Goal**: Two-variable matrix layout with row/column scale sharing

**Files to create**:
- `avenger-chart/src/marks/faceting/grid.rs`
- `avenger-chart/src/marks/faceting/compiled_grid.rs`

**New features**:
- `.rows(column)` and `.cols(column)` methods
- Two-variable grouping: `df.group_by(vec![row_col, col_col])`
- `FacetValue::Multi(vec![row_val, col_val])`
- `ScaleSharingGrid` enum with SharedRows/SharedCols

**Implementation highlights**:
```rust
// In FacetGrid::compile()
let groups = df
    .group_by(vec![col(&self.row_column), col(&self.col_column)])?
    .aggregate(vec![], vec![])?
    .collect().await?;

for batch in groups {
    for row_idx in 0..batch.num_rows() {
        let row_val = extract_scalar(&batch, row_idx, &self.row_column)?;
        let col_val = extract_scalar(&batch, row_idx, &self.col_column)?;
        let facet_val = FacetValue::Multi(vec![row_val, col_val]);
        // ... rest same as FacetRow
    }
}

// In compute_shared_domains()
match scale_sharing {
    ScaleSharingGrid::SharedRows => {
        // Group subplots by row value
        // Compute unified domains within each row
    }
    ScaleSharingGrid::SharedCols => {
        // Group subplots by col value
        // Compute unified domains within each column
    }
    // ... other modes
}
```

**Tests**:
- Two-variable grouping produces correct (row, col) pairs
- SharedRows/SharedCols domain computation
- Rows-only and Cols-only modes work
- **Visual**: `test_facet_grid_two_way.png` - 3×4 matrix

---

### Phase 6: Manual Facet Implementation (Advanced)

**Duration**: 5+ days

**Goal**: Fully data-driven positioning with aggregate expressions

**Complexity**: Highest - requires dual coordinate system generics

**Files to create**:
- `avenger-chart/src/marks/faceting/manual.rs`
- `avenger-chart/src/marks/faceting/compiled_manual.rs`

**New challenges**:
- `Facet<OuterC, InnerC>` - dual coordinate system generics
- Position channels (x, y, width, height) as aggregate expressions
- Channel constraints: must evaluate to single value per facet group
- Scale configuration for position channels (confusing - reconsider?)

**Defer until**: Row/Column/Wrap/Grid are working and validated by users

---

### Phase 7: Layout System Integration (Parallel to Phases 2-6)

**Goal**: Make ChartLayout support faceted plots

**Changes needed**:
1. **Detect faceting marks** in `ChartLayout::new()`
2. **Multi-subplot regions**: Allocate space for N subplots instead of 1 plot area
3. **Axis positioning**: Handle `AxisDisplay::Edges` mode
4. **Shared scale axes**: Position axes between subplots for shared dimensions

**Implementation approach**:
- Start simple: Treat entire faceted mark as one big plot area (Phase 2-3)
- Enhance: Add proper subplot regions with shared axes (Phase 4-5)
- Polish: Edge-only axes, facet labels, borders (Phase 6+)

---

### Phase 8: Polish & Documentation

**Duration**: Ongoing

**Tasks**:
- Add facet labels (strip text showing facet values)
- Add facet borders
- Performance optimization (parallel subplot compilation?)
- Comprehensive examples in book
- Migration guide for async Mark::compile()

---

## Implementation Timeline

| Phase | Duration | Deliverable | Tests |
|-------|----------|-------------|-------|
| 0 | 1 day | Async Mark::compile | All existing tests pass |
| 1 | 2-3 days | Infrastructure (enums, layout, domains) | Unit tests |
| 2 | 3-4 days | FacetRow working end-to-end | Integration + visual |
| 3 | 1-2 days | FacetColumn | Integration + visual |
| 4 | 2-3 days | FacetWrap | Integration + visual |
| 5 | 3-4 days | FacetGrid | Integration + visual |
| 6 | 5+ days | Manual Facet (optional) | Integration + visual |
| 7 | Parallel | Layout integration | Visual tests |
| 8 | Ongoing | Polish | User feedback |

**Total**: ~3 weeks for Row/Column/Wrap/Grid (most useful features)

**Milestone 1** (after Phase 2): FacetRow working - first usable faceting
**Milestone 2** (after Phase 5): All automatic layouts working - feature complete
**Milestone 3** (after Phase 6): Manual Facet - advanced use cases

---

## Getting Started

**Recommended order**:
1. Phase 0 - Make all marks async (required for everything else)
2. Phase 1 - Build infrastructure (types, layout logic)
3. Phase 2 - FacetRow (first working demo!)
4. Validate with users, gather feedback
5. Phases 3-5 - Complete automatic layouts
6. Phase 6 - Manual Facet (if needed based on user requests)

**First PR**: Phases 0 + 1 together (infrastructure without user-facing API changes)
**Second PR**: Phase 2 (FacetRow - first feature!)
**Subsequent PRs**: One phase per PR for easier review
