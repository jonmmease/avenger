# Layout Algorithm

The Avenger Chart layout system is a sophisticated two-pass algorithm that automatically computes component positions and sizes based on canvas dimensions, content requirements, and user specifications. This page provides a deep dive into how the layout engine works, including debugging techniques and the mathematical flow of the computation.

## Overview

The layout algorithm solves a complex constraint satisfaction problem: given a canvas area and various chart components (titles, axes, legends, plot area), how do we arrange everything to maximize the plot area while ensuring all components have adequate space?

## Two-Pass Layout System

The layout computation happens in two distinct passes:

### Pass 1: Grid Construction and Overflow Measurement

In the first pass, the system:
1. Creates an Avenger frame grid structure
2. Measures each component's space requirements
3. Calculates "overflow" - the space needed by axes and guides beyond the plot area
4. Uses an estimated plot area (80% of available space) for initial measurements

### Pass 2: Native Frame Layout Computation

The second pass:
1. Takes the grid template from Pass 1
2. Resolves the Avenger frame tracks with fixed and fractional sizing
3. Computes exact pixel positions for all components
4. Handles flexible sizing for components like colorbar legends

## Layout Components

The layout system manages these primary components:

- **Canvas Area**: The total available space for the chart
- **Plot Area**: The rectangular region where data is rendered
- **Titles**: Chart title and subtitle
- **Axes**: X and Y axis guides with labels and ticks
- **Legends**: Symbol, color, size, and colorbar legends
- **Margins**: Flexible spacing around the plot area

## Overflow Calculation

Overflow represents how much space guide elements (axes, ticks, labels) require **beyond the plot area boundaries**. This is measured relative to the plot area, not the canvas.

The system measures overflow by:
1. Estimating the plot area as 80% of the canvas dimensions
2. Rendering axes and guides assuming this estimated plot size
3. Calculating the bounding box of all rendered axis marks
4. Computing how much the axis marks extend beyond the plot area edges:
   - `overflow_left = max(0, plot_left_edge - axis_bbox.min_x)`
   - `overflow_right = max(0, axis_bbox.max_x - plot_right_edge)`
   - `overflow_top = max(0, plot_top_edge - axis_bbox.min_y)`
   - `overflow_bottom = max(0, axis_bbox.max_y - plot_bottom_edge)`

**Important**: Titles and legends are NOT part of overflow. They are measured separately and get their own dedicated grid rows/columns. Overflow is exclusively for guide elements that protrude from the plot area.

## Debug Visualization

Set the `AVENGER_CHART_DEBUG_LAYOUT` environment variable to visualize the layout algorithm in action. This draws magenta rectangles around every layout component, making it easy to understand spacing and alignment.

### Example with Debug Layout

Here's a comprehensive example showing all major layout components with debug visualization enabled. The magenta rectangles show the bounds of each layout component:

```rust,render
use avenger_chart::prelude::*;
use datafusion::prelude::*;
use std::env;

// Enable debug layout visualization for this example
env::set_var("AVENGER_CHART_DEBUG_LAYOUT", "1");

let ctx = SessionContext::new();
let df = ctx
    .read_parquet(avenger_sample_data::iris_path(), ParquetReadOptions::default())
    .await?;

let plot = Plot::<Cartesian>::new()
    .data(df)
    .title("Iris Dataset Analysis")
    .subtitle("Sepal measurements with species classification")
    .mark(
        Symbol::new()
            .x_with(col("sepal_length"), |c| c
                .axis(|a| a
                    .title("Sepal Length (cm)")
                    .grid(true)
                )
            )
            .y_with(col("sepal_width"), |c| c
                .axis(|a| a
                    .title("Sepal Width (cm)")
                    .grid(true)
                )
            )
            .fill_with(col("species"), |c| c
                .scale_with::<Ordinal>(|s| s)
                .legend(|l| l
                    .title("Species")
                    .position(LegendPosition::Right)
                )
            )
            .size_with(col("petal_length"), |c| c
                .scale(|s| s.range_interval(lit(50.0), lit(400.0)))
                .legend(|l| l
                    .title("Petal Length")
                    .position(LegendPosition::Right)
                )
            )
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;

// Clean up - remove the environment variable
env::remove_var("AVENGER_CHART_DEBUG_LAYOUT");

Ok(evaluated)
```

