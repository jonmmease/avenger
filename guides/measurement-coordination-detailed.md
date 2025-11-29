# Comprehensive Analysis: Measurement and Two-Pass Coordination System in avenger-chart

## Overview

The avenger-chart rendering system uses a sophisticated two-pass architecture to coordinate layout and spacing across faceted visualizations. This analysis documents the complete data flow during measurement, where spacing information flows bidirectionally between facet levels.

## Architecture: Three-Level Organization

The system operates across three key layers:

1. **Facet Evaluation Layer** (`facet_evaluation.rs`)
   - Orchestrates the two-pass algorithm
   - Manages coordination context
   - Aggregates spacing needs from subplots

2. **Plot Rendering Layer** (`plot/compiled/mod.rs` and `rendering.rs`)
   - Measures guide overflow (axes, labels)
   - Computes layout with legends
   - Provides measurement APIs

3. **Facet Guide Layer** (`facet/guide.rs`)
   - Measures nested facet overflow
   - Handles empty cell fallback
   - Computes per-cell dimensions

---

## Data Flow During Measurement (Pass 1)

### Phase 1A: Initial Setup (Lines 108-238)

**Input to `measure_pass<DimConfig>`:**
- `facet_coord`: Coordinate system transform (band/point spacing)
- `compiled_subplot`: The inner plot to measure
- `dimension_scale`: Band scale (row or column)
- `scale_sharing_by_channel`: x/y scale sharing modes
- `df`: Full dataset (before filtering)
- `facet_expr`: Expression for faceting (e.g., `col("row_facet")`)
- `effective_context`: RenderContext with coordination params

**Coordination Context Extraction (Lines 114-138):**
```rust
let coordination_context = FacetCoordinationContext::from_params(&context.params);

// Extract pre-computed domain from coordination (for nested facets with Shared scale sharing)
let domain_from_coordination = coordination_context
    .and_then(|ctx| ctx.get_inner_domain_for_channel(current_channel));

// Extract shared data extents for scales
let shared_data_extents = coordination_context
    .and_then(|ctx| ctx.shared_data_extents.clone());
```

**Domain Values Determination (Lines 215-237):**
- If coordination context provides domain: **use full domain** (grid-like behavior)
- Else: **extract domain from filtered data** (free faceting behavior)
- Sort domain values for deterministic ordering

**Key Decision Point:** Domain propagation mode is determined by `should_share_domain` in the outer facet's coordination detection:
- `ScaleSharing::Shared` → full domain propagation enabled
- `ScaleSharing::Free` → each facet uses filtered domain

### Phase 1B: Scale Building (Lines 141-189)

**Two-Phase Scale Building Strategy:**

1. **Build shared scales** once with approximate bandwidth (if any channels use `ScaleSharing::Shared`)
   ```rust
   let initial_bandwidth = band::bandwidth(&configured.config)?;
   let (width, height) = subplot_dims(initial_bandwidth, context);
   let initial_shared_scales = compiled_subplot
       .build_scales_from_builder(&builder, width, height, ...)
       .await?;
   ```

2. **Extend with shared data extents** - Only for Shared channels:
   ```rust
   if any_shared {
       let mut builder = build_scale_builder_from_dataframe(...);
       if let Some(ref extents) = shared_data_extents {
           // Only extend Shared channels, not Free channels
           let shared_only = extents.filter(|ch| mode == Shared);
           builder.extend_with_shared_extents(&shared_only);
       }
   }
   ```

**Fallback Scale Builder** (Lines 204-212):
- Built from full dataset when empty cell fallback is enabled
- Used by subplots with no data after filtering
- Prevents axis rendering errors in empty cells

### Phase 1C: Subprocess Measurement Loop (Lines 306-434)

**Concurrent Measurement Strategy:**
```rust
const MAX_CONCURRENT_MEASURE: usize = 4;  // Semaphore-bounded concurrency
let semaphore = Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_MEASURE));

// Pre-materialize work items to decouple from lifetimes
let work_items: Vec<(usize, SubplotIteration, SubplotRect)> = subplot_iter
    .zip(initial_rects.into_iter())
    .enumerate()
    .collect();

let outer_count = work_items.len();  // Store for coordination context updates
```

