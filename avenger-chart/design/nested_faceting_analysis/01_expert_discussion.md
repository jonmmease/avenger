# Expert Discussion: Nested Faceting in Avenger-Chart

## Executive Summary

You've identified a fundamental architectural issue in avenger-chart's faceting system: **nested faceting causes stack overflow because each facet level operates on the full dataset independently, rather than receiving filtered data from its parent**.

An expert panel analysis recommends **Lazy Compilation for Nested Facets** as the optimal solution. This approach defers compilation of nested facets until evaluation time when filtered data is available, matching how production visualization libraries (Vega, D3) handle this problem.

---

## Problem Analysis

### Root Cause: Data Flow Mismatch

The current architecture has a fundamental contract violation:

**At Compile Time:**
```
Plot<FacetColumn>::new().data(df)
  └─> Facet::compile(df)
      └─> Creates Arc<CompiledPlot> for inner Plot<FacetRow>
          └─> Plot<FacetRow>::data(df)  ← STILL FULL DATASET!
              └─> Facet::compile(df)
                  └─> STACK OVERFLOW: Infinite recursion
```

**At Evaluation Time (if it got there):**
```
CompiledFacetCol::evaluate_from_data()
  └─> Filters df by col_value
      └─> Passes filtered_df to CompiledFacetRow::evaluate_from_data()
          └─> Filters filtered_df by row_value
              └─> Renders actual data
```

**The Mismatch:** Code compiled for `df` (full dataset) doesn't match the filtered data passed during evaluation.

### Current Architecture Issues

1. **Single Compilation Pass**: `Arc<CompiledPlot>` is created once at plot compile time
2. **Data Lost at Compile**: Inner Plot doesn't know about filtering constraints
3. **No Lazy Evaluation**: Nested facets can't defer decisions until they have filtered data
4. **Violates Separation of Concerns**: Compilation should not depend on runtime filter values

### Why Stack Overflow Occurs

With nested faceting, the compilation process becomes:
```
Plot<FacetCol>::compile()
  └─> subprocess.compile()  [Plot<FacetRow>]
      └─> subprocess.compile()  [Plot<FacetRow> again!]
          └─> subprocess.compile()  [infinite recursion]
              └─> STACK OVERFLOW
```

The subplot stores `data: df`, so compilation doesn't terminate.

---

## Recommended Solution: Lazy Compilation for Nested Facets

### Overview

**Defer compilation of nested facets until evaluation time when filtered data is available.**

This is how production visualization systems handle this:
- **Vega**: Each facet level has its own data flow and compilation
- **D3**: Data binding happens at visualization time
- **Observable**: Progressive data transformation through facet levels

### Key Principle

> **Compiled marks should always match the data they were compiled for**

This invariant prevents the stack overflow and makes the system predictable.

### Architecture

**Hybrid approach: Different paths for single-level vs nested faceting**

#### Single-Level Facets (Status Quo - Works Fine)
```
Plot<FacetColumn> with df
  └─> FacetColumn::compile(df)
      └─> Creates Arc<CompiledPlot> for inner Plot
          └─> evaluate_from_data() called N times (once per col value)
              └─> Each call filters data by col_value
                  └─> Compiles marks with filtered data on-demand
```

#### Nested Facets (Lazy Compilation - NEW)
```
Plot<FacetColumn> with df
  └─> FacetColumn::compile(df)
      └─> Detects nested facet in subplot
      └─> Stores UNCOMPILED Plot<FacetRow> in CompiledFacetCol
          └─> During evaluate_from_data():
              └─> For each col_value:
                  └─> Filter df by col_value → filtered_df
                  └─> Compile inner Plot<FacetRow> with filtered_df
                      └─> Creates CompiledFacetRow specific to this filtered_df
                      └─> evaluate_from_data() with filtered_df
                          └─> For each row_value:
                              └─> Filter filtered_df by row_value
                              └─> Render actual marks
```

### Implementation Outline

#### Phase 1: Structural Changes

1. **Store Optional Uncompiled Plot**
   ```rust
   pub struct CompiledFacetCol {
       pub(crate) state: CompiledMarkState,
       pub(crate) compiled_subplot: Arc<CompiledPlot>,
       pub(crate) uncompiled_subplot: Option<Plot<InnerC>>,  // ← NEW
       // ... existing fields
   }
   ```

2. **Detect Nesting at Compile Time**
   ```rust
   async fn compile(...) -> Result<Arc<dyn CompiledMark>, AvengerChartError> {
       // Check if inner plot has facet marks
       let has_nested_facet = subplot_marks
           .iter()
           .any(|m| m.mark_type().starts_with("facet"));

       if has_nested_facet {
           // Store uncompiled for lazy compilation later
           CompiledFacetCol {
               compiled_subplot,
               uncompiled_subplot: Some(inner_plot.clone()),  // Keep original
               // ...
           }
       } else {
           // Current path: single compilation
           CompiledFacetCol {
               compiled_subplot,
               uncompiled_subplot: None,
               // ...
           }
       }
   }
   ```

