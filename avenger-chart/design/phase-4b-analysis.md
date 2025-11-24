# Phase 4b Implementation Analysis

## Current State (lines 859-1400+)

The `evaluate_from_data()` method currently uses a manual two-pass approach:

### Pass 1: Measure Overflow (lines 1008-1118)
- Creates 2D overflow_grid manually
- Uses nested SubplotIterator loops for row×col iteration
- For each cell:
  - Merges row/col contexts via `merge_grid_facet_contexts()`
  - Filters data for (row_value, col_value) pair
  - Builds scales via `scale_grouping.build_scales_for_position()`
  - Measures overflow via `build_plot_components()` in Measure mode
  - Stores in `overflow_grid[row_idx][col_idx]`

### Between Passes: Calculate Padding (lines 1078-1118)
- Manually calculates max_vertical_gap (row padding)
- Manually calculates max_horizontal_gap (col padding)
- Adds configured facet_spacing to both

### Pass 2: Final Rendering (lines 1120-1400+)
- Creates new BandPositionIterators from updated scales
- Another nested loop over row×col
- For each cell:
  - Gets x_offset from col_band_pos.start()
  - Gets y_offset from row_band_pos.start()
  - Builds scales again
  - Renders via `build_plot_components()` in Render mode
  - Positions scene group at [x_offset, y_offset]

## Target State (Using coord.transform())

### Setup (KEEP - lines 870-1006)
- Validation
- Extract domain values
- Sort domains
- Get expressions
- Build ScaleGrouping

### Pass 1: Measure via coord.transform()
```rust
// Extract positions from scales
let row_positions: Vec<f32> = /* from row scale or fallback */;
let col_positions: Vec<f32> = /* from col scale or fallback */;

// Build position_channels and position_values
let mut position_channels = HashMap::new();
position_channels.insert("row", ScalarOrArray::new_array(row_positions));
position_channels.insert("column", ScalarOrArray::new_array(col_positions));

let mut position_values = HashMap::new();
position_values.insert("row", row_domain_vals.clone());
position_values.insert("column", col_domain_vals.clone());

// Call coord.transform() to get SubplotGeometry with grid fields populated
let coord = /* FacetGrid coord - need to get from somewhere? */;
let initial_geometry = coord.transform(
    &position_channels,
    Some(&position_values),
    context.plot_width,
    context.plot_height,
)?;

// Downcast to SubplotGeometry
let initial_rects = initial_geometry
    .as_any()
    .downcast_ref::<SubplotGeometry>()
    .ok_or_else(|| AvengerChartError::InternalError("Expected SubplotGeometry".into()))?
    .rects.as_slice();

// Call helper function
let overflow_grid = measure_grid_overflow(
    initial_rects,
    &self.compiled_subplot,
    &scale_grouping,
    &row_expr,
    &col_expr,
    &df,
    &context.session_context,
    &context.params,
).await?;
```

### Between Passes: Calculate Padding
```rust
let (row_padding_px, col_padding_px) = calculate_grid_padding(&overflow_grid);

let row_overflow = extract_row_overflow(&overflow_grid);
let col_overflow = extract_col_overflow(&overflow_grid);

// Update coord with measured padding
let updated_coord = coord.with_measured_padding(&crate::coords::PaddingSpec::Grid {
    row_padding_px,
    col_padding_px,
    row_overflow: row_overflow.clone(),
    col_overflow: col_overflow.clone(),
});
```

### Pass 2: Final Rendering via updated coord.transform()
```rust
// Extract UPDATED positions from rebuilt scales (with padding incorporated)
let updated_row_positions: Vec<f32> = /* from updated row scale */;
let updated_col_positions: Vec<f32> = /* from updated col scale */;

// Rebuild position_channels with updated positions
let mut final_position_channels = HashMap::new();
final_position_channels.insert("row", ScalarOrArray::new_array(updated_row_positions));
final_position_channels.insert("column", ScalarOrArray::new_array(updated_col_positions));

let mut final_position_values = HashMap::new();
final_position_values.insert("row", row_domain_vals.clone());
final_position_values.insert("column", col_domain_vals.clone());

// Get final geometry from updated coord
let final_geometry = updated_coord.transform(
    &final_position_channels,
    Some(&final_position_values),
    context.plot_width,
    context.plot_height,
)?;

let final_rects = final_geometry
    .as_any()
    .downcast_ref::<SubplotGeometry>()
    .ok_or_else(|| AvengerChartError::InternalError("Expected SubplotGeometry".into()))?
    .rects.as_slice();

// Render each cell using rect.x, rect.y (NOT manual calculation!)
let mut all_marks = Vec::new();
for rect in final_rects {
    let row_idx = rect.row_index.expect("Missing row_index");
    let col_idx = rect.col_index.expect("Missing col_index");
    let row_value = &rect.value;
    let col_value = rect.col_value.as_ref().expect("Missing col_value");

    // Build subplot context (need merge_grid_facet_contexts equivalent)
    // Filter data
    // Build scales
    // Render
    // Position at [rect.x, rect.y]
}
```

