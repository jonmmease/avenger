# Axis Configuration from Channel Encodings - Implementation Plan

## Overview

Enable axis configuration directly from mark channel encodings, providing the same elegant API pattern that we now have for scales and legends. This would allow axes to be configured at the point where the channel is defined, creating a more intuitive and cohesive API.

## Goal

Support this API pattern:
```rust
Symbol::new()
    .x_with(col("price"), |c| c
        .scale(|s| s.domain((0.0, 100.0)))
        .axis(|a| a.title("Price ($)").grid(true))
    )
```

## Current Architecture

### Constraints
1. **Coordinate systems are generic over axes**: `Cartesian<A: CartesianAxis>`
2. **Marks inherit axis genericity**: `Symbol<Cartesian<A>>`  
3. **ChannelValue is NOT generic**: Works across all coordinate systems
4. **Typed channels are NOT generic**: `PositionChannel` doesn't know axis type
5. **Axes stored at Plot level**: In `axis_specs: HashMap<String, AxisSpec<C::Axis>>`

### Current Channel Configuration
- **Scales**: ✅ Configured via channels (`scale()` method)
- **Legends**: ✅ Configured via channels (`legend()` method)
- **Axes**: ❌ Must be configured at plot level

## Proposed Solution: Coordinate-Specific Channel Wrappers

### Core Idea
Since marks are already generic over their coordinate system's axis type, create coordinate-specific channel wrappers that can carry axis configuration.

### Implementation Strategy

#### Phase 1: Define Coordinate-Specific Channels

```rust
// In src/cartesian/channels.rs
pub struct CartesianPositionChannel<A: CartesianAxis> {
    inner: ChannelValue,
    axis_config: Option<Arc<dyn Fn(A) -> A + Send + Sync>>,
}

impl<A: CartesianAxis + Default> CartesianPositionChannel<A> {
    pub fn scale<F>(self, f: F) -> Self 
    where F: Fn(Scale<Auto>) -> Scale<Auto> + Send + Sync + 'static
    {
        Self {
            inner: self.inner.scale(f),
            axis_config: self.axis_config,
        }
    }
    
    pub fn axis<F>(mut self, f: F) -> Self 
    where F: Fn(A) -> A + Send + Sync + 'static
    {
        self.axis_config = Some(Arc::new(f));
        self
    }
    
    pub fn band(self, band: f64) -> Self {
        Self {
            inner: self.inner.band(band),
            axis_config: self.axis_config,
        }
    }
}
```

#### Phase 2: Update Mark State (Type-Safe Version)

```rust
// In src/marks/state.rs
pub struct MarkState<C: CoordinateSystem> {
    pub channels: IndexMap<String, ChannelValue>,
    pub data: Option<DataFrame>,
    pub facet_strategy: FacetStrategy,
    
    // NEW: Store axis configurations from channels - fully type-safe!
    #[doc(hidden)]
    pub axis_configs: HashMap<String, Arc<dyn Fn(C::Axis) -> C::Axis + Send + Sync>>,
}
```

This approach is **much better** because:
- **No downcasting needed** - The type is known at compile time
- **No `Any` trait** - Direct type safety
- **No runtime type errors** - Mismatches caught at compilation
- **Better performance** - No dynamic dispatch overhead

#### Phase 3: Update Mark Position Methods

```rust
// In src/cartesian/marks/symbol.rs
impl<A: CartesianAxis + Default + 'static> Symbol<Cartesian<A>> {
    pub fn x_with<F>(self, value: impl Into<ChannelValue>, f: F) -> Self
    where
        F: FnOnce(CartesianPositionChannel<A>) -> CartesianPositionChannel<A>,
    {
        let channel_value: ChannelValue = value.into();
        let channel = CartesianPositionChannel::<A> {
            inner: channel_value,
            axis_config: None,
        };
        let configured = f(channel);
        
        // Store channel value
        let mut mark = self.with_channel_value("x", configured.inner);
        
        // Store axis config if present - type-safe, no casting!
        if let Some(axis_config) = configured.axis_config {
            mark.state_mut()
                .axis_configs
                .insert("x".to_string(), axis_config);
        }
        
        mark
    }
}
```

#### Phase 4: Extract Axis Configs During Plot Building

```rust
// In src/render.rs or plot builder
impl<C: CoordinateSystem> Plot<C> {
    fn collect_axis_configs(&mut self) {
        for mark in &self.marks {
            // Direct access, no downcasting needed!
            for (channel, axis_config) in mark.state().axis_configs.iter() {
                if let Some(axis_spec) = self.axis_specs.get_mut(channel) {
                    // Apply the configuration directly - fully type-safe
                    let configured = axis_config(axis_spec.axis.clone());
                    axis_spec.axis = configured;
                }
            }
        }
    }
}
```

#### Phase 5: Support Polar Coordinate System

