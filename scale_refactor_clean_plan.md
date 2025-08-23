# Scale Type Safety Refactor Plan

## Goal
Provide compile-time type safety for scale configuration, ensuring users can only call methods that are valid for their chosen scale type.

## Recommended Design: Generic Scale with Phantom Types

### Core Structure

```rust
use std::marker::PhantomData;

/// Type-safe scale with compile-time method resolution
pub struct Scale<S: ScaleSpec = Auto> {
    scale_impl: Arc<dyn ScaleImpl>,
    domain: ScaleDomain,
    range: ScaleRange,
    options: HashMap<String, Expr>,
    _phantom: PhantomData<S>,
}

/// Marker trait for scale types
pub trait ScaleSpec: 'static {
    fn create_impl() -> Arc<dyn ScaleImpl>;
    fn name() -> &'static str;
}

// Marker types for each scale
pub struct Linear;
pub struct Log;
pub struct Pow;
pub struct Sqrt;
pub struct Symlog;
pub struct Time;
pub struct Band;
pub struct Point;
pub struct Ordinal;
pub struct Threshold;
pub struct Quantile;
pub struct Quantize;
pub struct Auto; // For automatic type inference

impl ScaleSpec for Linear {
    fn create_impl() -> Arc<dyn ScaleImpl> {
        Arc::new(LinearScale)
    }
    fn name() -> &'static str { "linear" }
}
// ... similar impls for other types
```

### Generic Methods (Available on ALL Scales)

```rust
impl<S: ScaleSpec> Scale<S> {
    /// Set the domain from data
    pub fn domain(self, domain: impl Into<ScaleDomain>) -> Self { ... }
    
    /// Set the domain as an interval
    pub fn domain_interval(self, min: impl Into<Expr>, max: impl Into<Expr>) -> Self { ... }
    
    /// Set the range
    pub fn range(self, range: impl Into<ScaleRange>) -> Self { ... }
    
    /// Set a generic option
    pub fn option(self, key: &str, value: impl Into<Expr>) -> Self { ... }
}
```

### Type-Specific Methods

```rust
// Linear scale specific methods
impl Scale<Linear> {
    pub fn nice(self, value: bool) -> Self {
        self.option("nice", value)
    }
    
    pub fn zero(self, value: bool) -> Self {
        self.option("zero", value)
    }
    
    pub fn clamp(self, value: bool) -> Self {
        self.option("clamp", value)
    }
}

// Band scale specific methods
impl Scale<Band> {
    pub fn padding_inner(self, value: f32) -> Self {
        self.option("padding_inner", value)
    }
    
    pub fn padding_outer(self, value: f32) -> Self {
        self.option("padding_outer", value)
    }
    
    pub fn align(self, value: f32) -> Self {
        self.option("align", value)
    }
    
    pub fn round(self, value: bool) -> Self {
        self.option("round", value)
    }
}

// Point scale specific methods
impl Scale<Point> {
    pub fn padding(self, value: f32) -> Self {
        self.option("padding", value)
    }
    
    pub fn align(self, value: f32) -> Self {
        self.option("align", value)
    }
}

// Ordinal scale specific methods
impl Scale<Ordinal> {
    pub fn unknown(self, value: impl Into<Expr>) -> Self {
        self.option("unknown", value)
    }
}
```

## End User API

### Creating Scales Directly

```rust
use avenger_chart::scales::{Scale, Linear, Band, Ordinal};

// Create a linear scale with type-specific methods
let linear_scale = Scale::<Linear>::new()
    .domain_interval(0.0, 100.0)
    .range_interval(0.0, 500.0)
    .nice(true)        // ✓ Compiles - Linear has nice()
    .zero(true)        // ✓ Compiles - Linear has zero()
    .clamp(false);     // ✓ Compiles - Linear has clamp()
    // .padding(0.5)   // ✗ Won't compile - Linear doesn't have padding()

// Create a band scale with different methods
let band_scale = Scale::<Band>::new()
    .domain_discrete(vec!["A", "B", "C"])
    .range_interval(0.0, 500.0)
    .padding_inner(0.1)  // ✓ Compiles - Band has padding_inner()
    .padding_outer(0.05) // ✓ Compiles - Band has padding_outer()
    .round(true);        // ✓ Compiles - Band has round()
    // .nice(true)       // ✗ Won't compile - Band doesn't have nice()

// Create an ordinal scale for colors
let color_scale = Scale::<Ordinal>::new()
    .domain_discrete(vec!["cat", "dog", "bird"])
    .range_colors(vec![BLUE, RED, GREEN])
    .unknown(GRAY);      // ✓ Compiles - Ordinal has unknown()
```

### Plot API with Explicit Types

