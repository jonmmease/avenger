# Channel API Implementation Plan

## Overview
This document outlines the implementation of a type-safe channel API for avenger-chart that:
- Prevents legends on position channels at compile time
- Supports conditional encoding with mixed scaled/unscaled values
- Provides a clean, ergonomic API with paired methods
- Eliminates the need for backward compatibility

## 1. Core Channel Types

### 1.1 Update ChannelValue Enum
Location: `src/marks/channel.rs`

```rust
pub enum ChannelValue {
    /// Scaled values go through scale transformation
    Scaled {
        expr: Expr,
        scale_name: Option<String>,
        band: Option<f64>,  // Existing field for band scaling
        scale_config: Option<Arc<dyn Fn(Scale<Auto>) -> Scale<Auto> + Send + Sync>>,
        legend_config: Option<Arc<dyn Fn(Legend) -> Legend + Send + Sync>>,
    },
    
    /// Identity values bypass scaling (literals, explicit identity)
    Identity {
        expr: Expr,
    },
    
    /// Conditional encoding with test expressions and branches
    Conditional {
        conditions: Vec<(Expr, ConditionalValue)>,  // (test, value) pairs
        otherwise: ConditionalValue,  // Default/fallback value
        // Scale and legend apply to the result AFTER condition resolves
        scale_config: Option<Arc<dyn Fn(Scale<Auto>) -> Scale<Auto> + Send + Sync>>,
        legend_config: Option<Arc<dyn Fn(Legend) -> Legend + Send + Sync>>,
    }
}

/// Values in conditional branches
#[derive(Clone)]
pub enum ConditionalValue {
    /// Field reference that should be scaled (if parent is Conditional with scale)
    Field(Expr),
    
    /// Literal value that bypasses scaling
    Value(Expr),
}
```

### 1.2 Typed Channel Wrappers
Location: `src/marks/channel_types.rs`

```rust
/// Thin wrappers for compile-time type safety
#[derive(Clone)]
pub struct PositionChannel(pub ChannelValue);

#[derive(Clone)]
pub struct ColorChannel(pub ChannelValue);

#[derive(Clone)]
pub struct SizeChannel(pub ChannelValue);

#[derive(Clone)]
pub struct ShapeChannel(pub ChannelValue);

#[derive(Clone)]
pub struct OpacityChannel(pub ChannelValue);

#[derive(Clone)]
pub struct AngleChannel(pub ChannelValue);

#[derive(Clone)]
pub struct StrokeWidthChannel(pub ChannelValue);

#[derive(Clone)]
pub struct TextChannel(pub ChannelValue);

// Enable conversion back to ChannelValue for storage
impl From<PositionChannel> for ChannelValue {
    fn from(channel: PositionChannel) -> Self { channel.0 }
}

impl From<ColorChannel> for ChannelValue {
    fn from(channel: ColorChannel) -> Self { channel.0 }
}

// ... similar for all channel types
```

## 2. Channel Implementations

### 2.1 Position Channel (No Legend)
Location: `src/marks/channel_impls.rs`

