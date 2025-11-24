# Design Plan: Align GridFacet with Standard Mark Pattern

## Executive Summary

FacetGrid currently bypasses the coordinate transform system, duplicating positioning logic that already exists in `FacetGrid.transform()`. This plan proposes refactoring `CompiledFacetGrid.evaluate_from_data()` to use the same pattern as FacetRow/FacetCol, eliminating ~200 lines of duplicate code and bringing GridFacet into full architectural alignment.

## Current State Analysis

### FacetRow/FacetColumn (✅ Aligned)

**Pattern** (from `facet_evaluation.rs:221-271`):
1. Extract positions from band scale
2. Build position_channels HashMap
3. **Call coord.transform(position_channels, ...)** → SubplotGeometry
4. Downcast and extract rects
5. **Use rect.x/rect.y for positioning** (line 591)

### FacetGrid (❌ Not Aligned)

**Current Implementation** (`facet.rs:683-1267`):
- **Ignores coord parameter** (line 688: `_coord`)
- Manually builds BandPositionIterator for rows (lines 1009-1027)
- Manually builds BandPositionIterator for cols (lines 1039-1104)
- Manually calculates positions: `x_offset = col_band_pos.start()` (line 1135)
- Manually calculates positions: `y_offset = row_band_pos.start()` (line 1136)

**Existing Transform** (`coord.rs:379-437`):
- ✅ Already implemented!
- Takes "row" and "column" position channels
- Calls compute_band_layout for both dimensions
- Creates 2D grid of SubplotRects (nested loops, lines 421-434)
- Returns SubplotGeometry with flattened rects array

**Key Discovery**: The coordinate transform for GridFacet is ALREADY CORRECT. We just need to use it!

## Root Cause

CompiledFacetGrid was likely implemented before the two-pass pattern was established for FacetRow/FacetCol. It implements the two-pass algorithm inline without using the coordinate transform.

## Proposed Solution

Refactor `CompiledFacetGrid.evaluate_from_data()` to follow the same two-pass pattern as FacetRow/FacetCol, but adapted for 2D grids.

## Detailed Design

### Phase 1: Data Structure Enhancement

**Current SubplotRect** (`coords.rs`):
```rust
pub struct SubplotRect {
    pub value: ScalarValue,  // Only stores ONE value
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}
```

**Proposed Enhancement**:
```rust
pub struct SubplotRect {
    pub value: ScalarValue,  // Primary facet value (row for GridFacet)
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,

    // NEW: Grid cell coordinates (None for FacetRow/FacetCol)
    pub row_index: Option<usize>,
    pub col_index: Option<usize>,
    pub col_value: Option<ScalarValue>,  // Column value for GridFacet
}
```

**Rationale**:
- Allows mapping between flat rects array and 2D grid structure
- Enables overflow lookup: `overflow_grid[rect.row_index.unwrap()][rect.col_index.unwrap()]`
- Preserves backward compatibility (new fields are Option<>)

### Phase 2: Update FacetGrid.transform()

**Current** (`coord.rs:421-434`):
```rust
let mut rects = Vec::with_capacity(row_starts.len() * col_starts.len());
for (row_idx, &y) in row_starts.iter().enumerate() {
    for (_col_idx, &x) in col_starts.iter().enumerate() {
        let value = row_values.get(row_idx).cloned().unwrap_or(ScalarValue::Null);
        rects.push(crate::coords::SubplotRect::new(
            value, x, y, col_bandwidth, row_bandwidth,
        ));
    }
}
```

**Proposed**:
```rust
// Extract column values
let col_values = position_values
    .and_then(|pv| pv.get("column"))
    .map(|v| v.as_slice())
    .unwrap_or(&[]);

let mut rects = Vec::with_capacity(row_starts.len() * col_starts.len());
for (row_idx, &y) in row_starts.iter().enumerate() {
    for (col_idx, &x) in col_starts.iter().enumerate() {
        let row_value = row_values.get(row_idx).cloned().unwrap_or(ScalarValue::Null);
        let col_value = col_values.get(col_idx).cloned();

        rects.push(crate::coords::SubplotRect {
            value: row_value,  // Primary value is row
            x,
            y,
            width: col_bandwidth,
            height: row_bandwidth,
            row_index: Some(row_idx),
            col_index: Some(col_idx),
            col_value,
        });
    }
}
```

### Phase 3: Refactor CompiledFacetGrid.evaluate_from_data()

Replace the current implementation (~580 lines) with the standard two-pass pattern (~250 lines).

**Structure**:

