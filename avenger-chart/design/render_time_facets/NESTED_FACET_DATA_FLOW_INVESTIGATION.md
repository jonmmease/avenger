# Nested Facet Data Flow Investigation

**Date**: 2025-11-23
**Investigator**: Claude Code
**Purpose**: Understand exactly how data flows from outer facets to inner facets to determine what changes are needed for Phase 2

---

## Executive Summary

**FINDING**: The current system ALREADY supports passing filtered data from outer facets to inner facets through the `data` parameter in `evaluate_from_data()`. Phase 2 does NOT require new plumbing - we just need to:

1. **In `evaluate_facet()`**: Add `data_override` parameter and use it to extract keys at render time
2. **In facet `evaluate_from_data()` methods**: Convert the `data` RecordBatch to DataFrame and pass it to `evaluate_facet()` as `data_override`

The mechanism for nested faceting ALREADY EXISTS and works correctly.

---

## Complete Data Flow Trace

### 1. Outer Facet: `CompiledFacetRow::evaluate_from_data()`

**File**: `avenger-chart/src/facet/marks/facet.rs:204-247`

```rust
async fn evaluate_from_data(
    &self,
    _data: Option<&RecordBatch>,  // CURRENTLY IGNORED (underscore prefix)
    _scalars: &RecordBatch,
    context: &RenderContext,
    coord: Box<dyn CoordinateSystemTransform>,
) -> Result<(Vec<SceneMark>, LayoutUpdates), AvengerChartError> {
    evaluate_facet::<RowDimensionConfig>(
        coord.as_ref(),
        &self.compiled_subplot,
        &self.state,
        // ... other params
        None,  // facet_keys currently None (we removed distinct_keys in Phase 1)
        // ... closures
    )
    .await
}
```

**Key Point**: The `data` parameter is currently IGNORED (has underscore prefix). Phase 2 will USE it.

---

### 2. Facet Evaluation: `evaluate_facet()`

**File**: `avenger-chart/src/facet/marks/facet_evaluation.rs:77-670`

#### Step 2a: Get DataFrame (currently from compile-time data)

**Lines 125-144**: DataFrame is retrieved from compile-time data:

```rust
let df = state
    .data
    .dataframe_with_context(ctx)
    .ok_or_else(|| {
        AvengerChartError::InternalError("Facet mark requires data".into())
    })?;
```

**PHASE 2 CHANGE**: Replace this with logic that uses `data_override` if provided:

```rust
let df = if let Some(override_df) = data_override {
    override_df  // Use data from parent facet
} else {
    state.data.dataframe_with_context(ctx)...  // Use compile-time data
};
```

#### Step 2b: Filter DataFrame by facet value

**Lines 278-280 (Pass 1)** and **Lines 541-543 (Pass 2)**:

```rust
// Filter df by facet_value
let filter_df: DataFrame = df
    .clone()
    .filter(facet_expr.clone().eq(lit(iteration.facet_value.clone())))?;
```

**Key Point**: Each subplot gets its own filtered DataFrame based on the facet value.

---

### 3. Subplot Rendering: `compiled_subplot.build_plot_components()`

**File**: `avenger-chart/src/plot/compiled/rendering.rs:1122-1280`

#### Step 3a: Pass filtered DataFrame as data_override

**facet_evaluation.rs:603-614**:

```rust
let components = compiled_subplot
    .build_plot_components(
        width,
        height,
        ctx,
        &iteration.params,
        &scale_provider,
        crate::plot::compiled::EvaluationMode::Render,
        Some(&filter_df),  // ← FILTERED DATA PASSED HERE
        true,  // Plot area mode
    )
    .await?;
```

#### Step 3b: Store data_override in df_opt

**rendering.rs:1228**:

```rust
let df_opt = data_override;  // ← STORED FOR USE BY MARKS
```

---

### 4. Mark Evaluation: Individual marks get filtered data

**File**: `avenger-chart/src/plot/compiled/rendering.rs:330-510`

Each mark in the subplot is evaluated with the filtered DataFrame.

#### Step 4a: Determine DataFrame source

**Lines 340-380** (summarized logic):

