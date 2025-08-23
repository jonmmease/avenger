# Generic Cartesian Coordinate System Design

## Overview
Enable external crates to provide custom axis implementations for the Cartesian coordinate system while maintaining full backward compatibility.

## Core Design

### 1. Generic Cartesian Structure

```rust
// In avenger-chart/src/cartesian/coord.rs

use std::marker::PhantomData;
use crate::axis::AxisTrait;
use crate::cartesian::CartesianAxis;

/// Cartesian coordinate system with configurable axis type
/// 
/// The default axis type is `CartesianAxis`, so existing code continues to work
/// without specifying the generic parameter.
#[derive(Clone)]
pub struct Cartesian<A: AxisTrait = CartesianAxis> {
    _phantom: PhantomData<A>,
}

// Convenient constructors
impl Cartesian {
    /// Create a Cartesian coordinate system with the default axis type
    pub fn new() -> Self {
        Cartesian::<CartesianAxis>::default()
    }
}

impl<A: AxisTrait> Cartesian<A> {
    /// Create a Cartesian coordinate system with a custom axis type
    pub fn with_custom_axis() -> Self {
        Cartesian {
            _phantom: PhantomData,
        }
    }
}

impl<A: AxisTrait> Default for Cartesian<A> {
    fn default() -> Self {
        Cartesian {
            _phantom: PhantomData,
        }
    }
}
```

### 2. Updated CoordinateSystem Implementation

```rust
#[async_trait::async_trait]
impl<A> CoordinateSystem for Cartesian<A> 
where 
    A: AxisTrait + Clone + Default + Send + Sync + 'static,
{
    type Axis = A;

    fn required_channels(&self) -> &'static [&'static str] {
        &["x", "y"]
    }

    fn default_range(&self, channel: &str, width: f64, height: f64) -> Option<(f64, f64)> {
        match channel {
            "x" => Some((0.0, width)),
            "y" => Some((height, 0.0)),
            _ => None,
        }
    }

    fn create_default_axes(
        &self,
        scales: &HashMap<String, ConfiguredScale>,
        marks: &[Box<dyn Mark<Self>>],
    ) -> HashMap<String, Self::Axis> {
        let mut default_axes = HashMap::new();
        
        for channel in ["x", "y"] {
            if scales.get(channel).is_some() {
                // Use Default trait to create axis instance
                let mut axis = A::default();
                
                // If A implements a configuration trait, apply defaults
                if let Some(configurable) = axis.as_any_mut().downcast_mut::<dyn ConfigurableAxis>() {
                    configurable.set_channel(channel);
                    configurable.set_title(extract_axis_title_from_marks(marks, channel));
                }
                
                default_axes.insert(channel.to_string(), axis);
            }
        }
        
        default_axes
    }
    
    // ... rest of implementation remains the same
}
```

### 3. Enhanced AxisTrait for Better Extensibility

```rust
// In avenger-chart/src/axis.rs

/// Core trait for all axis types
pub trait AxisTrait: Send + Sync + 'static {
    fn clone_box(&self) -> Box<dyn AxisTrait>;
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
    fn into_any(self: Box<Self>) -> Box<dyn Any>;
}

/// Optional trait for axes that can be configured programmatically
pub trait ConfigurableAxis: AxisTrait {
    fn set_channel(&mut self, channel: &str);
    fn set_title(&mut self, title: Option<String>);
    fn set_position(&mut self, position: AxisPosition);
}

/// Optional trait for axes that can render themselves
pub trait RenderableAxis: AxisTrait {
    fn render(
        &self,
        scale: &ConfiguredScale,
        bounds: &AxisBounds,
    ) -> Result<Vec<SceneMark>, AvengerChartError>;
}
```

### 4. Updated Plot Methods

```rust
// In avenger-chart/src/cartesian/plot.rs

impl<A: AxisTrait + Clone + Default> Plot<Cartesian<A>> {
    pub fn axis_x<F>(mut self, f: F) -> Self
    where
        F: Fn(A) -> A + Send + Sync + 'static,
    {
        self.axis_specs
            .insert("x".to_string(), AxisSpec::Local(Arc::new(f)));
        self
    }

    pub fn axis_y<F>(mut self, f: F) -> Self
    where
        F: Fn(A) -> A + Send + Sync + 'static,
    {
        self.axis_specs
            .insert("y".to_string(), AxisSpec::Local(Arc::new(f)));
        self
    }
}
```

## Usage Examples

### Example 1: Existing Code (No Changes Required!)

