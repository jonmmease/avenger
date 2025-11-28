# Coordinate Transform Analysis: Regular Marks vs Facets

## Summary

Regular marks (Symbol, Rect, Line) use `coord.transform()` to convert scaled position data into scene mark coordinates. **Facets already MOSTLY use this pattern correctly** but there's an asymmetry: regular marks use one geometry type output, while facets use a different approach with `band_iter_pass2`.

**Key Finding**: `SubplotGeometry` contains EXACTLY the information needed to position scene groups. The `band_iter_pass2` exists mainly for iteration convenience and contains the same geometric information as `SubplotGeometry`.

---

## Part 1: How Regular Marks Use coord.transform()

### Example: Cartesian Symbol (avenger-chart/src/cartesian/marks/symbol.rs:126-168)

```
evaluate_from_data():
  1. Extract position channels (x, y)
  2. Call coord.transform(&position_channels, None, width, height)
     -> Returns: Box<dyn PlotGeometry>
  3. Downcast to PointGeometry (the geometry type for Cartesian)
     -> Fields: x: ScalarOrArray<f32>, y: ScalarOrArray<f32>
  4. Use geometry.x and geometry.y directly in SceneSymbolMark creation
```

**Result**: Position data flows: Data → Scale → Coordinates (via transform) → Scene Mark

### Example: Cartesian Rect (avenger-chart/src/cartesian/marks/rect.rs:136-200)

```
evaluate_from_data():
  1. Extract TWO position channels:
     - corner1: x, y
     - corner2: x2, y2
  2. Call coord.transform() TWICE (once per corner):
     - geometry1 = coord.transform(&corner1_channels, None, width, height)
     - geometry2 = coord.transform(&corner2_channels, None, width, height)
  3. Downcast both to PointGeometry
  4. Extract x and y from both geometries
  5. Create SceneRectMark with both sets of coordinates
```

**Result**: Dual corners transformed separately, results merged into single mark

---

## Part 2: The PlotGeometry Type System

### Base Type (avenger-chart/src/coords.rs:14-31)

```rust
#[typetag::serde(tag = "type")]
pub trait PlotGeometry: Send + Sync + 'static {
    fn as_any(&self) -> &dyn std::any::Any;
}
```

### PointGeometry (for Cartesian, Polar, ZeroD)

```rust
pub struct PointGeometry {
    pub x: ScalarOrArray<f32>,
    pub y: ScalarOrArray<f32>,
}
```

### SubplotGeometry (for Facet coordinates)

```rust
pub struct SubplotGeometry {
    pub rects: Vec<SubplotRect>,  // One rect per facet
}

pub struct SubplotRect {
    pub value: ScalarValue,       // The facet value (e.g., "setosa")
    pub x: f32,                   // Position in plot coordinates
    pub y: f32,
    pub width: f32,               // Size of subplot
    pub height: f32,
}
```

**Key difference**: 
- PointGeometry: Arrays of per-data-point coordinates
- SubplotGeometry: One rect per unique facet value describing its bounding box

---

## Part 3: How Facet Coordinates Work

### The Transform Method (avenger-chart/src/facet/coord.rs)

**FacetRow.transform()** receives:
- `position_channels`: HashMap with key="row" → ScalarOrArray<f32> (band positions)
- `position_values`: HashMap with key="row" → Vec<ScalarValue> (actual facet values like "A", "B", "C")
- `plot_width`, `plot_height`: Available space

**Returns**: `SubplotGeometry` containing:
- One `SubplotRect` per band position
- Each rect has:
  - `value`: The original domain value from position_values
  - Position and size: Computed from band layout

**Example for FacetRow** (lines 103-149):
```
Input:
  - row positions: [0.0, 100.0, 200.0]  (from band scale)
  - row values: ["A", "B", "C"]
  - plot_height: 300.0

Output: SubplotGeometry {
  rects: [
    SubplotRect { value: "A", x: 0, y: 0, width: 300, height: 100 },
    SubplotRect { value: "B", x: 0, y: 100, width: 300, height: 100 },
    SubplotRect { value: "C", x: 0, y: 200, width: 300, height: 100 },
  ]
}
```

### Band Layout Computation

`compute_band_layout()` (lines 46-80):
1. Takes: positions (array from scale), extent (plot height/width), padding
2. Returns: (starts, bandwidth)
   - starts: Vector of start positions for each band
   - bandwidth: Single value shared by all bands (effective height/width after padding)