## Critical Questions

### Q1: Where does the initial `coord` come from?
**Answer**: It's passed as parameter `_coord` (line 865), but currently unused. The design says to use it!

### Q2: How to extract positions from scales?
**Looking at facet_evaluation.rs:187-195**:
```rust
// Extract scaled positions from the facet dimension scale
let initial_positions: Vec<f32> = dimension_scale
    .apply_to_scalar_vec_f32(initial_domain_vals.as_slice())?
    .into_iter()
    .collect();
```

**BUT**: GridFacet doesn't have a single dimension_scale, it has TWO scales (row and col).

Need to look at how GridFacet builds its scales... checking context.scales.get("row") and context.scales.get("column")

### Q3: How to rebuild scales with padding?
**Looking at facet_evaluation.rs:353-400**: Shows pattern for FacetRow/Col where they:
1. Clone the scale config
2. Insert padding_inner_px into options
3. Rebuild the scale
4. Extract new positions

For GridFacet, need to rebuild BOTH row and col scales with their respective paddings.

### Q4: What about merge_grid_facet_contexts()?
Still needed for building subplot-specific params. The refactoring doesn't eliminate this - it just moves WHEN we call it.

Currently: Called in both Pass 1 and Pass 2 loops
After: Called only in Pass 2 loop (Pass 1 is handled by measure_grid_overflow helper)

### Q5: SubplotIterator still needed?
Looking at the code... SubplotIterator provides FacetContext for parameter substitution.

For rendering (Pass 2), we still need to:
- Iterate over cells
- Build merged params per cell
- Filter data per cell
- Build scales per cell
- Render per cell

The difference: Instead of using BandPositionIterator.start() for positioning, we use rect.x and rect.y.

**Insight**: We DON'T need SubplotIterator in the refactored version! The rects from coord.transform() already contain:
- row_value, col_value (for filtering)
- row_index, col_index (for scale building)
- x, y (for positioning)

We can iterate directly over final_rects!

## Implementation Plan

### Step 1: Use the passed-in coord parameter
Change line 865 from `_coord` to `coord`.

### Step 2: Extract initial positions from scales
```rust
// After line 1006 (after ScaleGrouping is built)

// Extract initial positions from scales (or use fallback for degenerate cases)
let initial_row_positions: Vec<f32> = if let Some(row_scale) = row_scale_opt {
    row_scale.apply_to_scalar_vec_f32(&row_domain_vals)?
} else {
    // Fallback: evenly distribute across plot height
    (0..num_rows).map(|i| (i as f32) * band_h).collect()
};

let initial_col_positions: Vec<f32> = if let Some(col_scale) = col_scale_opt {
    col_scale.apply_to_scalar_vec_f32(&col_domain_vals)?
} else {
    // Fallback: evenly distribute across plot width
    (0..num_cols).map(|i| (i as f32) * band_w).collect()
};
```

### Step 3: Build position_channels and call coord.transform()
```rust
use std::collections::HashMap;

let mut position_channels_pass1 = HashMap::new();
position_channels_pass1.insert(
    "row",
    avenger_common::value::ScalarOrArray::new_array(initial_row_positions),
);
position_channels_pass1.insert(
    "column",
    avenger_common::value::ScalarOrArray::new_array(initial_col_positions),
);

let mut position_values_pass1 = HashMap::new();
position_values_pass1.insert("row", row_domain_vals.clone());
position_values_pass1.insert("column", col_domain_vals.clone());

let initial_geometry = coord.transform(
    &position_channels_pass1,
    Some(&position_values_pass1),
    context.plot_width,
    context.plot_height,
)?;

let initial_rects = initial_geometry
    .as_any()
    .downcast_ref::<crate::coords::SubplotGeometry>()
    .ok_or_else(|| {
        AvengerChartError::InternalError(
            "Expected SubplotGeometry from GridFacet coord transform".into(),
        )
    })?
    .rects
    .as_slice();
```

### Step 4: Call measure_grid_overflow
REPLACE lines 1008-1118 with:
```rust
let overflow_grid = measure_grid_overflow(
    initial_rects,
    &self.compiled_subplot,
    &scale_grouping,
    &row_expr,
    &col_expr,
    &df,
    &context.session_context,
    &context.params,
)
.await?;
```