```rust
// Explicitly typed scale configuration
plot
    .scale_x_with::<Linear>(|s| s
        .domain_interval(0.0, 100.0)
        .nice(true)
        .zero(true)
    )
    .scale_y_with::<Log>(|s| s
        .domain_interval(1.0, 1000.0)
        .base(10.0)
    )
    .scale_color_with::<Ordinal>(|s| s
        .domain_discrete(vec!["A", "B", "C"])
        .range_scheme("category10")
    );
```

### Plot API with Automatic Type Inference

```rust
// Auto type - scale type inferred from data
plot
    .scale_x(|s| s
        .domain_data("my_field")
        // No type-specific methods available here
        // Scale type determined at runtime from data type
    )
    .scale_color(|s| s
        .range_scheme("viridis")
        // Type inferred from usage context
    );
```

### Mixed Type Safety Levels

```rust
plot
    // Explicit type when you know what you want
    .scale_x_with::<Time>(|s| s
        .domain_interval(start_date, end_date)
        .nice("month")  // Time-specific nice options
    )
    // Auto type when you want inference
    .scale_y(|s| s
        .domain_data("value")
    )
    // Explicit type for fine control
    .scale_size_with::<Pow>(|s| s
        .domain_interval(0.0, 100.0)
        .range_interval(2.0, 20.0)
        .exponent(0.5)  // Pow-specific method
    );
```

## Key Benefits

1. **Compile-time safety** - Can't call invalid methods for a scale type
2. **Better IDE support** - Autocomplete shows only valid methods
3. **Clear intent** - Scale type is explicit in the code
4. **Flexible** - Can use explicit types or automatic inference
5. **Discoverable** - Each scale type's capabilities are clear from its impl block

## Implementation Notes

### Type Conversions

```rust
impl<S: ScaleSpec> Scale<S> {
    /// Convert to a different scale type
    pub fn into_type<T: ScaleSpec>(self) -> Scale<T> {
        Scale {
            scale_impl: T::create_impl(),
            domain: self.domain,
            range: self.range,
            options: self.options,
            _phantom: PhantomData,
        }
    }
    
    /// Convert to Auto type for storage
    pub fn into_auto(self) -> Scale<Auto> {
        Scale {
            scale_impl: self.scale_impl,
            domain: self.domain,
            range: self.range,
            options: self.options,
            _phantom: PhantomData,
        }
    }
}
```

### Storage and Retrieval

Since all typed scales can be converted to `Scale<Auto>`, we can store them uniformly:

```rust
struct PlotScales {
    scales: HashMap<String, Scale<Auto>>,
}

impl PlotScales {
    pub fn add<S: ScaleSpec>(&mut self, name: String, scale: Scale<S>) {
        self.scales.insert(name, scale.into_auto());
    }
    
    pub fn get(&self, name: &str) -> Option<&Scale<Auto>> {
        self.scales.get(name)
    }
}
```

## Examples of Type-Specific Behaviors

### Color Scales

```rust
impl Scale<Linear> {
    /// Linear color interpolation
    pub fn interpolate(self, mode: InterpolationMode) -> Self {
        self.option("interpolate", mode)
    }
}

impl Scale<Ordinal> {
    /// Set colors from a named scheme
    pub fn range_scheme(self, name: &str) -> Self {
        self.range(ColorScheme::from_name(name))
    }
}
```

### Positional Scales

```rust
impl Scale<Linear> {
    /// Set padding as a fraction of the domain
    pub fn padding(self, value: f32) -> Self {
        self.option("padding", value)
    }
}

impl Scale<Band> {
    /// Get the bandwidth (only available after scale is configured)
    pub fn bandwidth(&self) -> f32 {
        // Computed from domain cardinality and range
    }
}
```

### Time Scales

```rust
impl Scale<Time> {
    /// Nice domain to time intervals
    pub fn nice(self, interval: TimeInterval) -> Self {
        self.option("nice", interval)
    }
    
    /// Custom tick formatting
    pub fn tick_format(self, format: &str) -> Self {
        self.option("tick_format", format)
    }
}
```

## Advantages Over Other Approaches

### vs. Extension Traits
- **Cleaner API** - Methods are directly on the type, not imported traits
- **Better errors** - "method not found" vs "trait not in scope"
- **Simpler mental model** - One type per scale, not Scale + traits

### vs. Builder Pattern
- **Unified API** - Same Scale type throughout, not separate builders
- **Composable** - Can pass partially configured scales around
- **Consistent** - All scales work the same way

### vs. Wrapper Types
- **Less boilerplate** - No need for Deref implementations
- **Type safety throughout** - Not just at construction time
- **Cleaner storage** - Convert to Auto for uniform storage

## Migration Strategy

1. Implement new generic Scale alongside existing Scale
2. Deprecate old Scale methods
3. Update examples and documentation
4. Provide migration guide with common patterns
5. Remove old implementation in next major version