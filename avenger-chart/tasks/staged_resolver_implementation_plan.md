# Staged Channel Resolver Implementation Plan

## Goal
Eliminate `Scale.to_expr()` usage for non-positional channels by using `ConfiguredScale` objects that are already built during Stage 1 of scale processing.

## Current Processing Flow

```
Stage 1: Process non-positional scales (size, stroke_width, fill, etc.)
  ├─ Infer domains from data
  ├─ Apply default ranges
  └─ Scale is complete ✓

Stage 2: Process positional scales (x, y)
  ├─ Need to calculate radius for padding
  ├─ Radius depends on size/stroke_width expressions
  ├─ Currently uses Scale.to_expr() for ALL scales ❌
  └─ Could use ConfiguredScale for Stage 1 scales ✓
```

## Implementation Steps

### Step 1: Modify render() to build ConfiguredScale after Stage 1

**File**: `src/render.rs`
**Location**: After line 216 (end of Stage 1)

```rust
// After Stage 1: Create ConfiguredScale for non-positional scales
let mut configured_non_positional = HashMap::new();
for (name, scale) in &non_positional_scales {
    let configured = scale
        .clone()
        .build(plot_area_width, plot_area_height)
        .await?;
    configured_non_positional.insert(name.clone(), configured);
}
```

### Step 2: Update create_channel_resolver signature

**File**: `src/plot.rs`
**Current signature**:
```rust
fn create_channel_resolver<'a>(
    mark: &'a dyn Mark<C>,
    encodings: &'a indexmap::IndexMap<String, ChannelValue>,
    scales: &'a HashMap<String, Scale>,
) -> impl Fn(&str) -> Expr + 'a
```

**New signature**:
```rust
fn create_channel_resolver<'a>(
    mark: &'a dyn Mark<C>,
    encodings: &'a indexmap::IndexMap<String, ChannelValue>,
    configured_scales: &'a HashMap<String, ConfiguredScale>,  // For already-processed scales
    unconfigured_scales: &'a HashMap<String, Scale>,          // For scales still being processed
) -> impl Fn(&str) -> Expr + 'a
```

### Step 3: Implement hybrid resolver logic

**File**: `src/plot.rs`
**Location**: Inside `create_channel_resolver`

```rust
move |channel_name: &str| -> Expr {
    if let Some(channel_value) = encodings.get(channel_name) {
        match channel_value {
            ChannelValue::Identity { .. } => {
                channel_value.expr().clone()
            }
            ChannelValue::Scaled { scale_name: custom_scale_name, band, .. } => {
                let scale_key = custom_scale_name
                    .as_ref()
                    .cloned()
                    .unwrap_or_else(|| strip_trailing_numbers(channel_name).to_string());

                // Try configured scales first (non-positional channels)
                if let Some(configured) = configured_scales.get(&scale_key) {
                    use crate::scales::ConfiguredScaleDataFusionExt;
                    
                    if let Some(band_value) = band {
                        configured.to_expr_with_band(channel_value.expr().clone(), *band_value)
                            .unwrap_or_else(|_| channel_value.expr().clone())
                    } else {
                        configured.to_expr(channel_value.expr().clone())
                            .unwrap_or_else(|_| channel_value.expr().clone())
                    }
                } else if let Some(scale) = unconfigured_scales.get(&scale_key) {
                    // Fall back to Scale.to_expr() for positional scales
                    let scale = if let Some(band_value) = band {
                        let scale_type = scale.get_scale_impl().scale_type();
                        if scale_type == "band" || scale_type == "point" {
                            scale.clone().option("band", lit(*band_value))
                        } else {
                            scale.clone()
                        }
                    } else {
                        scale.clone()
                    };
                    
                    scale.to_expr(channel_value.expr().clone())
                        .unwrap_or_else(|_| channel_value.expr().clone())
                } else {
                    channel_value.expr().clone()
                }
            }
        }
    } else if let Some(default_scalar) = mark.default_channel_value(channel_name) {
        lit(default_scalar)
    } else {
        lit(ScalarValue::Null)
    }
}
```