### Step 5: Calculate padding and update coord
REPLACE lines 1078-1118 with:
```rust
let (row_padding_px, col_padding_px) = calculate_grid_padding(&overflow_grid);

// Add configured facet_spacing
const DEFAULT_FACET_SPACING: f32 = 3.0;
let spacing = self.facet_spacing.unwrap_or_else(|| {
    let facet_ctx = context
        .theme
        .facet_context_with_params(context.params.clone());
    context
        .theme
        .query(&facet_ctx, "spacing")
        .and_then(|v| v.as_number())
        .map(|n| n as f32)
        .unwrap_or(DEFAULT_FACET_SPACING)
});

let row_padding_px = row_padding_px + spacing;
let col_padding_px = col_padding_px + spacing;

let row_overflow = extract_row_overflow(&overflow_grid);
let col_overflow = extract_col_overflow(&overflow_grid);

let updated_coord = coord.with_measured_padding(&crate::coords::PaddingSpec::Grid {
    row_padding_px,
    col_padding_px,
    row_overflow: row_overflow.clone(),
    col_overflow: col_overflow.clone(),
});
```

### Step 6: Rebuild scales with padding
This is the TRICKY part. Need to follow facet_evaluation.rs pattern but for TWO scales.

Looking at existing code around line 1120+, I see it rebuilds updated_scales...

**KEY INSIGHT**: The existing code around lines 1120-1200 already rebuilds the scales! We just need to use those updated scales to extract new positions.

### Step 7: Extract final positions and call updated_coord.transform()
```rust
// Extract final positions from updated scales
let final_row_positions: Vec<f32> = updated_scales
    .get("row")
    .ok_or_else(|| AvengerChartError::InternalError("Missing row scale".into()))?
    .apply_to_scalar_vec_f32(&row_domain_vals)?;

let final_col_positions: Vec<f32> = updated_scales
    .get("column")
    .ok_or_else(|| AvengerChartError::InternalError("Missing col scale".into()))?
    .apply_to_scalar_vec_f32(&col_domain_vals)?;

let mut final_position_channels = HashMap::new();
final_position_channels.insert(
    "row",
    avenger_common::value::ScalarOrArray::new_array(final_row_positions),
);
final_position_channels.insert(
    "column",
    avenger_common::value::ScalarOrArray::new_array(final_col_positions),
);

let mut final_position_values = HashMap::new();
final_position_values.insert("row", row_domain_vals.clone());
final_position_values.insert("column", col_domain_vals.clone());

let final_geometry = updated_coord.transform(
    &final_position_channels,
    Some(&final_position_values),
    context.plot_width,
    context.plot_height,
)?;

let final_rects = final_geometry
    .as_any()
    .downcast_ref::<crate::coords::SubplotGeometry>()
    .ok_or_else(|| {
        AvengerChartError::InternalError(
            "Expected SubplotGeometry from GridFacet coord transform".into(),
        )
    })?
    .rects
    .as_slice();
```

### Step 8: Render loop using rect.x, rect.y
REPLACE the entire nested BandPositionIterator loop (lines 1250-1400+) with:
```rust
let mut all_marks = Vec::new();

for rect in final_rects {
    let row_idx = rect.row_index.expect("SubplotRect missing row_index");
    let col_idx = rect.col_index.expect("SubplotRect missing col_index");
    let row_value = &rect.value;
    let col_value = rect.col_value.as_ref().expect("SubplotRect missing col_value");

    // Build merged params (still needed for parameter substitution)
    // Need to construct equivalent of row_iteration and col_iteration
    // Actually... we don't have SubplotIteration structs anymore!

    // PROBLEM: merge_grid_facet_contexts takes SubplotIteration<RowDimensionConfig>
    // and SubplotIteration<ColumnDimensionConfig>
    //
    // But we're not using SubplotIterator anymore!
    //
    // SOLUTION: We need to build the FacetContext directly, or refactor merge_grid_facet_contexts
}
```

**BLOCKER IDENTIFIED**: `merge_grid_facet_contexts()` takes SubplotIteration structs which come from SubplotIterator.

Need to either:
A. Still use SubplotIterator for iteration (but ignore its .start() positioning)
B. Refactor merge_grid_facet_contexts to work with row_idx/col_idx directly
C. Build FacetContext structs manually

## Recommendation

Option A is simplest for this refactoring:
- Keep using SubplotIterator for FacetContext management
- Zip it with final_rects to get both context and positioning
- Use rect.x/rect.y for positioning instead of BandPositionIterator.start()

