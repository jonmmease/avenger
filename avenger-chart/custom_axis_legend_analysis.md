# Supporting Custom Axis and Legend Implementations in External Crates

## Current Architecture Analysis

### Axis System

#### Current Structure
1. **AxisTrait** (`axis.rs`): Minimal trait with type erasure methods
   ```rust
   pub trait AxisTrait: Send + Sync {
       fn clone_box(&self) -> Box<dyn AxisTrait>;
       fn as_any(&self) -> &dyn Any;
       fn into_any(self: Box<Self>) -> Box<dyn Any>;
   }
   ```

2. **Coordinate System Integration**: Each `CoordinateSystem` has an associated `Axis` type
   ```rust
   type Axis: AxisTrait;
   
   fn create_default_axes(...) -> HashMap<String, Self::Axis>;
   async fn render_axes(...) -> Result<Vec<SceneMark>, AvengerChartError>;
   ```

3. **Concrete Implementations**: 
   - `CartesianAxis`: Full-featured axis with grid, labels, positioning
   - `PolarAxis`: Radial/angular axis support
   - `ZeroDAxis`: Empty axis for zero-dimensional coordinates

#### Key Observations
- ✅ **Already extensible**: External crates can define custom axis types via `AxisTrait`
- ✅ **Coordinate system coupling**: Axes are tied to coordinate systems (good design)
- ⚠️ **Limited trait surface**: `AxisTrait` only provides type erasure, no behavior contracts
- ⚠️ **Rendering responsibility**: Coordinate systems handle axis rendering, not axes themselves

### Legend System

#### Current Structure
1. **Legend struct** (`legend.rs`): Concrete struct, not trait-based
   ```rust
   pub struct Legend {
       pub visible: bool,
       pub title: Option<String>,
       pub position: Option<LegendPosition>,
       // ... many configuration fields
   }
   ```

2. **Legend Creation**: Handled entirely within `PlotRenderer`
   - `create_default_legends()`: Auto-generates legends for scales
   - `create_legends()`: Renders legend marks from configurations
   - Uses `avenger_guides::legend` for actual rendering

3. **Legend Types**: Determined by scale characteristics
   - Discrete legends (symbols, colors, shapes)
   - Continuous legends (gradients)
   - Size legends (nested symbols)

#### Key Observations
- ❌ **Not extensible**: `Legend` is a concrete struct, not a trait
- ❌ **Hardcoded rendering**: Legend creation logic is embedded in `PlotRenderer`
- ❌ **No customization hooks**: External crates cannot provide custom legend types
- ⚠️ **Tightly coupled**: Legend generation depends on internal scale details

## Requirements for External Extensibility

### For Custom Axes (Mostly Satisfied)
External crates can already:
1. ✅ Define custom axis types implementing `AxisTrait`
2. ✅ Create axes via coordinate system's `create_default_axes()`
3. ✅ Render axes via coordinate system's `render_axes()`

Missing capabilities:
1. ❌ Standardized axis behavior methods (e.g., `calculate_ticks()`, `format_labels()`)
2. ❌ Reusable axis rendering utilities
3. ❌ Axis-specific configuration validation

### For Custom Legends (Major Changes Needed)
External crates need to:
1. ❌ Define custom legend types
2. ❌ Control legend creation logic
3. ❌ Implement custom legend rendering
4. ❌ Integrate with scale system
5. ❌ Participate in layout calculations

## Recommended Design Changes

### 1. Enhanced Axis Trait System

```rust
/// Core trait for axis behavior
pub trait AxisBehavior: AxisTrait {
    /// Calculate tick positions based on scale
    fn calculate_ticks(&self, scale: &ConfiguredScale) -> Vec<f64>;
    
    /// Format tick labels
    fn format_labels(&self, values: &[f64]) -> Vec<String>;
    
    /// Estimate space requirements
    fn measure_overflow(&self, scale: &ConfiguredScale) -> f32;
    
    /// Validate configuration
    fn validate(&self) -> Result<(), AvengerChartError>;
}

/// Optional trait for axes that can render themselves
pub trait AxisRenderer: AxisBehavior {
    fn render(
        &self,
        scale: &ConfiguredScale,
        position: AxisPosition,
        bounds: &AxisBounds,
    ) -> Result<Vec<SceneMark>, AvengerChartError>;
}
```

### 2. Legend Trait System

