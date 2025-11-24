# Nested Facets Implementation Status

**Date**: 2025-11-24
**Status**: Implementation incomplete - architectural deadlock identified

## The Challenge

We are implementing nested faceting in avenger-chart, where one facet type (e.g., FacetRow) can be nested inside another (e.g., FacetColumn). The goal is to enable multi-level data visualization where:

```rust
Plot::<FacetColumn>::new().data(df.clone())  // Outer: facets by species
  .mark(Facet::new()
    .col_with(col("species"), ...)
    .subplot(
      Plot::<FacetRow>::new()  // Middle: facets by petal_width_bin (NO .data() call)
        .mark(Facet::new()
          .row_with(col("petal_width_bin"), ...)
          .subplot(
            Plot::<Cartesian>::new()  // Inner: scatter plot (receives filtered data)
              .mark(Symbol::new()...)
          )
        )
    )
  )
```

Data flows from parent to child via the `data_override` mechanism:
- **Parent facet**: Has compiled data (logical plan)
- **Child facet**: Has NO compiled data (prevents compilation recursion)
- **Runtime**: Parent filters data and passes to child via `data_override` parameter

### The Architectural Deadlock

The two-pass rendering system creates a fundamental incompatibility with nested facets:

1. **Pass 1 (Measure mode)**: Needs overflow measurements to compute layout
2. **Guides**: Need data to extract facet keys and compute accurate overflow
3. **Nested facets**: Have NO compiled data (`logical_plan: None` in `CompiledDataContext`)
4. **Result**: Either stack overflow (infinite recursion) or "Facet guide could not access data" error

#### What "Compiled Data" Actually Means

From `src/marks/compiled_data_context.rs`:

```rust
pub struct CompiledDataContext {
    logical_plan: Option<LogicalPlanNode>,  // ← Serialized DataFusion query plan
    channels: IndexMap<String, ChannelValue>,
}

pub fn dataframe_with_context(&self, ctx: &SessionContext) -> Option<DataFrame> {
    self.logical_plan.as_ref().and_then(|node| {  // ← Returns None for nested facets!
        node.to_logical_plan(ctx)
            .ok()
            .map(|plan| DataFrame::new(ctx.state().clone(), plan))
    })
}
```

**Key Insight**: Guides were designed to access a `LogicalPlanNode` (serialized query plan) stored during compilation. For nested facets, this is intentionally `None` to prevent infinite recursion during compilation. This means guides have NO data to work with.

## What's Been Implemented

Following recommendations from expert panel review, we implemented a three-part fix:

### 1. Detection Method: `has_facet_guide()`

**File**: `src/plot/compiled/mod.rs` (lines 193-211)

```rust
/// Check if this plot has a facet guide (FacetRow, FacetCol, or GridFacet)
pub(crate) fn has_facet_guide(&self) -> bool {
    if let Some(guide) = &self.compiled_guide {
        let type_name = std::any::type_name_of_val(guide.as_ref());
        type_name.contains("FacetRowGuide")
            || type_name.contains("FacetColGuide")
            || type_name.contains("GridFacetGuide")
    } else {
        false
    }
}
```

**Purpose**: Detect if a subplot is itself a facet plot, to avoid recursive measurement.

**Limitation**: Only checks if the *immediate* subplot has a facet guide. Doesn't detect deeper nesting patterns.

### 2. Pass 1 Modification: Skip Recursive Measurement

**File**: `src/facet/marks/facet_evaluation.rs` (lines 323-354)

```rust
let overflow = if compiled_subplot.has_facet_guide() {
    // Use estimated overflow for nested facets (20-40px is typical for axis labels)
    use crate::guide::OverflowSpaceRequirement;
    OverflowSpaceRequirement {
        top: 30.0,
        bottom: 30.0,
        left: 40.0,
        right: 20.0,
    }
} else {
    // Normal measurement for leaf plots
    let components = compiled_subplot
        .build_plot_components(
            width,
            height,
            ctx,
            &iteration.params,
            &scale_provider,
            crate::plot::compiled::EvaluationMode::Measure,
            Some(&filter_df),  // ← Pass filtered data to leaf plots
            true,
        )
        .await?;
    components.overflow.unwrap_or_default()
};
```

**Purpose**: During Pass 1, skip calling `build_plot_components` on nested facet subplots to prevent recursion.

**Issue**: The recursion happens through a different path. The inner FacetRow's subplot is Cartesian (not a facet), so `has_facet_guide()` returns false and allows the recursive call.

### 3. Fallback Estimates: Replace Zeros

**File**: `src/facet/guide.rs` (lines 98-101, 594-597)

```rust
} else {
    // No cached overflow available - use reasonable estimates
    // This happens when subplot is a nested facet (has_facet_guide() returns true)
    // Pass 1 skips recursive measurement to avoid stack overflow
    return Ok((30.0, 30.0, 40.0, 20.0));
}
```

**Purpose**: Instead of returning zero overflow when data is unavailable, return reasonable estimates (20-40px for typical axis labels).

**Current Status**: These estimates are used when the check succeeds, but don't help when recursion still occurs.

## Where Things Stand

### Current Test Result

Running the nested facet test:

```bash
cargo test test_col_with_nested_row -- --nocapture
```

**Result**: Stack overflow (thread 'test_col_with_nested_row' has overflowed its stack)

### Root Cause Analysis (from GPT-5 Codex)

The Codex agent analyzed the recursion path and identified:

**File**: `/tmp/codex_recursion_analysis_summary.md`

**Key Finding**: The recursion path is:

```
1. Outer FacetColumn evaluation → Pass 1 starts

2. facet_evaluation.rs:324 - Pass 1 calls build_plot_components on FacetRow subplot

3. rendering.rs:1152 - build_plot_components entry
   → rendering.rs:1220-1229 - compute_layout called
   → compute_layout passes None for data_override (line 1227)

4. Layout computation triggers guide.measure_overflow

5. For nested facets with no compiled data:
   - Guide cannot access data (logical_plan: None)
   - Guide fallback path may try to remeasure

6. Inner FacetRow's Pass 1 starts
   → Calls build_plot_components on Cartesian subplot
   → Should terminate (Cartesian doesn't recurse)

7. ISSUE: If subplot is also a facet, cycle repeats → Stack overflow
```

**Critical Finding from rendering.rs:1227**:

```rust
None, // Don't pass data_override during measurement - guides use compiled data
```

This comment reveals the architectural assumption: guides are designed to use compiled data (logical plans) during measurement. But nested facets have NO compiled data!

### The Data Flow Problem

**For Marks** (working):
- Marks receive runtime data via `data_override` parameter during Pass 2
- Nested facet marks successfully receive filtered data from parent

**For Guides** (broken):
- Guides extract data during `set_compiled_marks()` phase (compilation time)
- Guides expect `CompiledDataContext` to contain a `LogicalPlanNode`
- Nested facets have `logical_plan: None` by design
- Result: Guides cannot compute overflow for nested facets

### Why the Current Fix Doesn't Work

The `has_facet_guide()` check looks at the wrong level:

```rust
// facet_evaluation.rs:323
if compiled_subplot.has_facet_guide() {
    // Return estimated overflow
} else {
    // Call build_plot_components  ← This is where recursion happens!
}
```

**For our nested structure**:
- Outer FacetColumn checks if FacetRow `has_facet_guide()` → **true** ✓ (works)
- Inner FacetRow checks if Cartesian `has_facet_guide()` → **false** ✗ (allows recursion)

The inner FacetRow still calls `build_plot_components`, which triggers its own Pass 1, which checks for overflow, which tries to access data that doesn't exist.

## Recommendations from Codex Analysis

The Codex agent suggested examining:

1. **Guide Measurement Redesign**: Whether guides should defer measurement until after Pass 1 completes and overflow data is available

2. **Overflow Aggregation**: Whether the `overflow` parameter populated by Pass 1 should be used by guides instead of remeasuring

3. **Nested Facet Special Handling**: Whether nested facet plots should behave differently during measurement phase

4. **Data Flow Clarification**: How to provide filtered data to guides without triggering `build_plot_components` recursion

## Possible Solutions

### Option 1: Pass Overflow Up the Chain

Instead of guides computing their own overflow during measurement, use the overflow computed by Pass 1:

```rust
// In facet guide measure_overflow
if let Some(cached_overflow) = row_overflow {
    // Use overflow data that was already computed
    return Ok(aggregate_overflow(cached_overflow));
}
```

**Challenge**: Requires threading overflow data through guide measurement.

### Option 2: Provide Runtime Data to Guides

During Pass 1, pass `data_override` to guides so they can access filtered data:

```rust
// In rendering.rs:1227
Some(&filter_df), // Pass filtered data to guides during measurement
```

**Challenge**: This is the opposite of the current design assumption. Would need to verify guides can handle runtime data.

### Option 3: Multi-Pass Measurement for Nested Facets

Add a Pass 0 that measures leaf plots first, then uses those measurements for nested facet layout:

```
Pass 0: Measure leaf plots (Cartesian, Polar, etc.)
Pass 1: Use Pass 0 results to measure facet guides
Pass 2: Render final scene graph
```

**Challenge**: Significant architectural change to the two-pass system.

### Option 4: Detect Nested Facets During Compilation

Mark nested facets during compilation so guides know not to expect data:

```rust
pub struct CompiledPlot {
    is_nested_facet: bool,  // Set to true for subplots without data
    // ...
}
```

Then in guide measurement:
```rust
if compiled_subplot.is_nested_facet {
    // Use different measurement strategy
}
```

**Challenge**: Still need to define what the "different measurement strategy" is.

## Next Steps

1. **Review Codex full output**: The complete analysis may contain additional insights about the call graph

2. **Experiment with data_override in Pass 1**: Try passing filtered data to guides during measurement phase

3. **Consider guide interface redesign**: Add `measure_overflow_nested()` method that accepts runtime data

4. **Test simpler nested structure**: Try nesting Cartesian inside FacetRow to isolate the measurement issue

5. **Add instrumentation**: Add logging to trace exactly where the recursion occurs in the current implementation

## References

- **Test case**: `tests/visual_tests/test_nested_facets.rs`
- **Codex analysis**: `/tmp/codex_recursion_analysis_summary.md`
- **Key files**:
  - `src/facet/marks/facet_evaluation.rs` (Pass 1 measurement)
  - `src/plot/compiled/rendering.rs` (build_plot_components)
  - `src/facet/guide.rs` (facet guide measurement)
  - `src/marks/compiled_data_context.rs` (data storage structure)
  - `src/guide/coordinate_guide.rs` (guide trait definition)

## Conclusion

The nested facets implementation has uncovered a fundamental architectural constraint in the two-pass rendering system. The current design assumes guides have access to compiled data (logical plans), but nested facets intentionally have no compiled data to prevent compilation recursion. Resolving this will require either:

- Redesigning guide measurement to use runtime data instead of compiled data
- Using cached overflow results instead of recomputing during measurement
- Adding a multi-pass measurement strategy for nested structures

The three-part fix we implemented addresses surface-level recursion but doesn't solve the underlying data flow problem that prevents guides from functioning for nested facets.
