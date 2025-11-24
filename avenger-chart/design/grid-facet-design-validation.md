# GridFacet Design Validation Report

**Status**: Complete - Design is implementation-ready
**Date**: 2025-11-21
**Reviewer**: Claude Code (Sonnet 4.5)

## Executive Summary

The final design in `grid-facet-final-design.md` has been thoroughly validated against the codebase. **The design is complete and implementation-ready** with no critical gaps identified.

## Validation Methodology

Systematically checked the design against:
1. ✅ Trait hierarchy and signature changes
2. ✅ All coordinate system implementations
3. ✅ Data structure compatibility
4. ✅ Helper function specifications
5. ✅ LayoutUpdates integration
6. ✅ Serialization/deserialization compatibility

## Key Findings

### 1. Trait Signature Change ✅ COMPLETE

**Design Specification** (grid-facet-final-design.md:27-53):
```rust
pub enum PaddingSpec {
    Single {
        padding_px: f32,
        overflow: Vec<OverflowSpaceRequirement>,
    },
    Grid {
        row_padding_px: f32,
        col_padding_px: f32,
        row_overflow: Vec<OverflowSpaceRequirement>,
        col_overflow: Vec<OverflowSpaceRequirement>,
    },
}

pub trait CoordinateSystemTransform {
    fn with_measured_padding(
        &self,
        spec: &PaddingSpec,
    ) -> Box<dyn CoordinateSystemTransform>;
}
```

**Current Implementation** (src/coords.rs:211-218):
```rust
fn with_measured_padding(
    &self,
    padding_px: f32,
    overflow: Vec<OverflowSpaceRequirement>,
) -> Box<dyn CoordinateSystemTransform> {
    let _ = (padding_px, overflow);
    self.clone_box()
}
```

**Impact Analysis**:
- ✅ All coordinate systems use default implementation (Cartesian, Polar, ZeroDCoord)
- ✅ Only facet coordinates override it (FacetRow, FacetColumn, FacetGrid)
- ✅ Design correctly identifies all affected implementations

**Implementation Path**:
1. Add `PaddingSpec` enum to `src/coords.rs`
2. Change trait signature in `CoordinateSystemTransform`
3. Update FacetRow impl (src/facet/coord.rs:91-100)
4. Update FacetColumn impl (src/facet/coord.rs:226-235)
5. Update FacetGrid impl (src/facet/coord.rs:367-376)
6. Update call site in facet_evaluation.rs:343-346

### 2. SubplotRect Enhancement ✅ COMPLETE

**Design Specification** (grid-facet-final-design.md:67-138):
```rust
pub struct SubplotRect {
    pub value: ScalarValue,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row_index: Option<usize>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub col_index: Option<usize>,

    #[serde_as(as = "Option<FromInto<SerializableScalar>>")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub col_value: Option<ScalarValue>,
}
```

**Current Implementation** (src/coords.rs:34-63):
```rust
#[serde_as]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubplotRect {
    #[serde_as(as = "FromInto<SerializableScalar>")]
    pub value: ScalarValue,
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}
```

**Validation**:
- ✅ Uses `#[serde(default, skip_serializing_if = ...)]` for backward compatibility
- ✅ New fields are `Option<>` so existing code continues to work
- ✅ Provides both `new()` and `new_grid()` constructors
- ✅ Serialization attributes correctly specified

**No Gaps Found**: Design is complete and backwards-compatible.

### 3. FacetGrid.transform() Update ✅ COMPLETE

**Design Specification** (grid-facet-final-design.md:148-196):
- Extract row_values and col_values from position_values
- Compute band layouts for both dimensions
- Create grid of SubplotRects with indices populated
- Use `SubplotRect::new_grid()` constructor

**Current Implementation** (src/facet/coord.rs:379-437):
```rust
fn transform(...) -> Result<Box<dyn PlotGeometry>, AvengerChartError> {
    // Current: Creates SubplotRects without grid fields
    for (row_idx, &y) in row_starts.iter().enumerate() {
        for (_col_idx, &x) in col_starts.iter().enumerate() {
            let value = row_values.get(row_idx).cloned().unwrap_or(ScalarValue::Null);
            rects.push(crate::coords::SubplotRect::new(
                value, x, y, col_bandwidth, row_bandwidth,
            ));
        }
    }
}
```

**Gap Analysis**: None - design provides exact replacement code.

**Implementation Path**: Direct replacement of rect creation loop (lines 421-434).

### 4. Helper Functions ✅ SUFFICIENTLY DETAILED

**measure_grid_overflow** (grid-facet-final-design.md:242-281):
```rust
fn measure_grid_overflow(
    rects: &[SubplotRect],
    compiled_subplot: &CompiledPlot,
    scale_grouping: &ScaleGrouping,
    context: &RenderContext,
    // ... other params
) -> Result<Vec<Vec<OverflowSpaceRequirement>>, AvengerChartError>
```

