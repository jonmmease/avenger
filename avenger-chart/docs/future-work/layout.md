# Layout System

> **Note**: Code examples in this document are conceptual. Actual implementation will require proper imports and may differ in API details.

## Layouts

Layouts compose independent visualizations into dashboards. Unlike faceting and repeat (which are marks within a Plot), layouts operate at a higher level, combining already-compiled plots into multi-plot compositions.

### Core Concept

Everything that can be rendered implements the `Layout` trait:

```rust
pub trait Layout {
    fn render(&self, ctx: &RenderContext) -> Result<Image>;
}
```

- **`CompiledPlot`** implements `Layout` (a single visualization)
- **`GridLayout`** implements `Layout` (grid of layouts)
- **`FlexHLayout`** implements `Layout` (horizontal flex container)
- **`FlexVLayout`** implements `Layout` (vertical flex container)

This enables **recursive composition**: layouts can contain plots or other layouts.

### Layout Types

| Type | Purpose | Use Case |
|------|---------|----------|
| `CompiledPlot` | Single visualization | Implicit layout for single plots |
| `GridLayout` | CSS Grid-like positioning | Precise 2D layout control |
| `FlexHLayout` | Horizontal flex container | Side-by-side arrangements |
| `FlexVLayout` | Vertical flex container | Vertical stacking |

---

## GridLayout - Precise Grid Positioning

Create a grid with explicit row/column tracks and position layouts in specific cells.

### API

```rust
GridLayout::new()
    // Grid structure
    .columns(spec)           // e.g., "1fr 2fr 300px"
    .rows(spec)              // e.g., "200px auto 1fr"
    .gap(px)                 // Uniform gap
    .gap_row(px)             // Row gap override
    .gap_col(px)             // Column gap override
    .padding(px)             // Outer padding

    // Dashboard-level metadata
    .title(string)
    .subtitle(string)

    // Add children (any Layout)
    .add(layout, grid_area)

    // Render
    .render(&ctx)?
```

### Track Specifications

```rust
// String syntax (CSS Grid-like)
.columns("1fr 2fr 300px")           // 3 columns: flex, flex (2x), fixed
.rows("200px auto 1fr")             // 3 rows: fixed, content, flex

// Array syntax
.columns(vec!["1fr", "2fr", "300px"])
.rows(vec!["200px", "auto", "1fr"])
```

### Grid Areas

```rust
grid_area!(row, col)              // Single cell
grid_area!(row, col_start..col_end)  // Column span
grid_area!(row_start..row_end, col)  // Row span
grid_area!(row_start..row_end, col_start..col_end)  // Both
```

### Examples

#### Basic Dashboard

```rust
// Build visualizations
let price_chart = Plot::<Cartesian>::new()
    .data(stock_df)
    .title("Stock Price")
    .mark(Line::new().x(col("date")).y(col("close")))
    .compile(&ctx)?;

let volume_chart = Plot::<Cartesian>::new()
    .data(stock_df)
    .title("Volume")
    .mark(Rect::new().x(col("date")).y(col("volume")))
    .compile(&ctx)?;

let summary_chart = Plot::<Polar>::new()
    .data(category_df)
    .title("Category Breakdown")
    .mark(Arc::new().r(col("value")).theta(col("angle")))
    .compile(&ctx)?;

// Compose into grid
GridLayout::new()
    .title("Market Dashboard")
    .columns("2fr 1fr")           // Two columns: main (2x), sidebar (1x)
    .rows("250px 200px auto")     // Three rows
    .gap(20.0)
    .padding(30.0)
    .add(price_chart, grid_area!(1, 1))       // Row 1, col 1
    .add(volume_chart, grid_area!(2, 1))      // Row 2, col 1
    .add(summary_chart, grid_area!(1..3, 2))  // Rows 1-2, col 2 (spans 2 rows)
    .render(&ctx)?
```

#### Spanning Layouts

```rust
GridLayout::new()
    .title("Analysis Dashboard")
    .columns("1fr 1fr 1fr")
    .rows("100px 300px 300px")
    .gap(15.0)

    // Header spanning all columns
    .add(header_plot, grid_area!(1, 1..4))

    // Two plots in second row
    .add(plot_a, grid_area!(2, 1))
    .add(plot_b, grid_area!(2, 2..4))  // Spans columns 2-3

    // Three plots in third row
    .add(plot_c, grid_area!(3, 1))
    .add(plot_d, grid_area!(3, 2))
    .add(plot_e, grid_area!(3, 3))

    .render(&ctx)?
```

