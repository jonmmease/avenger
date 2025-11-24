# Plan: Move Facet Calculations to Render Time

**Date**: 2025-11-23
**Status**: Draft for Expert Review
**Goal**: Fix fundamental architecture issue where facet calculations at compile time don't account for render-time data transformations

---

## Executive Summary

### Problem Statement

Currently, facet marks extract distinct keys and compile subplots at **compile time**, but render parameters can transform the DataFrame, meaning:
- Facet keys extracted at compile time may not exist at render time
- New facet keys may appear at render time that weren't present at compile time
- Nested facets cause infinite recursion because inner facets compile before data filtering

### Proposed Solution

Move all faceting logic to render time:
1. Remove `distinct_keys` from `CompiledFacet*` structs
2. Extract facet keys from the actual render-time DataFrame
3. Store uncompiled subplot and compile it at render time with filtered data
4. This naturally enables nested faceting as a side benefit

### Timeline Estimate

- **Phase 1** (Structure changes): 1-2 weeks
- **Phase 2** (Render-time logic): 2-3 weeks
- **Phase 3** (Optimization): 1-2 weeks
- **Phase 4** (Testing & docs): 1 week
- **Total**: 5-8 weeks

---

## Current Architecture Analysis

### Compile Time (facet.rs:146-176)

```rust
async fn compile(
    &self,
    compiled_state: CompiledMarkState,
    session_context: &SessionContext,
) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
    // 1. Compile subplot with full dataset
    let compiled_subplot: Arc<CompiledPlot> = {
        let plot_owned: Plot<InnerC> = Clone::clone(&plot_clone);
        Arc::new(plot_owned.compile(session_context).await?)  // ← PROBLEM: Compiles now
    };

    // 2. Extract distinct keys from compile-time DataFrame
    let distinct_keys = {
        let df = compiled_state.data.dataframe_with_context(session_context)...;
        FacetKeyExtractor::extract_keys(&df, &expr).await?  // ← PROBLEM: Keys from compile-time data
    };

    Ok(Arc::new(CompiledFacetRow {
        state: compiled_state,
        compiled_subplot,    // Compiled too early
        distinct_keys,       // Keys from wrong DataFrame
        // ...
    }))
}
```

### Render Time (facet_evaluation.rs:118-200)

```rust
pub async fn evaluate_facet<DimConfig: FacetDimensionConfig>(...) {
    // Get render-time DataFrame (potentially transformed by params)
    let df = state.data.dataframe_with_context(ctx).ok_or_else(...)?;

    // Extract domain from compile-time distinct_keys or scale
    let domain_vals: Vec<ScalarValue> = if let Some(keys) = facet_keys {
        keys.to_vec()  // ← PROBLEM: Using compile-time keys!
    } else {
        initial_domain_vals.clone()  // From scale, also compile-time
    };

    // Iterate using potentially stale keys
    for (iteration, rect) in subplot_iter.zip(initial_rects.iter()) {
        let filter_df: DataFrame = df
            .clone()
            .filter(facet_expr.clone().eq(lit(iteration.facet_value.clone())))?;

        // Filter might return empty DataFrame if key doesn't exist in render data!
        // Or we might miss new keys that appeared in render data!
    }
}
```

### Problems

1. **Render param mismatch**: Compile-time keys don't match render-time data
2. **Nested facets**: Inner facets compile before outer facets filter data → infinite recursion
3. **Wasted work**: Compiling subplot before knowing the actual data it will render
4. **Inflexibility**: Can't support dynamic faceting based on render params

---

## Proposed Architecture

### Key Principles

1. **Defer all faceting logic to render time** when we have the actual DataFrame
2. **Store uncompiled subplot** in CompiledFacet structs
3. **Compile-per-render** with the specific filtered data each subplot needs
4. **Extract keys from render-time DataFrame** to ensure consistency

### New Compile Time Behavior

```rust
async fn compile(
    &self,
    compiled_state: CompiledMarkState,
    session_context: &SessionContext,
) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
    // NEW: Store uncompiled subplot instead of compiling it
    let uncompiled_subplot: Arc<Plot<InnerC>> = {
        let plot_ref = self.subplot.as_ref().ok_or(...)?;
        Arc::new(plot_ref.clone())
    };

    // REMOVED: No distinct_keys extraction
    // REMOVED: No compiled_subplot

    Ok(Arc::new(CompiledFacetRow {
        state: compiled_state,
        uncompiled_subplot,  // NEW: Store for later compilation
        facet_title: self.facet_title.clone(),
        facet_spacing: self.facet_spacing,
    }))
}
```