**Validation**:
- ✅ Input: SubplotRects with row_index/col_index populated
- ✅ Output: 2D grid of overflow measurements
- ✅ Logic: Filter data per cell, build scales, measure overflow
- ✅ Sufficient detail for implementation

**calculate_grid_padding** - Referenced but not detailed in design:

**MINOR GAP IDENTIFIED**: Design mentions this helper (line 221, 225) but doesn't provide full signature.

**Proposed Specification**:
```rust
fn calculate_grid_padding(
    overflow_grid: &[Vec<OverflowSpaceRequirement>],
) -> (f32, f32) {
    // Returns (row_padding_px, col_padding_px)
    // Row padding: max of (row[i].bottom + row[i+1].top) across all rows
    // Col padding: max of (col[j].right + col[j+1].left) across all cols

    let row_padding = calculate_row_padding(overflow_grid);
    let col_padding = calculate_col_padding(overflow_grid);

    (row_padding, col_padding)
}

fn calculate_row_padding(overflow_grid: &[Vec<OverflowSpaceRequirement>]) -> f32 {
    let num_rows = overflow_grid.len();
    if num_rows <= 1 {
        return 0.0;
    }

    let mut max_gap = 0.0;
    for i in 0..(num_rows - 1) {
        // Get max bottom overflow for row i across all columns
        let max_bottom = overflow_grid[i].iter()
            .map(|o| o.bottom)
            .fold(0.0f32, f32::max);

        // Get max top overflow for row i+1 across all columns
        let max_top = overflow_grid[i + 1].iter()
            .map(|o| o.top)
            .fold(0.0f32, f32::max);

        max_gap = max_gap.max(max_bottom + max_top);
    }

    max_gap.ceil()
}

fn calculate_col_padding(overflow_grid: &[Vec<OverflowSpaceRequirement>]) -> f32 {
    let num_cols = overflow_grid.first().map(|row| row.len()).unwrap_or(0);
    if num_cols <= 1 {
        return 0.0;
    }

    let mut max_gap = 0.0;
    for j in 0..(num_cols - 1) {
        // Get max right overflow for col j across all rows
        let max_right = overflow_grid.iter()
            .map(|row| row[j].right)
            .fold(0.0f32, f32::max);

        // Get max left overflow for col j+1 across all rows
        let max_left = overflow_grid.iter()
            .map(|row| row[j + 1].left)
            .fold(0.0f32, f32::max);

        max_gap = max_gap.max(max_right + max_left);
    }

    max_gap.ceil()
}
```

**Rationale**: This matches the pattern used in `dimension_config.rs` for FacetRow/Col but extends it to 2D.

**helper extract_row_overflow / extract_col_overflow** (line 226-227):

**MINOR GAP IDENTIFIED**: Design references these but doesn't define them.

**Proposed Specification**:
```rust
fn extract_row_overflow(
    overflow_grid: &[Vec<OverflowSpaceRequirement>]
) -> Vec<OverflowSpaceRequirement> {
    // For each row, combine overflow from all columns
    overflow_grid.iter().map(|row_overflows| {
        // Take max overflow in each direction across all columns in this row
        row_overflows.iter().fold(
            OverflowSpaceRequirement::default(),
            |acc, o| OverflowSpaceRequirement {
                top: acc.top.max(o.top),
                bottom: acc.bottom.max(o.bottom),
                left: acc.left.max(o.left),
                right: acc.right.max(o.right),
            }
        )
    }).collect()
}

fn extract_col_overflow(
    overflow_grid: &[Vec<OverflowSpaceRequirement>]
) -> Vec<OverflowSpaceRequirement> {
    let num_cols = overflow_grid.first().map(|row| row.len()).unwrap_or(0);

    // For each column, combine overflow from all rows
    (0..num_cols).map(|col_idx| {
        overflow_grid.iter().fold(
            OverflowSpaceRequirement::default(),
            |acc, row| {
                let o = &row[col_idx];
                OverflowSpaceRequirement {
                    top: acc.top.max(o.top),
                    bottom: acc.bottom.max(o.bottom),
                    left: acc.left.max(o.left),
                    right: acc.right.max(o.right),
                }
            }
        )
    }).collect()
}
```

### 5. LayoutUpdates Integration ✅ COMPLETE

**Current Structure** (src/layout/info.rs:26-58):
```rust
pub struct LayoutUpdates {
    pub scales: HashMap<String, ConfiguredScaleWithSpec>,
    pub row_overflow_by_facet: Option<Vec<OverflowSpaceRequirement>>,
    pub col_overflow_by_facet: Option<Vec<OverflowSpaceRequirement>>,
}
```

**Design Requirement** (grid-facet-final-design.md:237):
```rust
Ok((marks, LayoutUpdates::with_padding_spec(updated_spec)))
```