```rust
let df: Arc<DataFrame> = if let Some(override_df) = df_opt {
    // Use the override from parent facet
    Arc::new(override_df.clone())
} else {
    // Use the mark's own compiled data
    mark.data_context()
        .dataframe_with_context(ctx)
        .ok_or_else(|| ...)?
};
```

#### Step 4b: Build RecordBatch from DataFrame

**Lines 413-440**:

```rust
let data_batch = if has_array_data {
    // Select array channels from df and collect into RecordBatch
    (*df).clone()
        .select(select_exprs)?
        .collect().await?
    // ... concat batches
} else {
    None
};
```

#### Step 4c: Call mark's evaluate_from_data()

**Lines 503-509**:

```rust
mark.evaluate_from_data(
    data_batch.as_ref(),  // ← RecordBatch from filtered DataFrame
    &scalar_batch,
    &context,
    coord_transform,
)
.await
```

**Key Point**: If the mark is a NESTED FACET (e.g., FacetColumn inside FacetRow), its `evaluate_from_data()` receives `data_batch` created from the FILTERED DataFrame!

---

## The Nested Facet Loop

For nested facets (e.g., `FacetRow` containing `FacetColumn`):

```
Outer FacetRow::evaluate_from_data(data=None)
  ↓
  evaluate_facet() gets full dataset from state.data
  ↓
  For each row value (e.g., Species="setosa"):
    ↓
    filter_df = df.filter(row == "setosa")  ← FILTERED DATA
    ↓
    compiled_subplot.build_plot_components(data_override=Some(&filter_df))
      ↓
      For each mark in subplot:
        ↓
        If mark is CompiledFacetCol:
          ↓
          Create RecordBatch from filter_df
          ↓
          Inner FacetCol::evaluate_from_data(data=Some(record_batch))  ← NESTED!
            ↓
            CURRENT: Ignores `data`, uses state.data (WRONG - gets full dataset!)
            ↓
            PHASE 2: Use `data` to create DataFrame via batch_to_dataframe()
            ↓
            evaluate_facet(data_override=Some(dataframe))
            ↓
            Extracts column keys from FILTERED data (only setosa rows)
            ↓
            For each column value within setosa subset:
              ↓
              Renders subplot with doubly-filtered data
```

---

## Current Problem (Why Nested Facets Fail)

**File**: `facet.rs:206`

```rust
async fn evaluate_from_data(
    &self,
    _data: Option<&RecordBatch>,  // ← IGNORED! (underscore prefix)
    ...
) {
    evaluate_facet::<RowDimensionConfig>(
        &self.state,  // ← Uses compile-time data from state
        None,  // ← No data_override passed
        ...
    )
}
```

**The Bug**: Inner facets ignore the `data` parameter and always use `state.data`, which is the FULL compile-time dataset, not the filtered data from the outer facet.

**Result**:
1. Inner facet extracts keys from FULL dataset (not just outer facet's subset)
2. Causes stack overflow when trying to render cells that don't exist in filtered data
3. Prevents nested faceting from working

---

## Phase 2 Solution

### Change 1: Add `data_override` parameter to `evaluate_facet()`

**File**: `facet_evaluation.rs:77-90`

**Before**:
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
    data_override: Option<DataFrame>,  // ← NEW: Override from parent facet
    _facet_title: Option<String>,
    facet_spacing: Option<f32>,
    context: &RenderContext,
    // facet_keys parameter REMOVED
    subplot_dims: impl Fn(f32, &RenderContext) -> (f32, f32),
    group_origin: impl Fn(f32) -> [f32; 2],
)
```

### Change 2: Extract keys from render-time data in `evaluate_facet()`

**File**: `facet_evaluation.rs` (insert after line 90)

```rust
// Get render-time DataFrame (might be override from parent facet or compile-time data)
let df = if let Some(override_df) = data_override {
    override_df  // Use filtered data from parent facet
} else {
    state
        .data
        .dataframe_with_context(ctx)
        .ok_or_else(|| {
            AvengerChartError::InternalError("Facet mark requires data".into())
        })?
};

