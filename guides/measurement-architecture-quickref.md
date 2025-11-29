# Quick Reference: Measurement and Coordination Architecture

## File Locations

| File | Layer | Key Functions |
|------|-------|----------------|
| `avenger-chart/src/facet/marks/facet_evaluation.rs` | Facet Evaluation | `measure_pass<DimConfig>()`, `render_pass<DimConfig>()`, `evaluate_facet<DimConfig>()` |
| `avenger-chart/src/plot/compiled/mod.rs` | Plot Rendering | `measure_with_scales()`, `build_scales_with_dimensions()` |
| `avenger-chart/src/plot/compiled/rendering.rs` | Plot Rendering | `compute_layout_with_fixed_plot_area()`, `compute_layout()` |
| `avenger-chart/src/facet/guide.rs` | Facet Guide | `FacetRowGuide.measure_overflow()`, `FacetColGuide.measure_overflow()` |
| `avenger-chart/src/facet/coordination.rs` | Coordination Context | `FacetCoordinationContext`, serialization/deserialization |

## Two-Pass Algorithm Overview

### Pass 1: Measurement (measure_pass)
1. **Setup** (lines 108-238)
   - Extract coordination context and domain values
   - Build initial scales with approximate band size
   
2. **Subplot Measurement** (lines 306-434)
   - Concurrent measurement of all subplots
   - For each: filter data, build scales, measure overflow
   - Collect overflow measurements
   
3. **Overflow Aggregation** (lines 436-481)
   - Compute global max overflow (guide-only)
   - Compute required gaps between adjacent cells
   
4. **Gap Computation** (lines 448-819)
   - Check for coordinated gap from outer facet
   - Compute cross-dimension gaps (if nested)
   - Build spacing_needs HashMap
   
5. **Scale Adjustment** (lines 529-611)
   - Rebuild dimension scale with measured padding
   - Recompute final rectangles with new spacing

**Output:** `FacetPass1Result` with:
- `overflow_measurements`: Per-subplot overflow
- `final_dimension_scale`: Band scale with padding
- `spacing_needs`: HashMap of computed gaps

### Phase 1.5: Coordination Context (lines 1530-1619)

Conditional re-run decision:
```
has_cross_dimension_gap = spacing_needs.contains_key("inter_row_gap" or "inter_col_gap")
  ↓ YES  (nested facets detected)
  └─→ Re-run measure_pass with coordinated_spacing in params
      (allows inner facets to use coordinated gaps)
  ↓ NO
  └─→ Use pass1 results directly
```

### Pass 2: Rendering (render_pass)
- Per-subplot iteration with final geometry
- Build plot components
- Position at calculated rectangles

## Key Data Structures

### FacetCoordinationContext
```rust
// For spacing coordination
pub coordinated_spacing: HashMap<String, f32>
  Keys: "inter_row_gap", "inter_col_gap", "legend_right", "legend_bottom"

// For domain propagation
pub inner_domain: Option<Vec<SerializableDomainValue>>
pub inner_domain_count: usize
pub inner_scale_sharing: ScaleSharing

// For position tracking
pub outer_position: usize
pub outer_count: usize
pub guide_ownership: GuideOwnership  // {Full, Edge, Suppress}

// For data extents
pub shared_data_extents: Option<HashMap<String, SerializableDataExtents>>
pub shared_data_extents_by_row: Option<HashMap<String, HashMap<String, ...>>>
pub shared_data_extents_for_column: Option<HashMap<String, ...>>
```

### FacetPass1Result
```rust
pub overflow_measurements: Vec<OverflowSpaceRequirement>
pub final_dimension_scale: ConfiguredScaleWithSpec
pub final_shared_scales: Option<HashMap<String, ConfiguredScaleWithSpec>>
pub final_rects: Vec<SubplotRect>
pub fallback_builder: Option<ScaleBuilder>
pub spacing_needs: HashMap<String, f32>
pub shared_data_extents: Option<HashMap<String, ...>>
```

## Critical Parameters

### Passed Down
- `context.params`: Base parameters from RenderContext
- `coordination_context`: Extracted from `__facet_coordination` param
- Per-iteration updates to `outer_position` and `outer_count`

### Passed Up
- `spacing_needs`: Computed gaps reported by each facet
- Aggregated by outer facet via max-reduce
- Re-injected via `coordinated_spacing` in Phase 1.5

## Key Measurement Functions

### `measure_pass<DimConfig>` (facet_evaluation.rs:94)
- Orchestrates entire Pass 1 measurement
- Parameterized by dimension (Row or Column)
- Returns `FacetPass1Result` with spacing_needs