The magenta rectangles show:
- **Outer rectangle**: Canvas area (default size)
- **Title/Subtitle boxes**: Top components
- **Axis regions**: Left and bottom areas including labels and titles
- **Legend container**: Right side with multiple legends
- **Plot area**: Central data rendering region

## Flexible Legends and Colorbars

Colorbar legends are special - they're configured to expand and fill available vertical space in their container. The native frame solver allocates remaining legend-container space across flexible legends:

```rust
// In the layout system, colorbars have:
flexible: true, // Expand to fill available space
```

### Example with Expanding Colorbar

```rust,render
use avenger_chart::prelude::*;
use datafusion::prelude::*;
use std::env;

// Enable debug layout visualization for this example
env::set_var("AVENGER_CHART_DEBUG_LAYOUT", "1");

let ctx = SessionContext::new();
let df = ctx
    .read_parquet(avenger_sample_data::iris_path(), ParquetReadOptions::default())
    .await?;

let plot = Plot::<Cartesian>::new()
    .data(df)
    .title("Colorbar Auto-Expansion Demo")
    .mark(
        Symbol::new()
            .x(col("sepal_length"))
            .y(col("sepal_width"))
            .fill_with(col("petal_width"), |c| c
                .legend(|l| l
                    .title("Petal Width (cm)")
                    .position(LegendPosition::Right)
                )
            )
            .size(100.0)
    );

let compiled = plot.compile(&ctx).await?;
let evaluated = compiled.evaluate(&ctx, None).await?;

// Clean up - remove the environment variable
env::remove_var("AVENGER_CHART_DEBUG_LAYOUT");

Ok(evaluated)
```

Notice how the colorbar automatically expands to fill the available vertical space in the legend container.

## Sizing Modes

The layout system supports three sizing strategies:

### 1. Fixed Canvas Size
```rust
.canvas_size(800.0, 600.0)  // Fixed outer dimensions
```

### 2. Fixed Plot Size
```rust
.plot_size(400.0, 300.0)  // Fixed inner plot area
```

### 3. Constrained Plot with Canvas
```rust
.canvas_size(800.0, 600.0)
.plot_size(400.0, 300.0)  // Both specified
```

When both are specified, margins become flexible using Avenger's fractional `fr(1.0)` frame tracks to fill remaining space.

## Layout Flow Diagram

This diagram shows the Avenger frame grid structure that Avenger Chart creates dynamically based on component requirements:

<div class="layout-diagram">
<svg viewBox="0 0 800 570" xmlns="http://www.w3.org/2000/svg" style="max-width: 100%; height: auto; margin: 2rem auto; display: block;"><rect x="10" y="10" width="780" height="550" fill="#fff" stroke="#495057" stroke-width="2"/><rect x="10" y="10" width="30" height="550" fill="#e9ecef" stroke="#adb5bd" stroke-width="1"/><text x="25" y="290" font-size="11" fill="#495057" font-family="sans-serif" text-anchor="middle" transform="rotate(-90 25 290)">Margin</text><rect x="760" y="10" width="30" height="550" fill="#e9ecef" stroke="#adb5bd" stroke-width="1"/><text x="775" y="290" font-size="11" fill="#495057" font-family="sans-serif" text-anchor="middle" transform="rotate(-90 775 290)">Margin</text><rect x="40" y="10" width="720" height="30" fill="#e9ecef" stroke="#adb5bd" stroke-width="1"/><text x="400" y="28" font-size="11" fill="#495057" font-family="sans-serif" text-anchor="middle">Margin</text><rect x="40" y="530" width="720" height="30" fill="#e9ecef" stroke="#adb5bd" stroke-width="1"/><text x="400" y="548" font-size="11" fill="#495057" font-family="sans-serif" text-anchor="middle">Margin</text><rect x="40" y="40" width="720" height="40" fill="#e7f5ff" stroke="#339af0" stroke-width="2" rx="4"/><text x="400" y="65" font-size="14" font-weight="bold" fill="#1864ab" font-family="sans-serif" text-anchor="middle">Title (optional)</text><rect x="40" y="80" width="720" height="35" fill="#e7f5ff" stroke="#339af0" stroke-width="2" rx="4"/><text x="400" y="102" font-size="13" fill="#1864ab" font-family="sans-serif" text-anchor="middle">Subtitle (optional)</text><rect x="95" y="115" width="545" height="30" fill="#fff3bf" stroke="#f59f00" stroke-width="2" rx="4"/><text x="367.5" y="135" font-size="12" fill="#e67700" font-family="sans-serif" text-anchor="middle">Overflow Top (guide)</text><g><rect x="40" y="145" width="55" height="355" fill="#fff3bf" stroke="#f59f00" stroke-width="2" rx="4"/><text x="67.5" y="322.5" font-size="12" fill="#e67700" font-family="sans-serif" text-anchor="middle" transform="rotate(-90 67.5 322.5)">Overflow Left</text><rect x="95" y="145" width="545" height="355" fill="#d3f9d8" stroke="#37b24d" stroke-width="3" rx="4"/><text x="367.5" y="327.5" font-size="18" font-weight="bold" fill="#2b8a3e" font-family="sans-serif" text-anchor="middle">Plot Area</text><rect x="640" y="145" width="55" height="355" fill="#fff3bf" stroke="#f59f00" stroke-width="2" rx="4"/><text x="667.5" y="322.5" font-size="12" fill="#e67700" font-family="sans-serif" text-anchor="middle" transform="rotate(-90 667.5 322.5)">Overflow Right</text><rect x="695" y="145" width="65" height="355" fill="#ffe3e3" stroke="#f03e3e" stroke-width="2" rx="4"/><text x="727.5" y="327.5" font-size="12" fill="#c92a2a" font-family="sans-serif" text-anchor="middle">Legends</text></g><rect x="95" y="500" width="545" height="30" fill="#fff3bf" stroke="#f59f00" stroke-width="2" rx="4"/><text x="367.5" y="520" font-size="12" fill="#e67700" font-family="sans-serif" text-anchor="middle">Overflow Bottom (guide)</text></svg>
</div>

