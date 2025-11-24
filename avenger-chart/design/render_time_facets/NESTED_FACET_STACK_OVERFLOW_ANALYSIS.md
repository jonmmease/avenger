# Nested Facet Stack Overflow Analysis

## Problem

After implementing Phase 2 of render-time facets, the nested facet test (`test_col_with_nested_row`) triggers a stack overflow during evaluation.

## Test Structure

```rust
Plot::<FacetColumn>::new()
    .data(df.clone())  // ← Outer plot has data
    .mark(
        Facet::new()
            .col_with(col("species"), ...)
            .subplot(
                Plot::<FacetRow>::new()
                    .data(df)  // ← Inner plot ALSO has data
                    .mark(
                        Facet::new()
                            .row_with(col("petal_width_bin"), ...)
                            .subplot(
                                Plot::<Cartesian>::new().mark(Symbol::new()...)
                            )
                    )
            )
    )
```

## Observed Behavior

Test output before stack overflow:
```
FacetCol::evaluate_from_data (Col)
evaluate_facet entering: channel=column
Pass 1 starting for channel=column
Pass 1 iteration
Pass 1 build_plot_components
FacetRow::evaluate_from_data (Row)
evaluate_facet entering: channel=row
Pass 1 starting for channel=row
Pass 1 iteration
Pass 1 build_plot_components

thread 'visual_tests::test_nested_facets::test_col_with_nested_row' has overflowed its stack
```

The pattern suggests infinite recursion during the measurement phase (Pass 1).

## Phase 2 Implementation

### Key Changes Made

1. **`wants_full_data_batch()` trait method** - Facets return `true` to request full DataFrame
2. **`evaluate_mark_with_plot_df` modification** - Preserves full DataFrame for facets
3. **`evaluate_facet` signature change** - Accepts `data_override: Option<&DataFrame>`
4. **Render-time key extraction** - Uses `FacetKeyExtractor` instead of compile-time keys
5. **Data override passing** - Each facet level passes filtered data to nested facets

### Expected Data Flow

1. **Outer FacetCol**:
   - `evaluate_from_data` called with `data=None` (top-level)
   - Extracts keys from `state.data` (compiled data)
   - For each column, filters data: `filter_df = df.filter(col_expr == col_value)`
   - Calls `build_plot_components(compiled_subplot, data_override=Some(&filter_df))`

2. **Inner FacetRow**:
   - `evaluate_mark_with_plot_df` converts DataFrame to RecordBatch (full columns preserved)
   - `evaluate_from_data` called with `data=Some(RecordBatch)`
   - Converts RecordBatch back to DataFrame via `batch_to_dataframe`
   - Should use `data_override` instead of `state.data`
   - Extracts keys from overridden data
   - For each row, filters data: `filter_df2 = override_df.filter(row_expr == row_value)`
   - Calls `build_plot_components(inner_subplot, data_override=Some(&filter_df2))`

3. **Innermost Cartesian**:
   - Receives filtered data from FacetRow
   - Renders Symbol marks

## Hypothesis: Why Stack Overflow?

### Hypothesis 1: Inner Plot's Compiled Data Interfering

The inner `Plot::<FacetRow>` has `.data(df)` attached. During compilation, this data is stored in `CompiledPlot.data`. When `evaluate_facet` runs, it checks:

```rust
let df = if let Some(override_df) = data_override {
    override_df.clone()  // Should use this!
} else {
    state.data.dataframe_with_context(ctx).ok_or_else(...)?  // ← Might use this instead?
}
```

**Problem**: If `data_override` is `Some` but somehow not being used, the inner facet would extract keys from the full dataset, creating the same facets recursively.

### Hypothesis 2: Scale Building Uses Wrong Data

In `facet_evaluation.rs` line 295-298:
```rust
compiled_subplot
    .build_scale_builder_from_dataframe(ctx, &iteration.params, &filter_df)
    .await?
```

