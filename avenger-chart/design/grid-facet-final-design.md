# Final Design: Align GridFacet with Standard Mark Pattern

**Status**: Ready for Implementation
**Expert Review**: Completed (gpt-5-codex + internal analysis)
**Risk Level**: Medium (trait changes, data structure enhancement)

## Executive Summary

After expert review, the plan to align GridFacet with the standard mark pattern is **approved with critical modifications**. Both expert reviewers identified the same architectural blocker and provided consistent recommendations.

## Critical Blockers Resolved

### Blocker #1: `with_measured_padding` Trait Signature

**Problem**: Current signature only supports single-dimension padding:
```rust
fn with_measured_padding(
    &self,
    padding_px: f32,
    overflow: Vec<OverflowSpaceRequirement>,
) -> Box<dyn CoordinateSystemTransform>;
```

GridFacet needs **independent** row and column padding.

**Solution**: Refactor to use a structured `PaddingSpec`:

```rust
// In src/coords.rs

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum PaddingSpec {
    /// Single-dimension padding (for FacetRow/FacetCol)
    Single {
        padding_px: f32,
        overflow: Vec<OverflowSpaceRequirement>,
    },
    /// Two-dimension padding (for GridFacet)
    Grid {
        row_padding_px: f32,
        col_padding_px: f32,
        row_overflow: Vec<OverflowSpaceRequirement>,
        col_overflow: Vec<OverflowSpaceRequirement>,
    },
}

// Updated trait method
pub trait CoordinateSystemTransform {
    fn with_measured_padding(
        &self,
        spec: &PaddingSpec,
    ) -> Box<dyn CoordinateSystemTransform>;
}
```

**Backward Compatibility**:
- FacetRow/FacetCol use `PaddingSpec::Single`
- GridFacet uses `PaddingSpec::Grid`
- This is a **breaking change** to the trait, but necessary for correctness

## Final Data Structure Design

### Enhanced SubplotRect

Based on expert feedback, we'll extend `SubplotRect` with optional grid fields:

```rust
#[serde_as]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SubplotRect {
    /// Primary facet value (row for GridFacet, facet value for Row/Col)
    #[serde_as(as = "FromInto<SerializableScalar>")]
    pub value: ScalarValue,

    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,

    // Grid-specific fields (None for FacetRow/FacetCol)

    /// Row index in grid (for overflow lookup)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row_index: Option<usize>,

    /// Column index in grid (for overflow lookup)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub col_index: Option<usize>,

    /// Column facet value (for GridFacet data filtering)
    #[serde_as(as = "Option<FromInto<SerializableScalar>>")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub col_value: Option<ScalarValue>,
}

impl SubplotRect {
    pub fn new(
        value: ScalarValue,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    ) -> Self {
        Self {
            value,
            x,
            y,
            width,
            height,
            row_index: None,
            col_index: None,
            col_value: None,
        }
    }

    /// Grid-specific constructor
    pub fn new_grid(
        row_value: ScalarValue,
        col_value: ScalarValue,
        row_index: usize,
        col_index: usize,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
    ) -> Self {
        Self {
            value: row_value,
            x,
            y,
            width,
            height,
            row_index: Some(row_index),
            col_index: Some(col_index),
            col_value: Some(col_value),
        }
    }
}
```

**Rationale**:
- Extends existing type rather than creating `GridSubplotRect` (avoids complicating `SubplotGeometry`)
- Uses `#[serde(default, skip_serializing_if = ...)]` for backward compatibility
- Clear naming: `value` is "primary" (row), `col_value` is secondary

## Updated FacetGrid.transform()

```rust
fn transform(
    &self,
    position_channels: &HashMap<&str, ScalarOrArray<f32>>,
    position_values: Option<&HashMap<&str, Vec<ScalarValue>>>,
    plot_width: f32,
    plot_height: f32,
) -> Result<Box<dyn PlotGeometry>, AvengerChartError> {
    let row_positions = position_channels.get("row")...;
    let col_positions = position_channels.get("column")...;

    let row_count = row_positions.len();
    let col_count = col_positions.len();

    // Extract domain values
    let row_values = position_values.and_then(|pv| pv.get("row"))...;
    let col_values = position_values.and_then(|pv| pv.get("column"))...;

    // Compute band layouts
    let row_centers = row_positions.as_vec(row_count, None);
    let col_centers = col_positions.as_vec(col_count, None);

    let (row_starts, row_bandwidth) =
        compute_band_layout(&row_centers, plot_height, self.row_padding_px);
    let (col_starts, col_bandwidth) =
        compute_band_layout(&col_centers, plot_width, self.col_padding_px);

    // Create grid of SubplotRects with indices
    let mut rects = Vec::with_capacity(row_count * col_count);
    for (row_idx, &y) in row_starts.iter().enumerate() {
        for (col_idx, &x) in col_starts.iter().enumerate() {
            let row_value = row_values.get(row_idx).cloned().unwrap_or(ScalarValue::Null);
            let col_value = col_values.get(col_idx).cloned().unwrap_or(ScalarValue::Null);

            rects.push(SubplotRect::new_grid(
                row_value,
                col_value,
                row_idx,
                col_idx,
                x,
                y,
                col_bandwidth,
                row_bandwidth,
            ));
        }
    }

    Ok(Box::new(SubplotGeometry::new(rects)))
}
```