**Per-Subplot Measurement (Lines 346-420):**

For each subplot at index `idx`:

1. **Update coordination context with position:**
   ```rust
   let params_base = FacetCoordinationContext::update_outer_position_in_params(
       &params_base,
       idx,  // Position within outer facet
       outer_count,  // Total subplots in outer facet
   );
   ```

2. **Filter data by facet value:**
   ```rust
   let filter_df: DataFrame = df
       .clone()
       .filter(facet_expr.eq(lit(iteration.facet_value.clone())))?;
   ```

3. **Build scales for this cell:**
   ```rust
   // For Shared channels: use initial_shared_scales (built from full dataset)
   // For Free channels: build from filter_df
   let free_scale_builder = if all_shared {
       None  // All scales are shared - don't rebuild
   } else {
       Some(compiled_subplot
           .build_scale_builder_from_dataframe(&ctx, &params_base, &filter_df)
           .await?)
   };

   let scales = build_scales_helper_with_fallback(
       &compiled_subplot,
       &initial_shared_scales,    // Shared scales
       &free_scale_builder,       // Free scales (from filtered data)
       &fallback_builder,         // Fallback for empty cells
       &filter_df,
       width,
       height,
       ...
   ).await?;
   ```

4. **Measure subplot overflow:**
   ```rust
   let (guide_only, total_overflow, legend_positions) = measure_subplot(
       &compiled_subplot,
       width,
       height,
       &ctx,
       &params_base,
       &scales,
       &filter_df,  // Data override: measurement uses filtered data
   ).await?;
   ```

**Measurement Call Chain:**
```
measure_subplot()
  → compiled_subplot.measure_with_scales()
    → compute_layout_with_fixed_plot_area()
      → compiled_guide.measure_overflow()
        → FacetRowGuide.measure_overflow() [if guide is facet guide]
          → Recursively measures nested facet subplots
```

### Phase 1D: Overflow Aggregation (Lines 436-481)

**Two Types of Overflow Tracking:**

1. **Guide-Only Overflow** (cross-subplot alignment):
   ```rust
   let global_max_overflow = {
       let mut max_overflow = OverflowSpaceRequirement::default();
       for overflow in &guide_only_measurements {
           max_overflow.top = max_overflow.top.max(overflow.top);
           // ... bottom, left, right
       }
       max_overflow
   };
   ```
   - Used for axis label alignment across subplots
   - Ensures consistent sizing for guides

2. **Total Overflow** (spacing calculation):
   ```rust
   let mut max_required_gap = 0.0f32;
   for i in 0..overflow_measurements.len().saturating_sub(1) {
       let gap = DimConfig::calculate_adjacent_overflow(
           &overflow_measurements[i],
           &overflow_measurements[i + 1],
       );
       max_required_gap = max_required_gap.max(gap);
   }
   ```
   - Calculates gap between adjacent cells
   - Dimension-specific: for rows, max(bottom[i] + top[i+1])

### Phase 1E: Gap Computation and Spacing Needs (Lines 448-819)

**Two Decision Paths:**

**Path A: Use Coordinated Gap from Outer Facet (Lines 485-527)**
```rust
let coordinated_gap_key = if DimConfig::is_row_facet() {
    "inter_row_gap"  // For FacetRow, check gap computed by outer FacetColumn
} else {
    "inter_col_gap"  // For FacetColumn, check gap computed by outer FacetRow
};

let measured_gap_from_outer = coordination_context
    .as_ref()
    .and_then(|ctx| ctx.get_coordinated_spacing(coordinated_gap_key));

let rounded_gap = if let Some(measured_gap) = measured_gap_from_outer {
    // Use pre-computed gap from outer facet
    measured_gap
} else {
    // Compute from local overflow
    (max_required_gap + spacing).ceil()
};
```

**Path B: Compute Cross-Dimension Gaps for Nested Facets (Lines 631-799)**

