# Nested Faceting Implementation Roadmap

## Overview

This roadmap outlines the implementation of lazy compilation for nested facets in avenger-chart. The solution defers compilation of faceted subplots until evaluation time, when filtered data is available.

## Phase 1: Structural Foundation (Weeks 1-2)

### Goals
- Modify facet mark structures to support lazy compilation
- Detect nested facets at compile time
- Establish testing infrastructure

### Tasks

#### Task 1.1: Modify CompiledFacetCol Structure
**File**: `/Users/jonmmease/VegaFusion/repos/avenger/avenger-chart/src/facet/marks/facet.rs`

Changes:
```rust
pub struct CompiledFacetCol {
    pub(crate) state: CompiledMarkState,
    pub(crate) compiled_subplot: Arc<CompiledPlot>,
    pub(crate) uncompiled_subplot: Option<Arc<Plot<Cartesian>>>, // NEW
    pub(crate) facet_title: Option<String>,
    pub(crate) facet_spacing: Option<f32>,
    pub(crate) distinct_keys: Vec<ScalarValue>,
}
```

- Use `Arc` for both compiled and uncompiled subplots for consistency
- Make uncompiled plot optional (only stored if nesting detected)
- Add serialization support (Serde)

#### Task 1.2: Modify CompiledFacetRow Structure
**File**: Same as above

```rust
pub struct CompiledFacetRow {
    pub(crate) state: CompiledMarkState,
    pub(crate) compiled_subplot: Arc<CompiledPlot>,
    pub(crate) uncompiled_subplot: Option<Arc<Plot<Cartesian>>>, // NEW
    pub(crate) facet_title: Option<String>,
    pub(crate) facet_spacing: Option<f32>,
    pub(crate) distinct_keys: Vec<ScalarValue>,
}
```

#### Task 1.3: Implement Nesting Detection
**File**: Same as above (in `Mark::compile()` methods)

```rust
// In impl Mark<FacetColumn> for Facet<InnerC>
async fn compile(
    &self,
    compiled_state: CompiledMarkState,
    session_context: &SessionContext,
) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
    let inner_plot = self.subplot.as_ref().ok_or(...)?;
    
    // NEW: Detect if inner plot contains facet marks
    let inner_marks = &inner_plot.marks;
    let has_nested_facet = inner_marks.iter()
        .any(|m| m.mark_type().starts_with("facet_"));
    
    let uncompiled = if has_nested_facet {
        Some(Arc::new(inner_plot.clone()))
    } else {
        None
    };
    
    let compiled_subplot = Arc::new(inner_plot.clone().compile(session_context).await?);
    
    Ok(Arc::new(CompiledFacetCol {
        state: compiled_state,
        compiled_subplot,
        uncompiled_subplot: uncompiled,
        // ... rest
    }))
}
```

#### Task 1.4: Add Nesting Detection Tests
**File**: `/Users/jonmmease/VegaFusion/repos/avenger/avenger-chart/tests/`

Create `test_nested_facet_detection.rs`:
```rust
#[tokio::test]
async fn test_detects_nested_facet() {
    // Create FacetColumn with FacetRow inner subplot
    // Compile and verify uncompiled_subplot is Some
    // Verify non-nested facets have uncompiled_subplot as None
}

#[tokio::test]
async fn test_deeply_nested_facets() {
    // Create FacetColumn(FacetRow(FacetColumn(...)))
    // Verify each level detects nesting
}
```

#### Task 1.5: Update CompiledFacetGrid Structure
**File**: Same as facet.rs

```rust
pub struct CompiledFacetGrid {
    pub(crate) state: CompiledMarkState,
    pub(crate) compiled_subplot: Arc<CompiledPlot>,
    pub(crate) uncompiled_subplot: Option<Arc<Plot<Cartesian>>>, // NEW
    pub(crate) row_title: Option<String>,
    pub(crate) col_title: Option<String>,
    pub(crate) row_keys: Vec<ScalarValue>,
    pub(crate) col_keys: Vec<ScalarValue>,
    pub(crate) facet_spacing: Option<f32>,
}
```

### Deliverables
- [ ] Updated struct definitions for CompiledFacetCol/Row/Grid
- [ ] Nesting detection implemented in compile()
- [ ] Unit tests for nesting detection
- [ ] No changes to evaluate_from_data() yet

