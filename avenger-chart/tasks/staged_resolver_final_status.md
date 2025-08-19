# Staged Resolver Implementation - Final Status

## ✅ Implementation Complete

We have successfully implemented a staged approach that eliminates `Scale.to_expr()` usage for non-positional channels.

## What We Achieved

### Before
- Channel resolver used `Scale.to_expr()` for ALL channels
- Non-positional scales were processed but not utilized efficiently
- Radius calculations used unoptimized scale expressions

### After  
- Non-positional scales are converted to `ConfiguredScale` after Stage 1
- Channel resolver exclusively uses `ConfiguredScale.to_expr()`
- Radius calculations use fully-configured, optimized scales
- `Scale.to_expr()` is no longer used in the channel resolver

## Implementation Details

### Stage 1: Non-Positional Scale Processing
```rust
// Process non-positional scales (size, stroke_width, fill, etc.)
for scale in non_positional_scales {
    process_scale(scale, ...)
}

// NEW: Create ConfiguredScale objects immediately
let configured_non_positional = HashMap::new();
for (name, scale) in &non_positional_scales {
    let configured = scale.build(...).await?;
    configured_non_positional.insert(name, configured);
}
```

### Stage 2: Positional Scale Processing with Configured Scales
```rust
// Process positional scales using configured non-positional scales
for scale in positional_scales {
    process_scale(
        scale,
        &configured_non_positional,  // Pass configured scales for radius
        ...
    )
}
```

### Channel Resolver: Now ConfiguredScale-Only
```rust
fn create_channel_resolver<'a>(
    mark: &'a dyn Mark<C>,
    encodings: &'a IndexMap<String, ChannelValue>,
    configured_scales: &'a HashMap<String, ConfiguredScale>,  // ONLY configured scales
) -> impl Fn(&str) -> Expr + 'a {
    // Uses ConfiguredScale.to_expr() exclusively
    if let Some(configured) = configured_scales.get(&scale_key) {
        configured.to_expr(channel_value.expr().clone())
    } else {
        channel_value.expr().clone()  // No scale
    }
}
```

## Remaining Scale.to_expr() Usage

The only remaining usage of `Scale.to_expr()` is now clearly isolated:

| Location | Purpose | Can Migrate? |
|----------|---------|--------------|
| `plot.rs:691` | Domain inference for data expressions | No - happens before ConfiguredScale exists |

This remaining usage is legitimate and necessary for the domain inference phase.

## Benefits Realized

1. **Architectural Clarity**: Clear separation between configured and unconfigured scales
2. **Correctness**: Radius calculations use fully-configured scales with proper domains
3. **Performance**: No redundant scale processing or expression building
4. **Migration Path**: Sets foundation for potential future complete elimination

## Test Results

✅ All tests pass including:
- `test_multi_series_line_all_encodings` - Complex multi-encoding test
- `test_symbol_padding_*` - Radius-aware padding tests  
- `test_arrow_symbol_asymmetric_padding` - Asymmetric radius handling
- All 33 library unit tests

## Next Steps

1. **Document the staged processing model** in architecture docs
2. **Consider further optimization** - Could we cache ConfiguredScale objects?
3. **Explore complete elimination** - Could we restructure to eliminate the last Scale.to_expr()?

## Conclusion

This refactoring successfully demonstrates that most uses of `Scale.to_expr()` were unnecessary. By recognizing that non-positional scales are fully configured before being needed for radius calculations, we eliminated a major source of technical debt and improved the architecture's clarity.