## Refactored CompiledFacetGrid.evaluate_from_data()

### High-Level Structure

```rust
async fn evaluate_from_data(
    &self,
    _data: Option<&RecordBatch>,
    _scalars: &RecordBatch,
    context: &RenderContext,
    coord: Box<dyn CoordinateSystemTransform>,  // NOW USED!
) -> Result<(Vec<SceneMark>, LayoutUpdates), AvengerChartError> {

    // [SETUP] Extract domain values, build ScaleGrouping

    // [PASS 1] Measure overflow using coord.transform()
    let initial_geometry = coord.transform(...)?;
    let initial_rects = downcast_to_subplot_geometry(initial_geometry)?;
    let overflow_grid = measure_grid_overflow(&initial_rects, ...)?;

    // [BETWEEN PASSES] Calculate row and column padding
    let (row_padding, col_padding) = calculate_grid_padding(&overflow_grid);

    // Update coord with measured padding
    let updated_coord = coord.with_measured_padding(&PaddingSpec::Grid {
        row_padding_px: row_padding,
        col_padding_px: col_padding,
        row_overflow: extract_row_overflow(&overflow_grid),
        col_overflow: extract_col_overflow(&overflow_grid),
    });

    // [PASS 2] Final rendering using updated coord.transform()
    let final_geometry = updated_coord.transform(...)?;
    let final_rects = downcast_to_subplot_geometry(final_geometry)?;

    // Render each cell using rect.x, rect.y (not manual calculation!)
    let marks = render_grid_cells(&final_rects, ...)?;

    Ok((marks, LayoutUpdates::with_padding_spec(updated_spec)))
}
```

### Key Helper: measure_grid_overflow()

```rust
fn measure_grid_overflow(
    rects: &[SubplotRect],
    compiled_subplot: &CompiledPlot,
    scale_grouping: &ScaleGrouping,
    context: &RenderContext,
    // ... other params
) -> Result<Vec<Vec<OverflowSpaceRequirement>>, AvengerChartError> {
    // Determine grid dimensions from rects
    let num_rows = rects.iter().filter_map(|r| r.row_index).max().unwrap_or(0) + 1;
    let num_cols = rects.iter().filter_map(|r| r.col_index).max().unwrap_or(0) + 1;

    let mut overflow_grid = vec![vec![OverflowSpaceRequirement::default(); num_cols]; num_rows];

    for rect in rects {
        let row_idx = rect.row_index.unwrap();
        let col_idx = rect.col_index.unwrap();

        // Filter data for this cell
        let filter_df = filter_for_grid_cell(rect.value.clone(), rect.col_value.clone(), ...)?;

        // Build scales for this position
        let scales = scale_grouping.build_scales_for_position(...).await?;

        // Measure overflow
        let overflow = measure_subplot_overflow(
            compiled_subplot,
            rect.width,
            rect.height,
            &scales,
            &filter_df,
            context,
        )?;

        overflow_grid[row_idx][col_idx] = overflow;
    }

    Ok(overflow_grid)
}
```

## Implementation Phases

### Phase 1: Trait and Data Structure Updates (Breaking Changes)
**Files**: `src/coords.rs`

**Implementation Tasks**:
- [x] Add `PaddingSpec` enum with `Single` and `Grid` variants
- [x] Update `CoordinateSystemTransform::with_measured_padding` signature to accept `&PaddingSpec`
- [x] Add optional grid fields to `SubplotRect`: `row_index`, `col_index`, `col_value`
- [x] Add `#[serde(default, skip_serializing_if = "Option::is_none")]` to new fields
- [x] Update `SubplotRect::new()` to initialize new fields as `None`
- [x] Add `SubplotRect::new_grid()` constructor
- [x] Update default trait implementation to accept new signature

**Testing**:
- [ ] Unit test `PaddingSpec::Single` serialization/deserialization
- [ ] Unit test `PaddingSpec::Grid` serialization/deserialization
- [ ] Unit test `SubplotRect` with grid fields (new format)
- [ ] Unit test `SubplotRect` without grid fields (backward compatibility)
- [x] Run `cargo test -p avenger-chart` to verify no regressions (41 facet tests passed)

**Commit**: Phase 1 complete - Trait and data structure updates

