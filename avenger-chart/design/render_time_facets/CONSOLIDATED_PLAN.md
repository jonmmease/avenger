# Render-Time Facets: Consolidated Implementation Plan

**Status**: Phase 1 Complete | Phase 2 Ready with Validated Approach
**Last Updated**: 2025-11-23
**Next Action**: Begin Phase 2 implementation with refined Option 5

---

## Executive Summary

Moving facet key extraction from compile time to render time to solve:
1. **Render Params Problem**: Keys extracted from wrong dataset
2. **Nested Faceting Problem**: Inner facets cause infinite recursion

**Solution**: Recursive data overrides with trait-based full DataFrame preservation for facets.

**Timeline**: 2-3 weeks (3 phases)
**Status**: Phase 0 & 1 complete, Phase 2 validated and ready

---

## Current Status

### ✅ Phase 0: Prototyping (COMPLETE)
**Commit**: `54c336c4`
- Implemented `batch_to_dataframe()` helper using DataFusion's `read_batch()` API
- All 41 facet visual tests pass
- Expert validated (no memory leaks, optimal approach)

### ✅ Phase 1: Structural Changes (COMPLETE)
**Commit**: `cf006b42`
- Removed `distinct_keys` from `CompiledFacetRow`, `CompiledFacetCol`, `CompiledFacetGrid`
- Removed compile-time key extraction from all `compile()` methods
- All tests still pass (no behavioral changes yet)

### ✅ Phase 1.5: Expert Validation & Conflict Resolution (COMPLETE)
**Investigation**: NESTED_FACET_DATA_FLOW_INVESTIGATION.md
**Resolution**: CONFLICT_RESOLUTION.md
**Outcome**: **Blocker identified and resolved**

#### Critical Finding
GPT-5 Codex identified that RecordBatch passed to nested facets only contains the facet channel column (e.g., "row"), missing data columns (x, y, color, etc.) needed by inner subplot marks.

#### Resolution: Refined Option 5
Add `wants_full_data_batch()` trait method to preserve full DataFrame for facet marks.

**Expert Consensus**: Both GPT-5 Codex and Gemini 3 Pro agree Phase 2 is feasible with this refinement.

### 🔄 Phase 2: Render-Time Logic (READY TO START)
**Estimated**: Week 2
**Blockers**: None ✅

---

## Phase 2: Detailed Implementation Plan

### Task 2.1: Add `wants_full_data_batch()` Trait Method ⭐ NEW

**File**: `avenger-chart/src/marks/mod.rs`

**Add to `CompiledMark` trait** (around line 145):
```rust
/// Whether this mark needs the full DataFrame as RecordBatch
///
/// Most marks only need columns for their specific channels (default: false).
/// Container marks like facets need all columns to pass to nested marks (return: true).
fn wants_full_data_batch(&self) -> bool {
    false  // Default: only select needed channels
}
```

**Rationale**: Trait-based approach is cleaner and more robust than string matching on mark_type.

### Task 2.2: Implement Trait for Facet Marks

**File**: `avenger-chart/src/facet/marks/facet.rs`

**Add to `impl CompiledMark for CompiledFacetRow`** (after line 199):
```rust
fn wants_full_data_batch(&self) -> bool {
    true  // Facets need full data for nested filtering
}
```

**Repeat for**:
- `CompiledFacetCol` (after line ~372)
- `CompiledFacetGrid` (after line ~764)

### Task 2.3: Modify `evaluate_mark_with_plot_df` ⭐ CRITICAL

**File**: `avenger-chart/src/plot/compiled/rendering.rs`

**Location**: Lines 620-640 (RecordBatch building section)

**Before**:
```rust
// Build array data batch if needed
let data_batch = if has_array_data {
    let mut select_exprs = vec![];
    for (name, expr) in &array_channels {
        select_exprs.push(expr.clone().alias(*name));
    }
    // ... select only array_channels
}
```