This minimizes changes to merge_grid_facet_contexts and other helper functions.

## Revised Step 8

```rust
let mut all_marks = Vec::new();

// Create subplot iterators for context management (NOT for positioning!)
let row_iter = SubplotIterator::<RowDimensionConfig>::new(
    row_domain_vals.clone(),
    context.params.clone(),
    scale_sharing_by_channel.clone(),
);

// Zip row iterator with rects (grouped by row)
for (row_iteration, row_rects) in row_iter.zip(final_rects.chunks(num_cols)) {
    let col_iter = SubplotIterator::<ColumnDimensionConfig>::new(
        col_domain_vals.clone(),
        context.params.clone(),
        scale_sharing_by_channel.clone(),
    );

    for (col_iteration, rect) in col_iter.zip(row_rects) {
        let merged_params = merge_grid_facet_contexts(&row_iteration, &col_iteration, num_rows, num_cols);

        // Filter data
        let filter_df = df
            .clone()
            .filter(row_expr.clone().eq(lit(row_iteration.facet_value.clone())))?
            .filter(col_expr.clone().eq(lit(col_iteration.facet_value.clone())))?;

        // Build scales
        let inner_scales = scale_grouping
            .build_scales_for_position(
                &self.compiled_subplot,
                row_iteration.index,
                col_iteration.index,
                rect.width,
                rect.height,
                &context.session_context,
                &merged_params,
            )
            .await?;

        // Render (existing code pattern)
        let scale_provider = crate::plot::compiled::scale_provider::PrebuiltScaleProvider {
            scales: inner_scales.clone(),
        };

        let components = self
            .compiled_subplot
            .build_plot_components(
                rect.width,
                rect.height,
                &context.session_context,
                &merged_params,
                &scale_provider,
                crate::plot::compiled::EvaluationMode::Render,
                Some(&filter_df),
                true,
            )
            .await?;

        // CRITICAL CHANGE: Use rect.x, rect.y instead of band_pos.start()
        let x_offset = rect.x;
        let y_offset = rect.y;

        // Position scene group (existing pattern)
        // ... rest of rendering code ...
    }
}
```

## Summary

The refactoring is actually quite manageable:

1. Use the passed coord parameter
2. Extract positions from scales
3. Call coord.transform() twice (Pass 1 and Pass 2)
4. Call helper functions for overflow measurement and padding calculation
5. Keep SubplotIterator for FacetContext management
6. Zip iterators with rects to get both context and positioning
7. Use rect.x/rect.y for positioning instead of BandPositionIterator.start()

The main complexity is juggling the two different data sources (SubplotIterator for context, rects for positioning).

## Agent Review Feedback

### GPT-5 Codex Review

**1. Position Extraction Logic**
- ✅ APPROVED with refinements
- Use `scale.scale_scalars_to_numeric(&domain_vals)` or `BandPositionIterator::from_scale` instead of `apply_to_scalar_vec_f32` (more proven pattern)
- Add assertion: `assert_eq!(row_positions.len(), row_domain_vals.len())` before transform
- Fallback for missing scale: Use `vec![0.0; num_rows]` (single value at origin) instead of `(i as f32) * band_h` to avoid double-shrinking when padding is applied later

**2. Zipping Pattern**
- ✅ WORKS with strong guardrails
- Add before zipping: `assert_eq!(final_rects.len(), num_rows * num_cols)`
- Add inside loops: `assert_eq!(row_iteration.index, rect.row_index.unwrap())` and `assert_eq!(col_iteration.index, rect.col_index.unwrap())`
- **Better alternative**: Prebuild `Vec<RowIteration>` and `Vec<ColIteration>`, then index by `rect.row_index/col_index` instead of relying on row-major zipping order

**3. Degenerate Grids**
- Zero `row_padding_px` when `num_rows == 1` to avoid shrinking single row
- Zero `col_padding_px` when `num_cols == 1` to avoid shrinking single column
- Keep overflow vectors even with zero padding (needed for guides)
- Ensure `merge_grid_facet_contexts` receives correct `grid_dimensions = (1, N)` or `(N, 1)`

**4. Scale Rebuilding**
- ✅ YES, must rebuild both row and col scales with `padding_inner_px`
- `coord.with_measured_padding` only adjusts geometry, NOT the scales
- When scale is absent (single value), skip rebuilding that axis and use fallback positions

**5. Iterator Count Matching**
- Add: `assert_eq!(row_iter.len(), num_rows)` before Pass 2
- Add: `assert_eq!(col_iter.len(), num_cols)` before Pass 2
- After transform: `assert_eq!(final_rects.len(), num_rows * num_cols)`
- Verify every rect has `row_index`/`col_index` within bounds

