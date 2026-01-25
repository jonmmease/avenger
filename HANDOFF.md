# FacetCol Overflow Alignment Issue - Handoff Document

## Problem Summary
The outer chart overflow (magenta debug rectangles) does not align with individual subplot overflows (colored debug rectangles) in FacetCol layouts, particularly on the **right edge**. The magenta right overflow rectangle is consistently narrower than the last subplot's right overflow, causing visual misalignment and potential clipping of axis labels.

## What Has Been Implemented

### Architecture Overview
The FacetCol rendering system uses a two-pass measurement approach:
1. **First pass (FacetColGuide::measure_overflow)**: Measures edge subplots to determine outer overflow requirements
2. **Second pass (actual rendering)**: Individual subplots render with their own overflow measurements (shown as colored debug rectangles)

### Current Edge Measurement Implementation
Located in `avenger-chart/src/facet/guide.rs`, lines ~716-941:

```rust
// Simplified, robust path: measure only edge subplots at final sizes
if !domain_vals.is_empty() {
    // 1. Compute final band width and positions
    let bp_iter = BandPositionIterator::from_configured_scale(col_scale)?;
    let band_w = bp_iter.bandwidth();

    // 2. Identify leftmost and rightmost facet values by band position
    let positions: Vec<(ScalarValue, f32)> = ...;
    // Find min/max band start positions

    // 3. For each edge (left/right):
    for source in &self.facet_sources {
        // Get filtered dataframes for left and right columns
        let df_left = df_full.clone().filter(col_expr.clone().eq(lit(left_val.clone())))?;
        let df_right = df_full.clone().filter(col_expr.eq(lit(right_val.clone())))?;

        // Build facet-specific params via SubplotIterator
        let iter = SubplotIterator::<ColDimensionConfig>::new(...);
        // Extract left_params and right_params for each edge

        // Build base scales (shared channels) using facet-specific params
        let left_base_scales = subplot.build_scales_for_dataframe(&df_full, band_w, plot_height, ctx, &left_params).await?;
        let right_base_scales = subplot.build_scales_for_dataframe(&df_full, band_w, plot_height, ctx, &right_params).await?;

        // Build filtered scales (free channels) using facet-specific params
        let left_scales_filt = subplot.build_scales_for_dataframe(&df_left, band_w, plot_height, ctx, &left_params).await?;
        let right_scales_filt = subplot.build_scales_for_dataframe(&df_right, band_w, plot_height, ctx, &right_params).await?;

        // Merge: start with base (shared), override non-shared channels with filtered
        let mut left_scales = left_base_scales.clone();
        let mut right_scales = right_base_scales.clone();
        for (ch, mode) in &scale_sharing_by_channel {
            if *mode != ScaleSharing::Shared {
                if let Some(s) = left_scales_filt.get(ch) { left_scales.insert(ch.clone(), s.clone()); }
                if let Some(s) = right_scales_filt.get(ch) { right_scales.insert(ch.clone(), s.clone()); }
            }
        }

        // Measure overflow using prebuilt scales
        let left_over = subplot.measure_guide_overflow_with_scales(&left_scales, band_w, plot_height, ctx, &left_params).await?;
        let right_over = subplot.measure_guide_overflow_with_scales(&right_scales, band_w, plot_height, ctx, &right_params).await?;

        left_max = left_max.max(left_over.left);
        right_max = right_max.max(right_over.right);
        // ... aggregate top/bottom
    }

    // Add facet label/title spacing
    // ... measure and add slab space

    return Ok(result);  // Early return with edge-measured values
}
```

### Rounding Protection
Multiple locations ensure overflow sizes are rounded up to avoid under-allocation:
- **Grid tracks**: `avenger-chart/src/layout/grid.rs` uses `ceil()` for overflow track sizes
- **Layout extraction**: `avenger-chart/src/layout/chart_layout.rs` ceils guide overflow dimensions
- **Debug visualization**: `avenger-chart/src/render/debug.rs` ceils overflow rectangles for consistent visuals

### Debug Logging
When `AVENGER_CHART_DEBUG_LAYOUT=1` is set:
- Each subplot prints its measured overflow: `"DEBUG subplot overflow: left=X, right=Y, ..."`
- FacetColGuide prints edge measurements: `"FACET_COL (edge-measured) result: left=X, right=Y, ..."`
- Outer layout prints second-pass overflow: `"SECOND-PASS overflow (outer): left=X, right=Y, ..."`
- Grid extraction prints final track widths: `"OUTER of-Right width=X"`

## The Mismatch

### Observed Behavior
In tests like `facet_col_free_scales` and `facet_col_iris_scatter`:
- **Last subplot (debug)** reports: `right=7.912` (or similar)
- **FACET_COL edge measurement** reports: `right=4.083` (or similar)
- **SECOND-PASS overflow** uses: `right=4.083`
- **OUTER of-Right width** allocates: `ceil(4.083) = 5.0`
- **Result**: Magenta rectangle is ~3-4px narrower than the green/colored subplot rectangle on the right edge

### Why This Matters
1. Visual misalignment in debug mode indicates measurement inconsistency
2. Potential for clipping axis labels if outer allocation is insufficient
3. The two measurement paths (edge measurement vs. individual subplot rendering) should produce identical results but don't

## Suspected Root Cause

The edge measurement path in `FacetColGuide::measure_overflow` uses:
```rust
subplot.build_scales_for_dataframe(&df, band_w, plot_height, ctx, &facet_params).await?
subplot.measure_guide_overflow_with_scales(&scales, band_w, plot_height, ctx, &facet_params).await?
```