**GAP IDENTIFIED**: Design references `LayoutUpdates::with_padding_spec()` but:
- ✅ Current LayoutUpdates already has `row_overflow_by_facet` and `col_overflow_by_facet` fields
- ❌ Design doesn't specify what `with_padding_spec()` should do differently from existing methods
- ❌ Unclear if we need new method or can use existing pattern

**Proposed Resolution**:

GridFacet should populate BOTH overflow fields:
```rust
Ok((
    marks,
    LayoutUpdates {
        scales: updated_scales,
        row_overflow_by_facet: Some(extract_row_overflow(&overflow_grid)),
        col_overflow_by_facet: Some(extract_col_overflow(&overflow_grid)),
    }
))
```

This matches the existing pattern - no new method needed. The design's `with_padding_spec()` was likely a conceptual placeholder.

### 6. Error Handling and Edge Cases ✅ ADDRESSED

**Design Coverage**:
- ✅ Degenerate cases: "Plan handles missing scales (already in current code)" (line 376)
- ✅ Serialization: Uses `#[serde(default, skip_serializing_if = ...)]` (line 350)
- ✅ Domain sorting: "Plan preserves domain sorting before transform" (line 375)

**Validation**:
- Current GridFacet code handles empty domains (src/facet/marks/facet.rs:711-727)
- ScalarValue serialization already working via `SerializableScalar`
- Domain ordering preserved by `scale_scalars_to_numeric()` maintaining input order

**No Additional Gaps**: Edge cases are covered.

## Minor Gaps Summary

1. **Helper Functions - Detailed Signatures**:
   - `calculate_grid_padding()` - Specification provided above ✅
   - `extract_row_overflow()` - Specification provided above ✅
   - `extract_col_overflow()` - Specification provided above ✅

2. **LayoutUpdates Pattern**:
   - Use existing struct fields, not new method ✅
   - Populate both row and col overflow fields ✅

## Implementation Checklist

Based on validation, the implementation phases from the design are confirmed complete:

### Phase 1: Trait and Data Structure Updates ✅
- [ ] Add `PaddingSpec` enum to src/coords.rs
- [ ] Update `CoordinateSystemTransform::with_measured_padding` signature
- [ ] Enhance `SubplotRect` with optional grid fields
- [ ] Add `SubplotRect::new_grid()` constructor
- [ ] Update unit tests for serialization

### Phase 2: Update FacetRow/FacetCol ✅
- [ ] Update `FacetRow::with_measured_padding` to use `PaddingSpec::Single`
- [ ] Update `FacetColumn::with_measured_padding` to use `PaddingSpec::Single`
- [ ] Update `evaluate_facet` call site (facet_evaluation.rs:343-346)
- [ ] Verify all 17 facet tests pass

### Phase 3: Update FacetGrid.transform() ✅
- [ ] Update `FacetGrid` struct with separate row/col padding fields
- [ ] Update `FacetGrid::with_measured_padding` to use `PaddingSpec::Grid`
- [ ] Update transform() to populate grid fields (lines 421-434)
- [ ] Add unit tests for dual padding

### Phase 4: Refactor CompiledFacetGrid.evaluate_from_data() ✅
- [ ] Implement `measure_grid_overflow()` helper
- [ ] Implement `calculate_grid_padding()` helper (spec provided above)
- [ ] Implement `extract_row_overflow()` helper (spec provided above)
- [ ] Implement `extract_col_overflow()` helper (spec provided above)
- [ ] Refactor to two-pass pattern using coord.transform()
- [ ] Remove manual BandPositionIterator usage
- [ ] Visual regression tests must pass

### Phase 5: Cleanup ✅
- [ ] Remove debug statements
- [ ] Add documentation
- [ ] Update external docs

## Conclusion

**The design is IMPLEMENTATION-READY** with only minor gaps filled by this validation report.

### Design Quality: EXCELLENT

- Comprehensive coverage of all affected systems
- Proper backward compatibility strategy
- Phased implementation with clear testing criteria
- Expert review addressed critical blocker

### Gaps: MINOR and RESOLVED

All gaps were implementation details that can be inferred from existing patterns:
- Helper function signatures (now specified)
- LayoutUpdates integration (clarified - use existing pattern)

### Recommendation: PROCEED WITH IMPLEMENTATION

The design in `grid-facet-final-design.md` combined with this validation report provides sufficient detail to begin implementation. The phased approach ensures each step can be validated before proceeding.

### Risk Assessment: LOW

- Breaking change is unavoidable but well-managed
- Serialization compatibility ensured via serde attributes
- All existing tests will validate correctness
- Phased approach allows early issue detection

## References

- Final Design: `design/grid-facet-final-design.md`
- Initial Plan: `design/grid-facet-alignment-plan.md`
- Current Trait: `src/coords.rs:202-283`
- Current SubplotRect: `src/coords.rs:34-63`
- Current LayoutUpdates: `src/layout/info.rs:26-58`
- FacetRow/Col Impl: `src/facet/coord.rs`
- GridFacet Impl: `src/facet/marks/facet.rs:683-1267`