```rust
impl PositionChannel {
    /// Create from expression (scaled by default)
    pub fn from_expr(expr: Expr) -> Self {
        Self(ChannelValue::Scaled {
            expr,
            scale_name: None,
            band: None,
            scale_config: None,
            legend_config: None,
            axis_config: None,
        })
    }
    
    /// Create from literal (unscaled)
    pub fn from_literal(value: f64) -> Self {
        Self(ChannelValue::Identity {
            expr: lit(value),
        })
    }

    /// Configure scale
    pub fn scale<F>(mut self, f: F) -> Self
    where F: Fn(Scale<Auto>) -> Scale<Auto> + Send + Sync + 'static {
        match &mut self.0 {
            ChannelValue::Scaled { scale_config, .. } => {
                *scale_config = Some(Arc::new(f));
            }
            ChannelValue::Identity { expr } => {
                // Convert to Scaled
                let expr = expr.clone();
                self.0 = ChannelValue::Scaled {
                    expr,
                    scale_name: None,
                    band: None,
                    scale_config: Some(Arc::new(f)),
                    legend_config: None,
                    axis_config: None,
                };
            }
            ChannelValue::Conditional { scale_config, .. } => {
                *scale_config = Some(Arc::new(f));
            }
        }
        self
    }
    
    /// Configure axis (for Cartesian coordinates)
    pub fn axis<A, F>(mut self, f: F) -> Self
    where 
        A: CartesianAxis + Default + 'static,
        F: Fn(A) -> A + Send + Sync + 'static 
    {
        let axis_fn = move |axis: Box<dyn Axis>| -> Box<dyn Axis> {
            // Try to downcast to the specific axis type
            if let Ok(typed_axis) = axis.into_any().downcast::<A>() {
                Box::new(f(*typed_axis)) as Box<dyn Axis>
            } else {
                // If it's not the expected type, create default and configure
                Box::new(f(A::default())) as Box<dyn Axis>
            }
        };
        
        match &mut self.0 {
            ChannelValue::Scaled { axis_config, .. } => {
                *axis_config = Some(Arc::new(axis_fn));
            }
            ChannelValue::Identity { expr } => {
                // Convert to Scaled with axis config
                let expr = expr.clone();
                self.0 = ChannelValue::Scaled {
                    expr,
                    scale_name: None,
                    band: None,
                    scale_config: None,
                    legend_config: None,
                    axis_config: Some(Arc::new(axis_fn)),
                };
            }
            ChannelValue::Conditional { axis_config, .. } => {
                *axis_config = Some(Arc::new(axis_fn));
            }
        }
        self
    }
    
    /// Disable axis
    pub fn no_axis(self) -> Self {
        self.axis::<DefaultCartesianAxis, _>(|mut a| {
            a.visible = false;
            a
        })
    }

    /// Opt out of scaling
    pub fn identity(self) -> Self {
        let expr = match self.0 {
            ChannelValue::Scaled { expr, .. } => expr,
            ChannelValue::Identity { expr } => expr,
            ChannelValue::Conditional { .. } => {
                // Can't convert conditional to identity
                return self;
            }
        };
        Self(ChannelValue::Identity { expr })
    }

    /// Apply band scaling
    pub fn band(mut self, ratio: f64) -> Self {
        match &mut self.0 {
            ChannelValue::Scaled { band, .. } => {
                *band = Some(ratio);
            }
            ChannelValue::Identity { expr } => {
                // Convert to Scaled with band
                let expr = expr.clone();
                self.0 = ChannelValue::Scaled {
                    expr,
                    scale_name: None,
                    band: Some(ratio),
                    scale_config: None,
                    legend_config: None,
                    axis_config: None,
                };
            }
            ChannelValue::Conditional { .. } => {
                // Band doesn't apply to conditional
                // Could log warning
            }
        }
        self
    }
    
    // No legend method - compile-time error if attempted!
}
```