### Risk Assessment
- **Low Risk**: Mechanical struct changes
- **Testing**: Create targeted tests for detection
- **Backwards Compatibility**: No breaking changes

---

## Phase 2: Lazy Compilation Logic (Weeks 3-5)

### Goals
- Implement `compile_with_data()` method
- Add branching logic in evaluate_from_data()
- Ensure Pass 1 and Pass 2 consistency

### Tasks

#### Task 2.1: Add compile_with_data() Method
**File**: `/Users/jonmmease/VegaFusion/repos/avenger/avenger-chart/src/plot/plot.rs`

```rust
impl<C: CoordinateSystem + Clone> Plot<C> {
    /// Compile plot with specific DataFrame context
    /// 
    /// This allows nested facets to compile with filtered data
    /// available at evaluation time.
    pub async fn compile_with_data(
        mut self,
        df: &DataFrame,
        session_context: &SessionContext,
    ) -> Result<Arc<CompiledPlot>, AvengerChartError> {
        // Override plot's data with provided DataFrame
        self.data_source = Some(df.clone());
        
        // Proceed with normal compilation
        self.compile(session_context).await
    }
}
```

#### Task 2.2: Refactor evaluate_facet to Support Lazy Path
**File**: `/Users/jonmmease/VegaFusion/repos/avenger/avenger-chart/src/facet/marks/facet_evaluation.rs`

Add variant that handles lazy compilation:
```rust
pub async fn evaluate_facet_lazy<DimConfig: FacetDimensionConfig>(
    uncompiled_subplot: &Arc<Plot<InnerC>>,
    compiled_subplot: &Arc<CompiledPlot>,
    // ... other params matching evaluate_facet
    facet_coord: &dyn CoordinateSystemTransform,
    context: &RenderContext,
    // ... rest same as evaluate_facet
) -> Result<(Vec<SceneMark>, LayoutUpdates), AvengerChartError> {
    // Use existing evaluate_facet with branching for lazy path
    // ...
}
```

#### Task 2.3: Update CompiledFacetCol::evaluate_from_data()
**File**: `/Users/jonmmease/VegaFusion/repos/avenger/avenger-chart/src/facet/marks/facet.rs`

```rust
#[async_trait]
impl CompiledMark for CompiledFacetCol {
    async fn evaluate_from_data(
        &self,
        _data: Option<&RecordBatch>,
        _scalars: &RecordBatch,
        context: &RenderContext,
        coord: Box<dyn CoordinateSystemTransform>,
    ) -> Result<(Vec<SceneMark>, LayoutUpdates), AvengerChartError> {
        use crate::facet::marks::facet_evaluation::evaluate_facet;

        // Branch based on whether we have uncompiled subplot
        if let Some(uncompiled) = &self.uncompiled_subplot {
            // Lazy compilation path - implement new logic
            self.evaluate_with_lazy_compilation(
                uncompiled,
                coord,
                context,
            ).await
        } else {
            // Original path - single compilation, multiple evaluations
            evaluate_facet::<ColumnDimensionConfig>(
                coord.as_ref(),
                &self.compiled_subplot,
                &self.state,
                self.facet_title.clone(),
                self.facet_spacing,
                context,
                if self.distinct_keys.is_empty() {
                    None
                } else {
                    Some(&self.distinct_keys)
                },
                // ... closures unchanged
            ).await
        }
    }
}
```

#### Task 2.4: Implement Lazy Evaluation Helper
**File**: Same as above

```rust
impl CompiledFacetCol {
    async fn evaluate_with_lazy_compilation(
        &self,
        uncompiled_subplot: &Arc<Plot<Cartesian>>,
        coord: Box<dyn CoordinateSystemTransform>,
        context: &RenderContext,
    ) -> Result<(Vec<SceneMark>, LayoutUpdates), AvengerChartError> {
        let df = self.state.data
            .dataframe_with_context(&context.session_context)
            .ok_or_else(|| {
                AvengerChartError::InternalError(
                    "FacetCol requires data".into()
                )
            })?;

        // Get column expression for filtering
        let col_expr = self.state.data
            .channels()
            .get(ColumnDimensionConfig::channel_name())
            .and_then(|cv| cv.expr(&context.session_context))
            .ok_or_else(|| {
                AvengerChartError::InternalError(
                    "Facet 'column' channel not found".into()
                )
            })?;

        let mut all_marks = Vec::new();

        // Iterate through column values
        for col_value in &self.distinct_keys {
            // FILTER FIRST
            let filtered_df = df.clone()
                .filter(col_expr.clone().eq(lit(col_value.clone())))?;

            // THEN COMPILE with filtered data
            let compiled = uncompiled_subplot.clone()
                .compile_with_data(&filtered_df, &context.session_context)
                .await?;

            // THEN EVALUATE - Pass filtered data through context
            let (marks, _) = compiled.evaluate_from_data(
                None,  // Let inner marks extract from filtered_df
                _scalars,
                &context.with_filtered_data(filtered_df)?,
                coord.clone(),
            ).await?;

            all_marks.extend(marks);
        }

        Ok((all_marks, LayoutUpdates::default()))
    }
}
```