### `measure_with_scales` (mod.rs:478)
- Lightweight subplot measurement entry point
- Used by facet Pass 1 for each subplot
- Critical parameter: `data_override` for nested facet filtering
- Returns: guide_only_overflow + total_overflow + legend info

### `compute_layout_with_fixed_plot_area` (rendering.rs:862)
- Measures guide overflow and legends
- Checks for coordinated spacing in params
- Returns layout with calculated overflow

### `FacetRowGuide.measure_overflow` (guide.rs:141)
- Measures nested facet subplots
- Iterates facet domain, filters data per cell
- Creates FacetContext with position for axis visibility
- Returns aggregated overflow

## Scale Sharing Modes

| Mode | Behavior | When Used |
|------|----------|-----------|
| `Shared` | Built once from full dataset, reused for all cells | Cross-plot consistency required |
| `Free` | Built per-cell from filtered data | Cell-specific domains |
| `SharedInRow` | Per-row extents computed, same row uses same scale | Rows coordinate, columns free |
| `SharedInColumn` | Per-column extents computed, same column uses same scale | Columns coordinate, rows free |

## Overflow Types

**guide_only_overflow**
- Includes: axis ticks, labels, titles
- Used for: cross-subplot axis alignment
- Consistent across subplots with same axes

**total_overflow**
- Includes: guide_only + legends + titles + subtitles
- Used for: outer facet spacing calculation
- Accounts for all content needing space

## Spacing Needs Keys

| Key | Computed By | Used By |
|-----|-------------|---------|
| `inter_row_gap` | FacetRow (own dimension) | FacetColumn (cross-dimension) |
| `inter_col_gap` | FacetColumn (own dimension) | FacetRow (cross-dimension) |
| `legend_right` | Any facet with legend | Used for unified legend alignment |
| `legend_left` | Any facet with legend | Used for unified legend alignment |
| `legend_top` | Any facet with legend | Used for unified legend alignment |
| `legend_bottom` | Any facet with legend | Used for unified legend alignment |

## Design Patterns

### Domain Propagation
- **Problem:** Nested facets need consistent domains (grid-like layout)
- **Solution:** Outer facet computes from full dataset, passes via context
- **Key Field:** `FacetCoordinationContext.inner_domain`

### Two-Pass Gap Coordination
- **Problem:** Inner and outer facet gaps must coordinate
- **Solution:** Compute gaps in Pass 1, re-run with coordinated values in Phase 1.5
- **Trigger:** `has_cross_dimension_gap` check at line 1539

### Recursive Measurement via data_override
- **Problem:** Axis visibility depends on cell-specific data presence
- **Solution:** Pass filtered DataFrame as `data_override` through entire measurement chain
- **Flow:** Outer facet → Plot → Guide → Nested facet subplots

### Concurrent Measurement with Semaphore
- **Problem:** Too many concurrent measurements overwhelm DataFusion
- **Solution:** Semaphore with `MAX_CONCURRENT_MEASURE = 4`
- **Location:** Lines 331-333, 910-911

## Common Modifications

### Adding a new spacing coordinate key
1. In `measure_pass`: Add to `spacing_needs` (line 801-836)
2. In `Phase 1.5`: Check for presence and re-measure if needed
3. In `compute_layout_with_fixed_plot_area`: Check for value and override overflow

### Changing gap computation
- **Own-dimension gap:** Modify `max_required_gap` calculation (lines 448-481)
- **Cross-dimension gap:** Modify `measured_row_gap` or `measured_col_gap` computation (lines 631-799)

### Adjusting measurement parameters
- **Band size:** Change `subplot_dims` closure
- **Spacing formula:** Modify rounding/ceiling in gap computation
- **Concurrency:** Adjust `MAX_CONCURRENT_MEASURE` constant

## Debugging Tips

### Enable debug output
```bash
AVENGER_CHART_DEBUG_LAYOUT=1 cargo test -p avenger-chart
```

### Common debug messages
- `measure_pass starting for channel=...`: Pass 1 measurement begins
- `measure_pass gap[i->j]: gap=...`: Per-gap computation
- `Re-running measure_pass with coordinated_spacing=...`: Phase 1.5 re-measure
- `FacetRowGuide: Computing overflow from data_override...`: Nested facet measurement

### Tracing parameter flow
1. Search for `__facet_coordination` param creation
2. Track `update_outer_position_in_params` calls
3. Check `with_coordinated_spacing` builder chain
4. Verify `get_coordinated_spacing` lookups