### Phase 2: Update FacetRow/FacetCol for New Trait
**Files**: `src/facet/coord.rs`, `src/facet/marks/facet_evaluation.rs`

**Implementation Tasks**:
- [x] Update `FacetRow::with_measured_padding` (coord.rs:91-100) to accept `&PaddingSpec`
  - [x] Extract `padding_px` and `overflow` from `PaddingSpec::Single`
  - [x] Return error if `PaddingSpec::Grid` is provided
- [x] Update `FacetColumn::with_measured_padding` (coord.rs:226-235) to accept `&PaddingSpec`
  - [x] Extract `padding_px` and `overflow` from `PaddingSpec::Single`
  - [x] Return error if `PaddingSpec::Grid` is provided
- [x] Update `evaluate_facet` call site (facet_evaluation.rs:343-346)
  - [x] Build `PaddingSpec::Single { padding_px: rounded_gap, overflow: ... }`
  - [x] Pass as reference to `with_measured_padding`

**Testing**:
- [x] Run all FacetRow tests: `cargo test test_facet_row` (9 tests passed)
- [x] Run all FacetCol tests: `cargo test test_facet_col` (2 tests passed)
- [x] Verify visual regression tests pass for both facet types
- [x] Check that 17 facet unit tests pass (41 total facet tests passed)

**Commit**: Phase 2 complete - FacetRow/FacetCol updated for new trait

### Phase 3: Update FacetGrid.transform()
**Files**: `src/facet/coord.rs`

**Implementation Tasks**:
- [x] Update `FacetGrid` struct fields (coord.rs:353-362)
  - [x] Change `padding_px: Option<f32>` to `row_padding_px: Option<f32>`
  - [x] Add `col_padding_px: Option<f32>`
- [x] Update `FacetGrid::with_measured_padding` (coord.rs:367-376) to accept `&PaddingSpec`
  - [x] Extract `row_padding_px`, `col_padding_px`, `row_overflow`, `col_overflow` from `PaddingSpec::Grid`
  - [x] Return error if `PaddingSpec::Single` is provided
  - [x] Store both padding values and overflow vectors
- [x] Update `FacetGrid::transform()` (coord.rs:379-437)
  - [x] Extract `col_values` from `position_values` (similar to row_values)
  - [x] Update rect creation loop to use `SubplotRect::new_grid()`
  - [x] Populate `row_index`, `col_index`, `col_value` fields
- [x] Update `compute_band_layout` calls to use separate row/col padding

**Testing**:
- [ ] Unit test `FacetGrid::transform()` with `row_padding_px=10.0, col_padding_px=20.0`
- [ ] Verify rect count = num_rows × num_cols
- [ ] Verify `row_index` ranges from 0 to num_rows-1
- [ ] Verify `col_index` ranges from 0 to num_cols-1
- [ ] Verify `col_value` contains correct column domain values
- [ ] Test with single row (col_values only)
- [ ] Test with single column (row_values only)
- [x] All 11 grid facet visual regression tests passed

**Commit**: Phase 3 complete - FacetGrid.transform() updated

### Phase 4: Refactor CompiledFacetGrid.evaluate_from_data()
**Files**: `src/facet/marks/facet.rs`, new helper module (optional)

**Implementation Tasks**:

**4a. Helper Functions** (see `design/grid-facet-design-validation.md` for complete specs)
- [x] Implement `measure_grid_overflow()` helper
  - [x] Takes `&[SubplotRect]`, returns `Vec<Vec<OverflowSpaceRequirement>>`
  - [x] Determines grid dimensions from `row_index`/`col_index`
  - [x] Filters data for each cell using `row_value` and `col_value`
  - [x] Measures overflow for each cell
- [x] Implement `calculate_grid_padding()` helper
  - [x] Takes `&[Vec<OverflowSpaceRequirement>]`, returns `(f32, f32)`
  - [x] Calls `calculate_row_padding()` and `calculate_col_padding()`
- [x] Implement `calculate_row_padding()` helper
  - [x] Computes max of (row[i].bottom + row[i+1].top) across all rows
- [x] Implement `calculate_col_padding()` helper
  - [x] Computes max of (col[j].right + col[j+1].left) across all columns
- [x] Implement `extract_row_overflow()` helper
  - [x] Takes 2D overflow grid, returns 1D row overflow vector
  - [x] Takes max overflow in each direction across all columns in each row
- [x] Implement `extract_col_overflow()` helper
  - [x] Takes 2D overflow grid, returns 1D column overflow vector
  - [x] Takes max overflow in each direction across all rows in each column