### 2.2 Color Channel (With Legend)
```rust
impl ColorChannel {
    /// Create from expression (scaled by default)
    pub fn from_expr(expr: Expr) -> Self {
        Self(ChannelValue::Scaled {
            expr,
            scale_name: None,
            band: None,
            scale_config: None,
            legend_config: None,
        })
    }
    
    /// Create from literal (unscaled)
    pub fn from_literal(value: &str) -> Self {
        Self(ChannelValue::Identity {
            expr: lit(value),
        })
    }

    /// Configure scale
    pub fn scale<F>(mut self, f: F) -> Self
    where F: Fn(Scale<Auto>) -> Scale<Auto> + Send + Sync + 'static {
        // Same as PositionChannel
        match &mut self.0 {
            ChannelValue::Scaled { scale_config, .. } => {
                *scale_config = Some(Arc::new(f));
            }
            ChannelValue::Identity { expr } => {
                let expr = expr.clone();
                self.0 = ChannelValue::Scaled {
                    expr,
                    scale_name: None,
                    band: None,
                    scale_config: Some(Arc::new(f)),
                    legend_config: None,
                };
            }
            ChannelValue::Conditional { scale_config, .. } => {
                *scale_config = Some(Arc::new(f));
            }
        }
        self
    }

    /// Configure legend
    pub fn legend<F>(mut self, f: F) -> Self
    where F: Fn(Legend) -> Legend + Send + Sync + 'static {
        match &mut self.0 {
            ChannelValue::Scaled { legend_config, .. } => {
                *legend_config = Some(Arc::new(f));
            }
            ChannelValue::Identity { .. } => {
                // Legends don't apply to identity values
                // Could log warning or convert to Scaled
            }
            ChannelValue::Conditional { legend_config, .. } => {
                *legend_config = Some(Arc::new(f));
            }
        }
        self
    }
    
    /// Disable legend
    pub fn no_legend(self) -> Self {
        self.legend(|mut l| {
            l.visible = false;
            l
        })
    }

    /// Opt out of scaling
    pub fn identity(self) -> Self {
        // Same as PositionChannel
        let expr = match self.0 {
            ChannelValue::Scaled { expr, .. } => expr,
            ChannelValue::Identity { expr } => expr,
            ChannelValue::Conditional { .. } => return self,
        };
        Self(ChannelValue::Identity { expr })
    }
    
    /// Add conditional encoding
    pub fn when<E, V>(mut self, test: E, value: V) -> Self
    where 
        E: Into<Expr>,
        V: IntoColor
    {
        let test_expr = test.into();
        let value_channel = value.into_color();
        
        // Determine if the value should be scaled or not
        let conditional_value = match value_channel.0 {
            ChannelValue::Identity { expr } => ConditionalValue::Value(expr),
            ChannelValue::Scaled { expr, .. } => ConditionalValue::Field(expr),
            ChannelValue::Conditional { .. } => {
                // Nested conditionals not supported in branches
                panic!("Cannot use conditional as branch value");
            }
        };
        
        match self.0 {
            ChannelValue::Conditional { ref mut conditions, .. } => {
                // Add to existing conditions
                conditions.push((test_expr, conditional_value));
            }
            current => {
                // Convert current value to ConditionalValue for otherwise branch
                let otherwise = match current {
                    ChannelValue::Identity { expr } => ConditionalValue::Value(expr),
                    ChannelValue::Scaled { expr, .. } => ConditionalValue::Field(expr),
                    _ => unreachable!(),
                };
                
                // Create new Conditional
                self.0 = ChannelValue::Conditional {
                    conditions: vec![(test_expr, conditional_value)],
                    otherwise,
                    scale_config: None,
                    legend_config: None,
                };
            }
        }
        self
    }
}
```

### 2.3 Size Channel (With Legend)
Similar to ColorChannel but with size-specific conversions.

### 2.4 Other Channels
Shape, Opacity, Angle, StrokeWidth, Text - each with appropriate legend support or not.

## 3. Conversion Traits

Location: `src/marks/channel_traits.rs`

```rust
/// Trait for types convertible to position channels
pub trait IntoPosition {
    fn into_position(self) -> PositionChannel;
}

impl IntoPosition for PositionChannel {
    fn into_position(self) -> PositionChannel { self }
}

impl IntoPosition for Expr {
    fn into_position(self) -> PositionChannel {
        PositionChannel::from_expr(self)
    }
}

impl IntoPosition for f64 {
    fn into_position(self) -> PositionChannel {
        PositionChannel::from_literal(self)
    }
}

impl IntoPosition for f32 {
    fn into_position(self) -> PositionChannel {
        PositionChannel::from_literal(self as f64)
    }
}

impl IntoPosition for i32 {
    fn into_position(self) -> PositionChannel {
        PositionChannel::from_literal(self as f64)
    }
}

/// Trait for types convertible to color channels
pub trait IntoColor {
    fn into_color(self) -> ColorChannel;
}

impl IntoColor for ColorChannel {
    fn into_color(self) -> ColorChannel { self }
}

impl IntoColor for &str {
    fn into_color(self) -> ColorChannel {
        ColorChannel::from_literal(self)
    }
}

impl IntoColor for String {
    fn into_color(self) -> ColorChannel {
        ColorChannel::from_literal(&self)
    }
}

impl IntoColor for Expr {
    fn into_color(self) -> ColorChannel {
        ColorChannel::from_expr(self)
    }
}

// Similar traits for Size, Shape, Opacity, etc.
```

