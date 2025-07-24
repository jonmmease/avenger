# Adjust API Design: Information Flow Analysis

## Information Available at Each Stage

### 1. Mark Definition Time
```rust
Symbol::new()
    .data(df)           // Have: Raw DataFrame
    .x("category")      // Have: Column names/expressions
    .y("value")         // Have: Column names/expressions
    .size(50.0)         // Have: Static visual properties
    .adjust(???)        // Don't have: Scales, viewport, actual values
```

**Problem**: At mark definition time, we don't know:
- Scale mappings (domain/range)
- Viewport dimensions
- Actual data values after transforms

### 2. Chart Composition Time
```rust
Chart::new()
    .width(800)
    .height(600)
    .add(scatter)       // Now know: Viewport dimensions
    .x_scale(...)       // Now know: Scale definitions
    .y_scale(...)
```

### 3. Render Time
Now we have everything:
- Scaled visual coordinates
- Viewport dimensions
- Bounding boxes
- All marks in the chart

## The Adjust API Challenge

The `.adjust()` method on marks creates a timing problem:

```rust
// This is defined before we know scales or viewport
let mark = Symbol::new()
    .data(df)
    .x("category")
    .y("value")
    .adjust(|scaled_df, dims| {
        // This closure runs later when we have scales
        // But it's defined now
    });
```

## Design Options

### Option A: Keep Adjust on Marks (Original Proposal)
```rust
trait Adjust {
    fn adjust(&self, df: DataFrame, dimensions: PlotDimensions) -> Result<DataFrame>;
}

// Usage
.adjust(Jitter::new().x(10.0))
.adjust(|df, dims| { /* custom logic */ })
```

**Pros**:
- Composable with marks
- Follows builder pattern
- Adjustments travel with marks

**Cons**:
- Defined before context is known
- Can't reference other marks
- No access to scales (only scaled values)

### Option B: Adjust as Chart-Level Operation
```rust
Chart::new()
    .add(Symbol::new().data(df).x("x").y("y").name("scatter"))
    .add(Text::new().from("scatter").text("label"))
    .adjust("scatter", Jitter::new().x(10.0))
    .adjust_all(|marks_df, scales, dims| {
        // Access to all marks and scales
    })
```

**Pros**:
- Full context available
- Can coordinate between marks
- Access to scales if needed

**Cons**:
- Breaks mark encapsulation
- More verbose
- Order of operations less clear

### Option C: Two-Phase Adjustments
```rust
// Phase 1: Mark-local adjustments (on mark)
Symbol::new()
    .adjust(Jitter::new())  // Knows it will jitter, but not by how much

// Phase 2: Chart-level adjustments (on chart)
Chart::new()
    .layout(ForceDirected::new())  // Multi-mark coordination
```

## Information Requirements Analysis

### What Adjust Functions Actually Need

1. **Jitter**:
   - ✓ Current positions (x, y)
   - ✓ Pixel amount to jitter
   - ✗ Don't need: scales, other marks

2. **Dodge**:
   - ✓ Current positions
   - ✓ Mark sizes/bounds
   - ✓ Grouping information
   - ? Maybe: viewport width (for edge cases)

3. **Smart Labels**:
   - ✓ Source mark positions
   - ✓ Text bounds
   - ✓ Other marks to avoid
   - ✓ Viewport bounds

4. **Force Layout**:
   - ✓ All mark positions
   - ✓ Mark sizes
   - ✓ Viewport bounds
   - ? Maybe: scale functions (for constraints)

## Recommendation: Scaled DataFrame is Sufficient

The DataFrame with scaled values + bbox actually provides everything needed:

```typescript
interface ScaledDataFrame {
    // Scaled visual coordinates
    x: number
    y: number
    size: number  // In visual units
    
    // Bounding box
    bbox: {
        x_min: number
        y_min: number
        x_max: number
        y_max: number
    }
    
    // Original data (for grouping/filtering)
    [original_columns]: any
}
```

This abstraction works because:
1. Adjustments operate in visual space (post-scale)
2. Bbox provides size information
3. Original columns allow grouping/filtering
4. Don't need scale functions - already applied

## API Refinement

Keep the mark-level `.adjust()` but clarify the contract:

```rust
impl<C: CoordinateSystem> Mark<C> {
    /// Add a post-scale adjustment
    /// 
    /// Adjustments are applied after scaling, in visual coordinates.
    /// The DataFrame contains:
    /// - Scaled positions (x, y, etc.) in pixels
    /// - Bounding boxes for each mark
    /// - Original data columns for reference
    pub fn adjust(mut self, adjustment: impl Into<Box<dyn Adjust>>) -> Self {
        self.config.adjustments.push(adjustment.into());
        self
    }
}
```

For multi-mark coordination, could add a chart-level API later:

```rust
Chart::new()
    .add(marks...)
    .apply_layout(|all_marks_df, dimensions| {
        // Global adjustments
    })
```