### New Render Time Behavior

```rust
pub async fn evaluate_facet<DimConfig: FacetDimensionConfig>(
    uncompiled_subplot: &Arc<Plot<InnerC>>,  // NEW: Uncompiled subplot
    state: &CompiledMarkState,
    // ... other params
) -> Result<(Vec<SceneMark>, LayoutUpdates), AvengerChartError> {
    // Get render-time DataFrame
    let df = state.data.dataframe_with_context(ctx).ok_or_else(...)?;

    // Extract facet expression
    let facet_expr = state.data.channels()
        .get(DimConfig::channel_name())
        .and_then(|cv| cv.expr(ctx))
        .ok_or_else(...)?;

    // NEW: Extract distinct keys from RENDER-TIME DataFrame
    let domain_vals = FacetKeyExtractor::extract_keys(&df, &facet_expr).await?;

    // Build initial scales using extracted keys
    // ... scale building logic ...

    // PASS 1: Measurement
    for facet_value in &domain_vals {
        // Filter DataFrame for this facet
        let filter_df = df.clone()
            .filter(facet_expr.clone().eq(lit(facet_value.clone())))?;

        // NEW: Compile subplot with FILTERED data
        let compiled_subplot = uncompiled_subplot
            .clone()
            .with_data(filter_df.clone())  // NEW: Attach filtered data
            .compile(ctx)
            .await?;

        // Measure overflow with correctly compiled subplot
        let components = compiled_subplot
            .build_plot_components(..., Some(&filter_df), ...)
            .await?;

        overflow_measurements.push(components.overflow.unwrap_or_default());
    }

    // Calculate padding...
    // Rebuild scales with padding...

    // PASS 2: Rendering
    for facet_value in &domain_vals {
        let filter_df = df.clone()
            .filter(facet_expr.clone().eq(lit(facet_value.clone())))?;

        // NEW: Compile again with filtered data (or use cached from Pass 1)
        let compiled_subplot = uncompiled_subplot
            .clone()
            .with_data(filter_df.clone())
            .compile(ctx)
            .await?;

        // Render with correctly compiled subplot
        let components = compiled_subplot
            .build_plot_components(..., Some(&filter_df), ...)
            .await?;

        // Position and add marks...
    }
}
```

---

## Implementation Phases

### Phase 1: Structural Changes (1-2 weeks)

#### 1.1: Modify CompiledFacet Structs

**File**: `avenger-chart/src/facet/marks/facet.rs`

**Changes**:
```rust
// Before:
pub struct CompiledFacetRow {
    pub(crate) state: CompiledMarkState,
    pub(crate) compiled_subplot: Arc<CompiledPlot>,  // REMOVE
    pub(crate) distinct_keys: Vec<ScalarValue>,      // REMOVE
    pub(crate) facet_title: Option<String>,
    pub(crate) facet_spacing: Option<f32>,
}

// After:
pub struct CompiledFacetRow {
    pub(crate) state: CompiledMarkState,
    pub(crate) uncompiled_subplot: Arc<Plot<Cartesian>>,  // NEW
    pub(crate) facet_title: Option<String>,
    pub(crate) facet_spacing: Option<f32>,
}
```

Apply same changes to:
- `CompiledFacetCol`
- `CompiledFacetGrid`

#### 1.2: Update Compile Methods

**File**: `avenger-chart/src/facet/marks/facet.rs`

Remove:
- Subplot compilation logic
- Distinct key extraction
- `FacetKeyExtractor::extract_keys()` calls at compile time

Add:
- Store uncompiled subplot as `Arc<Plot<InnerC>>`

**Estimated changes**: ~100 lines modified across 3 compile methods

#### 1.3: Add Plot::with_data() Method

**File**: `avenger-chart/src/plot/plot.rs`

**New method**:
```rust
impl<C: CoordinateSystem + Clone> Plot<C> {
    /// Attach a DataFrame to this plot for compilation
    ///
    /// This is used by facets to compile subplots with filtered data.
    pub fn with_data(mut self, df: DataFrame) -> Self {
        // TODO: How to attach DataFrame to Plot?
        // Options:
        // 1. Add optional DataFrame field to Plot struct
        // 2. Modify Data enum to support DataFrame directly
        // 3. Other approach?
        self
    }
}
```

**QUESTION FOR EXPERTS**: What's the best way to attach a DataFrame to a Plot for compilation?

