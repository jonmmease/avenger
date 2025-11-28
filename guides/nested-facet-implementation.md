# Nested Facet Implementation Deep Dive

## Overview

Nested facets allow FacetRow or FacetColumn marks to contain subplots that are themselves facets (FacetColumn or FacetRow). This creates hierarchical faceting structures like:
- **FacetColumn with nested FacetRow**: A column of rows (facet-of-facets)
- **FacetRow with nested FacetColumn**: A row of columns

Example from test_nested_facets.rs:
```
Outer: FacetColumn by species (3 columns)
  └─ Middle: FacetRow by petal_width_bin (variable rows per column)
    └─ Inner: Cartesian with Symbol marks (scatter plot)
```

## How Nesting Currently Works

### 1. Data Flow Through Nested Levels

**Key mechanism**: The `data_override` parameter

- **Outer facet** has explicit `.data(df)` attachment
- **Middle facet** (inner facet mark) has NO `.data()` call - receives filtered data via `data_override`
- **Inner mark** (Cartesian) has NO `.data()` call - receives doubly-filtered data via `data_override`

This is enforced in `facet.rs` (lines 153-161):
```rust
// Validate that subplot doesn't have its own data
if plot_ref.data.is_some() {
    return Err(AvengerChartError::InvalidArgument(
        "Nested facet plots should not have their own data attached. \
         Data flows from the parent facet to child plots."
    ));
}
```

**Flow**:
1. Outer facet receives full iris dataset
2. For species="setosa", filters to setosa rows, passes via `data_override` to middle facet
3. Middle facet receives setosa rows, filters by petal_width, passes filtered subset to Cartesian plot
4. Cartesian plot renders with the doubly-filtered data (setosa + narrow/medium/wide)

### 2. Two-Pass Evaluation Algorithm

Both FacetRow and FacetCol use identical two-pass algorithm parameterized by `FacetDimensionConfig`:

**Pass 1 (Measurement)**: Determine required spacing
```rust
pub async fn evaluate_facet<DimConfig: FacetDimensionConfig>(...) {
    // Extract facet dimension scale (row or col)
    // Extract domain values (e.g., ["setosa", "versicolor", "virginica"])
    // For each subplot:
    //   - Filter data by facet value
    //   - Measure subplot guide overflow
    // Compute max required spacing (facet adjacent overflow + theme spacing)
    // Rebuild facet dimension scale with measured padding
}
```

**Pass 2 (Rendering)**: Render with correct spacing
```rust
// Rebuild shared scales with final band size (critical for data alignment)
// Position and render each subplot with correct spacing
// Facet guides (labels/titles) rendered separately by guide system
```

From `facet_evaluation.rs` measure_pass (lines 113-440):
- Concurrent measurement of subplots (bounded to 4 concurrent to avoid overwhelming DataFusion)
- Collects both `guide_only` and `total_overflow` for each subplot
- Computes max overflow using dimension-specific rules

### 3. Scale Sharing Modes

Defined in `channel/config_traits.rs`:
```rust
pub enum ScaleSharing {
    Shared,           // One domain for all facets
    Free,             // Independent per facet
    SharedInRow,      // Share within each row (across columns)
    SharedInColumn,   // Share within each column (across rows)
}
```

**Normalization for nested facets** (lines 887-902 in facet_evaluation.rs):
```rust
scale_sharing_by_channel.into_iter()
    .map(|(ch, mode)| {
        let normalized = match mode {
            ScaleSharing::SharedInColumn if DimConfig::is_row_facet() => ScaleSharing::Shared,
            ScaleSharing::SharedInRow if DimConfig::is_col_facet() => ScaleSharing::Shared,
            other => other,
        };
        (ch, normalized)
    })
    .collect();
```

**Why this normalization?**
- FacetRow is 1-column, so SharedInColumn becomes Shared (only one column exists)
- FacetColumn is 1-row, so SharedInRow becomes Shared (only one row exists)
- This simplifies the logic for single-dimension faceting

### 4. FacetContext: Position and Visibility Information

Passed through `params` to all subplots for axis/guide logic.