However, individual subplot rendering (which produces the correct colored debug rectangles) uses:
```rust
// In avenger-chart/src/plot/compiled/rendering.rs, build_plot_components()
let scale_builder = build_scale_builder_from_marks(&marks, &scale_specs, &coord_transform, &data, Some(filtered_df), ctx, &facet_params, theme).await?;
let provider = DynamicScaleProvider { builder: &scale_builder, plot: self };
self.build_plot_components(band_w, plot_height, ctx, &facet_params, &provider, EvaluationMode::Measure, Some(&filtered_df), true).await?;
```

**Key difference**: The rendering path uses `build_scale_builder_from_marks()` which creates a proper scale builder that can be queried multiple times, and then calls `build_plot_components()` with a `ScaleProvider`. This path includes:
- Full axis configuration evaluation
- Axis layout options (tick count, label formatting, etc.)
- Theme-aware scale building
- Proper evaluation mode handling

The edge measurement path bypasses this infrastructure and directly builds scales via `build_scales_for_dataframe()`, which may not capture all the axis configuration logic that affects overflow (e.g., tick count calculations, label formatting decisions, axis title positioning).

## Proposed Solution

Replace the edge measurement in `FacetColGuide::measure_overflow` (lines ~716-941) with the **exact renderer evaluation path** used for individual subplots:

### Step-by-Step Implementation

1. **For each edge facet value (left and right)**:
   ```rust
   // Build scale builder using facet-specific params
   use crate::plot::compiled::scales::build_scale_builder_from_marks;
   let scale_builder = build_scale_builder_from_marks(
       &compiled_subplot.marks,
       &compiled_subplot.scale_specs,
       &compiled_subplot.coord_transform,
       &compiled_subplot.data,
       Some(&filtered_df),  // Use filtered DF for free channels
       ctx,
       &facet_params,       // Facet-specific params from SubplotIterator
       theme
   ).await?;
   ```

2. **Create a scale provider**:
   ```rust
   use crate::plot::compiled::scale_provider::DynamicScaleProvider;
   let provider = DynamicScaleProvider {
       builder: &scale_builder,
       plot: &compiled_subplot,
   };
   ```

3. **Call build_plot_components in Measure mode**:
   ```rust
   let components = compiled_subplot.build_plot_components(
       band_w,
       plot_height,
       ctx,
       &facet_params,
       &provider,
       crate::plot::compiled::EvaluationMode::Measure,
       Some(&filtered_df),
       true  // dimensions_are_plot_area
   ).await?;

   let overflow = components.overflow.unwrap_or_default();
   ```

4. **Aggregate across edges**:
   ```rust
   left_max = left_max.max(left_overflow.left);
   right_max = right_max.max(right_overflow.right);
   top_max = top_max.max(left_overflow.top.max(right_overflow.top));
   bottom_max = bottom_max.max(left_overflow.bottom.max(right_overflow.bottom));
   ```

5. **Add facet label/title spacing** (keep existing logic for measuring slab)

6. **Keep rounding protection** (already in place in grid.rs and chart_layout.rs)

### Files to Modify

- **`avenger-chart/src/facet/guide.rs`**:
  - Modify `FacetColGuide::measure_overflow` (lines ~716-941)
  - Replace scale building and measurement with `build_scale_builder_from_marks` + `build_plot_components`
  - Ensure facet-specific params are used for both builder and components call

### Expected Outcome

After this change, the logs should show:
```
EDGE left via build_plot_components: left=X.XXX, right=Y.YYY, ...
EDGE right via build_plot_components: left=X.XXX, right=7.912, ...  // Should match DEBUG subplot
DEBUG last subplot overflow: left=X.XXX, right=7.912, ...
SECOND-PASS overflow (outer): left=X.XXX, right=7.912, ...
OUTER of-Right width=8.0  // ceil(7.912)
```

And visually:
- Magenta outer right rectangle width = 8.0
- Green/colored last subplot right rectangle width = ceil(7.912) = 8.0
- **Perfect alignment** on the right edge

## Additional Context

### SubplotIterator
Ensures consistent `FacetContext` params across all measurement and rendering calls:
- Located in `avenger-chart/src/facet/subplot_iterator.rs`
- Automatically adds facet position info to params
- Handles scale sharing mode for proper scale building

### BandPositionIterator
Used to identify edge columns by band position (not index):
- Located in `avenger-chart/src/facet/band_positions.rs`
- Iterates over band positions from a configured scale
- Provides `start()` and `bandwidth()` for each band

### Scale Grouping
For grid facets (not used in FacetCol, but relevant for understanding scale architecture):
- Located in `avenger-chart/src/facet/scale_grouping.rs`
- Manages scale builders based on sharing mode
- Demonstrates proper scale builder pattern

### EvaluationMode
Located in `avenger-chart/src/plot/compiled/mod.rs`:
- `EvaluationMode::Measure`: Returns overflow, no scene marks
- `EvaluationMode::Render`: Returns full scene graph with marks

## Testing

Run these tests with debug layout enabled:
```bash
AVENGER_CHART_DEBUG_LAYOUT=1 cargo test -p avenger-chart --test visual_regression test_facet_col_free_scales -- --nocapture
AVENGER_CHART_DEBUG_LAYOUT=1 cargo test -p avenger-chart --test visual_regression test_facet_col_iris_scatter -- --nocapture
```

Look for:
1. Edge measurement logs matching individual subplot logs
2. SECOND-PASS overflow matching edge measurements
3. OUTER track widths matching ceil(subplot overflow)
4. Visual alignment of magenta and colored rectangles (no gap on right)

## Notes

- Top edge alignment is already correct (facet label slab is properly added)
- Left edge may show slight difference due to ceil rounding (expected and acceptable)
- Right edge is the primary concern and should match exactly after fix
- Keep all existing rounding protection (ceil operations) in place
- Maintain early return pattern in measure_overflow for edge-only measurement