For `FacetColumn` with nested `FacetRow`:
```rust
let measured_row_gap = if DimConfig::is_col_facet() {
    if already_computed { None }  // Skip if 2nd pass
    else if has_nested_row_facet && inner_domain_count > 1 {
        // Heuristic: gap = max_bottom_overflow + spacing
        // max_bottom represents x-axis at bottom of each row
        let max_bottom = overflow_measurements.iter()
            .map(|o| o.bottom)
            .fold(0.0f32, f32::max);
        let gap = (max_bottom + spacing).ceil();
        Some(gap)
    } else {
        None
    }
} else {
    None
};
```

This computes the gap for the inner (different) dimension.

**Spacing Needs Report (Lines 801-836):**
```rust
let mut spacing_needs = HashMap::new();

// Report own dimension's gap
spacing_needs.insert("inter_row_gap".to_string(), rounded_gap);  // if row facet

// Report cross-dimension gaps (computed for nested inner facets)
if let Some(row_gap) = measured_row_gap {
    spacing_needs.insert("inter_row_gap".to_string(), row_gap);  // FacetColumn computed
}
if let Some(col_gap) = measured_col_gap {
    spacing_needs.insert("inter_col_gap".to_string(), col_gap);  // FacetRow computed
}

// Report legend overflow for alignment
if all_legend_positions.contains(&LegendPosition::Right) {
    spacing_needs.insert("legend_right".to_string(), global_max_overflow.right);
}
```

### Phase 1F: Scale Adjustment with Measured Padding (Lines 529-611)

**Create PaddingSpec with Measured Values:**
```rust
let padding_spec = crate::coords::PaddingSpec::Single {
    padding_px: rounded_gap,
    overflow: overflow_measurements.clone(),
};
let updated_facet_coord = facet_coord.with_measured_padding(&padding_spec);
```

**Rebuild Dimension Scale with New Padding:**
```rust
let mut new_config = effective_configured.config.clone();
new_config.options.insert(
    "padding_inner_px".to_string(),
    Scalar::from_f32(rounded_gap),
);

let updated_spec = dimension_scale.spec().clone().option(
    "padding_inner_px",
    lit(ScalarValue::Float32(Some(rounded_gap))),
);

let updated_configured = avenger_scales::scales::ConfiguredScale {
    scale_impl: effective_configured.scale_impl.clone(),
    config: new_config,
};

let final_dimension_scale = ConfiguredScaleWithSpec::new(updated_spec, updated_configured);
```

**Recompute Final Rectangles with New Padding:**
```rust
let final_positions = final_configured.scale_scalars_to_numeric(&domain_vals)?;

let final_geometry_with_padding = updated_facet_coord.transform(
    &temp_position_channels,
    Some(&temp_position_values),
    context.plot_width,
    context.plot_height,
)?;

let mut final_rects = final_geometry_with_padding
    .as_any()
    .downcast_ref::<SubplotGeometry>()?
    .rects
    .clone();

// Adjust last rect to fill remaining space (avoid rounding gaps)
if let Some(last_rect) = final_rects.last_mut() {
    if DimConfig::is_row_facet() {
        last_rect.height = context.plot_height - last_rect.y;
    } else {
        last_rect.width = context.plot_width - last_rect.x;
    }
}
```

---

## Phase 1.5: Coordination Context Construction (Lines 1530-1619)

**Two-Pass Decision Logic:**

