# OverflowSpaceRequirement Analysis

## Definition
File: `/Users/jonmmease/VegaFusion/repos/avenger/avenger-chart/src/guide/overflow.rs`

```rust
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct OverflowSpaceRequirement {
    pub top: f32,
    pub bottom: f32,
    pub left: f32,
    pub right: f32,
}
```

Simple struct with four fields representing space requirements in four directions.

---

## How Overflow is Measured: RELATIVE TO RECT BOUNDS

The overflow is measured as **distance BEYOND the edges of a bounding rectangle** (relative measurement).

### Cartesian Coordinate Guide (`src/cartesian/guide.rs:187-277`)

This is the clearest example of overflow measurement:

```rust
// Calculate scale boundaries (plot is at origin for measurement)
let (scale_left, scale_right) = if let Some(x_scale) = x_scale {
    let x_range = x_scale.numeric_interval_range()?;
    (x_range.0.min(x_range.1), x_range.0.max(x_range.1))
} else {
    (0.0, plot_width)
};

// Calculate overflow relative to scale boundaries
const THRESHOLD: f32 = 1.0; // Ignore overflows less than 1px
let left = (scale_left - min_x).max(0.0);      // How much beyond left edge
let right = (max_x - scale_right).max(0.0);    // How much beyond right edge
let top = (scale_top - min_y).max(0.0);        // How much beyond top edge
let bottom = (max_y - scale_bottom).max(0.0);  // How much beyond bottom edge
```

**Key insight**: 
- `left = scale_left - min_x` → if axis label extends to x = -10 and scale starts at 0, left = 10 pixels
- `right = max_x - scale_right` → if legend extends to x = 410 and scale ends at 400, right = 10 pixels
- **This is relative measurement, not absolute coordinates**

---

## Facet System Overflow Measurement

### FacetRowGuide (`src/facet/guide.rs:38-214`)

Method: `compute_max_subplot_overflow()` - shared helper used by both `measure_overflow()` and `evaluate()`

**Key behaviors:**

1. **With pre-measured overflow (from rendering pipeline):**
   - Lines 71-95: Receives `overflow: Option<&Vec<OverflowSpaceRequirement>>`
   - For each subplot's overflow in the vec:
     - Aggregates `top` and `bottom`: takes maximum across all subplots
     - Uses first subplot's `left` overflow
     - Uses last subplot's `right` overflow
   
   ```rust
   // Aggregate top/bottom across all subplots
   for overflow_item in per_facet_overflow {
       top = top.max(overflow_item.top);
       bottom = bottom.max(overflow_item.bottom);
   }
   
   // Use first subplot's left overflow
   if let Some(first) = per_facet_overflow.first() {
       max_left = max_left.max(first.left);
   }
   
   // Use last subplot's right overflow
   if let Some(last) = per_facet_overflow.last() {
       max_right = max_right.max(last.right);
   }
   ```

2. **Fallback remeasurement (cache miss):**
   - Lines 96-211: If no pre-measured overflow provided
   - Measures each subplot by calling `build_plot_components()` in Measure mode
   - Gets overflow from each component
   - Takes maximum of all overflows (lines 207-208)

### Measurement in measure_overflow() method (lines 283-440)

After computing max subplot overflow, adds additional space:

1. **For facet labels:**
   - Line 348: `measure_facet_label_slab()` returns estimated width
   - Lines 402-433: Adds label space to child overflow based on axis position

2. **For unified y-axis title:**
   - Lines 373-395: Measures unified title height if present
   - Lines 412-426: Adds title space and gap to overflow

### FacetColGuide (lines 643-1051)

Similar pattern to FacetRowGuide:
- `compute_max_subplot_overflow()` (lines 646-810)
- For edge subplots: uses first's `top`, last's `bottom`, max of `left`/`right` across all
- `measure_overflow()` (lines 884-1051): adds facet label and title space

**Note:** Uses comparison of top/bottom overflow to infer x-axis position:
```rust
let place_below = top_max > bottom_max;
```
If top overflow > bottom overflow, axis is likely at top.

---

## How Overflow Values are Used in Layout

### In evaluate() methods - extending plot bounds (examples from both guides):

**FacetRowGuide evaluate() (lines 540-556):**
```rust
let render_plot_bounds = if place_on_left {
    // Labels on left: extend leftward by left overflow
    LayoutBounds {
        x: plot_bounds.x - max_left_child,        // Subtract to move left
        y: plot_bounds.y,
        width: plot_width + max_left_child,       // Add back to width
        height: plot_height,
    }
} else {
    // Labels on right: extend rightward by right overflow
    LayoutBounds {
        x: plot_bounds.x,
        y: plot_bounds.y,
        width: plot_width + max_right_child,      // Add to width
        height: plot_height,
    }
};
```

**FacetColGuide evaluate() (lines 1138-1154):**
```rust
let render_plot_bounds = if place_below {
    // Labels below: extend downward by bottom overflow
    LayoutBounds {
        x: plot_bounds.x,
        y: plot_bounds.y,
        width: plot_width,
        height: plot_height + subplot_max_bottom,  // Add to height
    }
} else {
    // Labels above: extend upward by top overflow
    LayoutBounds {
        x: plot_bounds.x,
        y: plot_bounds.y - subplot_max_top,        // Subtract to move up
        width: plot_width,
        height: plot_height + subplot_max_top,     // Add back to height
    }
};
```

This shows that overflow values are **distances** used to extend plot bounds.

---

## Coordinate System Examples

### In evaluate() - positioning titles

**FacetRowGuide unified y-title positioning (lines 588-591):**
```rust
let x_bottom = if axis_on_right {
    plot_bounds.x + plot_width + max_right_child + gap  // Place beyond right edge
} else {
    plot_bounds.x - max_left_child - gap                 // Place beyond left edge
};
```

**FacetColGuide unified x-title positioning (lines 1209-1216):**
```rust
let y_unified = if x_axis_at_top {
    // X-axis at top: unified title goes above plot
    plot_bounds.y - subplot_max_top - gap_axis
} else {
    // X-axis at bottom: unified title goes just below x-axis labels
    plot_bounds.y + plot_height + subplot_max_bottom + gap_axis
};
```

---

## Summary

**OverflowSpaceRequirement fields represent:**
- RELATIVE DISTANCES from rect bounds where content extends
- NOT absolute coordinates
- Used to extend layout bounds and position guide elements (labels, titles)

**Measurement hierarchy in facets:**
1. Subplot overflow measured (axes, legends)
2. For facet row/col guides: aggregate/select specific subplots' overflows
3. Add space for facet labels and titles
4. Return final guide overflow requirement
5. Layout system uses these values to extend bounds and position elements
