# Nested Facet Recursion Analysis

## 1. Recursion Path

The infinite recursion (Stack Overflow) occurs in the measurement phase (Pass 1) of nested facet evaluation.

**Cycle:**
1. `evaluate_facet` (Outer Facet, Pass 1) calls `compiled_subplot.build_plot_components(...)` (Measurement Mode).
2. `build_plot_components` calls `self.compute_layout(...)`.
3. `compute_layout` calls `guide.measure_overflow(..., overflow=None)`.
4. `FacetGuide::measure_overflow` (for Inner Facet) attempts to measure subplots to determine axis space.
   - **Recursive Step:** It iterates through subplots and calls `subplot.build_plot_components(...)` to measure them.
5. `subplot.build_plot_components` (Inner Facet) calls `self.compute_layout(...)`.
6. `compute_layout` calls `guide.measure_overflow(...)`.
7. If the Inner Facet's subplot is *also* a facet (or if the logic doesn't distinguish leaf plots correctly), the cycle repeats.

**Correction:**
In the provided codebase snapshot, `FacetRowGuide::measure_overflow` was modified to return a fixed estimate `(30.0, 30.0, 40.0, 20.0)` when `overflow` is `None`. This *should* break the recursion at step 4 by avoiding the call to `build_plot_components`.

However, the user reported recursion. My trace revealed that `evaluate_facet.rs` was actually using `measure_with_scales` which calls `compute_layout_with_fixed_plot_area`. This path *also* calls `measure_overflow`.

## 2. Why `has_facet_guide()` didn't fix it

The `has_facet_guide()` check at line 325 of `facet_evaluation.rs` (seen in `git diff`) was intended to skip recursive measurement:

```rust
let overflow = if compiled_subplot.has_facet_guide() {
    // Use estimated overflow
    OverflowSpaceRequirement { top: 30.0, ... }
} else {
    compiled_subplot.build_plot_components(...)
}
```

If this check were effective, it would stop recursion at step 1.

**Failure Analysis:**
The recursion trace I observed actually showed that the recursion *was* broken (the test progressed to Pass 2 and then crashed with a Panic, not Stack Overflow, in the final runs). The initial "Stack Overflow" reported by the user (and seen in my early traces) was likely due to a different issue or state:

1. **Scale Ticks Recursion:** My investigation hints that stack overflow might have occurred in `avenger-scales` or `avenger-guides` during `scale.ticks()` or `make_numeric_axis_marks` when the plot width was 0 (caused by the layout issues from the "fix").
2. **Layout Issue:** The "fix" (fixed overflow estimate) caused the calculated `bandwidth` for the outer facet to be 0 (or negative/invalid). This propagated to the Leaf plot, causing `width=0`.
3. **Panic:** The panic `ScaleError(DomainFromPaddingError(InvalidInput("Screen width must be positive")))` confirms that the root cause of the final crash is invalid layout dimensions, not infinite recursion of `evaluate_facet`.

## 3. Call Graph (Recursion Broken but Layout Broken)

1. `evaluate_facet` (Column Facet)
2. `measure_with_scales` (Inner Row Facet)
   - Calls `compute_layout_with_fixed_plot_area`
   - Calls `FacetRowGuide::measure_overflow(overflow=None)`
   - **Returns Fixed Estimate** (Recursion Broken)
   - Returns `layout.overflow`
3. `evaluate_facet` (Column Facet) continues Pass 1 -> calculates `rounded_gap` -> `padding_inner_px`.
   - **Issue:** The calculated padding/spacing results in `bandwidth = 0`.
4. `evaluate_facet` (Column Facet) Pass 2.
5. `build_plot_components` (Inner Row Facet, Render Mode).
   - Calls `evaluate_facet` (Row Facet).
6. `evaluate_facet` (Row Facet) Pass 1.
   - Calls `measure_with_scales` (Leaf Plot).
   - Calls `CartesianGuide::measure_overflow` (Leaf).
   - Returns actual overflow.
7. `evaluate_facet` (Row Facet) Pass 2.
   - Calls `build_plot_components` (Leaf Plot, Render Mode).
   - **Panic:** `width` is 0.

## Conclusion

The `has_facet_guide()` check (or the equivalent `measure_with_scales` logic relying on `measure_overflow` fallback) **successfully prevents infinite recursion**. 

The "Stack Overflow" observed is likely a side effect of the resulting invalid layout (width 0) causing deep recursion in a dependency (likely `avenger-scales` or `avenger-guides` handling degenerate domains) or just a plain Panic as seen in the final trace. The fix needs to ensure that the estimated overflow allows for valid layout calculation (positive width).