#### Phase 2: Lazy Compilation Logic

3. **Add Method to Compile with Filtered Data**
   ```rust
   // Extension trait or new method on Plot
   impl<C: CoordinateSystem> Plot<C> {
       async fn compile_with_data(
           self,
           df: &DataFrame,
           session_context: &SessionContext,
       ) -> Result<Arc<CompiledPlot>, AvengerChartError> {
           // Use filtered df in data context during compilation
           // Let marks compile against filtered schema
       }
   }
   ```

4. **Update evaluate_from_data() with Branching Logic**
   ```rust
   async fn evaluate_from_data(
       &self,
       _data: Option<&RecordBatch>,
       _scalars: &RecordBatch,
       context: &RenderContext,
       coord: Box<dyn CoordinateSystemTransform>,
   ) -> Result<(Vec<SceneMark>, LayoutUpdates), AvengerChartError> {
       let df = self.state.data.dataframe_with_context(&context.session_context)?;

       // Branch based on nesting
       if self.uncompiled_subplot.is_some() {
           // Lazy compilation path
           self.evaluate_with_lazy_compilation(
               df, context, coord, // ... params
           ).await
       } else {
           // Current path (single compilation + multiple evaluations)
           evaluate_facet::<ColumnDimensionConfig>(
               coord.as_ref(),
               &self.compiled_subplot,
               // ... existing parameters
           ).await
       }
   }
   ```

5. **Implement Lazy Evaluation**
   ```rust
   async fn evaluate_with_lazy_compilation(
       &self,
       df: DataFrame,
       context: &RenderContext,
       coord: Box<dyn CoordinateSystemTransform>,
       // ... other parameters
   ) -> Result<(Vec<SceneMark>, LayoutUpdates), AvengerChartError> {
       let mut all_marks = Vec::new();

       for col_value in &col_domain_vals {
           // FILTER FIRST
           let filtered_df = df.clone()
               .filter(col_expr.clone().eq(lit(col_value.clone())))?;

           // THEN COMPILE with filtered data
           let inner_plot = self.uncompiled_subplot.as_ref().unwrap();
           let compiled = inner_plot.clone()
               .compile_with_data(&filtered_df, session_context)
               .await?;

           // THEN EVALUATE with filtered data
           let (marks, _) = compiled.evaluate_from_data(
               Some(&filtered_batch),
               scalars,
               context,
               coord.clone(),
           ).await?;

           all_marks.extend(marks);
       }

       Ok((all_marks, LayoutUpdates::default()))
   }
   ```

#### Phase 3: Optimization (Later)

- **Caching**: Cache compiled nested plots by facet value combination
- **Parallelization**: Compile/evaluate multiple facet values in parallel
- **Memoization**: Reuse compilation results when possible

---

## Design Precedents from Production Visualization Libraries

### Vega
- **Facets are declarative specs** with their own data flow
- **Each level has explicit data transforms** before subdivision
- **Compilation happens post-transform** (bottom-up)
- **Supports arbitrary nesting** naturally
- Key learning: *Facets consume transformed data, not original*

### D3
- **No separate compilation phase**
- **Data binding at visualization time** (selection.data())
- **Progressive selection nesting** matches hierarchical data filtering
- **Supports arbitrary group nesting** (svg groups)
- Key learning: *Defer binding decisions until data context is clear*

### Observable/Vega-Lite
- **Declarative facet specs** with field references
- **Each facet level filters progressively**
- **Transformation order is explicit** in specification
- **Compiles to Vega marks** after all transformations
- Key learning: *Separate specification from data at compile time*

### Plotly
- **Subplots are independent plots**
- **Each subplot receives filtered data**
- **No special nesting syntax** (use subplot grid instead)
- **Works because data passing is explicit**
- Key learning: *Make data flow explicit in architecture*

---

## Why Lazy Compilation is the Best Approach

### Comparison of Solutions

| Aspect | Lazy Compilation | Data Threading | Template Compilation |
|--------|-----------------|-----------------|----------------------|
| **Handles Nesting** | ✅ Arbitrary depth | ⚠️ Requires context growth | ⚠️ Only specific depths |
| **Data Consistency** | ✅ Perfect match | ✅ Explicit passing | ❌ Deferred binding |
| **Compilation Cost** | ⚠️ Runtime cost | ✅ Upfront cost | ✅ Single pass |
| **Code Complexity** | ✅ Contained | ❌ Pervasive changes | ❌ Template machinery |
| **Performance** | ⚠️ Needs optimization | ✅ Predictable | ✅ Reusable |
| **Maintenance** | ✅ Clear logic flow | ❌ Context threading | ❌ Generic machinery |
| **Matches Industry** | ✅ Vega/D3 patterns | ❌ Custom approach | ❌ Custom approach |

