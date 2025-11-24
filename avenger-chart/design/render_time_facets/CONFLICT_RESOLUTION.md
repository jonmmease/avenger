# Expert Conflict Resolution: RecordBatch Contents

**Date**: 2025-11-23
**Status**: RESOLVED - GPT-5 Codex is CORRECT

---

## The Conflict

**GPT-5 Codex**: RecordBatch only contains facet channel column, missing data needed by inner marks.

**Gemini 3 Pro**: Data override mechanism works, approach is feasible.

---

## Deep Dive Investigation

### Complete Data Flow Trace

#### 1. Outer FacetRow::evaluate_from_data() called
- **Input**: `data = None` (outer facet has no parent data override)
- **Location**: facet.rs:204-230

#### 2. evaluate_facet() processes outer facet
- **Input**: Gets full dataset from `state.data`
- **Action**: Filters dataset for each row value
- **Code**: facet_evaluation.rs:278-280
```rust
let filter_df: DataFrame = df
    .clone()
    .filter(facet_expr.clone().eq(lit(iteration.facet_value.clone())))?;
```
- **Result**: `filter_df` contains **ALL columns**, filtered by row value

#### 3. build_plot_components() called for subplot
- **Input**: `data_override = Some(&filter_df)`
- **Location**: facet_evaluation.rs:313-324
- **Critical**: filter_df has ALL columns (x, y, color, col, etc.)

#### 4. evaluate_mark_with_plot_df() called for inner FacetCol
- **Input**: `provided_plot_df = Some(&filter_df)` ← **Has ALL columns**
- **Location**: rendering.rs:1232-1240

#### 5. DataFrame selection logic
- **Location**: rendering.rs:564-577
```rust
let df_ref = if let Some(df_override) = provided_plot_df.cloned() {
    Some(df_override)  // ← Uses override with ALL columns
} else if ...
```
- **Result**: `df_ref = Arc::new(filter_df)` with ALL columns

#### 6. Channel splitting
- **Location**: rendering.rs:604-618
- **Facet channels**: FacetCol only has ONE channel - "col"
```rust
fn supported_channels(&self) -> Vec<ChannelDescriptor> {
    vec![ChannelDescriptor {
        name: ColDimensionConfig::channel_name(),  // "col"
        required: true,
        allow_column_ref: true,
    }]
}
```

#### 7. RecordBatch creation
- **Location**: rendering.rs:620-640
```rust
let data_batch = if has_array_data {
    let mut select_exprs = vec![];
    for (name, expr) in &array_channels {
        select_exprs.push(expr.clone().alias(*name));
    }
    // Only selects "col" channel expression
    (*df).clone().select(select_exprs)?.collect().await?
}
```
- **Result**: RecordBatch contains **ONLY "col" column**

#### 8. Inner FacetCol::evaluate_from_data() called
- **Input**: `data = Some(record_batch_with_only_col_column)`
- **Problem**: Inner FacetCol needs to evaluate its subplot marks, which need x, y, color columns

---

## The Critical Question

**Does the inner facet need the full RecordBatch?**

### Current Phase 1 Implementation
Inner facets **ignore the `data` parameter** (prefixed with underscore):
```rust
async fn evaluate_from_data(
    &self,
    _data: Option<&datafusion::arrow::record_batch::RecordBatch>,  // ← IGNORED
    _scalars: &datafusion::arrow::record_batch::RecordBatch,
    context: &RenderContext,
    coord: Box<dyn crate::coords::CoordinateSystemTransform>,
)
```

### Proposed Phase 2 Implementation
Inner facets would **USE the `data` parameter**:
```rust
async fn evaluate_from_data(
    &self,
    data: Option<&datafusion::arrow::record_batch::RecordBatch>,  // ← TO BE USED
    _scalars: &datafusion::arrow::record_batch::RecordBatch,
    context: &RenderContext,
    coord: Box<dyn crate::coords::CoordinateSystemTransform>,
) -> Result<(Vec<SceneMark>, crate::layout::LayoutUpdates), AvengerChartError> {
    // PROPOSED PHASE 2 CODE:
    // 1. Convert RecordBatch to DataFrame using batch_to_dataframe()
    // 2. Pass DataFrame to evaluate_facet as data_override
    // 3. evaluate_facet uses it to extract facet keys and filter for subplots
}
```

**The problem**: The RecordBatch only has "col" column. When converted to DataFrame:
- ✅ Can extract distinct "col" values for facet keys
- ❌ Cannot pass to subplot's build_plot_components - missing x, y, color columns

---

## GPT-5 Codex's Concern Validated

**Codex is CORRECT**: The RecordBatch passed to nested facets contains only the facet channel column, not all columns needed by inner subplot marks.

**Why Gemini missed it**: Gemini validated the plumbing (data_override is passed correctly) but didn't trace what's IN the RecordBatch at each level.

---

## The Solution

### Option 1: Store DataFrame Reference (BLOCKED)
- **Problem**: Can't store DataFrame in CompiledMark (not thread-safe)
- **Status**: Not viable

### Option 2: Pass Full DataFrame Through RecordBatch
- **Approach**: When mark is a facet, include ALL columns in RecordBatch, not just facet channel
- **Challenge**: Requires detecting facet marks and special-casing them
- **Status**: Possible but messy