**After**:
```rust
// Build array data batch if needed
let data_batch = if mark.wants_full_data_batch() && df_ref.is_some() {
    // For container marks (facets): preserve ALL columns for nested marks
    let datafusion_params = crate::utils::params_to_datafusion(params);
    let batch = if let Some(param_values) = datafusion_params {
        (*df)
            .clone()
            .with_param_values(param_values)?  // ← CRITICAL: Apply params!
            .collect()
            .await?
    } else {
        (*df).clone().collect().await?
    };

    if batch.is_empty() {
        // Return empty batch WITH SCHEMA for facets (enables key extraction)
        let schema = df.schema();
        Some(datafusion::arrow::record_batch::RecordBatch::new_empty(schema))
    } else {
        use datafusion::arrow::compute::concat_batches;
        let schema = batch[0].schema();
        Some(concat_batches(&schema, &batch)?)
    }
} else if has_array_data {
    // Normal path: select only needed channels
    let mut select_exprs = vec![];
    for (name, expr) in &array_channels {
        select_exprs.push(expr.clone().alias(*name));
    }
    // ... rest of existing code
}
```

**Critical Fixes** (from GPT-5 Codex review):
1. ✅ Apply `with_param_values()` - prevents silently ignored parameterized filters
2. ✅ Return empty batch with schema - enables deterministic key extraction
3. ✅ Works for both provided_plot_df and mark-level data sources

### Task 2.4: Modify `evaluate_facet` Signature

**File**: `avenger-chart/src/facet/marks/facet_evaluation.rs`

**Before** (line ~77):
```rust
pub async fn evaluate_facet<DimConfig: FacetDimensionConfig>(
    facet_coord: &dyn CoordinateSystemTransform,
    compiled_subplot: &Arc<CompiledPlot>,
    state: &CompiledMarkState,
    _facet_title: Option<String>,
    facet_spacing: Option<f32>,
    context: &RenderContext,
    facet_keys: Option<&[ScalarValue]>,  // ← REMOVE
    subplot_dims: impl Fn(f32, &RenderContext) -> (f32, f32),
    group_origin: impl Fn(f32) -> [f32; 2],
)
```

**After**:
```rust
pub async fn evaluate_facet<DimConfig: FacetDimensionConfig>(
    facet_coord: &dyn CoordinateSystemTransform,
    compiled_subplot: &Arc<CompiledPlot>,
    state: &CompiledMarkState,
    data_override: Option<&DataFrame>,  // ← NEW: For nested facets
    _facet_title: Option<String>,
    facet_spacing: Option<f32>,
    context: &RenderContext,
    // facet_keys parameter REMOVED
    subplot_dims: impl Fn(f32, &RenderContext) -> (f32, f32),
    group_origin: impl Fn(f32) -> [f32; 2],
)
```

### Task 2.5: Extract Keys at Render Time

**File**: `avenger-chart/src/facet/marks/facet_evaluation.rs`

**Add after getting SessionContext** (around line 115):
```rust
// Determine data source: parent override OR compiled data
let df = if let Some(override_df) = data_override {
    // Nested facet: use filtered data from parent
    override_df.clone()
} else {
    // Top-level facet: use compiled data
    state
        .data
        .dataframe_with_context(ctx)
        .ok_or_else(|| {
            AvengerChartError::InternalError(
                format!("Facet requires data but none is available").into(),
            )
        })?
};

// Extract facet keys at render time from actual data
let facet_keys = FacetKeyExtractor::extract_keys(&df, &facet_expr).await?;
```

**Remove**: Lines that used compile-time keys (old line ~131)

### Task 2.6: Update Facet `evaluate_from_data` Methods

**File**: `avenger-chart/src/facet/marks/facet.rs`

**For `CompiledFacetRow::evaluate_from_data`** (line ~204):

**Before**:
```rust
async fn evaluate_from_data(
    &self,
    _data: Option<&datafusion::arrow::record_batch::RecordBatch>,  // ← Ignored
    _scalars: &datafusion::arrow::record_batch::RecordBatch,
    context: &RenderContext,
    coord: Box<dyn crate::coords::CoordinateSystemTransform>,
) -> Result<(Vec<SceneMark>, crate::layout::LayoutUpdates), AvengerChartError> {
    evaluate_facet::<RowDimensionConfig>(
        coord.as_ref(),
        &self.compiled_subplot,
        &self.state,
        self.facet_title.clone(),
        self.facet_spacing,
        context,
        None,  // ← Always None
        ...
    ).await
}
```