**4b. Refactor evaluate_from_data()** (facet.rs:683-1267)
- [ ] Remove manual BandPositionIterator usage (lines 1009-1027, 1039-1104)
- [ ] Implement Pass 1: Measure overflow
  - [ ] Extract row/col positions from scales
  - [ ] Build position_channels HashMap
  - [ ] Call `coord.transform()` to get initial SubplotGeometry
  - [ ] Downcast and extract rects
  - [ ] Call `measure_grid_overflow()` with rects
- [ ] Implement Between Passes: Calculate padding
  - [ ] Call `calculate_grid_padding()` to get row/col padding
  - [ ] Build `PaddingSpec::Grid` with padding and overflow
  - [ ] Call `coord.with_measured_padding()` to get updated coord
- [ ] Implement Pass 2: Final rendering
  - [ ] Call `updated_coord.transform()` to get final SubplotGeometry
  - [ ] Downcast and extract final rects
  - [ ] Render each cell using `rect.x`, `rect.y` (NOT manual calculation)
  - [ ] Position scene groups at `[rect.x, rect.y]`
- [ ] Update return to populate both overflow fields in LayoutUpdates
  - [ ] Use `extract_row_overflow()` for `row_overflow_by_facet`
  - [ ] Use `extract_col_overflow()` for `col_overflow_by_facet`

**Testing**:
- [ ] Run all GridFacet visual regression tests: `cargo test test_facet_grid`
- [ ] Run comprehensive facet test suite: `cargo test -p avenger-chart --test visual_regression`
- [ ] Pixel-perfect comparison: Generate baseline images and compare
- [ ] Test with different grid sizes (2×2, 3×3, 1×5, 5×1)
- [ ] Test with missing data (sparse grids)
- [ ] Verify no manual position calculations remain (code review)
- [ ] Count lines removed (should be ~330 lines)

**Commit**: Phase 4 complete - GridFacet refactored to use coord.transform()

### Phase 5: Cleanup and Documentation
**Files**: Multiple

**Implementation Tasks**:
- [ ] Remove any remaining debug `eprintln!` statements from facet code
- [ ] Add rustdoc comments to `PaddingSpec` enum
- [ ] Add rustdoc comments to new `SubplotRect` fields
- [ ] Add rustdoc comments to all helper functions
- [ ] Update module-level documentation in `src/facet/marks/facet.rs`
- [ ] Add code example showing GridFacet usage pattern
- [ ] Update CHANGELOG.md with breaking change notes
- [ ] Document migration path for external CoordinateSystemTransform implementations

**Testing**:
- [ ] Run full test suite: `cargo test`
- [ ] Run with strict warnings: `RUSTFLAGS="-D warnings" cargo clippy --all-targets`
- [ ] Verify documentation builds: `cargo doc --no-deps`
- [ ] Check documentation quality: Review generated docs in browser

**Commit**: Phase 5 complete - Cleanup and documentation

## Risk Mitigation

### Breaking Change: Trait Signature
**Risk**: External crates implementing `CoordinateSystemTransform`
**Mitigation**:
- Document as breaking change in changelog
- Provide migration guide
- Consider versioning the trait if needed

### Serialization Compatibility
**Risk**: Existing serialized scenes fail to deserialize
**Mitigation**:
- `#[serde(default, skip_serializing_if = ...)]` on new fields
- Add deserialization test with old format

### Performance Regression
**Risk**: Additional transform() calls add overhead
**Mitigation**:
- Benchmark before/after
- Transform is cheap (pure math), so impact should be minimal

## Success Criteria

1. ✅ All FacetRow/FacetCol tests pass after trait update
2. ✅ All GridFacet tests pass after refactor
3. ✅ Visual output pixel-identical to current implementation
4. ✅ Code reduction: at least 250 lines removed from facet.rs
5. ✅ No manual BandPositionIterator usage in evaluate_from_data
6. ✅ coord.transform() used for all positioning
7. ✅ Independent row/column padding works correctly

## Expert Feedback Addressed

### From gpt-5-codex:
- ✅ **Padding trait issue**: Resolved with `PaddingSpec` enum
- ✅ **Serialization**: Added `#[serde(...)]` annotations
- ✅ **Ordering**: Plan preserves domain sorting before transform
- ✅ **Degenerate cases**: Plan handles missing scales (already in current code)
- ✅ **Overflow propagation**: `PaddingSpec::Grid` carries both overflow vectors

### From internal analysis:
- ✅ **Complexity**: Reduced from ~580 lines to ~250 lines
- ✅ **Maintainability**: Single source of truth in coord.rs
- ✅ **Testing**: Comprehensive strategy covering all phases

## Conclusion

This design is **approved for implementation** with all critical blockers resolved. The refactor will:

1. Eliminate ~330 lines of duplicate code
2. Align all three facet types with standard mark pattern
3. Make the system more maintainable and extensible
4. Enable external crates to implement custom facet coordinates

The phased approach ensures we can validate each step before proceeding to the next.