Key fields:
```rust
pub struct FacetContext {
    position: (row_idx, col_idx),          // Subplot position in grid
    grid_dimensions: (num_rows, num_cols),  // Total grid size
    unified_channels: HashSet<String>,      // Channels shown by facet guide (e.g., {"y"} for row)
    scale_sharing: HashMap<String, ScaleSharing>,  // Per-channel sharing mode
}
```

**For nested facets**, the SubplotIterator merges parent and child contexts (lines 112-120 in subprocess_iterator.rs):
```rust
let unified_channels = if let Some(parent_ctx) = FacetContext::from_params(&self.base_params) {
    // Merge parent's unified_channels with this dimension's
    let mut merged = parent_ctx.unified_channels;
    merged.extend(DimConfig::unified_channels());
    merged
} else {
    DimConfig::unified_channels()
};
```

This ensures nested facets accumulate unified channels from all parent levels.

### 5. Overflow Aggregation Rules

**FacetRowGuide** (lines 173-182 in guide.rs):
- **Left**: first subplot's left (leftmost edge)
- **Right**: last subplot's right (rightmost edge)
- **Top/Bottom**: max across all subplots

**FacetColGuide** (analogous):
- **Top**: first subplot's top (topmost edge)
- **Bottom**: last subplot's bottom (bottommost edge)
- **Left/Right**: max across all subplots

**Why edge-specific?**
- Only edge subplots' overflow extends beyond the facet boundary
- Interior subplots' overflow is internal spacing

### 6. Coordinate Transform for Nested Facets

**FacetRow** uses position-specific closures:
```rust
// Height varies (band_height), width is fixed (plot_width)
|band_height: f32, ctx: &RenderContext| (ctx.plot_width, rounded)

// Translate vertically
|y_pos: f32| [0.0, rounded_y]
```

**FacetColumn** uses complementary closures:
```rust
// Width varies (band_width), height is fixed (plot_height)
|band_width: f32, ctx: &RenderContext| (rounded, ctx.plot_height)

// Translate horizontally
|x_pos: f32| [rounded_x, 0.0]
```

These closures abstract row vs column orientation from the shared `evaluate_facet` algorithm.

## Coordination Mechanisms Between Nested Levels

### 1. Data Filtering

The outer facet's render pass filters data by facet value and passes via `data_override`:
```rust
let filter_df = df.clone().filter(facet_expr.eq(lit(iteration.facet_value.clone())))?;
// Pass filter_df to inner plot's render
```

The inner facet receives this filtered subset and further filters by its own facet value.

### 2. Scale Building

**Shared scales** are built from the parent's filtered data:
- Outer facet builds scales from full dataset
- If inner facet has SharedInRow/SharedInColumn → normalization to Shared
- Inner facet's scales built from outer facet's filtered data

This ensures axes align correctly across nested levels.

### 3. Overflow Measurement

**For nested facets**, guide.rs `compute_max_subplot_overflow` (lines 39-326) handles three cases:

1. **With data_override** (line 74): Measuring inner facet's Cartesian subplots
   - Filter data_override by inner facet values
   - Measure each inner subplot
   - Return aggregated overflow

2. **With pre-computed overflow parameter** (line 193): Top-level facet
   - Use overflow measurements from rendering pipeline

3. **With source data** (line 219): Computing nested facet overflow
   - Filter source data by outer facet values
   - Measure inner facet's subplots
   - Return aggregated overflow

**Key insight**: When a nested facet's subplots are Cartesian (not facets), the guide fallback returns `(30, 30, 40, 20)` if no facet expression is available. This is conservative but prevents crashes.

### 4. FacetContext Propagation

Each nesting level adds its own FacetContext information:
- Level 1 (outer FacetColumn): position=(0, col_idx), grid=(1, 3)
- Level 2 (inner FacetRow): position=(row_idx, 0), grid=(num_rows, 1)
- Merged: unified_channels = {"x"} ∪ {"y"} = {"x", "y"}

The Cartesian subplots see both parent's and inner facet's context, suppressing both x and y titles.

## Current Limitations and Gaps

### 1. Guide Overflow Fallback is Conservative

When nested facet subplots don't provide facet expressions, guide.rs returns hardcoded `(30, 30, 40, 20)` (line 191, 311, 322).

**Problem**: This doesn't reflect actual subplot overflow, especially for Cartesian subplots with legends or other overflow-generating elements.