```rust
async fn evaluate_from_data(
    &self,
    _data: Option<&RecordBatch>,
    _scalars: &RecordBatch,
    context: &RenderContext,
    coord: Box<dyn CoordinateSystemTransform>,  // NOW USED!
) -> Result<(Vec<SceneMark>, LayoutUpdates), AvengerChartError> {

    // ============================================================
    // SETUP: Extract domain values and build initial scales
    // ============================================================
    let row_domain_vals = ...;
    let col_domain_vals = ...;
    let row_scale = context.scales.get("row");
    let col_scale = context.scales.get("column");

    // Build ScaleGrouping for subplot scales
    let scale_grouping = ScaleGrouping::build(...).await?;

    // ============================================================
    // PASS 1: Measure Overflow
    // ============================================================

    // Extract initial positions from scales
    let initial_row_positions = row_scale.scale_scalars_to_numeric(&row_domain_vals)?;
    let initial_col_positions = col_scale.scale_scalars_to_numeric(&col_domain_vals)?;

    // Build position channels for coord.transform()
    let mut position_channels = HashMap::new();
    position_channels.insert("row", ScalarOrArray::Array(initial_row_positions));
    position_channels.insert("column", ScalarOrArray::Array(initial_col_positions));

    let mut position_values = HashMap::new();
    position_values.insert("row", row_domain_vals.clone());
    position_values.insert("column", col_domain_vals.clone());

    // CALL TRANSFORM (Pass 1)
    let initial_geometry = coord.transform(
        &position_channels,
        Some(&position_values),
        context.plot_width,
        context.plot_height,
    )?;

    let initial_rects = initial_geometry
        .as_any()
        .downcast_ref::<SubplotGeometry>()?
        .rects.clone();

    // Measure overflow using rect.row_index/col_index to index into grid
    let mut overflow_grid = vec![vec![OverflowSpaceRequirement::default(); num_cols]; num_rows];

    for rect in &initial_rects {
        let row_idx = rect.row_index.unwrap();
        let col_idx = rect.col_index.unwrap();

        // Build subplot at (rect.x, rect.y) with (rect.width, rect.height)
        // Measure overflow
        // Store in overflow_grid[row_idx][col_idx]
    }

    // ============================================================
    // BETWEEN PASSES: Calculate Padding
    // ============================================================

    let max_vertical_gap = calculate_row_padding(&overflow_grid);
    let max_horizontal_gap = calculate_col_padding(&overflow_grid);

    // Rebuild scales with padding
    let mut updated_scales = context.scales.clone();
    // ... insert padding_inner_px into row and column scales ...

    // Update coord with measured padding
    let updated_coord = coord.with_measured_padding(/* ... */);

    // ============================================================
    // PASS 2: Final Rendering
    // ============================================================

    // Extract final positions from rebuilt scales
    let final_row_positions = updated_row_scale.scale_scalars_to_numeric(&row_domain_vals)?;
    let final_col_positions = updated_col_scale.scale_scalars_to_numeric(&col_domain_vals)?;

    // Build final position channels
    let mut final_position_channels = HashMap::new();
    final_position_channels.insert("row", ScalarOrArray::Array(final_row_positions));
    final_position_channels.insert("column", ScalarOrArray::Array(final_col_positions));

    // CALL TRANSFORM (Pass 2)
    let final_geometry = updated_coord.transform(
        &final_position_channels,
        Some(&position_values),
        context.plot_width,
        context.plot_height,
    )?;

    let final_rects = final_geometry
        .as_any()
        .downcast_ref::<SubplotGeometry>()?
        .rects.clone();

    // Render each subplot using rect coordinates
    let mut marks = Vec::new();

    for rect in final_rects {
        let row_idx = rect.row_index.unwrap();
        let col_idx = rect.col_index.unwrap();

        // Filter data for this cell
        // Build scales for this position
        // Render subplot components

        // Position scene groups at (rect.x, rect.y) - NOT manual calculation!
        let data_group = SceneGroup {
            origin: [rect.x, rect.y].into(),
            marks: components.data_marks,
            clip: components.clip,
            ...
        };
        marks.push(SceneMark::Group(data_group));
    }

    Ok((marks, LayoutUpdates::with_scales(updated_scales)))
}
```

## Benefits

### 1. Architectural Alignment
- All three facet types follow the same pattern
- Consistent use of coordinate transforms
- Easier to understand and maintain

### 2. Code Reduction
- **Current GridFacet**: ~580 lines in evaluate_from_data
- **Proposed GridFacet**: ~250 lines (estimate)
- **Savings**: ~330 lines of duplicate positioning logic

### 3. Single Source of Truth
- Positioning logic lives in `coord.transform()`
- No manual BandPositionIterator usage
- Easier to fix bugs and add features

### 4. Extensibility
- External crates can implement custom grid facet coordinates
- Transform abstraction is properly utilized
- Mark extension API is consistent

## Implementation Phases

### Phase 1: Enhance Data Structures (Low Risk)
1. Add optional fields to SubplotRect
2. Update FacetGrid.transform() to populate new fields
3. Verify FacetRow/FacetCol still work (new fields should be None)

### Phase 2: Refactor evaluate_from_data (Medium Risk)
1. Implement Pass 1 (overflow measurement) using transform
2. Implement Between Passes (padding calculation)
3. Implement Pass 2 (final rendering) using transform
4. Run all GridFacet tests to verify correctness

### Phase 3: Cleanup (Low Risk)
1. Remove BandPositionIterator usage from facet.rs
2. Remove manual positioning calculations
3. Add documentation explaining the pattern

## Testing Strategy

1. **Unit tests**: Verify SubplotRect field population
2. **Integration tests**: All existing GridFacet visual regression tests must pass
3. **Comparison tests**: Render same plot with old and new code, compare outputs
4. **Performance tests**: Ensure no regression (transform is cheap)

## Open Questions for Expert Review

1. **SubplotRect design**: Is adding optional fields the right approach, or should we create a GridSubplotRect subtype?

2. **with_measured_padding signature**: Currently takes single padding value. GridFacet needs row_padding AND col_padding. Should we:
   - Add second parameter?
   - Create a PaddingConfig struct?
   - Call it twice (once for rows, once for cols)?

3. **Overflow measurement**: Can we extract the overflow measurement loop into a shared helper function used by all three facet types?

4. **Position values handling**: For GridFacet, we have two sets of values (row and column). Is the current HashMap<&str, Vec<ScalarValue>> sufficient, or do we need a more sophisticated structure?

5. **Backward compatibility**: Any concerns about changing SubplotRect structure?

## Success Criteria

1. ✅ All existing GridFacet tests pass
2. ✅ Visual output is pixel-identical to current implementation
3. ✅ Code reduction of at least 250 lines
4. ✅ No manual BandPositionIterator usage in evaluate_from_data
5. ✅ coord.transform() is used for all positioning
6. ✅ Architecture matches FacetRow/FacetCol pattern
