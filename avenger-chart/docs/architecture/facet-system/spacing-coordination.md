# Spacing Coordination System

This document describes the named spacing keys system and overflow handling in the avenger-chart facet architecture. It explains how facets coordinate spacing across nested levels and how overflow measurements drive layout decisions.

## Overview

The spacing coordination system solves a fundamental problem in nested faceting: **how do sibling subplots agree on consistent spacing when each measures its own overflow independently?**

Consider a grid of facets where each cell has different axis label lengths. Without coordination, each column would have different widths based on its Y-axis labels, and each row would have different heights based on its X-axis labels. The spacing coordination system ensures:

1. All rows share the same inter-row gap (accommodating the worst-case X-axis labels)
2. All columns share the same inter-column gap (accommodating the worst-case Y-axis labels)
3. Legends and other guide elements align consistently

The solution uses a **named spacing keys** pattern where each facet reports its spacing requirements as key-value pairs. Parent facets aggregate these by taking the maximum value for each key, then pass the coordinated values back during the render pass.

## SpacingNeeds and MeasurementResult

### MeasurementResult Struct

**Location:** `avenger-chart/src/guide/overflow.rs:51`

```rust
pub struct MeasurementResult {
    /// Final overflow after internal coordination
    pub overflow: OverflowSpaceRequirement,

    /// Named spacing values computed during measurement
    /// Keys: "inter_row_gap", "inter_col_gap", "legend_*", etc.
    pub spacing_needs: HashMap<String, f32>,
}
```

The `MeasurementResult` struct combines two pieces of information:

1. **overflow**: The space required beyond the plot area for guides (axes, labels, titles)
2. **spacing_needs**: A map of named spacing requirements that need coordination with siblings

### OverflowSpaceRequirement Struct

**Location:** `avenger-chart/src/guide/overflow.rs:26`

```rust
pub struct OverflowSpaceRequirement {
    pub top: f32,
    pub bottom: f32,
    pub left: f32,
    pub right: f32,
}
```

Overflow tracks the space needed in each direction beyond the plot area. This includes:
- Axis tick labels and titles
- Facet labels and titles
- Legend content (when positioned at edges)

## Spacing Key Constants

**Location:** `avenger-chart/src/guide/overflow.rs:7` (module `spacing_keys`)

| Constant | Value | Purpose |
|----------|-------|---------|
| `INTER_ROW_GAP` | `"inter_row_gap"` | Gap between adjacent rows in a row facet |
| `INTER_COL_GAP` | `"inter_col_gap"` | Gap between adjacent columns in a column facet |
| `LEGEND_RIGHT` | `"legend_right"` | Space needed for legend on right side |
| `LEGEND_LEFT` | `"legend_left"` | Space needed for legend on left side |
| `LEGEND_TOP` | `"legend_top"` | Space needed for legend on top |
| `LEGEND_BOTTOM` | `"legend_bottom"` | Space needed for legend on bottom |
| `NESTED_MEASUREMENT` | `"nested_measurement"` | Marker for nested measurement context |

### Additional Constants in Coordination Module

**Location:** `avenger-chart/src/facet/coordination.rs:22-32`

| Constant | Value | Purpose |
|----------|-------|---------|
| `SHARED_OVERFLOW_LEFT` | `"shared_overflow_left"` | Left overflow for cross-subplot alignment |
| `SHARED_OVERFLOW_RIGHT` | `"shared_overflow_right"` | Right overflow for cross-subplot alignment |
| `SHARED_OVERFLOW_TOP` | `"shared_overflow_top"` | Top overflow for cross-subplot alignment |
| `SHARED_OVERFLOW_BOTTOM` | `"shared_overflow_bottom"` | Bottom overflow for cross-subplot alignment |

### Dimension-Specific Keys via Trait

**Location:** `avenger-chart/src/facet/dimension_config.rs:78-82`

The `FacetDimensionConfig` trait provides dimension-specific gap keys:

```rust
fn inter_gap_key() -> &'static str;
```

- `RowDimensionConfig::inter_gap_key()` returns `"inter_row_gap"` (line 155)
- `ColumnDimensionConfig::inter_gap_key()` returns `"inter_col_gap"` (line 230)

This abstraction allows the same algorithm to work for both row and column facets.

## Aggregation Logic