#### 1.4: Update Serialization

**File**: `avenger-chart/src/facet/marks/facet.rs`

- Update Serde derives to handle `Arc<Plot<InnerC>>`
- Test serialization/deserialization of uncompiled subplots
- Ensure backward compatibility if needed

**Deliverables**:
- [ ] Modified struct definitions
- [ ] Updated compile methods (no compilation, no key extraction)
- [ ] `Plot::with_data()` method implemented
- [ ] Serialization tests pass

---

### Phase 2: Render-Time Logic (2-3 weeks)

#### 2.1: Update evaluate_facet Signature

**File**: `avenger-chart/src/facet/marks/facet_evaluation.rs`

**Current**:
```rust
pub async fn evaluate_facet<DimConfig: FacetDimensionConfig>(
    facet_coord: &dyn CoordinateSystemTransform,
    compiled_subplot: &Arc<CompiledPlot>,  // CHANGE THIS
    state: &CompiledMarkState,
    _facet_title: Option<String>,
    facet_spacing: Option<f32>,
    context: &RenderContext,
    facet_keys: Option<&[ScalarValue]>,    // REMOVE THIS
    // ...
) -> Result<(Vec<SceneMark>, LayoutUpdates), AvengerChartError>
```

**New**:
```rust
pub async fn evaluate_facet<DimConfig: FacetDimensionConfig, InnerC: CoordinateSystem>(
    facet_coord: &dyn CoordinateSystemTransform,
    uncompiled_subplot: &Arc<Plot<InnerC>>,  // CHANGED
    state: &CompiledMarkState,
    _facet_title: Option<String>,
    facet_spacing: Option<f32>,
    context: &RenderContext,
    // facet_keys REMOVED
    // ...
) -> Result<(Vec<SceneMark>, LayoutUpdates), AvengerChartError>
```

#### 2.2: Extract Keys at Render Time

**File**: `avenger-chart/src/facet/marks/facet_evaluation.rs`

Add at start of `evaluate_facet`:
```rust
// Get render-time DataFrame
let df = state.data.dataframe_with_context(ctx).ok_or_else(|| {
    AvengerChartError::InternalError("Facet mark requires plot or mark data".into())
})?;

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

// NEW: Extract distinct keys from render-time DataFrame
let domain_vals = FacetKeyExtractor::extract_keys(&df, &facet_expr).await?;
```

#### 2.3: Implement Lazy Compilation in Pass 1

**File**: `avenger-chart/src/facet/marks/facet_evaluation.rs`

In the Pass 1 measurement loop:
```rust
for facet_value in &domain_vals {
    // Filter DataFrame
    let filter_df: DataFrame = df
        .clone()
        .filter(facet_expr.clone().eq(lit(facet_value.clone())))?;

    // NEW: Compile subplot with filtered data
    let compiled_subplot = compile_subplot_with_data(
        uncompiled_subplot,
        &filter_df,
        ctx,
        &iteration.params,
    ).await?;

    let (width, height) = subplot_dims(band_size, context);

    // Build scales for this compiled subplot
    let scales = build_scales_helper(
        &compiled_subplot,  // Use freshly compiled subplot
        &initial_shared_scales,
        &free_scale_builder_pass1,
        &filter_df,
        width,
        height,
        ctx,
        &iteration.params,
    ).await?;

    // Measure overflow
    let components = compiled_subplot
        .build_plot_components(
            width,
            height,
            ctx,
            &iteration.params,
            &scale_provider,
            EvaluationMode::Measure,
            Some(&filter_df),
            true,
        )
        .await?;

    overflow_measurements.push(components.overflow.unwrap_or_default());
}
```

#### 2.4: Implement Lazy Compilation in Pass 2

Similar logic for Pass 2 rendering loop.

#### 2.5: Add Compilation Helper

**File**: `avenger-chart/src/facet/marks/facet_evaluation.rs`

```rust
/// Compile a subplot with filtered data
async fn compile_subplot_with_data<C: CoordinateSystem>(
    uncompiled: &Arc<Plot<C>>,
    df: &DataFrame,
    ctx: &SessionContext,
    params: &ChannelParams,
) -> Result<Arc<CompiledPlot>, AvengerChartError> {
    let plot_with_data = uncompiled
        .as_ref()
        .clone()
        .with_data(df.clone());

    Ok(Arc::new(plot_with_data.compile(ctx).await?))
}
```

#### 2.6: Update GridFacet evaluate_from_data