**Key components:**
- **Margins**: Outer rows and columns (may be flexible or fixed depending on sizing mode)
- **Title/Subtitle**: Optional top rows with measured text heights
- **Guide Overflows**: Space for axes, ticks, and labels that extend beyond plot edges
- **Plot Area**: Central region where marks are rendered (uses `fr(1.0)` for flexible sizing)
- **Legend Containers**: Right, left, top, or bottom positions for legend groups (fixed widths)

Only components that are present and overflow regions larger than `MIN_GUIDE_OVERFLOW_SIZE` are added to the grid, keeping the structure efficient.

## Layout Computation Flow

The layout algorithm follows a precise two-pass process:

### Pass 1: Measurement and Grid Construction

**1. Estimate Initial Plot Area**

The system starts by assuming the plot area will occupy 80% of the canvas dimensions. This is purely an estimate used for measurement purposes - the final plot area size will be computed by the native frame solver in Pass 2.

**2. Measure Guide Overflow**

Using the estimated plot dimensions, the system constructs the scene graph representation of all axis guides (ticks, labels, grid lines) and measures their bounding boxes. This measurement is **renderer-independent** - it operates on geometric data structures, not rendered pixels.

Overflow is calculated as the distance guide elements extend beyond the estimated plot area edges on each side (left, right, top, bottom).

**Important**: Overflow only captures guide elements that protrude from the plot area. Titles and legends are measured separately and are not part of overflow.

**3. Measure Other Components**

- **Titles**: Text height is measured and multiplied by a line height factor (1.15 for title, 1.1 for subtitle)
- **Legends**: Each legend is measured based on its content (symbols, text, colorbar gradients)

**4. Build Dynamic Frame Grid**

The system constructs an Avenger frame grid by adding rows and columns only for components that exist:

- **Columns**: Margins → Overflow Left (if needed) → **Plot Area** (flexible) → Overflow Right (if needed) → Legend Containers → Margins
- **Rows**: Margins → Title (if exists) → Subtitle (if exists) → Overflow Top (if needed) → **Plot Area** (flexible) → Overflow Bottom (if needed) → Margins

Small overflows below a minimum threshold are omitted to keep the grid efficient.

### Pass 2: Native Frame Layout Computation

The native frame solver takes the grid template and canvas dimensions and computes final pixel positions for every component.

**Key Behavior**:
- **Fixed-size tracks** (titles, overflows, legends) get exactly the space they measured
- **Flexible tracks** (plot area, margins) expand to fill remaining space using Avenger fractional units (`fr`)
- The plot area "absorbs" whatever space is left after allocating fixed-size components