#### Complex Layout with Faceting

```rust
// Build a faceted visualization
let faceted_plot = Plot::<Cartesian>::new()
    .data(iris_df)
    .title("Iris Measurements by Species")
    .mark(
        FacetWrap::new()
            .facet_by(vec!["species"])
            .columns(3)
            .subplot(
                Plot::<Cartesian>::new()
                    .mark(Symbol::new().x(col("sepal_length")).y(col("sepal_width")))
            )
    )
    .compile(&ctx)?;

let summary_plot = Plot::<Cartesian>::new()
    .data(summary_df)
    .title("Summary Statistics")
    .mark(Rect::new().x(col("metric")).y(col("value")))
    .compile(&ctx)?;

// Combine in grid
GridLayout::new()
    .title("Iris Analysis Dashboard")
    .columns("3fr 1fr")
    .rows("100%")
    .gap(25.0)
    .add(faceted_plot, grid_area!(1, 1))   // Faceted plot on left
    .add(summary_plot, grid_area!(1, 2))   // Summary on right
    .render(&ctx)?
```

---

## FlexHLayout - Horizontal Arrangement

Arrange layouts horizontally with flexible sizing. Children flow left-to-right.

### API

```rust
FlexHLayout::new()
    // Layout options
    .gap(px)                 // Space between children
    .padding(px)             // Outer padding
    .align_items(alignment)  // Vertical alignment

    // Dashboard-level metadata
    .title(string)
    .subtitle(string)

    // Add children with flex configuration
    .add(layout, flex_item!())
    .add(layout, flex_item!().grow(n))
    .add(layout, flex_item!().width(px))
    .add(layout, flex_item!().shrink(n))

    // Render
    .render(&ctx)?
```

### Flex Item Configuration

```rust
flex_item!()                    // Default (no growth, natural size)
flex_item!().grow(2.0)          // Takes 2x space of grow(1.0)
flex_item!().width(300)         // Fixed width
flex_item!().shrink(0.5)        // Shrinks half as much
flex_item!().grow(1.0).width(200)  // Minimum width but can grow
```

### Examples

#### Side-by-Side Comparison

```rust
let plot_2020 = Plot::<Cartesian>::new()
    .data(df_2020)
    .title("2020")
    .mark(Symbol::new().x(col("x")).y(col("y")))
    .compile(&ctx)?;

let plot_2021 = Plot::<Cartesian>::new()
    .data(df_2021)
    .title("2021")
    .mark(Symbol::new().x(col("x")).y(col("y")))
    .compile(&ctx)?;

let plot_2022 = Plot::<Cartesian>::new()
    .data(df_2022)
    .title("2022")
    .mark(Symbol::new().x(col("x")).y(col("y")))
    .compile(&ctx)?;

FlexHLayout::new()
    .title("Year-over-Year Comparison")
    .gap(20.0)
    .add(plot_2020, flex_item!().grow(1.0))  // Equal widths
    .add(plot_2021, flex_item!().grow(1.0))
    .add(plot_2022, flex_item!().grow(1.0))
    .render(&ctx)?
```

#### Sidebar Layout

```rust
let sidebar_plot = Plot::<Cartesian>::new()
    .data(controls_df)
    .title("Controls")
    .mark(Rect::new().x(col("param")).y(col("value")))
    .compile(&ctx)?;

let main_plot = Plot::<Cartesian>::new()
    .data(main_df)
    .title("Main Visualization")
    .mark(Symbol::new().x(col("x")).y(col("y")).fill(col("category")))
    .compile(&ctx)?;

FlexHLayout::new()
    .gap(15.0)
    .add(sidebar_plot, flex_item!().width(250))  // Fixed sidebar
    .add(main_plot, flex_item!().grow(1.0))      // Main grows to fill
    .render(&ctx)?
```

#### Proportional Sizing

```rust
FlexHLayout::new()
    .title("Metrics Dashboard")
    .gap(10.0)
    .add(plot_a, flex_item!().grow(2.0))  // Takes 2/5 of space
    .add(plot_b, flex_item!().grow(2.0))  // Takes 2/5 of space
    .add(plot_c, flex_item!().grow(1.0))  // Takes 1/5 of space
    .render(&ctx)?
```