### Per-Subplot Collection

During `measure_pass`, each subplot's spacing needs are collected:

**Location:** `avenger-chart/src/facet/marks/facet_evaluation.rs:517-543`

```rust
async fn measure_subplot(...) -> Result<(
    OverflowSpaceRequirement,  // guide_only_overflow
    OverflowSpaceRequirement,  // total_overflow
    HashSet<LegendPosition>,
    HashMap<String, f32>,      // spacing_needs from inner guide
), AvengerChartError> {
    // ... measure with scales ...
    let spacing_needs = compiled_subplot
        .get_guide_spacing_needs(width, height, ctx, params, scales, Some(filter_df))
        .await?;
    Ok((guide_only, total_overflow, legend_positions, spacing_needs))
}
```

### Max-Based Aggregation

After all subplots are measured, spacing needs are aggregated using maximum values:

**Location:** `avenger-chart/src/facet/marks/facet_evaluation.rs:664-678`

```rust
// Aggregate spacing_needs from all subplots using max per key
let mut aggregated_spacing_needs: HashMap<String, f32> = HashMap::new();

for (_, guide_only, total_overflow, legend_positions, spacing_needs) in sorted {
    guide_only_measurements.push(guide_only);
    overflow_measurements.push(total_overflow);
    all_legend_positions.extend(legend_positions);

    // Aggregate spacing_needs: use max for each key
    for (key, value) in spacing_needs {
        aggregated_spacing_needs
            .entry(key)
            .and_modify(|existing| *existing = existing.max(value))
            .or_insert(value);
    }
}
```

### MeasurementResult Helper Methods

**Location:** `avenger-chart/src/guide/overflow.rs:61-85`

```rust
impl MeasurementResult {
    /// Add a spacing need and return self for chaining
    pub fn with_spacing(mut self, key: impl Into<String>, value: f32) -> Self {
        self.spacing_needs.insert(key.into(), value);
        self
    }

    /// Merge child spacing_needs using max aggregation for each key
    pub fn merge_spacing_needs(mut self, other: HashMap<String, f32>) -> Self {
        for (key, value) in other {
            self.spacing_needs
                .entry(key)
                .and_modify(|v| *v = v.max(value))
                .or_insert(value);
        }
        self
    }
}
```

## Overflow Types

The system tracks two types of overflow, used for different purposes:

### Guide-Only Overflow

**Purpose:** Cross-subplot alignment for axis labels and titles.

Guide-only overflow measures just the space needed for axis elements (ticks, labels, titles). This is used to ensure consistent sizing across subplots that share scales - all subplots need the same space for their Y-axis labels even if some have shorter labels.

**Collection Pattern:**
```rust
let (guide_only, total_overflow, ...) = measure_subplot(...).await?;
guide_only_measurements.push(guide_only);
```

**Usage:** Guide-only overflow is aggregated to compute `global_max_overflow`, which is applied to all subplots to ensure alignment.

### Total Overflow

**Purpose:** Inter-cell spacing calculation.

Total overflow includes guide overflow plus any additional content like legends. This is used to compute the gap needed between adjacent cells.

**Collection Pattern:**
```rust
overflow_measurements.push(total_overflow);
```

**Usage:** Total overflow feeds into gap calculation functions.

### Gap Calculation Functions

**Location:** `avenger-chart/src/facet/guide_measurement.rs`

#### Inter-Row Gap (lines 22-33)

```rust
pub fn calculate_inter_row_gap(overflows: &[OverflowSpaceRequirement], safety_margin: f32) -> f32 {
    if overflows.len() < 2 {
        return 0.0;
    }
    let max_gap = overflows
        .windows(2)
        .map(|pair| pair[0].bottom + pair[1].top)
        .fold(0.0_f32, f32::max);
    max_gap + safety_margin
}
```

For adjacent rows, the required gap is: `bottom_overflow[row_i] + top_overflow[row_i+1]`. The function returns the maximum gap needed across all adjacent pairs.

#### Inter-Column Gap (lines 49-60)

```rust
pub fn calculate_inter_col_gap(overflows: &[OverflowSpaceRequirement], safety_margin: f32) -> f32 {
    if overflows.len() < 2 {
        return 0.0;
    }
    let max_gap = overflows
        .windows(2)
        .map(|pair| pair[0].right + pair[1].left)
        .fold(0.0_f32, f32::max);
    max_gap + safety_margin
}
```