```rust
// In src/polar/channels.rs
pub struct PolarPositionChannel<A: PolarAxis> {
    inner: ChannelValue,
    axis_config: Option<Arc<dyn Fn(A) -> A + Send + Sync>>,
}

impl<A: PolarAxis + Default> PolarPositionChannel<A> {
    pub fn scale<F>(self, f: F) -> Self 
    where F: Fn(Scale<Auto>) -> Scale<Auto> + Send + Sync + 'static
    {
        Self {
            inner: self.inner.scale(f),
            axis_config: self.axis_config,
        }
    }
    
    pub fn axis<F>(mut self, f: F) -> Self 
    where F: Fn(A) -> A + Send + Sync + 'static
    {
        self.axis_config = Some(Arc::new(f));
        self
    }
}

// In src/polar/marks/symbol.rs
impl<A: PolarAxis + Default + 'static> Symbol<Polar<A>> {
    pub fn r_with<F>(self, value: impl Into<ChannelValue>, f: F) -> Self
    where
        F: FnOnce(PolarPositionChannel<A>) -> PolarPositionChannel<A>,
    {
        // Similar to Cartesian x_with implementation
    }
    
    pub fn theta_with<F>(self, value: impl Into<ChannelValue>, f: F) -> Self
    where
        F: FnOnce(PolarPositionChannel<A>) -> PolarPositionChannel<A>,
    {
        // Similar to Cartesian y_with implementation
    }
}
```

## Benefits

1. **Consistency**: Same pattern for scales, axes, and legends
2. **Locality**: All channel configuration in one place
3. **Type Safety**: Compile-time checking of axis types
4. **Discoverability**: IDE autocomplete shows available options
5. **Composability**: Channel configurations can be extracted into functions

## Example Usage

### Simple Case
```rust
Symbol::new()
    .x_with(col("x"), |c| c
        .scale(|s| s.domain((0.0, 100.0)))
        .axis(|a| a.title("X Value"))
    )
```

### Complex Configuration
```rust
Symbol::new()
    .x_with(col("price"), |c| c
        .scale(|s| s.domain((0.0, 1000.0)).nice(true))
        .axis(|a| a
            .title("Price ($)")
            .grid(true)
            .format_number("$,.2f")
            .label_angle(-45.0)
            .tick_count(10)
        )
    )
    .y_with(col("quantity"), |c| c
        .scale_with::<Log>(|s| s.base(10))
        .axis(|a| a
            .title("Quantity (log scale)")
            .position(AxisPosition::Right)
            .grid(false)
        )
    )
```

### Extracting Common Configurations
```rust
fn price_channel(col_name: &str) -> impl FnOnce(CartesianPositionChannel<DefaultCartesianAxis>) -> CartesianPositionChannel<DefaultCartesianAxis> {
    move |c| c
        .scale(|s| s.nice(true))
        .axis(|a| a
            .title("Price ($)")
            .format_number("$,.2f")
        )
}

// Usage
Symbol::new()
    .x_with(col("wholesale_price"), price_channel("Wholesale"))
    .y_with(col("retail_price"), price_channel("Retail"))
```

## Implementation Tasks

- [ ] Create `CartesianPositionChannel<A>` struct
- [ ] Add `axis_configs: HashMap<String, Arc<dyn Fn(C::Axis) -> C::Axis>>` to `MarkState`
- [ ] Update `x_with`, `y_with`, `x2_with`, `y2_with` methods for Cartesian marks
- [ ] Create `PolarPositionChannel<A>` struct
- [ ] Update `r_with`, `theta_with`, `r2_with`, `theta2_with` methods for Polar marks  
- [ ] Add axis config extraction in plot builder (`collect_axis_configs`)
- [ ] Update plot-level axis merging logic (last mark wins for conflicts)
- [ ] Write unit tests for axis configuration storage and retrieval
- [ ] Write integration tests for full plot rendering with channel axes
- [ ] Add visual regression tests
- [ ] Update documentation with new patterns
- [ ] Add examples showing the new API

## Testing Strategy

1. **Unit Tests**
   - Axis config storage and retrieval
   - Type safety with different axis types
   - Config merging with plot-level axes

2. **Integration Tests**
   - Full plot rendering with channel-level axis config
   - Mixed plot-level and channel-level configs
   - External coordinate systems and axes

3. **Visual Tests**
   - Ensure axis rendering unchanged
   - Test all axis properties configurable via channels

## Risks and Mitigations

| Risk | Mitigation |
|------|------------|
| Complex generic types | Good documentation and examples |
| Performance overhead | Arc pointers are cheap; configs applied once |
| Axis config conflicts | Clear precedence rules: last mark wins |

## Success Criteria

1. All axis properties configurable via channels
2. Type-safe compilation with proper axis types
3. Performance impact < 1% on plot construction
4. Works with both Cartesian and Polar coordinate systems
5. External coordinate systems can use the same pattern

## Open Questions

1. Should we support axis configuration on non-position channels? (No - axes are only for position channels)
2. How to handle axis configuration conflicts between multiple marks? (Last mark wins, same as current plot-level axis configuration)
3. Should axis configs be composable (e.g., combining multiple configs)?
4. Do we need a builder pattern for complex axis configurations?

## Next Steps

1. Review and approve this plan
2. Create feature branch `axis-channel-config`
3. Implement Phase 1 (CartesianPositionChannel)
4. Write initial tests
5. Gather feedback on API ergonomics