## 4. Mark API Updates

### 4.1 Remove Old Channel Methods
Delete all existing channel methods from marks (x, y, fill, size, etc.)

### 4.2 Add New Paired Methods
Location: `src/marks/symbol.rs` (and similar for other marks)

```rust
impl<C: CoordinateSystem> Symbol<C> {
    // ============= Color Channels =============
    
    /// Set fill color (simple case)
    pub fn fill(mut self, value: impl IntoColor) -> Self {
        let channel = value.into_color();
        self.state.channels.insert("fill", channel.into());
        self
    }
    
    /// Set fill color with configuration
    pub fn fill_with<F>(mut self, value: impl Into<Expr>, config: F) -> Self
    where F: FnOnce(ColorChannel) -> ColorChannel {
        let channel = config(ColorChannel::from_expr(value.into()));
        self.state.channels.insert("fill", channel.into());
        self
    }
    
    /// Set stroke color (simple case)
    pub fn stroke(mut self, value: impl IntoColor) -> Self {
        let channel = value.into_color();
        self.state.channels.insert("stroke", channel.into());
        self
    }
    
    /// Set stroke color with configuration
    pub fn stroke_with<F>(mut self, value: impl Into<Expr>, config: F) -> Self
    where F: FnOnce(ColorChannel) -> ColorChannel {
        let channel = config(ColorChannel::from_expr(value.into()));
        self.state.channels.insert("stroke", channel.into());
        self
    }
    
    // ============= Size Channels =============
    
    /// Set size (simple case)
    pub fn size(mut self, value: impl IntoSize) -> Self {
        let channel = value.into_size();
        self.state.channels.insert("size", channel.into());
        self
    }
    
    /// Set size with configuration
    pub fn size_with<F>(mut self, value: impl Into<Expr>, config: F) -> Self
    where F: FnOnce(SizeChannel) -> SizeChannel {
        let channel = config(SizeChannel::from_expr(value.into()));
        self.state.channels.insert("size", channel.into());
        self
    }
    
    // ... similar for shape, opacity, angle, etc.
}

// Position channels are coordinate-system specific
impl Symbol<Cartesian> {
    /// Set x position (simple case)
    pub fn x(mut self, value: impl IntoPosition) -> Self {
        let channel = value.into_position();
        self.state.channels.insert("x", channel.into());
        self
    }
    
    /// Set x position with configuration
    pub fn x_with<F>(mut self, value: impl Into<Expr>, config: F) -> Self
    where F: FnOnce(PositionChannel) -> PositionChannel {
        let channel = config(PositionChannel::from_expr(value.into()));
        self.state.channels.insert("x", channel.into());
        self
    }
    
    // ... similar for y, x2, y2
}

impl Symbol<Polar> {
    /// Set theta position (simple case)
    pub fn theta(mut self, value: impl IntoPosition) -> Self {
        let channel = value.into_position();
        self.state.channels.insert("theta", channel.into());
        self
    }
    
    /// Set theta position with configuration
    pub fn theta_with<F>(mut self, value: impl Into<Expr>, config: F) -> Self
    where F: FnOnce(PositionChannel) -> PositionChannel {
        let channel = config(PositionChannel::from_expr(value.into()));
        self.state.channels.insert("theta", channel.into());
        self
    }
    
    // ... similar for radius, theta2, radius2
}
```

