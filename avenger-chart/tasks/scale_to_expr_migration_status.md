# Scale.to_expr() Migration Status

## Migration Complete ✅

We have successfully migrated most uses of `Scale.to_expr()` to `ConfiguredScale.to_expr()`:

### Migrated Components

1. **Mark Rendering Pipeline** (`render.rs`)
   - `render_mark()` now accepts `HashMap<String, ConfiguredScale>`
   - `apply_channel_scale()` uses `ConfiguredScale.to_expr()` extension method
   - Eliminated redundant scale building during rendering

2. **Axis Generation** (`coords.rs`)
   - `create_default_axes()` now accepts `ConfiguredScale`
   - `render_axes()` uses pre-built `ConfiguredScale` objects
   - Axes no longer create their own `ConfiguredScale` instances

## Remaining Usage (Cannot Migrate)

### Domain Inference Phase (`plot.rs:691`)

The `create_channel_resolver` method still uses `Scale.to_expr()` and **cannot be migrated** because:

1. **Timing**: Called during domain inference, BEFORE `ConfiguredScale` objects exist
2. **Purpose**: Builds query expressions to determine what data flows through scales
3. **Dependencies**: Required for radius-aware domain calculation

```rust
// plot.rs:create_channel_resolver
scale.to_expr(channel_value.expr().clone())  // Line 691
```

This usage is called from:
- `gather_scale_domain_expressions_with_radius()` 
- Which is called from `infer_scale_domain()`
- Which happens BEFORE scale configuration

## Architecture Summary

The dual scale system now has clear separation:

```
Domain Inference Phase (Scale.to_expr())
    ↓
    Gather data expressions
    Build query plans
    Determine domains
    ↓
Scale Configuration (Scale → ConfiguredScale)
    ↓
    Resolve domains to arrays
    Build ConfiguredScale objects
    ↓
Rendering Phase (ConfiguredScale.to_expr())
    ↓
    Render marks
    Generate axes
    Create legends
```

## Recommendation

**Remove the deprecation warning from `Scale.to_expr()`** and instead document that it should only be used during the domain inference phase. The method is necessary for building query expressions before data is available.

## Benefits Achieved

1. **Eliminated duplicate ConfiguredScale creation** - Axes now reuse the configured scales
2. **Consistent architecture** - Clear separation between inference and rendering phases
3. **Performance improvement** - No redundant scale building during rendering
4. **Cleaner code** - Render pipeline consistently uses ConfiguredScale

## Next Steps

1. Update `Scale.to_expr()` documentation to clarify its role in domain inference
2. Consider renaming to `to_inference_expr()` to make the purpose clearer
3. Add architectural documentation explaining the two-phase scale system