```rust
// Check for cross-dimension gaps (only exist from 2D overflow analysis)
let has_cross_dimension_gap = if DimConfig::is_col_facet() {
    pass1.spacing_needs.contains_key("inter_row_gap")  // FacetColumn computed row gap
} else {
    pass1.spacing_needs.contains_key("inter_col_gap")  // FacetRow computed column gap
};

if has_cross_dimension_gap {
    // NESTED FACETS: Need to re-run measure_pass
    // This allows inner facets to use the coordinated gap during their measurement
    
    let mut coord_ctx = FacetCoordinationContext::from_params(&effective_context.params)
        .unwrap_or_default();
    
    coord_ctx = coord_ctx.with_coordinated_spacing(pass1.spacing_needs.clone());
    
    // Update params with new coordination context
    let mut new_params = context.params.clone();
    new_params.extend(coord_ctx.to_params());
    
    // Re-run measure_pass with updated context
    let pass2 = measure_pass::<DimConfig, _>(
        facet_coord,
        compiled_subplot,
        dimension_scale,
        &scale_sharing_by_channel,
        &df,
        &facet_expr,
        facet_spacing,
        &updated_ctx,  // Updated context with coordinated_spacing
        &subplot_dims,
    ).await?;
    
    (pass2, updated_ctx)
} else if has_spacing_needs {
    // STANDALONE FACETS: Just pass coordination context
    // No inner facets, so no need to re-measure
    
    let mut coord_ctx = FacetCoordinationContext::from_params(&effective_context.params)
        .unwrap_or_default();
    
    coord_ctx = coord_ctx.with_coordinated_spacing(pass1.spacing_needs.clone());
    
    let mut new_params = context.params.clone();
    new_params.extend(coord_ctx.to_params());
    
    (pass1, updated_ctx)  // Use pass1, no re-measure needed
} else {
    (pass1, effective_context.clone())
}
```

**Key Insight:** The re-run of `measure_pass` (for nested facets) happens AFTER pass1 completes, with coordination context now populated. This allows inner facets to access the gap values when they measure.

---

## Phase 2: Rendering with Coordinated Parameters (Lines 853-1630)

The `render_pass<DimConfig>` function takes `final_pass` (from Phase 1 or 1.5) and renders subplots.

**Per-Subplot Rendering (Lines 945-1186):**

For each subplot at index `idx`:

1. **Update coordination context (same as measurement):**
   ```rust
   let params_base = FacetCoordinationContext::update_outer_position_in_params(
       &params_base,
       idx,
       outer_count,
   );
   ```

2. **Add SharedInColumn extents to params (for inner facets):**
   ```rust
   let column_extents_for_inner = if !shared_in_column_channels.is_empty() {
       let extents = free_scale_builder.extract_serializable_extents(...);
       Some(extents)
   } else {
       None
   };
   
   if let Some(column_extents) = column_extents_for_inner {
       merged_params = FacetCoordinationContext::update_shared_data_extents_for_column_in_params(
           &merged_params,
           column_extents,
       );
   }
   ```

3. **Build plot components with all params:**
   ```rust
   let components = render_subplot(
       &compiled_subplot,
       width,
       height,
       &ctx,
       &merged_params,  // Contains coordination context + position updates
       &scale_provider,
       &filter_df,
   ).await?;
   ```

---

## Key Function: `measure_with_scales`

**Location:** `plot/compiled/mod.rs:478`

**Purpose:** Lightweight measurement for facet Pass 1

**Signature:**
```rust
pub async fn measure_with_scales(
    &self,
    width: f32,
    height: f32,
    ctx: &SessionContext,
    params: &IndexMap<String, ScalarValue>,
    scales: &HashMap<String, ConfiguredScaleWithSpec>,
    data_override: Option<&DataFrame>,
) -> Result<(
    OverflowSpaceRequirement,  // guide_only_overflow
    OverflowSpaceRequirement,  // total_overflow (+ legends)
    LegendLayoutInfo,
    HashSet<LegendPosition>,
), AvengerChartError>
```

**Flow:**
```
measure_with_scales()
  → compute_layout_with_fixed_plot_area(width, height, scales, ..., data_override)
    → compiled_guide.measure_overflow(configured_scales, None, None, width, height, ...)
      → [If guide is FacetRowGuide]
        → Iterates over facet domain
        → Filters data by facet value
        → Measures each cell's overflow
        → Returns aggregated overflow
    → Measures legends (same dimensions)
    → Returns guide_only + total overflow
```

**Important:** The `data_override` parameter is critical:
- During nested facet measurement: `data_override` = filtered data for this cell
- This ensures axis visibility decisions are based on cell-specific data

---

## Key Function: `compute_layout_with_fixed_plot_area`

**Location:** `plot/compiled/rendering.rs:862`

**Purpose:** Compute layout assuming plot area dimensions are fixed