### Gemini-3 Pro Review

**Overall Verdict**: ✅ APPROVED - Solid, well-structured plan

**1. Architecture**
- ✅ Two-pass `coord.transform()` is the correct pattern
- Aligns GridFacet with CoordinateSystem trait contract
- Pass 1 (Measure) → Calculate padding → Pass 2 (Final) is standard visualization engine cycle

**2. Code Complexity**
- ✅ Net simplification
- **Removes**: Manual band arithmetic, BandPositionIterator logic, manual 2D grid spacing
- **Adds**: Setup code (extracting positions, building channel maps) - worthwhile trade-off

**3. Data Flow & Consistency**
- ✅ Valid with CRITICAL ordering requirement
- **Must ensure**: `row_positions` vector comes from **exact same sorted `row_domain_vals`** that SubplotIterator uses
- This link ensures `rects[i]` corresponds to `iteration[i]`

**4. Helper Functions**
- ✅ Logical and necessary
- `measure_grid_overflow`: Essential for readability
- `calculate_grid_padding`: Isolates "max adjacent gap" logic

**5. Warnings**
- Handle empty `final_rects` to avoid panics in `.chunks()`
- Strict ordering is critical - any deviation breaks the zipping contract

## Refined Implementation Plan

Based on agent feedback, here are the key refinements to the original plan:

### Step 2 Refinement: Position Extraction
```rust
// Use proven pattern from facet_evaluation.rs
let initial_row_positions: Vec<f32> = if let Some(row_scale) = row_scale_opt {
    row_scale.scale_scalars_to_numeric(&row_domain_vals)?
} else {
    // Fallback: single value at origin
    vec![0.0; row_domain_vals.len()]
};

let initial_col_positions: Vec<f32> = if let Some(col_scale) = col_scale_opt {
    col_scale.scale_scalars_to_numeric(&col_domain_vals)?
} else {
    vec![0.0; col_domain_vals.len()]
};

// Critical: Verify lengths match
assert_eq!(initial_row_positions.len(), row_domain_vals.len());
assert_eq!(initial_col_positions.len(), col_domain_vals.len());
```

### Step 5 Refinement: Degenerate Grid Handling
```rust
let (row_padding_px, col_padding_px) = calculate_grid_padding(&overflow_grid);

// Zero padding for single-dimension axes to avoid shrinking
let row_padding_px = if num_rows == 1 { 0.0 } else { row_padding_px + spacing };
let col_padding_px = if num_cols == 1 { 0.0 } else { col_padding_px + spacing };
```

### Step 8 Alternative: Index-Based Lookup (More Robust)
```rust
// Prebuild iteration vectors for index-based lookup
let row_iters: Vec<_> = SubplotIterator::<RowDimensionConfig>::new(
    row_domain_vals.clone(),
    context.params.clone(),
    scale_sharing_by_channel.clone(),
).collect();

let col_iters: Vec<_> = SubplotIterator::<ColumnDimensionConfig>::new(
    col_domain_vals.clone(),
    context.params.clone(),
    scale_sharing_by_channel.clone(),
).collect();

// Verify counts
assert_eq!(row_iters.len(), num_rows);
assert_eq!(col_iters.len(), num_cols);
assert_eq!(final_rects.len(), num_rows * num_cols);

let mut all_marks = Vec::new();

for rect in final_rects {
    let row_idx = rect.row_index.expect("SubplotRect missing row_index");
    let col_idx = rect.col_index.expect("SubplotRect missing col_index");

    // Index-based lookup - no reliance on ordering!
    let row_iteration = &row_iters[row_idx];
    let col_iteration = &col_iters[col_idx];

    // Double-check (can be debug_assert in production)
    assert_eq!(row_iteration.index, row_idx);
    assert_eq!(col_iteration.index, col_idx);

    // ... rest of rendering logic using rect.x, rect.y for positioning
}
```

## Open Questions for Other Agents

1. ~~Is the position extraction logic correct? (Step 2)~~ ✅ ANSWERED
2. ~~Is zipping SubplotIterator with rects the right approach? (Step 8)~~ ✅ ANSWERED - index-based lookup is better
3. ~~Any edge cases with degenerate grids (1×N or N×1)?~~ ✅ ANSWERED - zero padding for single-dimension axes
4. ~~How to handle the case where row or col scale doesn't exist?~~ ✅ ANSWERED - use `vec![0.0; len]`
5. ~~Should we verify that SubplotIterator produces exactly num_rows iterations?~~ ✅ ANSWERED - add assertions