**Note**: May need to extend RenderContext with method to override data:
```rust
impl RenderContext {
    pub fn with_filtered_data(&self, df: DataFrame) -> Result<RenderContext, AvengerChartError> {
        let mut ctx = self.clone();
        ctx.override_dataframe = Some(Arc::new(df));
        Ok(ctx)
    }
}
```

#### Task 2.5: Do Same for CompiledFacetRow
**File**: Same file

Repeat Tasks 2.3-2.4 for `CompiledFacetRow::evaluate_from_data()` and `evaluate_with_lazy_compilation()`.

#### Task 2.6: Handle CompiledFacetGrid
**File**: Same file

CompiledFacetGrid is more complex (2D grid). For now:
- Store uncompiled_subplot
- Detect nesting
- For lazy path, it needs special handling for both row and col filtering

```rust
// Pseudocode for GridFacet lazy evaluation
if has_nested_facet {
    for row_value in &row_keys {
        for col_value in &col_keys {
            // Filter by BOTH row AND col
            let filtered = df
                .filter(row_expr.eq(row_value))?
                .filter(col_expr.eq(col_value))?;
            
            // Compile with doubly-filtered data
            let compiled = uncompiled.clone()
                .compile_with_data(&filtered, session_context)
                .await?;
            
            // Evaluate...
        }
    }
}
```

#### Task 2.7: Integration Tests for Lazy Compilation
**File**: `/Users/jonmmease/VegaFusion/repos/avenger/avenger-chart/tests/`

Create `test_nested_facet_lazy_compilation.rs`:
```rust
#[tokio::test]
async fn test_nested_facet_col_row_compilation() {
    // Create FacetColumn with FacetRow inner subplot
    // Compile plot
    // Verify compilation succeeds (no stack overflow)
    // Render and verify output
}

#[tokio::test]
async fn test_nested_facet_passes_filtered_data() {
    // Create nested facet
    // Verify inner facet receives filtered data in evaluate
    // Check that inner marks operate on correct subset
}

#[tokio::test]
async fn test_triple_nested_facets() {
    // FacetCol(FacetRow(FacetCol(...)))
    // Verify all levels compile and render correctly
}
```

### Deliverables
- [ ] `compile_with_data()` method implemented
- [ ] Branching logic in evaluate_from_data() for all facet types
- [ ] Lazy evaluation helper methods
- [ ] Two-pass consistency maintained
- [ ] Integration tests passing
- [ ] No regressions in single-level faceting

### Risk Assessment
- **Medium Risk**: Complex control flow and data threading
- **Testing**: Comprehensive integration tests required
- **Backwards Compatibility**: Existing faceting must continue to work

---

## Phase 3: Optimization (Weeks 6-7)

### Goals
- Parallelize nested facet compilation
- Implement result caching
- Profile and validate performance

### Tasks

#### Task 3.1: Implement Parallel Compilation
**File**: `/Users/jonmmease/VegaFusion/repos/avenger/avenger-chart/src/facet/marks/facet.rs`

```rust
use rayon::prelude::*;

async fn evaluate_with_lazy_compilation_parallel(
    &self,
    uncompiled_subplot: &Arc<Plot<Cartesian>>,
    // ...
) -> Result<(Vec<SceneMark>, LayoutUpdates), AvengerChartError> {
    // Use rayon for parallel compilation of filtered subplots
    let compiled_results: Vec<_> = self.distinct_keys
        .par_iter()
        .map(|col_value| {
            // Compile each variant in parallel
        })
        .collect::<Result<Vec<_>, _>>()?;
    
    // Combine results maintaining order
}
```