```rust
use avenger_chart::{Plot, cartesian::Cartesian};

// All of these continue to work without modification
let plot1 = Plot::new(Cartesian);

let plot2 = Plot::new(Cartesian::new())
    .axis_x(|axis| axis.title("Time").grid(true))
    .axis_y(|axis| axis.title("Value"));

let plot3 = Plot::new(Cartesian::default())
    .mark(Symbol::new().x("a").y("b"));
```

### Example 2: Custom Axis from External Crate

```rust
// In external crate: custom_viz/src/lib.rs

use avenger_chart::{
    axis::{AxisTrait, ConfigurableAxis, RenderableAxis},
    cartesian::Cartesian,
    Plot,
};

/// A logarithmic axis with custom tick generation
#[derive(Clone, Default)]
pub struct LogarithmicAxis {
    title: Option<String>,
    base: f64,
    show_minor_ticks: bool,
    tick_format: Option<String>,
}

impl LogarithmicAxis {
    pub fn base(mut self, base: f64) -> Self {
        self.base = base;
        self
    }
    
    pub fn show_minor_ticks(mut self, show: bool) -> Self {
        self.show_minor_ticks = show;
        self
    }
}

impl AxisTrait for LogarithmicAxis {
    // ... implement required methods
}

impl ConfigurableAxis for LogarithmicAxis {
    fn set_title(&mut self, title: Option<String>) {
        self.title = title;
    }
    // ... other methods
}

impl RenderableAxis for LogarithmicAxis {
    fn render(&self, scale: &ConfiguredScale, bounds: &AxisBounds) 
        -> Result<Vec<SceneMark>, AvengerChartError> 
    {
        // Custom rendering logic for logarithmic axis
        // - Calculate log-spaced major ticks
        // - Add minor ticks between powers
        // - Format labels as powers of base
        todo!()
    }
}

// Usage
let plot = Plot::new(Cartesian::<LogarithmicAxis>::with_custom_axis())
    .mark(Symbol::new().x("magnitude").y("frequency"))
    .axis_x(|axis| axis.base(10.0).show_minor_ticks(true))
    .axis_y(|axis| axis.base(2.0));
```

### Example 3: Mixed Axis Types (Advanced)

For cases where you want different axis types for x and y, we could extend the design:

```rust
pub struct Cartesian2<X: AxisTrait = CartesianAxis, Y: AxisTrait = CartesianAxis> {
    _phantom: PhantomData<(X, Y)>,
}

// Usage
let plot = Plot::new(
    Cartesian2::<CartesianAxis, LogarithmicAxis>::new()
);
```

## Migration Path

### Phase 1: Add Generic Parameter (Non-Breaking)
1. Add generic parameter with default to `Cartesian`
2. Update `CoordinateSystem` implementation
3. All existing code continues to work

### Phase 2: Enhance AxisTrait (Non-Breaking)
1. Add optional traits (`ConfigurableAxis`, `RenderableAxis`)
2. Implement for existing axis types
3. Document patterns for external implementers

### Phase 3: Export Utilities (Non-Breaking)
1. Export axis rendering helpers from `avenger_guides`
2. Provide example implementations
3. Add to `avenger-chart-external-test`

## Benefits

1. **Full Backward Compatibility**: Existing code works without changes
2. **Extensibility**: External crates can provide custom axes
3. **Type Safety**: Compile-time checking of axis types
4. **Flexibility**: Can mix different axis types if needed
5. **Discoverability**: IDEs show available methods for specific axis types

## Potential Issues and Solutions

### Issue 1: Serialization
If axes need to be serialized, the generic parameter complicates things.

**Solution**: Use type erasure for serialization contexts:
```rust
pub struct SerializedPlot {
    axes: HashMap<String, Box<dyn AxisTrait>>,
}
```

### Issue 2: Default Construction
External axis types must implement `Default` or we need another approach.

**Solution**: Use a factory trait:
```rust
pub trait AxisFactory<A: AxisTrait> {
    fn create_default(&self, channel: &str) -> A;
}
```

### Issue 3: Type Inference
Complex generic parameters might confuse type inference.

**Solution**: Provide type aliases:
```rust
pub type StandardCartesian = Cartesian<CartesianAxis>;
pub type LogCartesian = Cartesian<LogarithmicAxis>;
```

## Testing Strategy

1. **Compatibility Tests**: Ensure all existing tests pass without modification
2. **Generic Tests**: Test with different axis types
3. **External Crate Test**: Add to `avenger-chart-external-test`
4. **Documentation**: Show examples of custom axis implementations

## Conclusion

This design provides a clean path to making Cartesian coordinates extensible while maintaining complete backward compatibility. The default type parameter ensures existing code continues to work, while the generic parameter enables powerful customization options for advanced users.