---

## FlexVLayout - Vertical Stacking

Arrange layouts vertically with flexible sizing. Children flow top-to-bottom.

### API

```rust
FlexVLayout::new()
    // Layout options
    .gap(px)                 // Space between children
    .padding(px)             // Outer padding
    .align_items(alignment)  // Horizontal alignment

    // Dashboard-level metadata
    .title(string)
    .subtitle(string)

    // Add children with flex configuration
    .add(layout, flex_item!())
    .add(layout, flex_item!().grow(n))
    .add(layout, flex_item!().height(px))
    .add(layout, flex_item!().shrink(n))

    // Render
    .render(&ctx)?
```

### Examples

#### Vertical Dashboard

```rust
let price_plot = Plot::<Cartesian>::new()
    .data(stock_df)
    .title("Price")
    .mark(Line::new().x(col("date")).y(col("price")))
    .compile(&ctx)?;

let volume_plot = Plot::<Cartesian>::new()
    .data(stock_df)
    .title("Volume")
    .mark(Rect::new().x(col("date")).y(col("volume")))
    .compile(&ctx)?;

let indicators_plot = Plot::<Cartesian>::new()
    .data(stock_df)
    .title("Technical Indicators")
    .mark(Line::new().x(col("date")).y(col("rsi")))
    .compile(&ctx)?;

FlexVLayout::new()
    .title("Stock Analysis")
    .gap(15.0)
    .add(price_plot, flex_item!().grow(2.0))      // Main chart: 2x height
    .add(volume_plot, flex_item!().grow(1.0))     // Volume: 1x height
    .add(indicators_plot, flex_item!().height(100))  // Indicators: fixed 100px
    .render(&ctx)?
```

#### Fixed Header/Footer

```rust
FlexVLayout::new()
    .gap(0.0)
    .add(header_plot, flex_item!().height(80))   // Fixed header
    .add(main_plot, flex_item!().grow(1.0))      // Main grows to fill
    .add(footer_plot, flex_item!().height(60))   // Fixed footer
    .render(&ctx)?
```

---

## Nested Layouts

Layouts can contain other layouts, enabling complex hierarchical compositions.

### Example: Sidebar with Stacked Plots

```rust
// Build sidebar as vertical stack
let sidebar = FlexVLayout::new()
    .add(controls_plot, flex_item!().height(200))
    .add(legend_plot, flex_item!().grow(1.0));
    // Returns FlexVLayout (which implements Layout)

// Build main area
let main_plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(Symbol::new().x(col("x")).y(col("y")))
    .compile(&ctx)?;
    // Returns CompiledPlot (which implements Layout)

// Compose horizontally
FlexHLayout::new()
    .title("Application Dashboard")
    .gap(20.0)
    .add(sidebar, flex_item!().width(300))     // FlexVLayout as child
    .add(main_plot, flex_item!().grow(1.0))    // CompiledPlot as child
    .render(&ctx)?
```

### Example: Grid of Flex Layouts

```rust
// Top row: horizontal flex
let top_row = FlexHLayout::new()
    .add(plot_a, flex_item!().grow(1.0))
    .add(plot_b, flex_item!().grow(1.0))
    .add(plot_c, flex_item!().grow(1.0));

// Bottom left: vertical flex
let bottom_left = FlexVLayout::new()
    .add(plot_d, flex_item!().grow(1.0))
    .add(plot_e, flex_item!().grow(1.0));

// Compose in grid
GridLayout::new()
    .title("Complex Dashboard")
    .columns("2fr 1fr")
    .rows("250px 1fr")
    .gap(15.0)
    .add(top_row, grid_area!(1, 1..3))        // FlexH spans both columns
    .add(bottom_left, grid_area!(2, 1))       // FlexV in bottom-left
    .add(plot_f, grid_area!(2, 2))            // Single plot in bottom-right
    .render(&ctx)?
```

### Example: Deeply Nested