**After**:
```rust
async fn evaluate_from_data(
    &self,
    data: Option<&datafusion::arrow::record_batch::RecordBatch>,  // ← Use it!
    _scalars: &datafusion::arrow::record_batch::RecordBatch,
    context: &RenderContext,
    coord: Box<dyn crate::coords::CoordinateSystemTransform>,
) -> Result<(Vec<SceneMark>, crate::layout::LayoutUpdates), AvengerChartError> {
    // Convert RecordBatch to DataFrame if provided (for nested facets)
    let data_override = if let Some(batch) = data {
        Some(batch_to_dataframe(batch, &context.session_context)?)
    } else {
        None
    };

    evaluate_facet::<RowDimensionConfig>(
        coord.as_ref(),
        &self.compiled_subplot,
        &self.state,
        data_override.as_ref(),  // ← Pass override
        self.facet_title.clone(),
        self.facet_spacing,
        context,
        // facet_keys parameter removed
        ...
    ).await
}
```

**Repeat for**:
- `CompiledFacetCol::evaluate_from_data` (line ~377)
- `CompiledFacetGrid::evaluate_from_data` (line ~785)

### Task 2.7: Handle GridFacet Two Dimensions

**File**: `avenger-chart/src/facet/marks/facet_evaluation.rs`

GridFacet uses both row and column keys. The extraction logic needs both dimensions:

```rust
let row_keys = FacetKeyExtractor::extract_keys(&df, &row_expr).await?;
let col_keys = FacetKeyExtractor::extract_keys(&df, &col_expr).await?;
```

Update the grid-specific evaluation to use render-time extracted keys.

---

## Risk Assessment (UPDATED)

| Risk | Original | After Phase 1 | Current | Mitigation |
|------|----------|---------------|---------|------------|
| DataFrame Conversion | HIGH | LOW ✅ | **RESOLVED** | Used read_batch() API |
| RecordBatch Column Loss | N/A | N/A | **RESOLVED** | Trait-based full-data approach |
| Memory Leaks | HIGH | LOW ✅ | **RESOLVED** | Official DataFusion API |
| Param Application | N/A | N/A | **RESOLVED** | Apply with_param_values() |
| Empty Data Handling | N/A | N/A | **RESOLVED** | Return empty batch with schema |
| Breaking Serialization | MEDIUM | MEDIUM | MEDIUM | Use serde skip_serializing_if |
| Missing User API | LOW | LOW | LOW | Defer to follow-up |

---

## Critical Questions (RESOLVED)

### Q1: Can we convert RecordBatch to DataFrame efficiently?
**Status**: ✅ RESOLVED
**Answer**: Yes, using `SessionContext::read_batch()`. Validated with all tests passing.

### Q2: Will RecordBatch contain all needed columns for nested facets?
**Status**: ✅ RESOLVED
**Answer**: Not by default, but fixed with `wants_full_data_batch()` trait method. GPT-5 Codex validated this approach addresses the blocker.

### Q3: How do we handle backward compatibility for serialization?
**Status**: 🔄 OPEN
**Recommendation**: Use `#[serde(default, skip_serializing_if = "Vec::is_empty")]` for removed distinct_keys fields.

### Q4: What about performance regression?
**Status**: ✅ ACCEPTABLE
**Answer**: Acceptable tradeoff for rare nested facet case. Consider future optimization: cap columns to subplot's required set.

### Q5: How do we apply params in full-data path?
**Status**: ✅ RESOLVED
**Answer**: Apply `with_param_values()` before `collect()` in full-data branch (same as channel-select path).

---

## Success Criteria

### Must Have ✅
- [x] Phase 0: batch_to_dataframe() helper implemented
- [x] Phase 1: distinct_keys removed from compiled structs
- [x] Phase 1: Compile-time key extraction removed
- [x] Expert validation: No blockers identified
- [ ] Phase 2: wants_full_data_batch() trait added
- [ ] Phase 2: Full DataFrame preservation for facets
- [ ] Phase 2: Render-time key extraction working
- [ ] Phase 2: Nested facet test passes
- [ ] All existing facet tests pass

### Should Have 📋
- [ ] Performance within 10% of baseline
- [ ] Backward serde compatibility
- [ ] Migration guide for breaking changes

### Nice to Have 💡
- [ ] User-facing API for render-time data override (defer)
- [ ] Column subsetting optimization for facets

---

