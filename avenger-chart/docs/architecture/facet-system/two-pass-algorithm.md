# Two-Pass Measurement and Rendering Algorithm

This document explains the two-pass (actually three-phase) algorithm used by the avenger-chart facet system to measure and render faceted visualizations. Understanding this algorithm is essential for maintaining or extending the facet system.

## Table of Contents

1. [Overview](#overview)
2. [Why Two Passes?](#why-two-passes)
3. [Pass 1: Measurement](#pass-1-measurement)
4. [Phase 1.5: Coordination](#phase-15-coordination)
5. [Pass 2: Rendering](#pass-2-rendering)
6. [Data Flow Diagram](#data-flow-diagram)
7. [Code References](#code-references)
8. [Re-measurement Triggers](#re-measurement-triggers)

---

## Overview

The facet evaluation algorithm consists of three phases:

| Phase | Name | Purpose |
|-------|------|---------|
| **Pass 1** | Measurement | Measure guide overflow to determine spacing needs |
| **Phase 1.5** | Coordination | Aggregate and propagate spacing across facet levels |
| **Pass 2** | Rendering | Render subplots with final coordinated dimensions |

The algorithm is implemented generically via the `FacetDimensionConfig` trait, allowing the same code to handle both row and column faceting with dimension-specific behavior injected through trait methods.

---

## Why Two Passes?

A single-pass approach cannot correctly layout faceted visualizations because:

1. **Axis label space varies by cell**: Different facet cells may have different axis tick labels (e.g., "Jan" vs "September"), requiring different overflow space.

2. **Cross-cell alignment**: All cells in a row/column must have aligned plot areas, which requires knowing the maximum overflow across all cells before positioning any of them.

3. **Nested facet coordination**: When `FacetColumn` contains `FacetRow`, the inter-row gap must be consistent across all columns. This requires measuring all inner facets first, then re-measuring with the coordinated gap.

4. **Legend alignment**: Subplots with legends need consistent margins so legends align across cells.

**Example problem solved by two-pass:**

```
Without coordination:          With coordination:
+--------+  +------------+    +--------+  +--------+
|  Plot  |  |    Plot    |    | Plot   |  |  Plot  |
|   A    |  |     B      |    |   A    |  |    B   |
+--------+  +------------+    +--------+  +--------+
  ^^^^         ^^^^             ^^^^        ^^^^
  Y-axis       Y-axis           Y-axis      Y-axis
  (short)      (long labels)    (aligned)   (aligned)
```

---

## Pass 1: Measurement

**Entry Point:** `measure_pass<DimConfig>()` in `facet_evaluation.rs:119`

Pass 1 measures each subplot to determine:
- Guide overflow (axis labels, titles)
- Required gaps between cells
- Legend positions and sizes

### Phase 1A: Initial Setup (Lines 119-231)

**Inputs:**
- `facet_coord`: Coordinate system transform (band/point spacing)
- `compiled_subplot`: The inner plot template to measure
- `dimension_scale`: Band scale for the facet dimension (row or column)
- `scale_sharing_by_channel`: Map of channel -> ScaleSharing mode (Shared, Free, Level(n))
- `df`: Full dataset (before filtering by facet value)
- `facet_expr`: Expression for faceting (e.g., `col("species")`)
- `context`: RenderContext with coordination params from outer facets

**Key Operations:**

1. **Extract coordination context** from params (if present from outer facet):
   ```rust
   let coordination_context = FacetCoordinationContext::from_params(&context.params);
   ```

2. **Determine domain source**:
   - If coordination context provides domain: use it (for grid-like nested facets)
   - Otherwise: extract domain from filtered data

3. **Build shared scale builder** (if any channels use `ScaleSharing::Shared`):
   - Extends with `shared_data_extents` for full dataset range
   - Extends with `level_based_extents` for `Level(N)` hierarchical sharing

### Phase 1B: Scale Building (Lines 246-306)

Scales are built in two stages:

1. **Initial shared scales** - Built once from full dataset with approximate bandwidth:
   ```rust
   let initial_shared_scales = compiled_subplot
       .build_scales_from_builder(builder, width, height, ...)
       .await?;
   ```

2. **Fallback scale builder** - Built from full dataset for empty cell handling:
   ```rust
   let fallback_builder = if enable_empty_cell_fallback {
       Some(compiled_subplot.build_scale_builder_from_dataframe(..., df).await?)
   } else {
       None
   };
   ```

### Phase 1C: Subplot Measurement Loop (Lines 546-680)

Subplots are measured concurrently with bounded parallelism:

```rust
const MAX_CONCURRENT_MEASURE: usize = 4;
let semaphore = Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_MEASURE));
```

**Per-subplot measurement:**

1. **Update coordination context** with position info:
   ```rust
   let params_base = FacetCoordinationContext::update_outer_position_in_params(
       &params_base, adjusted_idx, outer_count,
   );
   ```

2. **Filter data** by facet value:
   ```rust
   let filter_df = df.filter(facet_expr.eq(lit(iteration.facet_value.clone())))?;
   ```

3. **Build scales** for this cell:
   - For Shared channels: use `initial_shared_scales`
   - For Free channels: build from `filter_df`
   - For empty cells: use `fallback_builder`

4. **Measure overflow** via `measure_subplot()`:
   ```rust
   let (guide_only, total_overflow, legend_positions, spacing_needs) =
       measure_subplot(&compiled_subplot, width, height, ctx, params, scales, filter_df).await?;
   ```

**Measurement call chain:**
```
measure_subplot()
  -> compiled_subplot.measure_with_scales()
    -> compute_layout_with_fixed_plot_area()
      -> compiled_guide.measure_overflow()
        -> [If guide is FacetRowGuide]
          -> Iterates over nested facet domain
          -> Measures each nested cell's overflow
          -> Returns aggregated overflow
```

### Phase 1D: Overflow Aggregation (Lines 682-727)

Two types of overflow are tracked:

**1. Guide-only overflow** (for axis label alignment):
```rust
let global_max_overflow = {
    let mut max_overflow = OverflowSpaceRequirement::default();
    for overflow in &guide_only_measurements {
        max_overflow.top = max_overflow.top.max(overflow.top);
        max_overflow.bottom = max_overflow.bottom.max(overflow.bottom);
        max_overflow.left = max_overflow.left.max(overflow.left);
        max_overflow.right = max_overflow.right.max(overflow.right);
    }
    max_overflow
};
```

**2. Total overflow** (for spacing calculation):
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

The `calculate_adjacent_overflow` is dimension-specific:
- **Row facets**: `max(bottom[i] + top[i+1])` - vertical gaps
- **Column facets**: `max(right[i] + left[i+1])` - horizontal gaps

### Phase 1E: Gap Computation (Lines 729-773)

**Two decision paths:**

**Path A: Use coordinated gap from outer facet**
```rust
let coordinated_gap_key = if DimConfig::is_row_facet() {
    "inter_row_gap"   // FacetRow checks for gap from outer FacetColumn
} else {
    "inter_col_gap"   // FacetColumn checks for gap from outer FacetRow
};

let measured_gap_from_outer = coordination_context
    .and_then(|ctx| ctx.get_coordinated_spacing(coordinated_gap_key));
```

**Path B: Compute from local overflow**
```rust
let rounded_gap = if let Some(measured_gap) = measured_gap_from_outer {
    measured_gap  // Use pre-computed gap
} else {
    (max_required_gap + spacing).ceil()  // Compute locally
};
```

### Phase 1F: Final Scale and Geometry (Lines 775-995)

1. **Rebuild dimension scale** with measured padding:
   ```rust
   new_config.options.insert("padding_inner_px", Scalar::from_f32(rounded_gap));
   ```

2. **Recompute final rectangles** using updated scale:
   ```rust
   let final_geometry_with_padding = updated_facet_coord.transform(...)?;
   let final_rects = final_geometry_with_padding.rects.clone();
   ```

3. **Build spacing_needs** report for coordination:
   ```rust
   let mut spacing_needs = HashMap::new();
   spacing_needs.insert("inter_row_gap".to_string(), rounded_gap);  // if row facet
   spacing_needs.insert("legend_right".to_string(), global_max_overflow.right);
   // ... etc
   ```

---

## Phase 1.5: Coordination

**Location:** `evaluate_facet()` in `facet_evaluation.rs:1866-1974`

Phase 1.5 determines whether re-measurement is needed and updates coordination context.

### Decision Logic

```rust
let has_cross_dimension_gap = if DimConfig::is_col_facet() {
    pass1.spacing_needs.contains_key("inter_row_gap")  // From nested FacetRow
} else {
    pass1.spacing_needs.contains_key("inter_col_gap")  // From nested FacetColumn
};
```

### Three Outcomes

**1. Cross-dimension gaps present (nested facets)**

Re-run `measure_pass` with coordinated spacing:

```rust
if has_cross_dimension_gap {
    let mut coord_ctx = FacetCoordinationContext::from_params(&effective_context.params)
        .unwrap_or_default();
    coord_ctx = coord_ctx.with_coordinated_spacing(pass1.spacing_needs.clone());

    // Re-run measure_pass with updated context
    let pass2 = measure_pass::<DimConfig, _>(..., &updated_ctx, ...).await?;
    (pass2, updated_ctx)
}
```

**2. Spacing needs present (standalone facets)**

Pass coordination context to render_pass without re-measuring:

```rust
else if has_spacing_needs {
    let mut coord_ctx = FacetCoordinationContext::from_params(&effective_context.params)
        .unwrap_or_default();
    coord_ctx = coord_ctx.with_coordinated_spacing(pass1.spacing_needs.clone());
    (pass1, updated_ctx)  // Use pass1 results, no re-measure
}
```

**3. No coordination needed**

Use pass1 results directly:

```rust
else {
    (pass1, effective_context.clone())
}
```

---

## Pass 2: Rendering

**Entry Point:** `render_pass<DimConfig>()` in `facet_evaluation.rs:1222`

Pass 2 renders subplots using the final measurements from Pass 1 (or Phase 1.5).

### Key Steps

1. **Extract domain from final rects**:
   ```rust
   let domain_vals_final: Vec<ScalarValue> = pass1.final_rects
       .iter()
       .map(|rect| rect.value.clone())
       .collect();
   ```

2. **Concurrent rendering** with semaphore-bounded parallelism:
   ```rust
   const MAX_CONCURRENT_RENDER: usize = 4;
   ```

3. **Per-subplot rendering**:
   - Filter data by facet value
   - Build scales (shared vs. free)
   - Extend with level-based extents for `Level(N)` sharing
   - Render via `build_plot_components()`
   - Position using group origin

4. **Assemble mark groups**:
   ```rust
   let data_group = SceneGroup {
       origin,
       marks: components.data_marks,
       clip: components.clip,
       zindex: Some(0),
       ...
   };
   all_marks.push(SceneMark::Group(data_group));
   // ... guide_marks, legend_marks, title_marks, debug_marks
   ```

---

## Data Flow Diagram

```
                              INPUT
                                |
                    Full DataFrame + Facet Expression
                                |
                                v
        +-----------------------------------------------+
        |              PASS 1: MEASUREMENT              |
        +-----------------------------------------------+
        |                                               |
        |  Phase 1A: Initial Setup                      |
        |    - Extract coordination context             |
        |    - Determine domain source                  |
        |                                               |
        |  Phase 1B: Scale Building                     |
        |    - Build shared scales (if Shared mode)     |
        |    - Build fallback builder (for empty cells) |
        |                                               |
        |  Phase 1C: Subplot Measurement Loop           |
        |    For each subplot (concurrent, max 4):      |
        |    +---------------------------------------+  |
        |    | - Filter data by facet value         |  |
        |    | - Build cell scales                  |  |
        |    | - Measure guide overflow             |  |
        |    | - Collect spacing_needs from guide   |  |
        |    +---------------------------------------+  |
        |                                               |
        |  Phase 1D: Overflow Aggregation               |
        |    - Compute global_max_overflow              |
        |    - Compute max_required_gap                 |
        |    - Aggregate spacing_needs (max per key)    |
        |                                               |
        |  Phase 1E: Gap Computation                    |
        |    - Check for coordinated gap from outer     |
        |    - Compute rounded_gap                      |
        |                                               |
        |  Phase 1F: Final Scale and Geometry           |
        |    - Rebuild dimension scale with padding     |
        |    - Compute final_rects                      |
        |    - Build spacing_needs report               |
        |                                               |
        +-----------------------------------------------+
                                |
                                v
                      FacetPass1Result {
                        overflow_measurements,
                        final_dimension_scale,
                        final_shared_scales,
                        final_rects,
                        fallback_builder,
                        spacing_needs,
                      }
                                |
                                v
        +-----------------------------------------------+
        |           PHASE 1.5: COORDINATION             |
        +-----------------------------------------------+
        |                                               |
        |  Check: has_cross_dimension_gap?              |
        |    - FacetColumn: check "inter_row_gap"       |
        |    - FacetRow: check "inter_col_gap"          |
        |                                               |
        |  +-- YES (nested facets) ---------------+     |
        |  | Update coordination context          |     |
        |  | Re-run measure_pass with coord gaps  |     |
        |  | -> pass2, updated_ctx                |     |
        |  +--------------------------------------+     |
        |                                               |
        |  +-- NO + has_spacing_needs ------------+     |
        |  | Update coordination context          |     |
        |  | Use pass1 results (no re-measure)    |     |
        |  | -> pass1, updated_ctx                |     |
        |  +--------------------------------------+     |
        |                                               |
        |  +-- NO + no spacing_needs -------------+     |
        |  | Use pass1 results directly           |     |
        |  | -> pass1, effective_context          |     |
        |  +--------------------------------------+     |
        |                                               |
        +-----------------------------------------------+
                                |
                                v
                     (final_pass, final_context)
                                |
                                v
        +-----------------------------------------------+
        |              PASS 2: RENDERING                |
        +-----------------------------------------------+
        |                                               |
        |  For each subplot (concurrent, max 4):        |
        |  +---------------------------------------+    |
        |  | - Filter data by facet value         |    |
        |  | - Build scales with level extents    |    |
        |  | - Render plot components             |    |
        |  | - Position at calculated rectangle   |    |
        |  +---------------------------------------+    |
        |                                               |
        |  Assemble mark groups:                        |
        |    - Data marks (zindex 0)                    |
        |    - Guide marks (zindex 1)                   |
        |    - Legend marks (zindex 2)                  |
        |    - Title marks (zindex 3)                   |
        |    - Debug marks (zindex 100)                 |
        |                                               |
        +-----------------------------------------------+
                                |
                                v
                              OUTPUT
                                |
                    (Vec<SceneMark>, LayoutUpdates)
```

---

## Code References

### Primary Functions

| Function | Location | Purpose |
|----------|----------|---------|
| `evaluate_facet<DimConfig>` | `facet_evaluation.rs:1707` | Entry point: orchestrates three phases |
| `measure_pass<DimConfig>` | `facet_evaluation.rs:119` | Pass 1: measures all subplots |
| `render_pass<DimConfig>` | `facet_evaluation.rs:1222` | Pass 2: renders all subplots |
| `measure_subplot` | `facet_evaluation.rs:518` | Helper: measures single subplot |
| `render_subplot` | `facet_evaluation.rs:1269` | Helper: renders single subplot |

### Supporting Functions

| Function | Location | Purpose |
|----------|----------|---------|
| `build_scales_helper_with_fallback` | `scale_helpers.rs` | Builds scales with shared/free/fallback logic |
| `FacetCoordinationContext::from_params` | `coordination.rs` | Deserializes coordination context |
| `FacetCoordinationContext::to_params` | `coordination.rs` | Serializes coordination context |
| `DimConfig::calculate_adjacent_overflow` | `dimension_config.rs` | Dimension-specific gap calculation |
| `measure_with_scales` | `plot/compiled/mod.rs:478` | Lightweight subplot measurement |
| `compute_layout_with_fixed_plot_area` | `plot/compiled/rendering.rs:862` | Computes layout with fixed dimensions |

### Key Data Structures

| Type | Location | Purpose |
|------|----------|---------|
| `FacetPass1Result` | `facet_evaluation.rs:87` | Output of measurement pass |
| `FacetCoordinationContext` | `coordination.rs` | Cross-level coordination data |
| `OverflowSpaceRequirement` | `guide.rs` | Guide overflow measurements |
| `SubplotIteration` | `subplot_iterator.rs` | Per-subplot iteration data |
| `SubplotRect` | `coords.rs` | Subplot position and dimensions |

---

## Re-measurement Triggers

Re-measurement (running `measure_pass` twice) is triggered when:

### 1. Nested Facets with Cross-Dimension Gaps

When `FacetColumn` contains `FacetRow` (or vice versa), the outer facet computes the cross-dimension gap:

```
FacetColumn with nested FacetRow:
  - Pass 1 computes inter_col_gap (horizontal gaps between columns)
  - Inner FacetRowGuide computes inter_row_gap via spacing_needs
  - Phase 1.5 detects "inter_row_gap" in spacing_needs
  - Re-runs measure_pass with coordinated inter_row_gap
  - Inner FacetRows use the coordinated gap instead of computing locally
```

**Key check:**
```rust
// facet_evaluation.rs:1875-1881
let has_cross_dimension_gap = if DimConfig::is_col_facet() {
    pass1.spacing_needs.contains_key("inter_row_gap")
} else {
    pass1.spacing_needs.contains_key("inter_col_gap")
};
```

### 2. Uniform Free Scaling

When `ScaleSharing::Free` is used with uniform cell counts (to create grid-like layouts with independent scales), phantom cells are added for sizing:

```rust
// facet_evaluation.rs:382-420
let phantom_placement = uniform_cell_count
    .map(|uniform_count| PhantomPlacement::compute(band_align, domain_vals.len(), uniform_count));
```

### 3. Preventing Infinite Recursion

The algorithm guards against re-computing already-coordinated gaps:

```rust
// facet_evaluation.rs:1004-1015
let already_computed = coord_ctx
    .as_ref()
    .map(|ctx| ctx.get_coordinated_spacing("inter_row_gap").is_some())
    .unwrap_or(false);

if already_computed {
    // Skip recomputation - we're in the second pass
    None
}
```

---

## Summary

The two-pass algorithm enables sophisticated faceted layouts by:

1. **Pass 1**: Measuring all subplots to determine space requirements
2. **Phase 1.5**: Coordinating spacing across facet levels
3. **Pass 2**: Rendering with final coordinated dimensions

Key design patterns:
- **Bounded concurrency** (4 concurrent tasks) prevents overwhelming DataFusion
- **FacetCoordinationContext** serialized to params enables cross-level communication
- **Named spacing keys** (`inter_row_gap`, `inter_col_gap`, `legend_right`, etc.) provide flexible coordination
- **Generic `DimConfig` trait** allows code reuse between row and column facets

For related documentation, see:
- [Overview](overview.md) - Module structure and core types
- [Spacing Coordination](spacing-coordination.md) - Named spacing key system
- [Scale Sharing Internals](scale-sharing-internals.md) - Domain propagation mechanisms