This is EXACTLY what `BandPosition` contains.

---

## Part 4: Current Facet Implementation (Two-Pass Algorithm)

### PASS 1 (avenger-chart/src/facet/marks/facet_evaluation.rs:181-310)

```
Purpose: Measure guide overflow to determine spacing

1. Extract initial band positions from dimension scale
2. Call facet_coord.transform(positions, values, width, height)
   -> Get initial_rects: Vec<SubplotRect>
3. For each rect/facet:
   - Render subplot with Measure mode
   - Collect overflow measurements
4. Calculate max required gap from overflow
```

### PASS 2 (avenger-chart/src/facet/marks/facet_evaluation.rs:356-599)

```
Purpose: Render with final padding applied

1. REBUILD facet dimension scale with measured padding
   - Scale now has: padding_inner_px = rounded_gap
   
2. Call facet_coord.transform() WITH REBUILT SCALE
   -> Get final_rects: Vec<SubplotRect>
   -> Now rects have correct positions including padding

3. Extract band_iter_pass2 from rebuilt scale
   - BandPositionIterator::from_scale(&final_dimension_scale)
   - Returns: Iterator of BandPosition (value, start position, bandwidth)

4. For each (SubplotIterator, BandPositionIterator) pair:
   - Also zip with final_rects
   - Extract position and size from rect OR band_pos
   - Create group with origin: [rect.x, rect.y]
   - Render subplot at that position
```

### Current State of band_iter_pass2

**Where it's used** (facet.rs:1030-1160, facet_evaluation.rs:533-599):

1. **In FacetGrid**: Used to iterate over grid positions
   ```rust
   let row_band_iter_pass2 = BandPositionIterator::from_scale(row_scale)?;
   let col_band_iter_pass2 = BandPositionIterator::from_scale(col_scale)?;
   
   for (iteration, band_pos) in subplot_iter.zip(band_iter) {
       let x_offset = band_pos.start();
       let y_offset = band_pos.start();
       // Use x_offset, y_offset for group origin
   }
   ```

2. **In FacetRow/Col**: Also used to extract position:
   ```rust
   for ((iteration, band_pos), rect) in subplot_iter.zip(band_iter).zip(final_rects) {
       let band_size = rect.height;  // Extract from rect, not band_pos!
       let (width, height) = subplot_dims(band_size, context);
   }
   ```

**The Asymmetry**:
- The code sometimes uses `band_pos.start()` for positioning
- The code sometimes uses `rect.x` or `rect.y` for positioning  
- The `final_rects` come directly from `coord.transform()` and already have x, y, width, height

---

## Part 5: Redundancy Analysis

### What band_iter_pass2 Provides

From `BandPositionIterator` (band_positions.rs):
```
value: ScalarValue       -> Same as SubplotRect.value
position: f32 (start())  -> Same as SubplotRect.x (for FacetColumn) or .y (for FacetRow)
bandwidth: f32           -> Same as SubplotRect.width (for FacetColumn) or .height (for FacetRow)
```

### What SubplotGeometry Provides

From `coord.transform()` output (facet/coord.rs):
```
For each SubplotRect:
  value: ScalarValue      -> The facet key
  x, y: f32               -> Position in plot coordinates
  width, height: f32      -> Dimensions of subplot area
```

**They contain the SAME information!**

### Current Code Pattern

The code often does:
```rust
for ((iteration, band_pos), rect) in subplot_iter.zip(band_iter).zip(final_rects) {
    let band_size = if DimConfig::is_row_facet() {
        rect.height  // <-- Uses rect, not band_pos!
    } else {
        rect.width   // <-- Uses rect, not band_pos!
    };
    let x_offset = band_pos.start();  // <-- Could use rect.x
    let y_offset = band_pos.start();  // <-- Could use rect.y
}
```

Could simplify to:
```rust
for (rect, iteration) in final_rects.iter().zip(subplot_iter) {
    let (width, height) = subplot_dims(
        if DimConfig::is_row_facet() { rect.height } else { rect.width },
        context
    );
    let x_offset = rect.x;
    let y_offset = rect.y;
}
```

---

## Part 6: Why Facets Currently Use band_iter_pass2

### Legitimate Use Cases

1. **Iteration pairing**: Facets need to pair SubplotIterator (logical) with band positions (geometric)
   - SubplotIterator manages FacetContext for scale evaluation
   - Band positions provide the geometric layout
   - Zipping them ensures 1:1 correspondence