## Implementation Checklist

### Phase 2 Tasks (In Order)

- [ ] **2.1**: Add `wants_full_data_batch()` to CompiledMark trait
- [ ] **2.2**: Implement trait for CompiledFacetRow/Col/Grid
- [ ] **2.3**: Modify evaluate_mark_with_plot_df with full-data path
  - [ ] Apply with_param_values() in full-data branch
  - [ ] Return empty batch with schema
  - [ ] Test with empty datasets
- [ ] **2.4**: Update evaluate_facet signature (add data_override)
- [ ] **2.5**: Add render-time key extraction in evaluate_facet
- [ ] **2.6**: Update facet evaluate_from_data methods
  - [ ] Convert RecordBatch to DataFrame
  - [ ] Pass to evaluate_facet
- [ ] **2.7**: Handle GridFacet two-dimensional keys

### Validation

- [ ] Compile check succeeds
- [ ] All existing facet tests pass (41 tests)
- [ ] Nested facet test passes (remove #[ignore])
- [ ] No memory leaks (long-running test)
- [ ] Performance benchmarks (<10% regression)

---

## Files Modified

### Phase 0 (Complete)
- `avenger-chart/src/facet/marks/facet_evaluation.rs` - Added batch_to_dataframe()

### Phase 1 (Complete)
- `avenger-chart/src/facet/marks/facet.rs` - Removed distinct_keys from all facet types

### Phase 2 (Pending)
- `avenger-chart/src/marks/mod.rs` - Add wants_full_data_batch() trait method
- `avenger-chart/src/facet/marks/facet.rs` - Implement trait, update evaluate_from_data()
- `avenger-chart/src/plot/compiled/rendering.rs` - Add full-data path in evaluate_mark_with_plot_df()
- `avenger-chart/src/facet/marks/facet_evaluation.rs` - Update evaluate_facet signature and logic

### Testing
- `avenger-chart/tests/visual_tests/test_nested_facets.rs` - Enable ignored test

---

## Expert Reviews

### GPT-5 Codex Review (CONFLICT_RESOLUTION.md)

**Key Findings**:
- ✅ Option 5 closes the data-loss gap
- ❌ String matching on mark_type is brittle
- ✅ Trait-based approach is cleaner
- ✅ Must apply params in full-data branch
- ✅ Must return empty batch with schema for facets

**Verdict**: "Phase 2 is feasible with the modified data flow"

### Gemini 3 Pro Review (NESTED_FACET_DATA_FLOW_INVESTIGATION.md)

**Key Findings**:
- ✅ Data override plumbing exists and works
- ✅ Investigation is correct
- ✅ Approach is feasible

**Verdict**: "Proceed as planned"

**Consensus**: Both experts agree after conflict resolution.

---

## Timeline

**Original estimate**: 2-3 weeks
**Current estimate**: 2-3 weeks ✅ (on track)

```
✅ Week 0: Prototyping & Expert Validation
  └─ batch_to_dataframe() implemented
  └─ Expert reviews complete
  └─ Blocker identified and resolved

✅ Week 1: Structural Changes (Phase 1)
  └─ distinct_keys removed
  └─ Compile-time extraction removed
  └─ All tests passing

🔄 Week 2: Render-Time Logic (Phase 2) ← WE ARE HERE
  └─ Trait method added
  └─ Full-data preservation implemented
  └─ Render-time extraction working
  └─ Nested facet test passing

⏸️ Week 3: Testing & Validation (Phase 3)
  └─ Performance benchmarks
  └─ Memory leak tests
  └─ Documentation updates
```

---

## Next Steps

1. Begin Phase 2 Task 2.1: Add `wants_full_data_batch()` trait method
2. Implement refined Option 5 following GPT-5 Codex recommendations
3. Validate each task with compilation and test runs
4. Enable nested facet test when Phase 2 complete

---

## References

- **Investigation**: NESTED_FACET_DATA_FLOW_INVESTIGATION.md - Complete data flow analysis
- **Conflict Resolution**: CONFLICT_RESOLUTION.md - Expert consensus on refined Option 5
- **Original Plan**: archive/PLAN.md - Superseded by recursive data override approach
- **Expert Reviews**: REVISED_PLAN_EXPERT_SYNTHESIS.md - DataFusion API validation