### Specific Advantages

1. **Maintains Invariant**: Compiled marks match their data
2. **Handles Arbitrary Nesting**: Works for FacetCol(FacetRow(FacetCol(...)))
3. **Clear Separation of Concerns**:
   - Compile = structure + schema
   - Evaluate = data binding + rendering
4. **Gradual Migration**: Non-nested facets unchanged
5. **Industry Standard**: Matches how Vega, D3 solve this
6. **Testable**: Each level can be tested independently

---

## Risk Mitigation Strategy

### Risk 1: Compilation Cost Moves to Render Time
**Impact**: Potentially slower rendering
**Mitigation**:
- Parallelize compilation of multiple facet values
- Cache compiled nested plots
- Lazy compilation only when detecting nesting (most cases stay fast)
- Benchmark before/after to validate acceptable performance

### Risk 2: Breaking Changes to Compilation API
**Impact**: Affects existing code and plugins
**Mitigation**:
- Keep current single-level facet path unchanged
- Only use lazy compilation when nested facet detected
- New API is additive, not breaking
- Provide migration guide

### Risk 3: Memory Overhead from Keeping Uncompiled Plot
**Impact**: Slightly larger CompiledFacet structs
**Mitigation**:
- Only stored when nesting detected (opt-in)
- Arc<CompiledPlot> already stored anyway
- Uncompiled plot is lightweight before binding data

### Risk 4: Parallel Compilation Resource Usage
**Impact**: More threads during rendering
**Mitigation**:
- Use bounded thread pool
- Respect system CPU count
- Add config option to disable parallelization
- Start with sequential, optimize later

---

## Implementation Phases and Effort

### Phase 1: Structural Foundation (1-2 weeks)
- Modify CompiledFacet* to store optional uncompiled Plot
- Add detection logic for nested facets
- Create tests for nested detection
- **Risk**: Low, mostly mechanical changes
- **Benefit**: Clear path forward established

### Phase 2: Lazy Compilation Logic (2-3 weeks)
- Implement lazy evaluation branch
- Handle evaluation flow for nested marks
- Ensure consistency across Pass 1 & 2
- Add integration tests for nesting
- **Risk**: Medium, complex control flow
- **Benefit**: Core feature working (unoptimized)

### Phase 3: Optimization (1-2 weeks)
- Parallelize nested compilation
- Implement caching layer
- Performance benchmarking
- **Risk**: Medium, dependent on Phase 2
- **Benefit**: Production-ready performance

### Phase 4: Documentation & Examples
- Update architecture docs
- Add nested faceting examples
- Document performance characteristics
- Create migration guide

---

## Key Questions Answered

### 1. What's the best architectural approach?
**Lazy compilation of nested facets.** Defer compilation until filtered data available. Matches industry patterns.

### 2. Pass filtered DataFrame through context?
**No.** Instead, recompile with filtered DataFrame as input. Cleaner and more explicit.

### 3. Add data parameter to evaluation methods?
**Not needed.** Lazy compilation approach handles it naturally in evaluate_from_data().

### 4. How maintain consistency across passes?
**By design.** Each facet level compiles with exact data it will evaluate. Perfect consistency.

### 5. Precedents in Vega/D3?
**Yes.** Both support arbitrary nesting via progressive data transformation. Lazy compilation is the standard pattern.

---

## Next Steps

1. **Validate Approach with Team**: Review expert recommendations with avenger maintainers
2. **Design Detailed API**: Specify exact interface changes for Plot/CompiledPlot
3. **Prototype Phase 1**: Implement structural changes and nesting detection
4. **Add Integration Tests**: Build test suite for nested facets before Phase 2
5. **Iterate on Feedback**: Refine design based on implementation experience
6. **Performance Benchmarking**: Establish baseline and optimization targets

---

## Conclusion

The nested faceting problem reveals a deeper architectural tension between **early compilation** (current approach) and **late data binding** (needed for nesting). The solution is to adopt a **hybrid strategy**: keep early compilation for simple facets (fast path), add lazy compilation for nested facets (correct path).

This approach:
- ✅ Fixes the stack overflow issue
- ✅ Supports arbitrary nesting depth
- ✅ Maintains data/code consistency
- ✅ Aligns with industry standards
- ✅ Keeps non-nested performance unchanged
- ✅ Provides clear migration path

The expert consensus is that lazy compilation is not a hack or workaround—it's the correct architectural solution for faceted visualization systems.