**File**: `avenger-chart/src/facet/marks/facet.rs`

GridFacet needs similar changes:
- Extract row and column keys from render-time DataFrame
- Compile subplot for each grid cell with doubly-filtered data
- Update measure_grid_overflow to accept uncompiled subplot

**Deliverables**:
- [ ] Updated `evaluate_facet` signature
- [ ] Render-time key extraction
- [ ] Lazy compilation in Pass 1 and Pass 2
- [ ] Compilation helper function
- [ ] GridFacet updated

---

### Phase 3: Optimization (1-2 weeks)

#### 3.1: Add Compilation Caching

**Problem**: Compiling the same subplot multiple times (once per facet value) is expensive.

**Solution**: Cache compiled subplots by a key that includes:
- Filtered DataFrame identity/hash
- Render params
- Session context

**File**: `avenger-chart/src/facet/marks/facet_evaluation.rs`

```rust
use std::collections::HashMap;

// Cache compiled subplots within a single evaluate_facet call
let mut subplot_cache: HashMap<SubplotCacheKey, Arc<CompiledPlot>> = HashMap::new();

struct SubplotCacheKey {
    facet_value: ScalarValue,
    // Additional fields as needed for cache key
}

async fn get_or_compile_subplot<C: CoordinateSystem>(
    cache: &mut HashMap<SubplotCacheKey, Arc<CompiledPlot>>,
    key: SubplotCacheKey,
    uncompiled: &Arc<Plot<C>>,
    df: &DataFrame,
    ctx: &SessionContext,
    params: &ChannelParams,
) -> Result<Arc<CompiledPlot>, AvengerChartError> {
    if let Some(compiled) = cache.get(&key) {
        return Ok(Arc::clone(compiled));
    }

    let compiled = compile_subplot_with_data(uncompiled, df, ctx, params).await?;
    cache.insert(key, Arc::clone(&compiled));
    Ok(compiled)
}
```

**Note**: Cache is scoped to a single `evaluate_facet` call, so Pass 1 and Pass 2 can reuse compilations.

#### 3.2: Parallelize Compilation

**Problem**: Compiling subplots for each facet value is independent work.

**Solution**: Use `tokio::spawn` or `futures::join_all` to compile in parallel.

```rust
use futures::future::join_all;

// Pass 1: Parallel compilation and measurement
let measurement_futures = domain_vals.iter().map(|facet_value| {
    let filter_df_future = df.clone()
        .filter(facet_expr.clone().eq(lit(facet_value.clone())));

    async move {
        let filter_df = filter_df_future?;
        let compiled = compile_subplot_with_data(...).await?;
        // Measure and return overflow
        Ok(overflow)
    }
});

let overflow_measurements = join_all(measurement_futures).await;
```

**QUESTION FOR EXPERTS**: Is parallel compilation safe given SessionContext usage?

#### 3.3: Benchmark Performance

**File**: `avenger-chart/benches/facet_compilation.rs` (new)

Create benchmarks comparing:
- Old: Compile-time compilation + render-time evaluation
- New: Render-time compilation + evaluation
- New + caching
- New + caching + parallelization

**Target**: Render-time compilation should be within 2x of compile-time for non-nested facets.

**Deliverables**:
- [ ] Compilation caching implemented
- [ ] Parallel compilation implemented
- [ ] Performance benchmarks
- [ ] Performance acceptable (<2x regression for non-nested)

---

### Phase 4: Testing & Documentation (1 week)

#### 4.1: Update Tests

**Files**: `avenger-chart/tests/visual_tests/test_facet_*.rs`

- Verify all existing facet tests still pass
- Add tests with render params that transform data
- Add nested faceting tests (previously ignored)
- Add tests for edge cases:
  - Empty facets (no data for a key)
  - Single facet (only one key)
  - Many facets (100+ keys for performance)

#### 4.2: Add Integration Tests

**File**: `avenger-chart/tests/render_param_facets.rs` (new)

```rust
#[tokio::test]
async fn test_facet_with_filter_param() {
    // Create faceted plot
    // Apply render param that filters data
    // Verify facets only show for filtered data, not compile-time data
}

#[tokio::test]
async fn test_nested_facets_work() {
    // FacetColumn with FacetRow inside
    // Verify no stack overflow
    // Verify correct rendering
}
```

#### 4.3: Update Documentation