## 5. Channel Resolution Updates

Location: `src/channel_resolution.rs`

```rust
impl ChannelResolution {
    fn process_channel_value(&mut self, name: &str, value: &ChannelValue) -> Result<()> {
        match value {
            ChannelValue::Scaled { expr, scale_name, band, scale_config, legend_config, axis_config } => {
                // Apply scale transformation
                let scale = self.get_or_create_scale(name, scale_name.as_deref())?;
                
                // Apply scale configuration if present
                if let Some(config) = scale_config {
                    scale.apply_config(config)?;
                }
                
                // Handle band if present
                if let Some(band_ratio) = band {
                    // col(":x") references are already resolved elsewhere
                    self.add_band_channel(name, expr, *band_ratio)?;
                } else {
                    self.add_scaled_channel(name, expr, scale)?;
                }
                
                // Configure legend if present
                if let Some(legend_config) = legend_config {
                    self.configure_legend(name, legend_config)?;
                }
                
                // Configure axis if present (for position channels)
                if let Some(axis_config) = axis_config {
                    self.configure_axis(name, axis_config)?;
                }
            }
            
            ChannelValue::Identity { expr } => {
                // Bypass scaling
                self.add_unscaled_channel(name, expr)?;
            }
            
            ChannelValue::Conditional { conditions, otherwise, scale_config, legend_config, axis_config } => {
                // Build CASE expression with mixed scaled/unscaled branches
                let case_expr = self.build_conditional_expr(
                    conditions, 
                    otherwise, 
                    scale_config.is_some()
                )?;
                
                // Apply scale config to the entire result if present
                if let Some(config) = scale_config {
                    let scale = self.get_or_create_scale(name, None)?;
                    scale.apply_config(config)?;
                    self.add_scaled_channel(name, &case_expr, scale)?;
                } else {
                    // No scaling - all values treated as literals
                    self.add_unscaled_channel(name, &case_expr)?;
                }
                
                // Configure legend for the channel
                if let Some(legend_config) = legend_config {
                    self.configure_legend(name, legend_config)?;
                }
                
                // Configure axis if present
                if let Some(axis_config) = axis_config {
                    self.configure_axis(name, axis_config)?;
                }
            }
        }
        
        Ok(())
    }
    
    fn build_conditional_expr(
        &mut self,
        conditions: &[(Expr, ConditionalValue)],
        otherwise: &ConditionalValue,
        apply_scale: bool,
    ) -> Result<Expr> {
        // Build a CASE WHEN expression
        let mut case_expr = CaseBuilder::new();
        
        // Add each condition
        for (test, value) in conditions {
            let branch_expr = match (value, apply_scale) {
                (ConditionalValue::Field(expr), true) => {
                    // This branch will be scaled by the parent
                    expr.clone()
                }
                (ConditionalValue::Field(expr), false) | 
                (ConditionalValue::Value(expr), _) => {
                    // Value branch or no scaling - use as is
                    expr.clone()
                }
            };
            case_expr = case_expr.when(test.clone(), branch_expr);
        }
        
        // Add otherwise branch
        let otherwise_expr = match (otherwise, apply_scale) {
            (ConditionalValue::Field(expr), true) => {
                // This branch will be scaled by the parent
                expr.clone()
            }
            (ConditionalValue::Field(expr), false) | 
            (ConditionalValue::Value(expr), _) => {
                // Value branch or no scaling - use as is
                expr.clone()
            }
        };
        
        Ok(case_expr.otherwise(otherwise_expr).build())
    }
}
```

## 6. Migration Strategy

### 6.1 Phase 1: Add New Types
1. Add typed channel wrappers
2. Add conversion traits
3. Update ChannelValue with Conditional variant

### 6.2 Phase 2: Update Marks
1. Add new paired methods (`fill`, `fill_with`, etc.)
2. Remove old methods
3. Update channel macros if needed