**Flow:**
```rust
1. Measure guide overflow (axes, labels)
   compiled_guide.measure_overflow(..., plot_width, plot_height, ...)

2. Check for coordinated spacing in params
   coord_ctx = FacetCoordinationContext::from_params(params);
   if let Some(legend_right) = ctx.get_coordinated_spacing("legend_right") {
       overflow.right = legend_right;  // Override with unified value
   }

3. Measure legends with fixed size
   legend_measurements = prepare_legend_measurements(..., available_size=plot_width x plot_height)

4. Create EvaluatedLayoutSpec with fixed plot area
   evaluated_spec = EvaluatedLayoutSpec {
       canvas: Auto,
       plot_area: Fixed { width: plot_width, height: plot_height },
       margins: { 0, 0, 0, 0 },  // Subplot manages own spacing
   }

5. Compute layout
   layout = ChartLayout::new(&overflow, &evaluated_spec, ...);
   result = layout.compute(&evaluated_spec)?;

6. Add legend dimensions to overflow
   for (position, measurement) in legend_measurements {
       match position {
           Right => result.overflow.right += measurement.size.width,
           ...
       }
   }

7. Return result with updated overflow
```

---

## Key Function: `FacetRowGuide.measure_overflow`

**Location:** `facet/guide.rs:141`

**Purpose:** Measure nested facet subplot overflow

**Two Measurement Paths:**

**Path A: With data_override (nested facet case)**
- Filters data by facet value
- Measures each cell individually
- Creates FacetContext with position info (for axis visibility)
- Calls `subplot.measure_with_scales()` per cell
- Aggregates overflow from all cells

**Path B: With cached overflow (facet grid case)**
- Uses pre-computed overflow from outer facet's measurement
- Returns aggregated values directly

**Key Code (Lines 176-349):**
```rust
if let Some(df) = data_override {
    // Nested facet: measure subplots
    for (row_idx, (domain_val, has_data)) in iteration_domain.iter().enumerate() {
        // Create cell DataFrame
        let cell_df = if *has_data {
            df.clone().filter(expr.eq(lit(domain_val.clone())))?
        } else {
            df.clone().limit(0, Some(0))?  // Empty DataFrame
        };
        
        // Build FacetContext with position for axis visibility
        let facet_ctx = FacetContext {
            position: (row_idx, parent_col),
            grid_dimensions: (grid_num_rows, parent_num_cols),
            ...
        };
        
        // Measure this cell
        let (_, total_overflow, _, _) = source.subplot.measure_with_scales(
            plot_width,
            band_height,
            ctx,
            &measure_params,  // Contains FacetContext
            &subplot_scales,
            Some(&cell_df),  // Data override
        ).await?;
        
        computed_overflow.push(total_overflow);
    }
}
```

---

## Coordination Context Data Structures

### `FacetCoordinationContext` Fields (coordination.rs)

**For Domain Propagation:**
- `inner_domain`: Pre-computed domain values from outer facet
- `inner_scale_sharing`: Scale sharing mode (Shared, Free, SharedInRow, SharedInColumn)
- `inner_domain_count`: Number of domain values from full dataset
- `should_propagate_domain`: Controlled by scale sharing mode

**For Spacing Coordination:**
- `coordinated_spacing`: HashMap<String, f32> with aggregated gap values
  - Keys: "inter_row_gap", "inter_col_gap", "legend_right", etc.
  - Values: Maximum spacing needed across all children
- `get_coordinated_spacing(key)`: Retrieve gap value by key

**For Data Extents:**
- `shared_data_extents`: Full dataset extents (for Shared scales)
- `shared_data_extents_by_row`: Per-row extents (for SharedInRow)
- `shared_data_extents_for_column`: Per-column extents (for SharedInColumn)

**For Guide Ownership (nested facets):**
- `guide_ownership`: {Full, Edge, Suppress} - controls which subplots render axes
- `outer_position`: Position within outer facet (0-indexed)
- `outer_count`: Total subplots in outer facet

---

## Summary: Data Flow Diagram