2. **Scale building**: Used to extract bandwidth for subplot dimensions
   ```rust
   let band_size = band_pos.bandwidth;
   let (width, height) = subplot_dims(band_size, context);
   ```
   This bandwidth is used to BUILD SCALES, not just position groups.

3. **Fallback positioning** (facet.rs:1064-1075): If band scale doesn't exist
   ```rust
   let band_pos = BandPosition::new(value, idx as f32 * band_h, band_h);
   ```

### Why SubplotGeometry Alone Isn't Sufficient (Currently)

**The issue**: SubplotGeometry provides position/size, but not per-subplot bandwidth in an accessible way for scale building.

Current flow:
```
BandPositionIterator
  → bandwidth (shared across all bands)
  → Used to compute (width, height) for subplot scale building

SubplotGeometry
  → width/height (per-rect, correct values)
  → But accessed via iteration over rects, less convenient for building
```

---

## Part 7: Key Architectural Difference

### Regular Marks Pattern
```
Position Data
    ↓
coord.transform()
    ↓
PointGeometry (x, y arrays)
    ↓
Direct use in SceneMark creation
```

### Facet Pattern (CURRENT)
```
Band Scale
    ├→ BandPositionIterator (for iteration + bandwidth)
    │
Band Positions (scaled)
    ↓
coord.transform()
    ↓
SubplotGeometry (rects with position + size)
    ↓
Used for group origin positioning
```

The extra `band_iter_pass2` exists because:
1. Iteration convenience (pairing with SubplotIterator)
2. Bandwidth extraction for scale building
3. NOT because positioning information is missing

---

## Part 8: Can Facets Work "Like Regular Marks"?

### The Answer: Yes, Mostly, With Caveats

**What Would Need to Change**:

1. **Remove explicit band_iter_pass2**: Rely entirely on SubplotGeometry rects
   ```rust
   // Current
   for ((iteration, band_pos), rect) in subplot_iter.zip(band_iter).zip(final_rects) {
       let x_offset = band_pos.start();
   }
   
   // Could be
   for (rect, iteration) in final_rects.iter().zip(subplot_iter) {
       let x_offset = rect.x;
   }
   ```

2. **Store bandwidth in context**: For scale building, extract from first rect
   ```rust
   let band_size = if DimConfig::is_row_facet() {
       final_rects[0].height
   } else {
       final_rects[0].width
   };
   ```

3. **Reconstruct iteration order**: Without band_iter_pass2 for pairing, need another way
   - Could enhance SubplotGeometry to include iteration index
   - Or rely on rect.value matching with SubplotIterator's order

**Challenges**:
- SubplotIterator and SubplotGeometry order MUST match
- Bandwidth must be consistent across all rects (currently assumed)
- Would lose the explicit "BandPosition" pairing pattern

---

## Part 9: SubplotGeometry Information Content

### Fields Available
```rust
pub struct SubplotRect {
    pub value: ScalarValue,   // ✓ Facet key
    pub x: f32,               // ✓ Position
    pub y: f32,               // ✓ Position
    pub width: f32,           // ✓ Subplot width
    pub height: f32,          // ✓ Subplot height
}
```

### What's Missing (if tried to use ONLY SubplotGeometry)
```
- Iteration order guarantee (implicitly ordered, but not explicit)
- Index/position in iteration (would need to track loop counter)
- Bandwidth as separate value (embedded in width/height)
```

### Could Be Added If Needed
```rust
pub struct SubplotRect {
    pub value: ScalarValue,
    pub index: usize,         // <- Iteration index
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}
```

---

## Conclusion

**SubplotGeometry contains sufficient information to position scene groups.** The `band_iter_pass2` is primarily used for:

1. **Iteration pairing convenience** - Keeps band positions alongside logical facet iterations
2. **Bandwidth extraction** - Provides a consistent bandwidth value for scale building
3. **Fallback handling** - Enables fallback to index-based positioning when band scale missing

The faceting system already follows the "use coord.transform()" pattern correctly. The `band_iter_pass2` is an optimization/convenience layer, not a necessity for positioning.

**Making facets "work exactly like regular marks" would require**:
- Removing explicit band iteration in favor of rect iteration
- Storing bandwidth metadata in SubplotGeometry or context
- Ensuring SubplotIterator and SubplotGeometry ordering always aligns