#### Task 3.2: Add Caching Layer
**File**: Consider new file `/Users/jonmmease/VegaFusion/repos/avenger/avenger-chart/src/facet/marks/facet_cache.rs`

```rust
pub struct CompiledFacetCache {
    // Cache compiled results by facet value combination
    // Key: (facet_col_value, facet_row_value) hash
    // Value: Arc<CompiledPlot>
}
```

#### Task 3.3: Performance Benchmarking
**File**: `/Users/jonmmease/VegaFusion/repos/avenger/avenger-chart/benches/`

Create `bench_nested_facet_performance.rs`:
```rust
// Benchmark:
// - Single-level facet (baseline)
// - 2-level nested facet
// - 3-level nested facet
// - Compare: lazy vs upfront compilation
```

### Deliverables
- [ ] Parallel compilation working
- [ ] Caching layer functional
- [ ] Performance benchmarks established
- [ ] Performance meets or exceeds baseline

### Risk Assessment
- **Medium Risk**: Parallelization adds complexity
- **Testing**: Benchmark regression detection needed
- **Backwards Compatibility**: Internal optimization, no breaking changes

---

## Phase 4: Documentation & Examples (Week 8)

### Goals
- Update architecture documentation
- Create example visualizations
- Document performance characteristics

### Tasks

#### Task 4.1: Update Architecture Docs
**File**: `/Users/jonmmease/VegaFusion/repos/avenger/avenger-chart/docs/`

Create/update `NESTED_FACETING.md`:
- Problem statement
- Lazy compilation approach
- Data flow diagrams
- API reference

#### Task 4.2: Add Example
**File**: `/Users/jonmmease/VegaFusion/repos/avenger/avenger-chart/examples/` or `tests/`

Create example showing nested faceting:
```rust
// Example: iris_nested_facet_demo.rs
// Facet iris dataset by Species columns, then PetalWidth bins by rows
```

#### Task 4.3: Performance Documentation
**File**: Same docs directory

Create `NESTED_FACETING_PERFORMANCE.md`:
- Compilation time comparisons
- Memory usage analysis
- Parallelization impact
- Optimization recommendations

### Deliverables
- [ ] Architecture documentation updated
- [ ] Working example provided
- [ ] Performance characteristics documented
- [ ] Migration guide for users

---

## Integration Checkpoints

### After Phase 1
- [ ] All struct changes compile
- [ ] Nesting detection tests pass
- [ ] No impact on existing code paths
- [ ] Build succeeds for all targets

### After Phase 2
- [ ] Nested facets compile without stack overflow
- [ ] Single-level faceting regressions: none
- [ ] Integration tests: all passing
- [ ] Layout/rendering output: visual inspection passed

### After Phase 3
- [ ] Performance benchmarks established
- [ ] No performance regressions
- [ ] Parallel compilation functional
- [ ] Cache effectiveness measured

### After Phase 4
- [ ] Examples render correctly
- [ ] Documentation complete
- [ ] User can understand and use nested faceting
- [ ] Release ready

---

## Success Criteria

1. **Functional**:
   - Nested FacetColumn with FacetRow works
   - Deeply nested facets (3+ levels) supported
   - No stack overflow or panics
   - Output matches expected visual results

2. **Performance**:
   - Nested faceting: acceptable runtime (< 2x single-level)
   - Single-level faceting: unchanged performance
   - Parallel compilation utilized when available

3. **Maintainability**:
   - Code is well-documented
   - Clear separation between lazy and eager paths
   - Tests cover all nesting scenarios

4. **Compatibility**:
   - Existing code unchanged
   - No breaking API changes
   - Can deploy in minor version update

---

## Risk Register

| Risk | Probability | Impact | Mitigation |
|------|-------------|--------|-----------|
| Compilation cost too high | Medium | High | Cache results, parallelize |
| Serialization issues | Medium | Medium | Test serialization early |
| Thread safety in parallel | Medium | High | Use Arc/Mutex correctly |
| Two-pass consistency | High | High | Rigorous testing, careful design |
| Performance regressions | Medium | Medium | Benchmark regression tests |

---

## Timeline Summary

```
Week 1-2:  Phase 1 - Structural foundation
Week 3-5:  Phase 2 - Lazy compilation logic  
Week 6-7:  Phase 3 - Optimization
Week 8:    Phase 4 - Documentation
          Release ready
```

Total effort: ~8 weeks for production-quality implementation.