For adjacent columns, the required gap is: `right_overflow[col_i] + left_overflow[col_i+1]`.

### Overflow Aggregation Rules

**Location:** `avenger-chart/src/facet/guide_measurement.rs:69-75`

```rust
pub fn aggregate_overflow(overflows: &[OverflowSpaceRequirement]) -> OverflowSpaceRequirement {
    overflows
        .iter()
        .fold(OverflowSpaceRequirement::default(), |acc, o| {
            acc.max_components(o)
        })
}
```

The aggregation takes the maximum of each component (top, bottom, left, right) across all overflow measurements.

## FacetPass1Result

**Location:** `avenger-chart/src/facet/marks/facet_evaluation.rs:87-115`

The result of measurement pass 1 includes spacing needs:

```rust
struct FacetPass1Result {
    /// Total overflow (including legends) for spacing calculations
    overflow_measurements: Vec<OverflowSpaceRequirement>,
    final_dimension_scale: ConfiguredScaleWithSpec,
    final_shared_scales: Option<HashMap<String, ConfiguredScaleWithSpec>>,
    final_rects: Vec<SubplotRect>,
    fallback_builder: Option<ScaleBuilder>,
    shared_data_extents: Option<HashMap<String, SerializableDataExtents>>,

    /// Named spacing needs reported by this facet for coordination with parent facets
    spacing_needs: HashMap<String, f32>,

    uniform_cell_count: Option<usize>,
    phantom_offset: usize,
}
```

The `spacing_needs` field contains all the aggregated spacing requirements computed during Pass 1, ready to be passed to Phase 1.5 coordination.

## Re-measurement Triggers

Re-measurement (Phase 1.5) is triggered when cross-dimension gaps are detected:

**Condition:** The presence of cross-dimension gap keys in `spacing_needs` indicates nested facets that require coordinated spacing.

**Location:** Referenced in `guides/measurement-coordination-detailed.md:1530-1619`

```rust
// Check for cross-dimension gaps (only exist from 2D overflow analysis)
let has_cross_dimension_gap = if DimConfig::is_col_facet() {
    pass1.spacing_needs.contains_key("inter_row_gap")  // FacetColumn computed row gap
} else {
    pass1.spacing_needs.contains_key("inter_col_gap")  // FacetRow computed column gap
};

if has_cross_dimension_gap {
    // NESTED FACETS: Need to re-run measure_pass
    let mut coord_ctx = FacetCoordinationContext::from_params(&effective_context.params)
        .unwrap_or_default();

    coord_ctx = coord_ctx.with_coordinated_spacing(pass1.spacing_needs.clone());

    // Re-run measure_pass with updated context
    let pass2 = measure_pass::<DimConfig, _>(...).await?;
}
```

### When Re-measurement Occurs

| Scenario | Has Cross-Dimension Gap | Re-measurement |
|----------|------------------------|----------------|
| FacetColumn only | No | No |
| FacetRow only | No | No |
| FacetColumn > FacetRow | Yes (`inter_row_gap`) | Yes |
| FacetRow > FacetColumn | Yes (`inter_col_gap`) | Yes |

### What Re-measurement Accomplishes

1. **Inner facets receive coordinated gaps:** The `coordinated_spacing` field in `FacetCoordinationContext` contains the aggregated spacing values.

2. **Consistent cell sizing:** Inner facets use `get_coordinated_spacing(key)` to apply the same gap regardless of their local overflow.

3. **Proper layout:** The second measurement produces accurate cell rectangles based on coordinated spacing.

## Coordinated Spacing in FacetCoordinationContext

**Location:** `avenger-chart/src/facet/coordination.rs:672-685`

```rust
/// Coordinated spacing values aggregated from child facets
///
/// During measure_pass, inner facets compute their spacing needs and report them in
/// FacetPass1Result::spacing_needs. The outer facet aggregates these by taking the max
/// of each named spacing value across all children, then passes the result back here
/// during render_pass.
///
/// Standard keys:
/// - "inter_row_gap": Gap between rows (computed by inner row facet)
/// - "inter_col_gap": Gap between columns (computed by inner column facet)
/// - "legend_right": Right margin for legend alignment
/// - "legend_bottom": Bottom margin for legend alignment
pub coordinated_spacing: HashMap<String, f32>,
```