This is why the plot area automatically adjusts when you add titles, legends, or have axes with long labels - the frame solver recalculates the flexible space to ensure everything fits within the canvas bounds.

### Why Two Passes?

The two-pass approach solves a circular dependency:
- **Pass 1**: We need plot dimensions to measure overflow, so we estimate them
- **Pass 2**: We use the measured overflow to compute the actual plot dimensions through grid layout

This converges because the overflow measurement is stable - axes rendered at slightly different plot sizes produce similar overflow amounts, making the 80% estimate accurate enough for measurement purposes.

### Radius-Aware Domain Refinement

After Pass 2 computes the final plot dimensions and scale ranges, the system recomputes domains for position scales that have radius expressions (like symbol size). This ensures that symbols won't be clipped at the plot area edges - the domain is adjusted to provide adequate padding based on the maximum symbol radius at each edge of the data extent.

This refinement uses the actual computed ranges from Pass 2, preserving the visual integrity of symbol marks even when their radii vary across the data.

## Limitations and Future Improvements

The current two-pass system has a known limitation: **guide overflow is measured using estimated plot dimensions, not the final dimensions**.

### The Issue

When the frame solver computes the final layout in Pass 2, the actual plot area may differ from the 80% estimate used in Pass 1. This can affect:

1. **Axis tick positioning**: Final plot dimensions might suggest different tick counts or positions
2. **Label wrapping**: Available space for axis labels might change
3. **Grid line placement**: The scale's domain-to-range mapping uses final dimensions

As a result, axis guides may end up with slightly more or slightly less space than they ideally need, potentially causing:
- Extra whitespace around axes
- Occasional label truncation or overlap (rare)
- Suboptimal tick placement

### Why This Happens

After Pass 2 determines the final plot size, domain refinement (for radius-aware padding) occurs. This domain adjustment could theoretically change axis tick values or label text, which would alter the measured overflow - but we don't remeasure.

### Future Direction

A more robust approach would iterate the two passes until convergence:

```
Loop:
  1. Measure overflow with current plot size estimate
  2. Compute layout with the native frame solver
  3. Refine domains with final ranges
  4. Check if overflow changed significantly
  5. If stable: done. If not: repeat with new estimate
```

This would ensure guides always have exactly the space they need. The current single-pass approach works well in practice because:
- The 80% estimate is usually close to the final size
- Domain refinement rarely affects axis label lengths significantly
- Small discrepancies are visually imperceptible in most charts

The convergence approach would add complexity and computation cost, so it's reserved for future optimization if needed.

## Debugging Tips

### Enable Debug Layout
```bash
AVENGER_CHART_DEBUG_LAYOUT=1 cargo test my_test
```

### Inspect Layout Bounds
With debug mode enabled, each magenta rectangle represents:
- **Position**: Top-left corner coordinates
- **Size**: Width and height in pixels
- **Hierarchy**: Nested rectangles show containment

### Common Issues

1. **Clipped Axes**: Increase canvas size or reduce font sizes
2. **Overlapping Legends**: Use different positions or stack vertically
3. **Small Plot Area**: Reduce component sizes or increase canvas

## Performance Considerations

The two-pass system is optimized for:
- **Single Layout Computation**: Layout is computed once during evaluation
- **Cached Measurements**: Text and component sizes are cached
- **Efficient Grid Construction**: the native frame grid has a small, deterministic O(n) track solver

## Advanced: Custom Layout Containers

Future versions will support custom layout containers for complex dashboards:

```rust
// Planned API
Container::horizontal()
    .child(Plot::new()...)
    .child(Plot::new()...)
    .spacing(10.0)
```

## Summary

The Avenger Chart layout algorithm provides:
- **Automatic**: No manual positioning required
- **Flexible**: Adapts to different canvas sizes
- **Debuggable**: Visual debugging with magenta rectangles
- **Efficient**: Two-pass system minimizes computation
- **Extensible**: Built on explicit Avenger frame grid concepts

Understanding this system helps you:
- Debug layout issues effectively
- Choose appropriate sizing strategies
- Optimize chart appearance
- Extend the system for custom needs

## See Also

- [Layout & Sizing](./layout.md) - User-facing layout API
- [Rendering Pipeline](./rendering.md) - How layout fits into rendering
- [Debugging Guide](../DEBUGGING.md) - Comprehensive debugging techniques