// Extract facet expression
let facet_expr = state
    .data
    .channels()
    .get(DimConfig::channel_name())
    .and_then(|cv| cv.expr(ctx))
    .ok_or_else(|| {
        AvengerChartError::InternalError(
            format!("Facet '{}' channel not found", DimConfig::channel_name()).into(),
        )
    })?;

// Extract distinct keys from RENDER-TIME data (not compile-time!)
let domain_vals = FacetKeyExtractor::extract_keys(&df, &facet_expr).await?;
```

**Remove old logic** at lines 216-220:
```rust
// DELETE THIS:
let domain_vals: Vec<ScalarValue> = if let Some(keys) = facet_keys {
    keys.to_vec()
} else {
    initial_domain_vals.clone()
};
```

### Change 3: Use `data` parameter in facet `evaluate_from_data()` methods

#### For FacetRow (facet.rs:204-247):

**Before**:
```rust
async fn evaluate_from_data(
    &self,
    _data: Option<&RecordBatch>,  // ← IGNORED
    _scalars: &RecordBatch,
    context: &RenderContext,
    coord: Box<dyn CoordinateSystemTransform>,
) -> Result<(Vec<SceneMark>, LayoutUpdates), AvengerChartError> {
    evaluate_facet::<RowDimensionConfig>(
        coord.as_ref(),
        &self.compiled_subplot,
        &self.state,
        self.facet_title.clone(),
        self.facet_spacing,
        context,
        None,  // facet_keys
        // ... closures
    )
    .await
}
```

**After**:
```rust
async fn evaluate_from_data(
    &self,
    data: Option<&RecordBatch>,  // ← NOW USED! (no underscore)
    _scalars: &RecordBatch,
    context: &RenderContext,
    coord: Box<dyn CoordinateSystemTransform>,
) -> Result<(Vec<SceneMark>, LayoutUpdates), AvengerChartError> {
    use crate::facet::marks::facet_evaluation::batch_to_dataframe;

    // Convert RecordBatch to DataFrame if provided (from parent facet)
    let data_override = if let Some(batch) = data {
        Some(batch_to_dataframe(batch, &context.session_context)?)
    } else {
        None  // Will use compile-time data
    };

    evaluate_facet::<RowDimensionConfig>(
        coord.as_ref(),
        &self.compiled_subplot,
        &self.state,
        data_override,  // ← PASS OVERRIDE!
        self.facet_title.clone(),
        self.facet_spacing,
        context,
        // facet_keys parameter removed
        // ... closures
    )
    .await
}
```

**Same pattern applies to**:
- `CompiledFacetCol::evaluate_from_data()` (facet.rs:~370-410)

### Change 4: GridFacet special handling

GridFacet doesn't use `evaluate_facet()` - it has custom logic. Need to:

1. Accept and use `data` parameter
2. Extract row_keys and col_keys from render-time DataFrame using `FacetKeyExtractor::extract_keys()`
3. Replace `Vec::new()` placeholders (lines 827, 840) with extracted keys

---

## Critical Insight: Why This Works

The key realization is that **the plumbing already exists**:

1. ✅ `build_plot_components()` accepts `data_override: Option<&DataFrame>`
2. ✅ It creates RecordBatch from data_override
3. ✅ It passes RecordBatch to `mark.evaluate_from_data(data, ...)`
4. ✅ Nested facets receive the RecordBatch in their `evaluate_from_data(data, ...)` method

The ONLY missing pieces are:

1. ❌ Inner facets **ignore** the `data` parameter (have underscore prefix)
2. ❌ `evaluate_facet()` doesn't have `data_override` parameter
3. ❌ Keys are extracted at compile time instead of render time

**Phase 2 fixes all three issues** by:
1. Using the `data` parameter (remove underscore)
2. Adding `data_override` to `evaluate_facet()`
3. Extracting keys from `data_override` (or compile-time data if None)

---

## No Additional Plumbing Required

**Confirmed**: We do NOT need to:
- ❌ Modify `SubplotIterator` (it just provides FacetContext)
- ❌ Change `build_plot_components()` signature (already has data_override)
- ❌ Add new data passing mechanisms
- ❌ Modify how RecordBatch is created from DataFrame

**All we need**:
- ✅ Add data_override parameter to evaluate_facet()
- ✅ Use data parameter in facet evaluate_from_data() methods
- ✅ Extract keys at render time from correct DataFrame

---

## Nested Facet Flow After Phase 2

```
Outer FacetRow::evaluate_from_data(data=None)
  ↓
  data_override = None (use compile-time data)
  ↓
  evaluate_facet(data_override=None)
    ↓
    df = state.data.dataframe_with_context()  ← Full dataset
    domain_vals = extract_keys(&df, &row_expr)  ← Extract row keys
    ↓
    For each row value (e.g., Species="setosa"):
      ↓
      filter_df = df.filter(row == "setosa")  ← FILTERED TO SETOSA
      ↓
      compiled_subplot.build_plot_components(data_override=Some(&filter_df))
        ↓
        For marks including Inner FacetCol:
          ↓
          Create RecordBatch from filter_df (only setosa rows)
          ↓
          Inner FacetCol::evaluate_from_data(data=Some(setosa_batch))
            ↓
            data_override = Some(batch_to_dataframe(setosa_batch))  ← Convert!
            ↓
            evaluate_facet(data_override=Some(setosa_dataframe))
              ↓
              df = setosa_dataframe  ← USE OVERRIDE!
              domain_vals = extract_keys(&df, &col_expr)  ← Keys from setosa only!
              ↓
              For each column value within setosa:
                ↓
                filter_df = df.filter(col == value)  ← Doubly filtered
                ↓
                Render subplot with correct data
