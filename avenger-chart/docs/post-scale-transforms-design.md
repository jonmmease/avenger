# Post-Scale Transforms Design Report

## Executive Summary

This report explores design options for adding post-scale transforms to avenger-chart. These transforms operate in visual/screen space after scales have been applied, enabling features like dodge positioning, jitter, and smart label placement.

## Transform Categories

### 1. Position Adjustment Transforms
Modify the visual positions of existing marks after scaling.

**Examples:**
- **Dodge**: Shifts marks to avoid overlap (e.g., side-by-side bars)
- **Jitter**: Adds random noise to positions (for overplotting)
- **Nudge**: Systematic offset (e.g., for paired observations)

**Characteristics:**
- Input: Scaled mark positions and visual properties
- Output: Adjusted positions
- Preserves mark count and properties

### 2. Derivative Mark Transforms
Create new visual elements based on existing marks.

**Examples:**
- **Labels**: Text positioned relative to marks
- **Connectors**: Lines connecting related marks
- **Annotations**: Callouts, arrows, etc.

**Characteristics:**
- Input: Scaled marks with bounds
- Output: New marks with their own visual properties
- Increases total mark count

### 3. Layout Transforms
Complex algorithms that consider multiple marks simultaneously.

**Examples:**
- **Force**: Physics simulation to separate overlapping marks
- **Pack**: Circle packing layouts
- **Voronoi**: Tessellation-based layouts

**Characteristics:**
- Input: All marks in a layer/group
- Output: New positions for all marks
- May be iterative/animated

## API Design Options

### Option 1: Transform Chain Extension

Extend the current transform system with a post-scale phase:

```rust
Line::new()
    .data(df)
    .x("date")
    .y("value")
    .transform(Bin::x("value"))  // Pre-scale transform
    .adjust(Jitter::y(5.0))      // Post-scale transform

// For derivative marks
let points = Symbol::new()
    .data(df)
    .x("x")
    .y("y");

let labels = Text::new()
    .derive_from(&points)
    .text("label")
    .adjust(SmartPlace::new());  // Avoid overlaps
```

**Pros:**
- Consistent with current API
- Clear separation of transform phases
- Composable

**Cons:**
- Requires new method (`.adjust()`) 
- Derivative marks need special handling

### Option 2: Unified Transform System with Phases

Make all transforms phase-aware:

```rust
pub trait Transform {
    fn phase(&self) -> TransformPhase;
    fn transform(&self, ctx: TransformContext) -> Result<TransformOutput>;
}

pub enum TransformPhase {
    PreScale,   // Current transforms
    PostScale,  // New transforms
}

pub enum TransformContext {
    Data(DataContext),
    Visual(VisualContext),
}

// Usage
Line::new()
    .data(df)
    .x("date")
    .y("value")
    .transform(Bin::x("value"))      // PreScale
    .transform(Jitter::y(5.0))       // PostScale
```

**Pros:**
- Single transform method
- Transforms self-describe their phase
- Flexible context passing

**Cons:**
- More complex transform trait
- Runtime phase checking needed

### Option 3: Layer-Based Approach

Post-scale transforms as layer operations:

```rust
let scatter = Layer::new()
    .mark(Symbol::new()
        .data(df)
        .x("x")
        .y("y")
        .size("value"))
    .dodge_overlap()  // Layer-level transform
    .with_labels(|mark| {
        Text::new()
            .text("name")
            .position(LabelPosition::Above)
            .avoid_overlap()
    });
```

**Pros:**
- Natural for multi-mark coordination
- Aligns with grammar of graphics layers
- Good for derivative marks

**Cons:**
- Different from current mark-centric API
- May require significant restructuring

## Implementation Architecture

### Visual Context Requirements

Post-scale transforms need access to:

```rust
pub struct VisualContext {
    // Scaled mark data
    marks: Vec<ScaledMark>,
    
    // Scale functions
    scales: ScaleSet,
    
    // Visual dimensions
    viewport: Viewport,
    
    // For derivative marks
    source_marks: Option<Vec<ScaledMark>>,
}

pub struct ScaledMark {
    // Original data
    data: DataRow,
    
    // Scaled positions
    x: f64,
    y: f64,
    
    // Visual properties
    size: f64,
    shape: Shape,
    
    // Computed bounds
    bounds: Bounds,
}
```

### Execution Pipeline

```
Data → DataTransforms → Scale → PostScaleTransforms → Render
         ↓                ↓              ↓
    DataContext      ScaleSet      VisualContext
```

## Specific Transform Designs

### Jitter Transform

```rust
pub struct Jitter {
    x_amount: Option<f64>,  // Visual units
    y_amount: Option<f64>,
    seed: Option<u64>,
}

impl PostScaleTransform for Jitter {
    fn transform(&self, ctx: &mut VisualContext) -> Result<()> {
        let mut rng = /* seeded RNG */;
        
        for mark in &mut ctx.marks {
            if let Some(amount) = self.x_amount {
                mark.x += rng.gen_range(-amount..amount);
            }
            if let Some(amount) = self.y_amount {
                mark.y += rng.gen_range(-amount..amount);
            }
        }
        Ok(())
    }
}
```