### 6.3 Phase 3: Update Tests
1. Update all test files to use new API
2. Ensure visual tests still pass
3. Add tests for conditional encoding

### 6.4 Phase 4: Cleanup
1. Remove plot-level scale/legend methods (except generic ones)
2. Remove old ChannelExpr trait methods
3. Update documentation

## 7. Example Usage

```rust
use avenger_chart::{Plot, Symbol, Cartesian, DefaultCartesianAxis};
use avenger_chart::axis::AxisPosition;
use datafusion::prelude::*;

let plot = Plot::<Cartesian>::new()
    .data(df)
    .mark(
        Symbol::new()
            // Simple cases - clean and intuitive
            .x(col("x_value"))
            .y(col("y_value"))
            .fill("#3498db")
            .size(100.0)
            
            // Configured position with axis - no legend available (compile error if tried)
            .x_with(col("category"), |c| c
                .scale(|s| s.domain((0.0, 100.0)))
                .band(0.8)
                .axis::<DefaultCartesianAxis, _>(|a| a
                    .title("Categories")
                    .position(AxisPosition::Top)
                    .label_angle(-45.0)
                )
            )
            
            // Y axis with grid and formatting
            .y_with(col("value"), |c| c
                .scale(|s| s.nice(true))
                .axis::<DefaultCartesianAxis, _>(|a| a
                    .title("Values")
                    .grid(true)
                    .format(",.0f")  // Thousands separator, no decimals
                )
            )
            
            // Configured color - legend available, no axis
            .fill_with(col("category"), |c| c
                .scale(|s| s.scheme("tableau10"))
                .legend(|l| l.title("Category").position(Right))
                // .axis() would be compile error - color channels don't have axes
            )
            
            // Conditional encoding with mixed scaling
            .size_with(col("population"), |c| c
                .when(col("selected"), 200.0)  // Unscaled literal (ConditionalValue::Value)
                .when(col("highlight"), col("alt_size"))  // Scaled field (ConditionalValue::Field)
                .scale(|s| s.domain((0.0, 1_000_000.0)))  // Scale applies to Field branches
                .legend(|l| l.title("Population"))
            )
    );
```

### Understanding Conditional Behavior

When using conditional encoding:

1. **Literal values** (strings, numbers) become `ConditionalValue::Value` and bypass scaling
2. **Field references** (columns) become `ConditionalValue::Field` and are scaled if the parent has a scale
3. **The scale/legend configuration applies to the entire channel**, not individual branches

```rust
// Example: Mix of scaled and unscaled values
.fill_with(col("category"), |c| c
    .when(col("highlight"), "red")        // "red" -> Value (not scaled)
    .when(col("selected"), "blue")        // "blue" -> Value (not scaled)  
    // otherwise uses col("category")      // Field (will be scaled)
    .scale(|s| s.scheme("tableau10"))     // Applies only to the Field branch
    .legend(|l| l.title("Category"))      // Applies to the whole channel
)
```

## 8. Benefits

1. **Type Safety**: Compile-time prevention of invalid configurations (e.g., legends on position channels)
2. **Clean API**: Simple cases remain simple, complex cases are explicit
3. **Flexibility**: Full control over scaling, legends, and conditional encoding
4. **Discoverability**: IDE autocomplete shows only valid methods for each channel type
5. **Future-Proof**: Easy to add specialized legend types later without breaking changes

## 9. Testing Strategy

### Unit Tests
- Channel type conversions
- Configuration methods
- Conditional encoding logic

### Integration Tests
- Mark building with new API
- Channel resolution with all ChannelValue variants
- Scale and legend configuration

### Visual Tests
- Ensure existing visual tests pass with new API
- Add tests for conditional encoding visualizations

## 10. Documentation Updates

- Update all examples to use new API
- Document type-safe channel pattern
- Provide migration guide from old API
- Add conditional encoding examples