### Option 3: Use Context/ThreadLocal Storage
- **Approach**: Store full DataFrame in thread-local or context during evaluation
- **Challenge**: Complex lifetime management, thread safety issues
- **Status**: Too complex

### Option 4: Rethink the Approach ⭐
**Key insight**: The `data` parameter in `evaluate_from_data` is NOT meant to carry the full filtered dataset. It's meant for marks that directly render data (scatter, line, etc.).

**For facet marks**, the correct approach is:
1. Keep getting full dataset from `state.data` in evaluate_facet
2. Don't rely on `data` RecordBatch parameter
3. The data_override mechanism works at the build_plot_components level, not evaluate_from_data level

**But this defeats Phase 2's goal**: We need facets to extract keys from filtered data, not compile-time data.

### Option 5: Change evaluate_mark_with_plot_df Signature ⭐⭐
**Core issue**: evaluate_mark_with_plot_df selects only mark's channels from DataFrame before creating RecordBatch.

**Solution**: Add a flag to preserve full DataFrame for facet marks:
```rust
pub async fn evaluate_mark_with_plot_df(
    &self,
    mark: &dyn CompiledMark,
    ...,
    provided_plot_df: Option<&DataFrame>,
    preserve_full_data: bool,  // ← NEW
) -> Result<...> {
    ...
    let data_batch = if preserve_full_data && provided_plot_df.is_some() {
        // For facet marks: convert entire DataFrame to RecordBatch
        let full_batch = provided_plot_df.unwrap().clone().collect().await?;
        if full_batch.is_empty() { None } else {
            use datafusion::arrow::compute::concat_batches;
            let schema = full_batch[0].schema();
            Some(concat_batches(&schema, &full_batch)?)
        }
    } else {
        // Normal path: select only needed channels
        ...existing code...
    }
}
```

**How to detect facet marks**:
```rust
let preserve_full_data = mark.mark_type().starts_with("facet_");
```

**Impact**:
- ✅ Minimal code changes
- ✅ Preserves all columns for nested facets
- ✅ No performance impact on non-facet marks
- ⚠️ Increases memory for facet marks (passes full data, not just one column)

---

## Recommendation (UPDATED AFTER GPT-5 CODEX REVIEW)

**Proceed with REFINED Option 5**: Modify `evaluate_mark_with_plot_df` with trait-based approach.

### GPT-5 Codex Review Summary

✅ **Confirms feasibility**: Option 5 adequately closes the data-loss gap
❌ **Rejects string matching**: `mark_type().starts_with("facet_")` is brittle
✅ **Proposes cleaner API**: Add `wants_full_data_batch()` trait method

### Refined Implementation Plan

**1. Add trait method to CompiledMark** (avenger-chart/src/marks/mod.rs)
```rust
fn wants_full_data_batch(&self) -> bool {
    false  // Default: only select needed channels
}
```

**2. Implement for facet marks** (avenger-chart/src/facet/marks/facet.rs)
```rust
// For CompiledFacetRow, CompiledFacetCol, CompiledFacetGrid
fn wants_full_data_batch(&self) -> bool {
    true  // Facets need full data for nested filtering
}
```

**3. Modify evaluate_mark_with_plot_df** (avenger-chart/src/plot/compiled/rendering.rs:620-640)
```rust
let data_batch = if mark.wants_full_data_batch() && df_ref.is_some() {
    // For facet marks: preserve ALL columns
    let datafusion_params = crate::utils::params_to_datafusion(params);
    let batch = if let Some(param_values) = datafusion_params {
        (*df)
            .clone()
            .with_param_values(param_values)?  // ← Apply params!
            .collect()
            .await?
    } else {
        (*df).clone().collect().await?
    };

    if batch.is_empty() {
        // Return empty batch WITH SCHEMA for facets
        Some(RecordBatch::new_empty(batch[0].schema()))
    } else {
        use datafusion::arrow::compute::concat_batches;
        let schema = batch[0].schema();
        Some(concat_batches(&schema, &batch)?)
    }
} else {
    // Normal path: select only needed channels
    ...existing code...
}
```

### Critical Fixes from GPT-5 Review

1. **Param handling**: Apply `with_param_values()` in full-DF branch (prevents silently ignored filters)
2. **Empty data**: Return empty batch with schema instead of None (allows deterministic key extraction)
3. **Mark-level data**: Apply full-data path regardless of data source (not just provided_plot_df)
4. **Performance**: Consider capping columns to subplot's required set (future optimization)

### Edge Cases Addressed

- ✅ Memory & perf: Noted as acceptable tradeoff for nested facets (rare case)
- ✅ Dictionary schemas: Handled by concat_batches schema validation
- ✅ Param application: Fixed by using same param path as channel-select branch
- ✅ Empty datasets: Fixed by returning empty batch with schema

---

## Consensus Achieved

**Both experts now agree**:
- GPT-5 Codex: "Phase 2 is feasible with the modified data flow"
- Implementation validated with trait-based approach
- No remaining blockers identified

### Next Steps

1. ✅ Update REVISED_PLAN.md with refined Option 5
2. ✅ Add trait method and implementation to Phase 2 tasks
3. ✅ Document param handling and empty-batch requirements
4. ⏸️ Begin Phase 2 implementation