This uses `filter_df` for scale building. But does the `compiled_subplot` have its own data that might interfere?

### Hypothesis 3: Recursive Compilation Issue

When the outer facet is compiled, its `compiled_subplot` field contains the compiled middle `Plot::<FacetRow>`. That compiled plot has `marks` which include a `CompiledFacetRow`.

During render:
1. Outer facet's `evaluate_facet` calls `build_plot_components` on its `compiled_subplot`
2. `build_plot_components` calls `evaluate_mark_with_plot_df` for each mark
3. For the `CompiledFacetRow` mark, this calls its `evaluate_from_data`
4. The inner facet's `evaluate_from_data` calls ITS `evaluate_facet`
5. Which calls `build_plot_components` on ITS `compiled_subplot`

**This should terminate** because the innermost subplot is a `Plot::<Cartesian>`, not another facet.

Unless...

### Hypothesis 4: Data Override Not Being Respected

**Critical Question**: When `build_plot_components` is called with `data_override=Some(filtered_df)`, does it actually pass that through to nested facets?

Looking at `build_plot_components` lines 1258-1269 (Measure mode):
```rust
for mark in &self.marks {
    let (_, layout_info) = self
        .evaluate_mark_with_plot_df(
            mark.as_ref(),
            &final_scales,
            plot_area_width,
            plot_area_height,
            ctx,
            &merged_params,
            df_opt,  // ← This is data_override
        )
        .await?;
    layout_updates.push(layout_info);
}
```

And `evaluate_mark_with_plot_df` (rendering.rs):
- Converts DataFrame to RecordBatch (if `wants_full_data_batch()` is true)
- Calls `mark.evaluate_from_data(Some(&data_batch), ...)`

So the data override IS being passed through. But...

## The Actual Problem

Looking at the test output again:
```
Pass 1 iteration
Pass 1 build_plot_components
FacetRow::evaluate_from_data (Row)
evaluate_facet entering: channel=row
Pass 1 starting for channel=row
Pass 1 iteration
Pass 1 build_plot_components
```

We see:
1. Outer facet Pass 1 iteration
2. Outer facet calls build_plot_components
3. Inner FacetRow::evaluate_from_data is called ✓
4. Inner facet starts its own Pass 1 ✓
5. Inner facet iteration ✓
6. Inner facet calls build_plot_components
7. **Then it should call Cartesian plot marks, but instead it loops back to step 3**

This suggests the inner facet is somehow looping back to itself, OR the innermost plot is ALSO a facet when it shouldn't be.

## Investigation Needed

1. **Check what `compiled_subplot` actually contains for the inner FacetRow**
   - Is it a `Plot::<Cartesian>` as expected?
   - Or is it somehow another facet?

2. **Verify data_override is being used**
   - Add more debug logging to show which DataFrame is being used for key extraction
   - Print out the keys extracted at each level

3. **Check for infinite recursion pattern**
   - Is the same facet being called repeatedly?
   - Or are different facets cycling?

## Debugging Added

Enhanced logging has been added to `facet_evaluation.rs`:
1. **Line 126-130**: Log whether using `data_override` (✅) or compiled data (⚠️)
2. **Line 222**: Log number of keys extracted for each facet channel

This will help us see:
- Whether nested facets are correctly receiving filtered data
- Whether they're extracting keys from the right DataFrame
- How many keys each facet level is extracting (should decrease as we go deeper)

## Expert Analysis Requested

GPT-5 Codex is now analyzing:
1. The stack overflow root cause
2. Whether data_override is being used correctly
3. Whether the middle Plot's compiled_subplot structure is causing issues
4. Potential fixes that preserve the render-time data override mechanism

## Next Steps

1. Wait for expert analysis from GPT-5 Codex (shell a3e76c)
2. Run test with enhanced logging once compilation finishes
3. Analyze the logging output to identify where the recursion begins
4. Implement the fix recommended by expert analysis