### Step 4: Update gather_scale_domain_expressions_with_radius

**File**: `src/plot.rs`
**Location**: Around line 709

Change from:
```rust
pub fn gather_scale_domain_expressions_with_radius(
    &self,
    scale_name: &str,
    scales: &HashMap<String, Scale>,
) -> Result<ScaleDomainWithRadius, AvengerChartError>
```

To:
```rust
pub fn gather_scale_domain_expressions_with_radius(
    &self,
    scale_name: &str,
    configured_scales: &HashMap<String, ConfiguredScale>,
    unconfigured_scales: &HashMap<String, Scale>,
) -> Result<ScaleDomainWithRadius, AvengerChartError>
```

Update the call to `create_channel_resolver`:
```rust
let resolve_channel = Self::create_channel_resolver(
    mark.as_ref(), 
    &resolved_encodings, 
    configured_scales,
    unconfigured_scales
);
```

### Step 5: Update infer_scale_domain in render.rs

**File**: `src/render.rs`
**Location**: Around line 2214

Pass the configured non-positional scales:
```rust
async fn infer_scale_domain(
    &self,
    scale: &mut Scale,
    name: &str,
    plot_area_width: f32,
    plot_area_height: f32,
    configured_non_positional: &HashMap<String, ConfiguredScale>,  // Add this parameter
    all_scales: &HashMap<String, Scale>,
) -> Result<(), AvengerChartError> {
    // ...
    let data_expressions_with_radius = self
        .plot
        .gather_scale_domain_expressions_with_radius(
            name, 
            configured_non_positional,  // Pass configured scales
            all_scales                  // Pass unconfigured scales
        )?;
    // ...
}
```

### Step 6: Update process_scale to pass configured scales

**File**: `src/render.rs`  
**Location**: Around line 2453

Add parameter for configured scales:
```rust
async fn process_scale<F>(
    &self,
    scale: &mut Scale,
    name: &str,
    plot_area_width: f32,
    plot_area_height: f32,
    configured_non_positional: &HashMap<String, ConfiguredScale>,  // Add this
    all_scales: &HashMap<String, Scale>,
    apply_scale_specific: F,
) -> Result<(), AvengerChartError>
```

### Step 7: Update Stage 2 processing loop

**File**: `src/render.rs`
**Location**: Around line 226

```rust
for name in pos_scale_names {
    let scale = positional_scales.get_mut(&name).unwrap();
    self.process_scale(
        scale,
        &name,
        plot_area_width,
        plot_area_height,
        &configured_non_positional,  // Pass configured non-positional scales
        &all_scales_for_process,
        |_scale_copy| {
            // Padding is now handled by radius-aware domain calculation
        },
    )
    .await?;
}
```

## Benefits

1. **Correctness**: Non-positional scales are fully configured when used for radius
2. **Performance**: No redundant scale expression building
3. **Architecture**: Clear separation between configured and unconfigured scales
4. **Migration Path**: Moves us closer to eliminating Scale.to_expr()

## Testing Strategy

1. **Verify radius calculations**: Ensure padding still works correctly
2. **Test all mark types**: Symbol, Line, Rect with various encodings
3. **Check scale combinations**: Ordinal size, linear stroke_width, etc.
4. **Performance**: Ensure no regression in render speed

## Edge Cases to Consider

1. **Self-reference**: A scale shouldn't use its own configured version
2. **Missing scales**: Handle gracefully when scales aren't found
3. **Band parameters**: Ensure band/point scales work with the hybrid approach
4. **Identity channels**: Should bypass scaling entirely

## Success Criteria

- [ ] All tests pass
- [ ] Radius-aware padding works correctly
- [ ] No uses of Scale.to_expr() for non-positional channels
- [ ] Performance is same or better
- [ ] Code is cleaner and more maintainable