**Current workaround**: Works for simple Cartesian subplots but breaks with:
- Cartesian subplots with legends
- Cartesian subplots with large axis labels
- Multiple nesting levels

### 2. Scale Sharing Normalization May Be Too Aggressive

For nested FacetRow:
- If mark declares `SharedInColumn` for x-axis
- Gets normalized to `Shared` because FacetRow is single-column
- Inner facets don't see the distinction

**Consequence**: All FacetRows behave identically for SharedInColumn/SharedInRow, losing potential expressiveness.

### 3. Limited Testing of Deep Nesting

Current test (test_nested_facets.rs) only covers:
- 2 levels: FacetColumn → FacetRow
- Cartesian leaf (not 3+ levels deep)

No tests for:
- FacetRow → FacetColumn
- 3+ nesting levels
- Grid facets nested inside other facets

### 4. Overflow Measurement Performance

Nested facet guides measure subplots synchronously during `measure_overflow`, which is called during guide measurement phase. This could be slow for deep nesting + many subplots.

**For comparison**: `measure_pass` in facet_evaluation.rs bounds concurrency to 4 to avoid overwhelming DataFusion. The guide measurement doesn't have this protection.

### 5. No Explicit Coordination Between Parent and Child Overflow

When outer facet's guide measures overflow:
1. It measures each subplot (which might be a facet)
2. Inner facets measure their own subplots in the same call
3. But there's no feedback loop - outer facet doesn't adjust its spacing based on inner facet's actual needs

This works because:
- Inner facets return their total overflow (including labels/titles)
- Outer facet uses this total as its base and adds its own content on top

But it's implicit and fragile if changes break the feedback.

### 6. Data Override Path in Guide Measurement

The guide's `compute_max_subplot_overflow` has three distinct code paths:
- data_override path (nested facet case)
- overflow parameter path (top-level case)
- source data path (outer facet with no override)

These paths have subtle differences in:
- How they filter data
- How they handle missing facet expressions
- What fallbacks they use

This suggests potential for refactoring to unify the logic.

### 7. FacetGrid Not Yet Nested

FacetGrid (2D grid) is implemented but cannot be nested inside other facets. The nesting infrastructure (data_override, FacetContext propagation) works for FacetGrid, but there's no test validating it.

## Implementation Files and Responsibilities

| File | Responsibility |
|------|-----------------|
| `facet/marks/facet.rs` | CompiledFacetRow/Col definitions, Mark<FacetRow/Col> impl, data_override validation |
| `facet/marks/facet_evaluation.rs` | Two-pass evaluate_facet algorithm, measure_pass, render_pass |
| `facet/marks/facet.rs` (lines 235-267) | FacetRow orientation closures (height varies, translate y) |
| `facet/marks/facet.rs` (lines 430-462) | FacetCol orientation closures (width varies, translate x) |
| `facet/guide.rs` | FacetRowGuide/ColGuide implementation, overflow measurement (including nested cases) |
| `facet/dimension_config.rs` | Trait defining row vs col abstraction (channel, position, overflow calc) |
| `facet/subplot_iterator.rs` | Iterator ensuring each subplot gets correct FacetContext with parent merging |
| `facet/context.rs` | FacetContext definition and helper methods (should_show_title, should_show_labels) |
| `facet/scale_grouping.rs` | Scale grouping by sharing mode (used by grid facet, not directly by row/col) |

## Key Insights

1. **Parameterization via DimConfig**: The code avoids duplication by using trait-based parameterization (RowDimensionConfig vs ColumnDimensionConfig). The same algorithm works for both by providing dimension-specific closures and trait methods.

2. **Two-pass rendering model**: Pass 1 measures guides to determine spacing. Pass 2 renders with corrected band sizes. This is essential for nested facets because inner facet's overflow must be known before outer facet can position it.

3. **FacetContext merging**: Nested facets accumulate context from all parent levels. This ensures visibility logic (should_show_title) correctly suppresses axes at multiple nesting levels.

4. **Data filtering is explicit**: Unlike grid facets which use complex scale grouping, row/col facets filter data explicitly for each subplot. Nested facets simply pass filtered data via data_override.

5. **Overflow is the coordination point**: Between outer and inner facets, overflow measurements are the primary coupling. Inner facet returns its total overflow (including labels), outer facet uses this as its base.
