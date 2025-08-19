# Potential Channel Resolver Refactoring

## Current Architecture Problem

The `create_channel_resolver` uses `Scale.to_expr()` even though some scales might already be configured when it's called.

## Key Insight: Two-Stage Processing

The render pipeline processes scales in two stages:

1. **Stage 1**: Non-positional scales (size, stroke_width, fill, etc.)
   - Domains are inferred
   - Scales are fully configured
   - These scales are DONE before Stage 2 starts

2. **Stage 2**: Positional scales (x, y)  
   - Need radius-aware domain inference
   - Radius depends on size/stroke_width from Stage 1
   - Currently uses Scale.to_expr() for ALL scales

## The Opportunity

When calculating radius in Stage 2, the size and stroke_width scales have **already been processed**. We could:

1. Create ConfiguredScale objects for Stage 1 scales immediately after processing
2. Pass a hybrid resolver that uses:
   - ConfiguredScale.to_expr() for already-configured scales (size, stroke_width)
   - Scale.to_expr() for not-yet-configured scales (x, y during their own processing)

## Proposed Implementation

```rust
// After Stage 1, create configured scales for non-positional channels
let mut configured_non_positional = HashMap::new();
for (name, scale) in &non_positional_scales {
    let configured = scale.build(plot_area_width, plot_area_height).await?;
    configured_non_positional.insert(name.clone(), configured);
}

// Create hybrid resolver for Stage 2
fn create_hybrid_channel_resolver<'a>(
    mark: &'a dyn Mark<C>,
    encodings: &'a IndexMap<String, ChannelValue>,
    configured_scales: &'a HashMap<String, ConfiguredScale>,  // Already configured
    unconfigured_scales: &'a HashMap<String, Scale>,          // Not yet configured
) -> impl Fn(&str) -> Expr + 'a {
    move |channel_name: &str| -> Expr {
        // Determine scale name...
        let scale_key = ...;
        
        // Try configured scales first (size, stroke_width, etc.)
        if let Some(configured) = configured_scales.get(&scale_key) {
            // Use ConfiguredScale.to_expr()
            configured.to_expr(channel_value.expr().clone())
                .unwrap_or_else(|_| channel_value.expr().clone())
        } else if let Some(scale) = unconfigured_scales.get(&scale_key) {
            // Fall back to Scale.to_expr() for not-yet-configured scales
            scale.to_expr(channel_value.expr().clone())
                .unwrap_or_else(|_| channel_value.expr().clone())
        } else {
            channel_value.expr().clone()
        }
    }
}
```

## Benefits

1. **Partial migration**: Use ConfiguredScale where possible
2. **Better architecture**: Makes it clear which scales are configured when
3. **Future-proof**: If we later restructure to configure all scales earlier, the resolver is ready

## Challenges

1. **Complexity**: Two different scale types in the resolver
2. **Timing**: Must ensure scales are configured in the right order
3. **Self-reference**: A scale can't use its own configured version during its own domain inference

## Alternative: Full Restructuring

Alternatively, we could restructure more radically:

1. First pass: Gather all domain expressions WITHOUT any scaling
2. Infer raw domains for ALL scales
3. Create ConfiguredScale for ALL scales
4. Second pass: Calculate radius using ConfiguredScale
5. Adjust positional scale domains for padding
6. Recreate ConfiguredScale for positional scales

This would eliminate Scale.to_expr() entirely but requires two passes and recreating some scales.

## Recommendation

The hybrid approach seems most pragmatic:
- Incremental improvement
- Maintains single-pass domain inference
- Sets up for future complete migration
- Clarifies which scales are available when