**Files**:
- `avenger-chart/docs/FACETING.md` - Architecture overview
- `avenger-chart/docs/DEBUGGING.md` - Update debugging info
- `avenger-chart/tasks/review_facet/README.md` - Update review guide

**Content**:
- Explain render-time compilation rationale
- Document performance characteristics
- Add examples of nested faceting
- Explain when compilation caching helps

#### 4.4: Update Comments

Update inline documentation in:
- `facet.rs` - Explain new compile-time behavior
- `facet_evaluation.rs` - Explain render-time key extraction and compilation

**Deliverables**:
- [ ] All existing tests pass
- [ ] New render param tests
- [ ] Nested faceting tests enabled and passing
- [ ] Documentation updated
- [ ] Code comments updated

---

## Critical Questions for Expert Review

### 1. Plot::with_data() Implementation

**Question**: What's the best architectural approach to attach a DataFrame to a Plot before compilation?

**Options**:
- **A**: Add `data: Option<DataFrame>` field to Plot struct
- **B**: Modify the `Data` enum to support DataFrame directly
- **C**: Pass DataFrame through compile() method signature
- **D**: Other approach?

**Considerations**:
- Need to ensure DataFrame is available during mark compilation
- Should work with existing data sources (table references, inline data)
- Serialization implications

### 2. Parallelization Safety

**Question**: Is it safe to compile multiple subplots in parallel given SessionContext usage?

**Considerations**:
- SessionContext is `&` reference, might be safe if read-only
- Plot compilation calls async methods on SessionContext
- Need to verify no shared mutable state

### 3. Caching Strategy

**Question**: Should subplot compilation cache persist across multiple render calls, or scope to single evaluate_facet?

**Options**:
- **A**: Cache within evaluate_facet only (simpler, no invalidation needed)
- **B**: Cache across renders (better performance, but need invalidation)

**Considerations**:
- Render params can change between renders
- DataFrame can change between renders
- Cache invalidation complexity

### 4. Backward Compatibility

**Question**: Do we need to maintain backward compatibility with serialized CompiledFacet structs?

**Considerations**:
- CompiledFacet is serializable (Serde derives)
- Removing fields is breaking change
- Might need migration path or version handling

### 5. GridFacet Compilation

**Question**: For GridFacet, should we compile once per row, once per column, or once per cell?

**Options**:
- **A**: Once per cell (most accurate, most expensive)
- **B**: Once per row (assumes columns have same structure)
- **C**: Once per column (assumes rows have same structure)

**Considerations**:
- GridFacet filters by both row AND column
- Different cells might have very different data
- Performance vs accuracy tradeoff

---

## Risk Assessment

### High Risk
- **Performance regression**: Render-time compilation slower than compile-time
  - **Mitigation**: Caching + parallelization + benchmarking

- **API breaking changes**: Removing fields from CompiledFacet structs
  - **Mitigation**: Check if these are public APIs, plan migration if needed

### Medium Risk
- **Plot::with_data() complexity**: Attaching DataFrame to Plot might be non-trivial
  - **Mitigation**: Expert review of approach before implementation

- **SessionContext thread safety**: Parallel compilation might have issues
  - **Mitigation**: Expert review + testing

### Low Risk
- **Test coverage**: Existing tests might not catch edge cases
  - **Mitigation**: Comprehensive test suite in Phase 4

---

## Success Criteria

### Must Have
- [ ] All existing visual tests pass
- [ ] Facets correctly reflect render-time data (not compile-time)
- [ ] Nested faceting works without stack overflow
- [ ] No regression >2x on non-nested facet performance

### Should Have
- [ ] Compilation caching reduces redundant work
- [ ] Parallel compilation improves performance on multi-core
- [ ] Clear documentation of new architecture

### Nice to Have
- [ ] Performance better than current implementation for some cases
- [ ] Backward compatibility maintained

---

## Open Questions

1. How to implement `Plot::with_data()`? (See Critical Question 1)
2. Is parallel compilation safe? (See Critical Question 2)
3. Should caching persist across renders? (See Critical Question 3)
4. Backward compatibility requirements? (See Critical Question 4)
5. GridFacet compilation granularity? (See Critical Question 5)

---

## Next Steps

1. **Expert Review**: Get feedback from GPT-5 Codex and Gemini on:
   - Feasibility of the approach
   - Answers to critical questions
   - Risk assessment accuracy
   - Timeline estimates

2. **Revise Plan**: Incorporate expert feedback

3. **User Approval**: Present final plan for implementation decision

4. **Begin Implementation**: Start with Phase 1 if approved