### Builder and Accessor Methods

**Location:** `avenger-chart/src/facet/coordination.rs:885-898`

```rust
/// Builder: Set coordinated spacing values from aggregated child spacing needs
pub fn with_coordinated_spacing(mut self, spacing: HashMap<String, f32>) -> Self {
    self.coordinated_spacing = spacing;
    self
}

/// Get a coordinated spacing value by key
pub fn get_coordinated_spacing(&self, key: &str) -> Option<f32> {
    self.coordinated_spacing.get(key).copied()
}
```

## Usage Example: Inter-Row Gap in FacetColumn

When a FacetColumn contains nested FacetRows, the coordination flow is:

1. **Pass 1 - FacetColumn measures each column:**
   - Each column contains a FacetRow
   - FacetRow measures its subplots and computes `inter_row_gap`
   - Returns spacing_needs with `inter_row_gap` key

2. **Pass 1 - FacetColumn aggregates:**
   - Takes max `inter_row_gap` across all columns
   - Detects cross-dimension gap (row gap in column facet)
   - Triggers Phase 1.5

3. **Phase 1.5 - Re-measurement:**
   - Creates `FacetCoordinationContext` with `coordinated_spacing`
   - Re-runs measurement for each column
   - Inner FacetRows use `get_coordinated_spacing("inter_row_gap")`

4. **Pass 2 - Rendering:**
   - All FacetRows use the same coordinated row gap
   - Rows align across columns

**Location for gap extraction:** `avenger-chart/src/facet/marks/facet_evaluation.rs:1046-1057`

```rust
// Extract inter_row_gap from aggregated spacing_needs (computed by inner FacetRowGuide)
if let Some(&gap) = aggregated_spacing_needs
    .get(crate::guide::spacing_keys::INTER_ROW_GAP)
{
    // Use coordinated gap
}
```

## Key Functions Reference

| Function | Location | Purpose |
|----------|----------|---------|
| `calculate_inter_row_gap` | `guide_measurement.rs:22` | Compute gap between adjacent rows |
| `calculate_inter_col_gap` | `guide_measurement.rs:49` | Compute gap between adjacent columns |
| `aggregate_overflow` | `guide_measurement.rs:69` | Max-aggregate overflow measurements |
| `MeasurementResult::with_spacing` | `overflow.rs:71` | Add spacing need to result |
| `MeasurementResult::merge_spacing_needs` | `overflow.rs:77` | Merge child spacing needs |
| `FacetCoordinationContext::with_coordinated_spacing` | `coordination.rs:889` | Set coordinated spacing |
| `FacetCoordinationContext::get_coordinated_spacing` | `coordination.rs:897` | Retrieve coordinated spacing |
| `FacetDimensionConfig::inter_gap_key` | `dimension_config.rs:82` | Get dimension-specific gap key |

## Design Rationale

### Why Named Keys?

The named keys pattern provides several benefits:

1. **Extensibility:** New spacing requirements can be added without changing function signatures.

2. **Selectivity:** Parent facets can choose which spacing values to coordinate vs. ignore.

3. **Debugging:** Named keys make it easy to trace spacing values through the system.

4. **Decoupling:** Inner and outer facets don't need to know each other's implementation details.

### Why Max Aggregation?

Taking the maximum value ensures:

1. **No clipping:** The worst-case spacing is used, preventing any cell from having insufficient space.

2. **Consistency:** All cells use the same spacing, creating a clean grid appearance.

3. **Simplicity:** No complex negotiation or iteration required.

### Why Two Overflow Types?

Separating guide-only and total overflow allows:

1. **Precise alignment:** Guide elements (axes) can be aligned without legends affecting the calculation.

2. **Flexible legends:** Legends can extend beyond the aligned guide area without disrupting the grid structure.

3. **Correct gap calculation:** Inter-cell gaps are based on total content, ensuring nothing overlaps.

## See Also

- [Overview](overview.md) - Module structure and core concepts
- [Two-Pass Algorithm](two-pass-algorithm.md) - Full measurement and rendering flow
- `guides/measurement-coordination-detailed.md` - Comprehensive 750-line analysis
- `guides/facet-layout-overflow-model.md` - Visual diagrams of overflow regions