### Dodge Transform

```rust
pub struct Dodge {
    padding: f64,  // Visual units between marks
    group_by: Option<String>,  // Optional grouping
}

impl PostScaleTransform for Dodge {
    fn transform(&self, ctx: &mut VisualContext) -> Result<()> {
        // Group marks by x position (and optional group)
        let groups = group_marks_by_position(&ctx.marks, &self.group_by);
        
        for group in groups {
            // Calculate total width needed
            let total_width = group.iter()
                .map(|m| m.visual_width())
                .sum::<f64>() 
                + self.padding * (group.len() - 1) as f64;
            
            // Position marks side-by-side
            let mut x_offset = -total_width / 2.0;
            for mark in group {
                mark.x += x_offset + mark.visual_width() / 2.0;
                x_offset += mark.visual_width() + self.padding;
            }
        }
        Ok(())
    }
}
```

### Smart Label Placement

```rust
pub struct SmartLabels {
    anchor: LabelAnchor,
    avoid_overlap: bool,
    avoid_marks: bool,
}

impl DerivativeTransform for SmartLabels {
    fn generate(&self, source: &[ScaledMark], ctx: &VisualContext) 
        -> Result<Vec<TextMark>> {
        
        let mut labels = Vec::new();
        
        for mark in source {
            let mut label = TextMark {
                text: mark.data.get("label"),
                x: mark.x,
                y: mark.y - mark.bounds.height / 2.0,  // Above
                // ...
            };
            
            if self.avoid_overlap {
                // Run collision detection and adjustment
                adjust_label_position(&mut label, &labels, &source);
            }
            
            labels.push(label);
        }
        
        Ok(labels)
    }
}
```

## Integration with Current System

### Extending MarkConfig

```rust
pub struct MarkConfig {
    pub mark_type: String,
    pub data: DataContext,
    pub details: Option<Vec<String>>,
    pub zindex: Option<i32>,
    pub shapes: Option<Vec<SymbolShape>>,
    
    // New fields
    pub post_scale_transforms: Vec<Box<dyn PostScaleTransform>>,
    pub derive_marks: Vec<DerivedMarkConfig>,
}

pub struct DerivedMarkConfig {
    pub mark_type: String,
    pub generator: Box<dyn DerivativeTransform>,
}
```

### Modified Rendering Pipeline

```rust
impl Chart {
    fn render(&self) -> Result<Scene> {
        let mut layers = Vec::new();
        
        for layer in &self.layers {
            // 1. Apply data transforms
            let data_ctx = apply_data_transforms(layer)?;
            
            // 2. Create marks with scales
            let mut scaled_marks = create_scaled_marks(data_ctx, &self.scales)?;
            
            // 3. Apply post-scale transforms
            let visual_ctx = VisualContext {
                marks: scaled_marks,
                scales: &self.scales,
                viewport: &self.viewport,
                source_marks: None,
            };
            
            for transform in &layer.post_scale_transforms {
                transform.transform(&mut visual_ctx)?;
            }
            
            // 4. Generate derivative marks
            let mut all_marks = visual_ctx.marks;
            for derived in &layer.derive_marks {
                let new_marks = derived.generator.generate(
                    &all_marks, 
                    &visual_ctx
                )?;
                all_marks.extend(new_marks);
            }
            
            layers.push(all_marks);
        }
        
        Ok(build_scene(layers))
    }
}
```

## Recommendations

1. **Start with Option 1** (Transform Chain Extension) as it's most compatible with the current API and provides clear phase separation.

2. **Implement a few key transforms first**:
   - `Jitter` - Simple position adjustment
   - `Dodge` - More complex, requires grouping
   - `TextLabels` - Derivative mark example

3. **Design for extensibility** - Make the post-scale transform trait flexible enough to handle future use cases like force layouts.

4. **Consider performance** - Some algorithms (collision detection) can be O(n²), so provide options for optimization (spatial indices, approximations).

5. **Plan for interactivity** - Post-scale transforms may need to re-run on zoom/pan, so design with that in mind.

## Open Questions

1. **Animation**: How do post-scale transforms interact with transitions?
2. **Interaction**: Should transforms respond to hover/selection states?
3. **Composition**: Can post-scale transforms be chained? In what order?
4. **Performance**: When to use GPU acceleration for these transforms?
5. **Declarative vs Imperative**: Should complex layouts be declarative or allow imperative code?

## Conclusion

Post-scale transforms represent a powerful addition to avenger-chart, enabling sophisticated visualizations that require spatial awareness. The proposed design maintains API consistency while adding the flexibility needed for these visual-space operations.