```rust
/// Core legend trait
pub trait LegendSpec: Send + Sync {
    /// Clone for type erasure
    fn clone_box(&self) -> Box<dyn LegendSpec>;
    
    /// Type erasure support
    fn as_any(&self) -> &dyn Any;
    
    /// Check if this legend type supports a given scale
    fn supports_scale(&self, scale: &ConfiguredScale) -> bool;
    
    /// Estimate size requirements
    fn measure(&self, scale: &ConfiguredScale) -> (f32, f32);
    
    /// Render the legend
    fn render(
        &self,
        scale: &ConfiguredScale,
        bounds: &LegendBounds,
    ) -> Result<Vec<SceneMark>, AvengerChartError>;
}

/// Registry for legend types
pub struct LegendRegistry {
    specs: Vec<Box<dyn LegendSpec>>,
}

impl LegendRegistry {
    /// Register a custom legend type
    pub fn register(&mut self, spec: Box<dyn LegendSpec>) {
        self.specs.push(spec);
    }
    
    /// Find appropriate legend for a scale
    pub fn find_legend(&self, scale: &ConfiguredScale) -> Option<&dyn LegendSpec> {
        self.specs.iter()
            .find(|spec| spec.supports_scale(scale))
            .map(|b| b.as_ref())
    }
}
```

### 3. Plot Builder Extensions

```rust
impl<C: CoordinateSystem> Plot<C> {
    /// Register a custom legend implementation
    pub fn register_legend<L: LegendSpec + 'static>(mut self, legend: L) -> Self {
        self.legend_registry.register(Box::new(legend));
        self
    }
    
    /// Override legend for specific channel
    pub fn legend_override<L: LegendSpec + 'static>(
        mut self,
        channel: &str,
        legend: L,
    ) -> Self {
        self.legend_overrides.insert(
            channel.to_string(),
            Box::new(legend),
        );
        self
    }
}
```

## Implementation Strategy

### Phase 1: Axis Enhancements (Low Risk)
1. Add `AxisBehavior` trait with default implementations
2. Add optional `AxisRenderer` trait
3. Export axis rendering utilities from `avenger_guides`
4. Update existing axes to implement new traits

### Phase 2: Legend Trait System (Medium Risk)
1. Create `LegendSpec` trait
2. Implement trait for existing legend types
3. Add `LegendRegistry` to `Plot`
4. Refactor `PlotRenderer` to use trait-based legends

### Phase 3: External Testing (Low Risk)
1. Add examples to `avenger-chart-external-test`
2. Document patterns for custom implementations
3. Export necessary utilities and types

## Example: Custom Legend Implementation

```rust
// In external crate
use avenger_chart::legend::{LegendSpec, LegendBounds};
use avenger_scales::scales::ConfiguredScale;
use avenger_scenegraph::marks::SceneMark;

/// Custom legend that shows data distribution
pub struct DistributionLegend {
    show_histogram: bool,
    show_quartiles: bool,
}

impl LegendSpec for DistributionLegend {
    fn supports_scale(&self, scale: &ConfiguredScale) -> bool {
        // Support continuous numeric scales
        matches!(scale.scale_type(), "linear" | "log" | "pow")
    }
    
    fn measure(&self, _scale: &ConfiguredScale) -> (f32, f32) {
        // Fixed size for distribution display
        (200.0, 100.0)
    }
    
    fn render(
        &self,
        scale: &ConfiguredScale,
        bounds: &LegendBounds,
    ) -> Result<Vec<SceneMark>, AvengerChartError> {
        // Custom rendering logic
        // - Draw histogram of data distribution
        // - Show quartile markers
        // - Add density curve
        todo!()
    }
}

// Usage
let plot = Plot::new(Cartesian)
    .mark(Symbol::new().x("value").y("count").fill("category"))
    .register_legend(DistributionLegend {
        show_histogram: true,
        show_quartiles: true,
    })
    .legend_override("y", DistributionLegend::new());
```

## Benefits of This Design

1. **Extensibility**: External crates can provide custom axes and legends
2. **Type Safety**: Trait-based design maintains compile-time guarantees
3. **Backward Compatibility**: Existing code continues to work
4. **Flexibility**: Users can mix built-in and custom implementations
5. **Reusability**: Traits enable code sharing between implementations

## Risks and Mitigation

### Risks
1. **API Surface**: More traits = more maintenance
2. **Breaking Changes**: Legend refactoring affects internal code
3. **Complexity**: More abstraction layers
4. **Performance**: Dynamic dispatch overhead

### Mitigation
1. Start with minimal trait methods, expand based on needs
2. Keep existing `Legend` struct as default implementation
3. Provide good defaults and helper utilities
4. Use static dispatch where possible via generics

## Conclusion

- **Axes**: Already mostly extensible, just needs behavioral traits
- **Legends**: Requires significant refactoring to support extensibility
- **Priority**: Focus on legend system as it's the bigger limitation
- **Approach**: Incremental changes maintaining backward compatibility