```rust
// Level 3: plots in vertical flex
let metrics_stack = FlexVLayout::new()
    .add(cpu_plot, flex_item!().grow(1.0))
    .add(memory_plot, flex_item!().grow(1.0))
    .add(disk_plot, flex_item!().grow(1.0));

// Level 2: sidebar (vertical) and main (grid)
let main_grid = GridLayout::new()
    .columns("1fr 1fr")
    .rows("1fr 1fr")
    .add(plot_a, grid_area!(1, 1))
    .add(plot_b, grid_area!(1, 2))
    .add(plot_c, grid_area!(2, 1))
    .add(plot_d, grid_area!(2, 2));

let sidebar = FlexVLayout::new()
    .add(header_plot, flex_item!().height(100))
    .add(metrics_stack, flex_item!().grow(1.0));  // Nested FlexV

// Level 1: horizontal composition
FlexHLayout::new()
    .title("Application Dashboard")
    .add(sidebar, flex_item!().width(300))     // FlexV containing FlexV
    .add(main_grid, flex_item!().grow(1.0))    // Grid
    .render(&ctx)?
```

---

## Layout vs Faceting/Repeat

| Aspect | Faceting/Repeat Marks | Layouts |
|--------|----------------------|---------|
| **Purpose** | Data/schema-driven subplot replication | Compose independent visualizations |
| **Level** | Mark within Plot | Top-level composition |
| **Data** | Shared source (filtered or full) | Independent sources per plot |
| **Relationship** | Related (template-based) | Potentially unrelated |
| **Coordinate Systems** | Inner can differ from outer | Each plot independent |
| **When to use** | Showing same viz for different groups/variables | Dashboard with distinct visualizations |

### When to Use Each

**Use Faceting** when:
- You have one dataset
- Want to show same visualization for different groups
- Groups are data-driven (species, regions, years)
- Need coordinated scale sharing

**Use Repeat** when:
- You have one dataset
- Want to show same visualization for different variables
- Variables are schema-driven (measurements, metrics)
- Need per-variable scale consistency

**Use Layout** when:
- You have multiple independent visualizations
- Different data sources
- Different chart types
- Dashboard composition
- Each plot designed separately

### Combining Them

You can freely combine faceting/repeat marks within plots that are then composed via layouts:

```rust
// Build a faceted plot
let faceted = Plot::<Cartesian>::new()
    .data(df1)
    .mark(
        FacetWrap::new()
            .facet_by(vec!["species"])
            .subplot(Plot::new().mark(...))
    )
    .compile(&ctx)?;

// Build a repeated plot
let repeated = Plot::<Cartesian>::new()
    .data(df2)
    .mark(
        RepeatRow::new()
            .variables(vec!["var1", "var2", "var3"])
            .subplot(|(var, idx)| Plot::new().mark(...))
    )
    .compile(&ctx)?;

// Build a regular plot
let simple = Plot::<Cartesian>::new()
    .data(df3)
    .mark(Symbol::new()...)
    .compile(&ctx)?;

// Compose all three in a grid
GridLayout::new()
    .title("Comprehensive Dashboard")
    .columns("1fr 1fr")
    .rows("auto auto")
    .add(faceted, grid_area!(1, 1))
    .add(repeated, grid_area!(1, 2))
    .add(simple, grid_area!(2, 1..3))
    .render(&ctx)?
```

---

## Single Plot Rendering

A `CompiledPlot` implements `Layout`, so you can render it directly:

```rust
// Explicit compilation
let compiled = Plot::<Cartesian>::new()
    .data(df)
    .title("My Chart")
    .mark(Symbol::new().x(col("x")).y(col("y")))
    .compile(&ctx)?;

compiled.render(&ctx)?  // Layout::render() for CompiledPlot

// Or use Plot's convenience method
Plot::<Cartesian>::new()
    .data(df)
    .title("My Chart")
    .mark(Symbol::new().x(col("x")).y(col("y")))
    .render(&ctx)?  // Compiles internally, then renders
```

This means the simple case requires no layout API - single plots "just work".

---

## Layout Design Principles

1. **Everything is a Layout**: `CompiledPlot`, `GridLayout`, `FlexHLayout`, `FlexVLayout` all implement the same `Layout` trait

2. **Recursive Composition**: Layouts contain `Box<dyn Layout>`, enabling arbitrary nesting

3. **Type Safety**: Generic `add<L: Layout>()` methods accept anything that implements `Layout`

4. **Implicit Simple Case**: `CompiledPlot` already implements `Layout`, no wrapping needed

5. **Explicit Composition**: Layouts only used when composing multiple visualizations

6. **Familiar Mental Models**: Grid follows CSS Grid, Flex follows CSS Flexbox

7. **Clean Separation**: Faceting/Repeat are marks (data-driven), Layouts are composition (structure-driven)