```

---

## Files That Need Changes

### 1. `facet_evaluation.rs`
- **Add import**: `use crate::facet::keys::FacetKeyExtractor;`
- **Modify signature** (line 77): Add `data_override: Option<DataFrame>` parameter
- **Remove parameter** (line 84): Delete `facet_keys: Option<&[ScalarValue]>`
- **Add logic** (after line 90): DataFrame selection and key extraction
- **Remove logic** (lines 216-220): Delete old facet_keys conditional

### 2. `facet.rs` - CompiledFacetRow
- **Modify signature** (line 206): Remove underscore from `_data` parameter
- **Add logic** (lines 211-218): Convert RecordBatch to DataFrame if provided
- **Modify call** (line 213): Pass `data_override` instead of `None`

### 3. `facet.rs` - CompiledFacetCol
- **Same changes as FacetRow** (lines ~370-410)

### 4. `facet.rs` - CompiledFacetGrid
- **Modify signature** (line ~768): Remove underscore from `_data` parameter
- **Add logic**: Convert RecordBatch to DataFrame if provided
- **Replace Vec::new()** (lines 827, 840): Use `FacetKeyExtractor::extract_keys()`

---

## Validation Plan

After implementing Phase 2 changes:

1. ✅ All existing facet tests should still pass (41 tests)
2. ✅ Remove `#[ignore]` from nested facet test
3. ✅ Nested facet test should pass without stack overflow
4. ✅ Keys extracted match the filtered data (not full dataset)

---

## Risk Assessment

| Risk | Likelihood | Impact | Mitigation |
|------|------------|--------|------------|
| Breaking existing tests | LOW | HIGH | All changes are additive (use data if provided, else use state.data) |
| Performance regression | LOW | LOW | Same filtering logic, just moved from compile to render time |
| Incorrect key extraction | LOW | MEDIUM | Use existing FacetKeyExtractor which is well-tested |
| RecordBatch conversion overhead | LOW | LOW | batch_to_dataframe() uses official DataFusion API |

**Overall Risk**: **LOW** - Changes are minimal and well-contained

---

## Conclusion

Phase 2 implementation is **straightforward** because:

1. ✅ The data flow mechanism already exists
2. ✅ We just need to USE the existing `data` parameter
3. ✅ Add one new parameter to `evaluate_facet()`
4. ✅ Extract keys at render time instead of compile time

**No architectural changes required** - just connecting existing pieces correctly.

**Estimated implementation time**: 2-3 hours for code changes + 1 hour for testing