```
Input: Full dataset + Facet expression
   ↓
[measure_pass - Phase 1]
   ├─ Extract domain values (or use coordination domain)
   ├─ Build initial scales (shared with full data, free with filtered)
   ├─ For each subplot:
   │   ├─ Filter data by facet value
   │   ├─ Measure subplot overflow (via guide)
   │   └─ Collect overflow measurements
   ├─ Compute required gaps from adjacent overflow
   ├─ Compute cross-dimension gaps (if nested facets detected)
   └─ Return: FacetPass1Result {
       overflow_measurements,
       final_dimension_scale (with padding),
       spacing_needs: {
           "inter_row_gap": f32,
           "inter_col_gap": f32 (if computed),
           "legend_right": f32,
           ...
       }
   }
   ↓
[Phase 1.5 - Coordination Context]
   ├─ Check if cross-dimension gaps present
   ├─ If yes: Re-run measure_pass with coordinated_spacing in params
   │   (allows inner facets to use coordinated gaps)
   └─ If no: Use pass1 results directly
   ↓
[render_pass - Phase 2]
   └─ For each subplot:
       ├─ Update coordination context with position
       ├─ Build scales with final padding
       ├─ Render plot components
       └─ Position at calculated rectangle
```

---

## Critical Design Patterns

### 1. Domain Propagation Strategy

**Problem:** Nested facets need consistent domains (grid-like layout)

**Solution:** 
- Outer facet computes full domain from complete dataset
- Passes via `FacetCoordinationContext.inner_domain`
- Inner facet uses passed domain instead of filtering
- Empty cells created for missing data combinations
- Fallback scales render axes for empty cells

### 2. Two-Pass Spacing Coordination

**Problem:** Inner and outer facet gaps must coordinate

**Solution:**
- Pass 1: Inner facets compute their own gaps
- Aggregation: Outer facets max-reduce spacing_needs across children
- Pass 1.5: Re-run inner facets with coordinated gaps in params
- Pass 2: Both use coordinated gaps consistently

### 3. Scale Sharing vs. Free Scales

**Shared Scales:**
- Built once from full dataset at beginning
- Reused for all cells
- Ensures consistent data mapping across all subplots

**Free Scales:**
- Built per-cell from filtered data
- Each subplot has independent domain
- Allows zooming/focusing on cell-specific patterns

### 4. Overflow Aggregation for Alignment

**guide_only_overflow:** Used for axis label alignment
- Computed from axis ticks/labels only
- Consistent across all subplots with same axes

**total_overflow:** Includes legends and titles
- Used for outer facet spacing calculation
- Accounts for all content needing space

### 5. Recursive Measurement via data_override

**Measurement Chain:**
- Outer facet calls `measure_with_scales(data_override=filtered_data)`
- CompiledPlot measures guide overflow with `data_override`
- Guide (if FacetRowGuide) iterates nested facet domain
- Each cell measured with `data_override=cell_filtered_data`
- Recursion continues if deeper nesting

---

## Parameter Flow Summary

### Parameters Down (measure_pass):
1. `context.params` (base parameters from RenderContext)
2. `coordination_context` (extracted from params)
3. `iteration.params` (per-iteration overrides)
4. Per-subplot params = base + coordination updates + iteration params

### Updates in Params:
- `FacetCoordinationContext` serialized to `"__facet_coordination"` param
- `FacetContext` serialized to `"__facet_context"` param
- `outer_position` and `outer_count` updated per iteration
- `shared_data_extents_for_column` added per column (FacetColumn outer)

### Parameters Up (returns):
- `FacetPass1Result.spacing_needs`: HashMap with computed gaps
- Aggregated by outer facet before Pass 1.5 re-measure
- Returned in `coordinated_spacing` field of updated context

---

## Entry Points and Call Sites

**From Facet Marks:**
- `FacetRowMark.evaluate()` → `evaluate_facet::<RowDimensionConfig>()`
- `FacetColMark.evaluate()` → `evaluate_facet::<ColDimensionConfig>()`

**From Plot Evaluation:**
- `CompiledPlot.evaluate()` → Iterates marks → Calls `mark.evaluate()`

**From Layout:**
- `guide.measure_overflow()` → Calls `FacetRowGuide.measure_overflow()`

